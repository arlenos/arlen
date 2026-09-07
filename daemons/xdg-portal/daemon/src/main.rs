//! xdg-desktop-portal-arlen daemon entry point.
//!
//! Registers `org.freedesktop.impl.portal.desktop.arlen` on the session
//! bus and serves the FileChooser, OpenURI, Screenshot, ScreenCast, Print and
//! Settings impl interfaces at `/org/freedesktop/portal/desktop`.
//!
//! Architecture decisions and edge-case handling live in
//! `docs/architecture/xdg-desktop-portal-arlen.md`. This file is the
//! plumbing: D-Bus bind, IPC server, picker pre-warm, idle loop.

mod document_portal;
mod interfaces;
mod consent;
mod picker_ipc;
mod picker_lifecycle;
mod print_ipc;
mod request;
mod sandbox;
mod sensing;
mod state;

use std::time::Duration;

use anyhow::Context;
use zbus::connection;

use crate::interfaces::settings::{Appearance, Settings};
use crate::interfaces::{file_chooser::FileChooser, open_uri::OpenUri, print::Print, screencast::ScreenCast, screenshot::Screenshot};
use crate::picker_ipc::PickerIpcHandle;
use crate::picker_lifecycle::PickerLifecycle;
use crate::state::DaemonState;

/// Well-known D-Bus name we register on the session bus. The
/// `xdg-desktop-portal` frontend dispatches to whichever backend is
/// declared in `arlen.portal` for `UseIn=arlen;`.
const BUS_NAME: &str = "org.freedesktop.impl.portal.desktop.arlen";

/// Object path the FileChooser and OpenURI interfaces are served at.
/// The frontend always queries this exact path.
const OBJECT_PATH: &str = "/org/freedesktop/portal/desktop";

/// Default idle window before the daemon exits when no requests are
/// open. Override via `ARLEN_PORTAL_IDLE_TIMEOUT_SECS` (handy for
/// dev sessions where 60 s would kick in mid-debug). See FA10 and
/// edge case E12 for the open-request-counter interaction.
const DEFAULT_IDLE_TIMEOUT_SECS: u64 = 60;

fn idle_timeout() -> Duration {
    let secs = std::env::var("ARLEN_PORTAL_IDLE_TIMEOUT_SECS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(DEFAULT_IDLE_TIMEOUT_SECS);
    Duration::from_secs(secs)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        // `warn` for everything, `info` for this backend. Every module is declared
        // here, so one directive covers the lot; the picker frontend beside it is a
        // separate crate with its own filter.
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                tracing_subscriber::EnvFilter::new("warn,xdg_desktop_portal_arlen=info")
            }),
        )
        .init();

    tracing::info!("starting xdg-desktop-portal-arlen");

    // Bring up the picker-IPC socket FIRST so the picker subprocess
    // we spawn next finds a server to connect to. Order matters: a
    // race where the picker connects before the listener exists
    // would surface as a "connection refused" inside the picker and
    // a needless respawn cycle.
    let picker_ipc = PickerIpcHandle::start()
        .await
        .context("start picker IPC")?;
    let picker_lifecycle = PickerLifecycle::new();

    // Pre-warm: spawn the picker-ui process now so the first
    // FileChooser call only pays the cost of a `.show()`, not the
    // ~300 ms WebKitGTK init (FA5, edge case E24). If the picker
    // binary is missing (devbox without a build), the spawn fails
    // and the FileChooser handler will surface a clear error to
    // callers when one finally arrives.
    if let Err(e) = picker_lifecycle.ensure_running().await {
        tracing::warn!(
            "picker-ui pre-warm failed: {e}. FileChooser calls will fail until the binary is available."
        );
    }

    let state = DaemonState::new(picker_ipc, picker_lifecycle);

    // The print dialog handback. Bound before the interfaces are served, so a
    // print that arrives in the first moments has somewhere to wait rather than
    // finding no socket and going ahead unattended.
    let print_dialog: crate::print_ipc::Shared = std::sync::Arc::new(
        tokio::sync::Mutex::new(crate::print_ipc::PendingPrints::default()),
    );
    let print_socket = xdg_portal_arlen_protocol::print::socket_path();
    match crate::print_ipc::bind(&print_socket) {
        Ok(listener) => {
            tracing::info!(socket = %print_socket.display(), "print dialog handback listening");
            tokio::spawn(crate::print_ipc::run(listener, print_dialog.clone()));
        }
        // Not fatal, and the consequence is stated rather than left to be
        // discovered: with no socket nobody can answer a print, so every print
        // waits out its timeout and is refused. That is the safe direction, and
        // the rest of the portal keeps working.
        Err(e) => tracing::error!(
            socket = %print_socket.display(),
            "cannot bind the print dialog handback, so printing will be refused: {e}"
        ),
    }

    let _conn = connection::Builder::session()
        .context("failed to connect to session bus")?
        .name(BUS_NAME)
        .with_context(|| format!("failed to claim D-Bus name {BUS_NAME}"))?
        .serve_at(OBJECT_PATH, FileChooser::new(state.clone()))
        .with_context(|| format!("failed to serve FileChooser at {OBJECT_PATH}"))?
        .serve_at(OBJECT_PATH, OpenUri::new(state.clone()))
        .with_context(|| format!("failed to serve OpenURI at {OBJECT_PATH}"))?
        .serve_at(OBJECT_PATH, Screenshot::new(state.clone()))
        .with_context(|| format!("failed to serve Screenshot at {OBJECT_PATH}"))?
        // ScreenCast is served on the bus but NOT yet listed in arlen.portal's
        // Interfaces line, so the frontend does not route ScreenCast here until
        // the PipeWire producer makes Start functional (capture-active #12).
        .serve_at(OBJECT_PATH, ScreenCast::new(state.clone()))
        .with_context(|| format!("failed to serve ScreenCast at {OBJECT_PATH}"))?
        .serve_at(OBJECT_PATH, Print::new(print_dialog.clone()))
        .with_context(|| format!("failed to serve Print at {OBJECT_PATH}"))?
        .serve_at(OBJECT_PATH, Settings::new())
        .with_context(|| format!("failed to serve Settings at {OBJECT_PATH}"))?
        .build()
        .await
        .context("failed to build D-Bus connection")?;

    tracing::info!(
        bus_name = BUS_NAME,
        path = OBJECT_PATH,
        "D-Bus interfaces ready"
    );

    // The appearance change signal. Every conforming toolkit reads dark/light
    // and the accent off the Settings interface once and then waits to be told,
    // so without this a foreign app keeps the appearance it started with until
    // it is restarted - which is the whole reason the interface has a signal.
    if let Err(e) = watch_appearance(&_conn) {
        tracing::warn!(
            "cannot watch the theme files, so foreign apps will not follow a theme change until they restart: {e}"
        );
    }

    // Idle-timeout loop: tick once per second and exit when no requests
    // have been open for IDLE_TIMEOUT. The state's request counter
    // protects against exit-while-pick-open (E12).
    let exit_signal = tokio::signal::ctrl_c();
    tokio::pin!(exit_signal);

    let timeout = idle_timeout();
    tracing::info!(idle_timeout_secs = timeout.as_secs(), "idle loop armed");
    let mut last_active = std::time::Instant::now();
    let mut picker_released = false;
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    loop {
        tokio::select! {
            _ = &mut exit_signal => {
                tracing::info!("received Ctrl-C, shutting down");
                break;
            }
            _ = tick.tick() => {
                if state.has_open_requests() {
                    last_active = std::time::Instant::now();
                    picker_released = false;
                    continue;
                }
                if last_active.elapsed() >= timeout && !picker_released {
                    // The idle timeout used to exit the process. It cannot any
                    // more: Settings is a push interface, and a backend that
                    // exits after a minute cannot tell anybody the theme
                    // changed - a foreign app would keep yesterday's colours
                    // until it restarted. What the timeout was actually paying
                    // for is the picker subprocess, which is the expensive part
                    // (a WebKitGTK view), so that is what gets released. The
                    // FileChooser handler calls `ensure_running` on the way in,
                    // so the next pick spawns a fresh one and pays the ~300 ms
                    // it was pre-warmed to avoid.
                    tracing::info!(
                        idle_for_secs = last_active.elapsed().as_secs(),
                        "idle: releasing the picker, staying up to serve Settings"
                    );
                    state.picker_lifecycle.shutdown().await;
                    picker_released = true;
                }
            }
        }
    }

    state.picker_lifecycle.shutdown().await;
    Ok(())
}

/// Emit `SettingChanged` when the person's theme files change.
///
/// The watcher fires several times for one save (editors write, rename and
/// touch), so a tick is a hint to re-read rather than a change in itself: the
/// task settles for a moment, resolves the appearance once, and emits only the
/// keys whose value actually moved. That matters because a signal per file event
/// would have every GTK and Qt app on the desktop re-theming three times for one
/// click of a switch.
fn watch_appearance(conn: &zbus::Connection) -> anyhow::Result<()> {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<()>();
    // The watcher owns its inotify thread and stops when it is dropped, so it is
    // moved into the task rather than left on the stack of a function that
    // returns immediately.
    let watcher = arlen_theme::ThemeWatcher::start(move || {
        let _ = tx.send(());
    })
    .context("watch the theme files")?;

    let conn = conn.clone();
    tokio::spawn(async move {
        let _watcher = watcher;
        let mut last = Appearance::current();
        while rx.recv().await.is_some() {
            // Settle: drain whatever else the one save produced.
            tokio::time::sleep(Duration::from_millis(150)).await;
            while rx.try_recv().is_ok() {}

            let now = Appearance::current();
            let changed = now.changed_keys(&last);
            if changed.is_empty() {
                continue;
            }
            last = now;
            emit_changed(&conn, &now, &changed).await;
        }
    });
    Ok(())
}

/// Push the named keys' new values out on the interface.
async fn emit_changed(conn: &zbus::Connection, now: &Appearance, keys: &[&'static str]) {
    let iface = match conn
        .object_server()
        .interface::<_, Settings>(OBJECT_PATH)
        .await
    {
        Ok(iface) => iface,
        Err(e) => {
            tracing::warn!("cannot reach the Settings interface to signal a change: {e}");
            return;
        }
    };
    for key in keys {
        let Some(value) = now.value_for(key) else {
            continue;
        };
        tracing::info!(key, "appearance changed, telling the frontend");
        if let Err(e) = Settings::setting_changed(
            iface.signal_emitter(),
            crate::interfaces::settings::NAMESPACE.to_string(),
            (*key).to_string(),
            value,
        )
        .await
        {
            tracing::warn!(key, "could not emit SettingChanged: {e}");
        }
    }
}

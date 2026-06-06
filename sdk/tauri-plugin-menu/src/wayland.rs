/// Wayland client connection for the titlebar protocol.
///
/// Connects to the compositor, binds `lunaris_titlebar_manager_v1`,
/// obtains a per-surface `lunaris_titlebar_v1` object, and dispatches
/// incoming events (mode_changed, tab_activated, etc.) as Tauri events.

use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Emitter, Runtime};
use wayland_client::{
    globals::{registry_queue_init, GlobalListContents},
    protocol::wl_registry,
    Connection, Dispatch, QueueHandle,
};

use crate::protocol::{lunaris_titlebar_manager_v1, lunaris_titlebar_v1};

/// Shared handle to the titlebar protocol object.
///
/// Commands write requests via this handle. The Wayland event loop
/// thread reads events and emits Tauri events.
pub struct TitlebarConnection {
    pub titlebar: Option<lunaris_titlebar_v1::LunarisTitlebarV1>,
    pub manager: Option<lunaris_titlebar_manager_v1::LunarisTitlebarManagerV1>,
    pub conn: Option<Connection>,
}

impl Default for TitlebarConnection {
    fn default() -> Self {
        Self {
            titlebar: None,
            manager: None,
            conn: None,
        }
    }
}

/// Plugin-managed shared state.
pub type SharedConnection = Arc<Mutex<TitlebarConnection>>;

/// Wayland dispatch state.
#[allow(dead_code)]
struct ClientData<R: Runtime> {
    app: AppHandle<R>,
    shared: SharedConnection,
    manager: Option<lunaris_titlebar_manager_v1::LunarisTitlebarManagerV1>,
}

/// Start the Wayland client thread.
///
/// Connects to the compositor, binds the titlebar manager global,
/// and enters the event dispatch loop. Events from the compositor
/// (mode_changed, tab_activated, etc.) are forwarded as Tauri events.
pub fn start<R: Runtime>(app: AppHandle<R>, shared: SharedConnection) {
    std::thread::Builder::new()
        .name("titlebar-wayland".into())
        .spawn(move || {
            if let Err(e) = run_client(app, shared) {
                log::error!("titlebar-wayland: client thread failed: {e}");
            }
        })
        .expect("failed to spawn titlebar-wayland thread");
}

fn run_client<R: Runtime>(
    app: AppHandle<R>,
    shared: SharedConnection,
) -> Result<(), Box<dyn std::error::Error>> {
    let conn = loop {
        match Connection::connect_to_env() {
            Ok(c) => break c,
            Err(e) => {
                log::debug!("titlebar-wayland: not ready, retrying in 1s: {e}");
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
        }
    };

    let (globals, mut event_queue) = registry_queue_init::<ClientData<R>>(&conn)?;
    let qh = event_queue.handle();

    // Bind the titlebar manager global.
    let manager = globals
        .bind::<lunaris_titlebar_manager_v1::LunarisTitlebarManagerV1, _, _>(
            &qh,
            1..=1,
            (),
        )
        .ok();

    if manager.is_none() {
        log::warn!("titlebar-wayland: lunaris_titlebar_manager_v1 not available");
    } else {
        log::info!("titlebar-wayland: titlebar manager bound");
    }

    // Store connection and manager in shared state.
    {
        let mut lock = shared.lock().unwrap();
        lock.conn = Some(conn);
        lock.manager = manager.clone();
    }

    let mut data = ClientData {
        app,
        shared,
        manager,
    };

    loop {
        if let Err(e) = event_queue.blocking_dispatch(&mut data) {
            log::error!("titlebar-wayland: dispatch error: {e}");
            return Err(e.into());
        }
    }
}

// ── Registry dispatch ────────────────────────────────────────────────────────

impl<R: Runtime> Dispatch<wl_registry::WlRegistry, GlobalListContents> for ClientData<R> {
    fn event(
        _state: &mut Self,
        _proxy: &wl_registry::WlRegistry,
        _event: wl_registry::Event,
        _data: &GlobalListContents,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        // Registry events handled by GlobalList internals.
    }
}

// ── Manager dispatch ─────────────────────────────────────────────────────────

impl<R: Runtime> Dispatch<lunaris_titlebar_manager_v1::LunarisTitlebarManagerV1, ()>
    for ClientData<R>
{
    fn event(
        _state: &mut Self,
        _proxy: &lunaris_titlebar_manager_v1::LunarisTitlebarManagerV1,
        _event: lunaris_titlebar_manager_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        // Manager has no events.
    }
}

// ── Per-surface titlebar dispatch ────────────────────────────────────────────

impl<R: Runtime> Dispatch<lunaris_titlebar_v1::LunarisTitlebarV1, ()> for ClientData<R> {
    fn event(
        state: &mut Self,
        _proxy: &lunaris_titlebar_v1::LunarisTitlebarV1,
        event: lunaris_titlebar_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            lunaris_titlebar_v1::Event::ModeChanged { mode } => {
                let mode_str = match mode.into_result() {
                    Ok(lunaris_titlebar_v1::Mode::Floating) => "floating",
                    Ok(lunaris_titlebar_v1::Mode::Tiled) => "tiled",
                    Ok(lunaris_titlebar_v1::Mode::Fullscreen) => "fullscreen",
                    Ok(lunaris_titlebar_v1::Mode::Frameless) => "frameless",
                    _ => "unknown",
                };
                let _ = state.app.emit("lunaris-titlebar://mode-changed", mode_str);
            }
            lunaris_titlebar_v1::Event::TabActivated { id } => {
                let _ = state.app.emit("lunaris-titlebar://tab-activated", &id);
            }
            lunaris_titlebar_v1::Event::TabClosed { id } => {
                let _ = state.app.emit("lunaris-titlebar://tab-closed", &id);
            }
            lunaris_titlebar_v1::Event::TabReordered { ids_json } => {
                let _ = state.app.emit("lunaris-titlebar://tab-reordered", &ids_json);
            }
            lunaris_titlebar_v1::Event::ButtonClicked { id } => {
                let _ = state.app.emit("lunaris-titlebar://button-clicked", &id);
            }
            lunaris_titlebar_v1::Event::BreadcrumbClicked { index, action } => {
                let _ = state.app.emit(
                    "lunaris-titlebar://breadcrumb-clicked",
                    serde_json::json!({ "index": index, "action": action }),
                );
            }
            lunaris_titlebar_v1::Event::SearchChanged { query } => {
                let _ = state.app.emit("lunaris-titlebar://search-changed", &query);
            }
            lunaris_titlebar_v1::Event::KeyboardAction { action } => {
                let _ = state.app.emit("lunaris-titlebar://keyboard-action", &action);
            }
        }
    }
}

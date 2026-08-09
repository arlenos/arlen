//! The launch socket: the one place a request to start something becomes a
//! process.
//!
//! The portal used to shell out to `xdg-open`, the file manager spawned the
//! `Exec` its picker chose, and the launcher had its own copy of the
//! confinement decision - three components each holding a third of the problem
//! and none holding the part that mattered. This serves all of them:
//! `$XDG_RUNTIME_DIR/arlen/launch.sock` takes an app id or a document, resolves
//! it, records who asked, and starts it the one confined way.
//!
//! The decisions live in `arlen_desktop_shell_core::launch` and are unit-tested
//! there. This is the shell around them: bind, attest the peer, read the files,
//! submit the record, spawn.

use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use arlen_desktop_shell_core::launch::{
    plan::Launch,
    search::{self, XdgEnv},
    service::{self, Caller, Served},
};
use arlen_launch_contract as proto;
use arlen_permissions::ConnectionAuth;
use audit_proto::sink::{AuditSink, LedgerAuditSink};
use tokio::net::{UnixListener, UnixStream};

/// Start serving launch requests, if the socket can be had.
///
/// A shell that cannot bind still has to come up - the bar, the launcher and
/// everything else do not depend on this - so a failure is logged and the
/// service is simply absent. Callers get a connection refusal, which is a
/// legible "the service is not there" rather than a hang.
///
/// This is called from Tauri's setup hook, which is NOT inside a tokio runtime.
/// Binding with `tokio::net::UnixListener` panicked on "there is no reactor
/// running" every time the shell started - on the main thread, in a function
/// whose whole contract is that it degrades quietly - and the bare `tokio::spawn`
/// underneath it would have panicked next for the same reason. The three sibling
/// IPC services in the same hook (clipboard, intent, search) all go through
/// `tauri::async_runtime::spawn`, which is the runtime this process has; this one
/// had grown its own spelling.
///
/// The socket is bound here rather than inside the task, so that when `setup`
/// returns the socket exists. A caller that connects the moment the shell is up
/// then gets served rather than refused, which the siblings do not guarantee.
pub fn spawn_launch_service() {
    let path = proto::socket_path();
    let listener = match bind(&path) {
        Ok(l) => l,
        Err(e) => {
            log::error!("launch service not started: {e}");
            return;
        }
    };
    if let Err(e) = listener.set_nonblocking(true) {
        log::error!("launch service not started: {e}");
        return;
    }
    log::info!("launch service listening on {}", path.display());
    tauri::async_runtime::spawn(async move {
        // Inside the runtime, so this is the first line that may touch tokio.
        let listener = match UnixListener::from_std(listener) {
            Ok(l) => l,
            Err(e) => {
                log::error!("launch service not started: {e}");
                return;
            }
        };
        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    tauri::async_runtime::spawn(async move {
                        if let Err(e) = handle(stream).await {
                            log::warn!("launch request: {e}");
                        }
                    });
                }
                Err(e) => {
                    log::error!("launch service accept failed, stopping: {e}");
                    return;
                }
            }
        }
    });
}

/// Bind the socket, owner-only.
///
/// A path that is already served by a live process is an error rather than
/// something to take over: two shells answering one socket would hand the same
/// request two different answers. A stale file left by a dead one is cleared,
/// which is the ordinary case after a crash.
fn bind(path: &Path) -> std::io::Result<std::os::unix::net::UnixListener> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if path.exists() {
        if std::os::unix::net::UnixStream::connect(path).is_ok() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AddrInUse,
                format!("{} is already served by a live process", path.display()),
            ));
        }
        let _ = std::fs::remove_file(path);
    }
    let listener = std::os::unix::net::UnixListener::bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}

/// Serve one request.
async fn handle(mut stream: UnixStream) -> Result<(), String> {
    let caller = attest(&stream);
    let request = proto::read_request(&mut stream)
        .await
        .map_err(|e| e.to_string())?;

    let env = XdgEnv::from_process();
    let handlers = search::load_mimeapps(&env);
    let confined = crate::shell_config::get_shell_config()
        .map(|c| c.launcher.confined)
        .unwrap_or(false);

    let served = service::serve(
        &request,
        &caller,
        &handlers,
        |id| search::load_entry(&env, id),
        mime_of,
        confined,
        // Per-app confinement, same rule and same source as the launcher's:
        // `profile_paths` lists every path the loader consults, so an app the
        // system holds no profile for is not routed through `arlen-run` and
        // cannot be stopped by turning the flag on.
        |id| {
            arlen_permissions::profile_paths(id)
                .iter()
                .any(|p| p.exists())
        },
    );

    // Before the act, not after: a record written afterwards is a record that a
    // crash between the two loses, and the one question this socket exists to
    // answer is what caused a program to start.
    record(&served).await;

    if let Some(launch) = &served.launch {
        spawn(launch)?;
    }
    proto::write_outcome(&mut stream, &served.outcome)
        .await
        .map_err(|e| e.to_string())
}

/// What kind of thing a document is, when the caller did not say.
///
/// `xdg-mime` rather than a MIME library: it is what the rest of the tree
/// already asks (the harness's file pills, the file manager's Open-With), so a
/// user who has taught their system that a file is one thing gets one answer
/// everywhere rather than two. A remote target has no local path to classify,
/// and a classification nobody could make is `None` - which the service reports
/// as nothing opening it, because from the requester's side that is the same
/// fact.
fn mime_of(target: &proto::Target) -> Option<String> {
    let path = target.path.as_ref()?;
    let out = std::process::Command::new("xdg-mime")
        .args(["query", "filetype", path])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let mime = String::from_utf8(out.stdout).ok()?;
    let mime = mime.trim();
    (!mime.is_empty()).then(|| mime.to_string())
}

/// Who is on the other end, as the kernel says rather than as they claim.
///
/// A peer whose binary the identity resolver does not recognise is `Unnamed`,
/// not refused: most of what a session runs is not a packaged application, and
/// opening a document grants nothing that would justify turning that into a
/// failure. The distinction is recorded rather than smoothed over.
fn attest(stream: &UnixStream) -> Caller {
    // Safe: `getuid` cannot fail and touches no shared state.
    let uid = unsafe { libc::getuid() };
    match ConnectionAuth::extract_from(stream, uid) {
        Ok(auth) => match auth.verify_alive() {
            Ok(()) => Caller::Named(auth.app_id().to_string()),
            // The peer went away between connecting and being asked about, so
            // the name we would record might already belong to someone else.
            Err(_) => Caller::Unnamed,
        },
        Err(_) => Caller::Unnamed,
    }
}

/// Submit the ledger record.
///
/// Best effort **for now**, and deliberately not silent: a launch grants the
/// caller nothing it could not do by spawning the program itself, so refusing
/// to open a document because the audit daemon is down would cost the user a
/// working desktop to protect a record of something they could have done
/// anyway. That reasoning expires with the confinement flip, when this socket
/// becomes a confined application's only way to start anything and the record
/// becomes the account of a real authority - at which point this fails closed,
/// in one place, like `service::admits`.
async fn record(served: &Served) {
    let sink = LedgerAuditSink::at_default_socket();

    // The daemon is back and something was lost while it was away: say so in the
    // ledger before the entry that proves it is back, so a reader meets the gap
    // in the order it happened. Left pending if this fails too - a gap that
    // cannot be written is not a gap that stops existing.
    if let Some(gap) = take_gap() {
        let marker = service::unrecorded_gap_event(gap.dropped, gap.span_ms());
        if sink.submit(marker).await.is_err() {
            restore_gap(gap);
        }
    }

    let event = service::launch_event(&served.audit);
    if let Err(e) = sink.submit(event).await {
        let dropped = note_drop();
        log::warn!(
            "launch not recorded ({}): caller {} outcome {} ({} unrecorded so far)",
            e,
            served.audit.caller,
            served.audit.outcome,
            dropped
        );
    }
}

/// Launches that went unrecorded, and when the first of them was.
#[derive(Clone, Copy)]
struct Gap {
    dropped: u64,
    since: std::time::Instant,
}

impl Gap {
    fn span_ms(&self) -> u64 {
        u64::try_from(self.since.elapsed().as_millis()).unwrap_or(u64::MAX)
    }
}

/// The pending gap, if the ledger is currently missing anything.
///
/// A process-global because there is one launch service per shell and one
/// ledger behind it; threading it through every request would be ceremony
/// around a single fact about the process.
static GAP: std::sync::Mutex<Option<Gap>> = std::sync::Mutex::new(None);

/// Count one unrecorded launch. Returns the running total.
fn note_drop() -> u64 {
    let mut guard = GAP.lock().unwrap_or_else(|e| e.into_inner());
    let gap = guard.get_or_insert(Gap {
        dropped: 0,
        since: std::time::Instant::now(),
    });
    gap.dropped += 1;
    gap.dropped
}

/// Take the pending gap, if there is one.
fn take_gap() -> Option<Gap> {
    GAP.lock().unwrap_or_else(|e| e.into_inner()).take()
}

/// Put a gap back after failing to record it, keeping the earlier start time and
/// any drops counted while the marker was in flight.
fn restore_gap(gap: Gap) {
    let mut guard = GAP.lock().unwrap_or_else(|e| e.into_inner());
    match guard.as_mut() {
        Some(current) => {
            current.dropped += gap.dropped;
            current.since = current.since.min(gap.since);
        }
        None => *guard = Some(gap),
    }
}

/// Start the planned argv, detached.
///
/// No shell: the argv came from the desktop-entry rules, and putting it back
/// through one would re-interpret a file name that has already been decided to
/// be data.
fn spawn(launch: &Launch) -> Result<(), String> {
    let argv = launch.argv();
    let (program, args) = argv.split_first().ok_or("nothing to run")?;
    std::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("spawn {program}: {e}"))?;
    Ok(())
}

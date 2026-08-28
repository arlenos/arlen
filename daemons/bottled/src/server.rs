//! The bottle daemon's Unix-socket server.
//!
//! WHY A DAEMON AND NOT A LIBRARY the Settings backend links: a launched Windows
//! program has to outlive the window that started it. A bottle whose supervising
//! process is the panel dies when somebody closes the panel, which is the one
//! thing a "runtime" must not do (`windows-apps-plan.md`).
//!
//! Per connection: authenticate the peer (`SO_PEERPIDFD` + uid), then field
//! requests until the peer closes or stops being alive. An auth failure drops the
//! connection without a word - a credential lookup that did not cleanly succeed
//! never serves.
//!
//! SAME-UID READS, no allowlist. The vocabulary here is read-only and the bottles
//! are the person's own; the config broker draws the same line, restricting only
//! its writers. When a mutating ask lands - create, forget, revoke a drive - it
//! needs its own admission and its own audit, and this note is where that starts.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::net::{UnixListener, UnixStream};

use arlen_permissions::peer_pidfd::PeerPidfd;

use crate::protocol::{
    create, forget, handle_request, install, launch, Problem, Request, Response,
};
use audit_proto::{AuditKind, AuditSink, IngestRequest, StructuralRecord};

/// The largest accepted frame body. A bottle list is a handful of short strings
/// per bottle, so 64 KiB is generous; a larger declared length is refused before
/// anything is allocated for it.
pub const MAX_FRAME: usize = 64 * 1024;

/// The daemon's socket: the `ARLEN_BOTTLED_SOCKET` override, else
/// `$XDG_RUNTIME_DIR/arlen/bottled.sock`, else `/run/arlen/bottled.sock`.
pub fn socket_path() -> PathBuf {
    if let Some(p) = std::env::var_os("ARLEN_BOTTLED_SOCKET") {
        return PathBuf::from(p);
    }
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/run"));
    base.join("arlen").join("bottled.sock")
}

/// Read one length-prefixed JSON frame, refusing an over-long one before
/// allocating for it.
pub async fn read_frame<R, T>(reader: &mut R) -> std::io::Result<T>
where
    R: tokio::io::AsyncRead + Unpin,
    T: for<'de> Deserialize<'de>,
{
    use tokio::io::AsyncReadExt;
    let mut len_buf = [0u8; 4];
    reader.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > MAX_FRAME {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "frame exceeds MAX_FRAME",
        ));
    }
    let mut body = vec![0u8; len];
    reader.read_exact(&mut body).await?;
    serde_json::from_slice(&body)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

/// Write one length-prefixed JSON frame.
pub async fn write_frame<W, T>(writer: &mut W, value: &T) -> std::io::Result<()>
where
    W: tokio::io::AsyncWrite + Unpin,
    T: Serialize,
{
    use tokio::io::AsyncWriteExt;
    let body = serde_json::to_vec(value)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    if body.len() > MAX_FRAME {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "frame exceeds MAX_FRAME",
        ));
    }
    writer.write_all(&(body.len() as u32).to_be_bytes()).await?;
    writer.write_all(&body).await?;
    writer.flush().await
}

/// App ids permitted to FORGET a bottle.
///
/// The reading vocabulary is open to any same-uid caller - the bottles are the
/// person's own and a read tells them nothing they could not read off the disk.
/// Forgetting is different: it throws away what somebody installed, so it is not
/// something every process that can reach this socket gets to ask for. The Settings
/// panel is the surface that offers it, and it is the only id here.
///
/// EXACT, never a prefix: a prefix match would admit every locally-built binary to
/// the one operation this gate exists for. The `dev.arlen-settings` spelling is the
/// cargo-run id and is admitted in debug only, the convention the capsule daemon
/// and the audit ingest both follow.
fn forget_caller_admitted(app_id: &str) -> bool {
    if app_id == "dev.arlen.settings" {
        return true;
    }
    #[cfg(debug_assertions)]
    if app_id == "dev.arlen-settings" {
        return true;
    }
    false
}

/// Field requests on one connection until the peer closes or dies.
pub async fn serve_connection(
    mut stream: UnixStream,
    bottles_dir: &Path,
    caller_uid: u32,
    audit: Arc<dyn AuditSink>,
) {
    // Read once per connection rather than per launch: they do not change while a
    // session runs, and a launch that asked the environment again could start a
    // program against a display the rest of the session is not using.
    let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/run"));
    let display = std::env::var("DISPLAY").ok();
    let peer = match PeerPidfd::from_socket(&stream, caller_uid) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("peer auth refused: {e}");
            return;
        }
    };
    // Resolved once, from the pinned pid. A caller whose id cannot be read is not
    // refused the connection - the reads are open to any same-uid peer - but it
    // will not pass the forget gate, which is the fail-closed direction.
    let app_id = arlen_permissions::identity::app_id_from_pid(peer.pid()).ok();
    loop {
        // Re-checked per request rather than once: a pid that has been recycled
        // must not inherit the session the original process opened.
        if !peer.is_alive() {
            tracing::warn!("peer no longer alive; dropping");
            return;
        }
        let request: Request = match read_frame(&mut stream).await {
            Ok(r) => r,
            // A closed connection or a frame this cannot read ends the session.
            Err(_) => return,
        };
        // The asks the pure dispatch cannot answer, because each needs something
        // outside the bottles directory: the host a program will run on, a trash
        // and a caller allowed to ask, or the runtime itself to be run. They come
        // back here rather than each writing its own reply - one write path, so a
        // fourth of them cannot quietly grow a different one.
        let response = match handle_request(bottles_dir, &request) {
            Some(r) => r,
            None => match &request {
                Request::Runtimes => runtimes().await,
                Request::Forget { id } => {
                    forget_for(bottles_dir, id, app_id.as_deref(), &*audit).await
                }
                Request::Create { id } => create(
                    bottles_dir,
                    id,
                    std::path::Path::new("/usr"),
                    &runtime_dir,
                    |p| p.exists(),
                    run_to_completion,
                ),
                Request::Install { id, installer } => install(
                    bottles_dir,
                    id,
                    installer,
                    std::path::Path::new("/usr"),
                    &runtime_dir,
                    display.as_deref(),
                    |p| p.exists(),
                    spawn_detached,
                ),
                Request::Launch { id } => launch(
                    bottles_dir,
                    id,
                    std::path::Path::new("/usr"),
                    &runtime_dir,
                    display.as_deref(),
                    |p| p.exists(),
                    spawn_detached,
                ),
                // NAMED rather than a catch-all, so the compiler is the thing that
                // notices. `handle_request` is exhaustive, so a new ask cannot be
                // forgotten there; this match had an `other` arm, so a variant that
                // returned `None` and was never routed compiled cleanly and dropped
                // every connection asking for it - at runtime, on a machine, with a
                // log line nobody was reading. Listing them means forgetting either
                // half is a build error.
                Request::ListBottles
                | Request::Health { .. }
                | Request::Prefix { .. }
                | Request::ClearCaches { .. }
                | Request::Programs { .. }
                | Request::SetProgram { .. } => {
                    // Unreachable: `handle_request` answers all of these. Kept as an
                    // arm rather than an `unreachable!()` so a daemon never panics on
                    // a reachable-in-future path.
                    tracing::error!(?request, "an ask that should have been answered was not");
                    return;
                }
            },
        };
        if write_frame(&mut stream, &response).await.is_err() {
            return;
        }
    }
}

/// Bind the socket 0600 and serve until the future is dropped.
pub async fn run(
    socket: &Path,
    bottles_dir: PathBuf,
    audit: Arc<dyn AuditSink>,
) -> std::io::Result<()> {
    if let Some(parent) = socket.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // A stale socket from a killed run would otherwise make the bind fail and the
    // daemon look broken on every restart.
    let _ = std::fs::remove_file(socket);
    let listener = UnixListener::bind(socket)?;
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o600))?;
    }
    let uid = current_uid();
    tracing::info!(socket = %socket.display(), "bottled listening");
    loop {
        let (stream, _) = listener.accept().await?;
        let dir = bottles_dir.clone();
        let sink = Arc::clone(&audit);
        tokio::spawn(async move {
            serve_connection(stream, &dir, uid, sink).await;
        });
    }
}

/// What this machine can run Windows programs with, by asking it.
///
/// ASKED, not looked up: a version string that comes from running `wine
/// --version` is the version that will actually run, where a path check would
/// only say a file is there. A machine with no Wine answers `None`, which is the
/// answer, and it takes a bounded time to say so.
async fn runtimes() -> Response {
    let wine = match tokio::time::timeout(
        std::time::Duration::from_secs(5),
        tokio::process::Command::new(crate::launch::WINE)
            .arg("--version")
            .output(),
    )
    .await
    {
        Ok(Ok(out)) if out.status.success() => {
            let v = String::from_utf8_lossy(&out.stdout).trim().to_string();
            // An empty answer from a program that exited cleanly is not a version.
            (!v.is_empty()).then_some(v)
        }
        // Absent, broken, or too slow to say. All three mean the same thing to
        // somebody wondering whether a Windows program can run here.
        _ => None,
    };
    Response::Runtimes { wine }
}

/// Forget a bottle, if the caller may and the ledger recorded that it happened.
///
/// AUDITED BEFORE THE ACT, and refused when the sink is down. This is the one ask
/// here that throws away somebody's files; a copy of it that happened with no
/// record is exactly what the ledger exists to make impossible. The read asks are
/// not audited, because reading a bottle list tells nobody anything they could not
/// read off their own disk.
async fn forget_for(
    bottles_dir: &Path,
    id: &str,
    app_id: Option<&str>,
    audit: &dyn AuditSink,
) -> Response {
    if !app_id.is_some_and(forget_caller_admitted) {
        tracing::warn!(app_id = app_id.unwrap_or("unresolved"), "refused a forget");
        // Best-effort: a refusal is worth recording, and the answer is the refusal
        // whether or not the ledger took it.
        let _ = audit.submit(forget_event("refused", id)).await;
        return Response::Refused {
            problem: Problem::NotAllowed,
        };
    }
    if audit.submit(forget_event("forgetting", id)).await.is_err() {
        tracing::warn!("audit unavailable, so nothing was forgotten");
        return Response::Refused {
            problem: Problem::CouldNotForget,
        };
    }
    forget(bottles_dir, id, |p| {
        arlen_freedesktop_trash::trash_for_current_user(&p.to_string_lossy())
            .map(|slot| PathBuf::from(slot.trashed().as_str()))
            .map_err(|e| format!("{e:?}"))
    })
}

/// The ledger entry for a forget attempt.
///
/// The bottle id travels; what was inside it does not. An id is what the person
/// named the thing, which is the least a record can carry and still be a record of
/// something.
fn forget_event(outcome: &str, id: &str) -> IngestRequest {
    IngestRequest {
        kind: AuditKind::Permission,
        structural: StructuralRecord {
            subject: format!("bottle.forget:{id}"),
            node_types: Vec::new(),
            relations: Vec::new(),
            result_count: None,
            duration_ms: None,
            outcome: outcome.to_string(),
            depth: None,
            capability_change: None,
        },
        forensic: None,
        call_chain_id: None,
        project_id: None,
    }
}

/// Start a confinement and answer with its pid, without waiting for it.
///
/// NOT WAITED ON, which is the point of the daemon: the Windows program outlives
/// the window that asked for it. It is reaped rather than left a zombie - a task
/// waits on it and throws the status away - and it still dies with this daemon,
/// because the shared confiner passes `--die-with-parent`. That is the lifetime a
/// desktop app should have: longer than the panel, no longer than the session.
fn spawn_detached(argv: &[String]) -> std::io::Result<u32> {
    let mut child = tokio::process::Command::new("bwrap")
        .args(argv)
        .kill_on_drop(false)
        .spawn()?;
    let pid = child.id().unwrap_or(0);
    tokio::spawn(async move {
        let _ = child.wait().await;
    });
    Ok(pid)
}

/// Run a confined argv and wait for it to finish.
///
/// WAITS, unlike [`spawn_detached`], and the difference is what the two are for. A
/// launch hands a program to the person and gets out of the way; a boot is a step
/// inside making a bottle, and the steps after it - severing the links, writing
/// the drive table - are only correct once Wine has finished writing the prefix.
///
/// Blocking the connection for the seconds a boot takes is deliberate: the caller
/// asked for a bottle and there is nothing to answer until there is one. Other
/// connections are unaffected, since each is its own task.
fn run_to_completion(argv: &[String]) -> Result<(), String> {
    let status = std::process::Command::new("bwrap")
        .args(argv)
        .status()
        .map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        // The code rather than a sentence: this is one half of a token the window
        // will render, and Wine's own output has already gone to the journal.
        Err(format!("wineboot exited {status}"))
    }
}

/// This process's uid, which every admitted peer must share.
fn current_uid() -> u32 {
    // SAFETY: `getuid` reads a process property and cannot fail.
    unsafe { libc::getuid() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_panel_may_forget_a_bottle() {
        assert!(forget_caller_admitted("dev.arlen.settings"));
        // Everything else, including the assistant and this daemon's own siblings.
        for other in [
            "ai-agent",
            "ai-daemon",
            "knowledge",
            "bottled",
            "dev.arlen.harness",
            "com.example.app",
            "",
        ] {
            assert!(
                !forget_caller_admitted(other),
                "{other} must not be able to throw away somebody's installed program"
            );
        }
        // The cargo-run spelling is a debug convenience and must never be a
        // release one.
        assert_eq!(
            forget_caller_admitted("dev.arlen-settings"),
            cfg!(debug_assertions)
        );
    }

    #[tokio::test]
    async fn a_frame_survives_the_round_trip_and_an_over_long_one_is_refused() {
        let (mut a, mut b) = tokio::net::UnixStream::pair().unwrap();
        write_frame(&mut a, &Request::ListBottles).await.unwrap();
        let back: Request = read_frame(&mut b).await.unwrap();
        assert_eq!(back, Request::ListBottles);

        // A declared length beyond the cap is refused before the body is read, so
        // a caller cannot make this process allocate on its say-so.
        use tokio::io::AsyncWriteExt;
        a.write_all(&((MAX_FRAME as u32) + 1).to_be_bytes())
            .await
            .unwrap();
        assert!(read_frame::<_, Request>(&mut b).await.is_err());
    }
}

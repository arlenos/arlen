//! The sentinel's socket: one question per connection, same-uid callers only.
//!
//! The switches are the person's own and so is the readout, so the line here is
//! the same one the config broker draws: any peer running as this user may ask,
//! and nobody else may reach the socket at all (mode 0600 plus a credential check
//! that has to succeed cleanly before a byte is read).
//!
//! WHAT GETS RECORDED, and it is deliberately not everything. Turning a
//! protection OFF goes in the audit ledger and is REFUSED if the ledger cannot
//! take it: a machine whose privacy detectors were switched off with no record is
//! the state somebody would want to arrange quietly, and refusing leaves the
//! detector running, which is the safe side of that failure. Turning one back on
//! is recorded best-effort - a protective act that a down ledger must not block.
//! What the detectors SEE is never recorded here at all: §5 is explicit that
//! logging ambient observations would manufacture the location history this
//! feature exists to avoid.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use arlen_permissions::peer_pidfd::PeerPidfd;
use audit_proto::{AuditKind, AuditSink, IngestRequest, StructuralRecord};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};

use crate::config::{self, Config, Detector};
use crate::host;
use crate::protocol::{
    posture_wire, readout_incomplete, Detectors, Request, Response, State,
};
use crate::read;

/// The largest accepted frame body. A request is a few short strings and the
/// answer is six posture lines, so 64 KiB is generous; a larger declared length
/// is refused before anything is allocated for it.
pub const MAX_FRAME: usize = 64 * 1024;

/// The daemon's socket: the `ARLEN_SENTINEL_SOCKET` override, else
/// `$XDG_RUNTIME_DIR/arlen/sentinel.sock`, else `/run/arlen/sentinel.sock`.
pub fn socket_path() -> PathBuf {
    if let Some(p) = std::env::var_os("ARLEN_SENTINEL_SOCKET") {
        return PathBuf::from(p);
    }
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/run"));
    base.join("arlen").join("sentinel.sock")
}

/// Bind the socket, replacing one a previous run left behind.
///
/// Only an existing SOCKET is removed. A regular file or a symlink at that path
/// is something else's, and unlinking it would make this daemon a way to delete
/// a file it was pointed at.
pub fn bind(path: &Path) -> std::io::Result<UnixListener> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if let Ok(meta) = std::fs::symlink_metadata(path) {
        use std::os::unix::fs::FileTypeExt;
        if meta.file_type().is_socket() {
            let _ = std::fs::remove_file(path);
        }
    }
    let listener = UnixListener::bind(path)?;
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    Ok(listener)
}

/// This process's uid, which every admitted peer must share.
fn current_uid() -> u32 {
    // Safe: `getuid` reads a process property and cannot fail.
    unsafe { libc::getuid() }
}

/// The ledger entry for a detector switch.
///
/// Carries which detector and which way, and nothing about what it saw. That is
/// the whole record: the ledger says what Arlen did to your machine, never what
/// was around it.
fn switch_event(detector: Detector, on: bool) -> IngestRequest {
    IngestRequest {
        kind: AuditKind::Permission,
        structural: StructuralRecord {
            subject: format!("sentinel.detector:{}", detector.as_str()),
            node_types: Vec::new(),
            relations: Vec::new(),
            result_count: None,
            duration_ms: None,
            outcome: if on { "enabled" } else { "disabled" }.to_string(),
            depth: None,
            capability_change: None,
        },
        forensic: None,
        call_chain_id: None,
        project_id: None,
    }
}

/// Everything one connection needs.
pub struct Context {
    /// Where the switches live.
    pub config_path: PathBuf,
    /// The ledger a switch is recorded in.
    pub audit: Arc<dyn AuditSink>,
}

/// Read the current state: the switches, and the exposure readout when that
/// detector is running.
///
/// A detector that is off produces no lines. Showing the last readout of a
/// detector nobody is running would be a page reporting on a watch that stopped.
pub async fn current_state(cfg: &Config) -> State {
    let (posture, incomplete) = if cfg.exposure.on {
        let readings = host::read_host().await;
        let lines = arlen_sentinel_detect::readout::compose(&read::postures(&readings));
        let incomplete = readout_incomplete(&lines);
        (posture_wire(&lines), incomplete)
    } else {
        (Vec::new(), false)
    };
    State {
        detectors: Detectors::from(cfg),
        posture,
        // NOT measured, and that is why it is an absence rather than `false`.
        // There is no microphone or camera portal in this build (the sensing
        // switches say so themselves), so nothing can answer whether something is
        // capturing right now. "Nothing is using your microphone" is the sharpest
        // sentence on this page and the one it must never say on no evidence.
        capture_active: None,
        // The tracker's coarse location grant is minted through the broker and
        // nothing has minted one, so this is a measured false rather than a
        // placeholder: the card correctly offers to ask for it.
        tracker_has_location: false,
        posture_incomplete: incomplete.then_some(true),
    }
}

/// Answer one request.
pub async fn handle(ctx: &Context, request: Request) -> Response {
    let (mut cfg, problem) = config::load(&ctx.config_path);
    if let Some(problem) = problem {
        tracing::warn!("{problem}");
    }
    match request {
        Request::GetState => Response::State(Box::new(current_state(&cfg).await)),
        Request::SetDetector { id, on } => {
            let Some(detector) = Detector::parse(&id) else {
                return Response::Refused {
                    message: config::Refused::NoSuchDetector(id).message(),
                };
            };
            // Turning a protection off is the act worth a record, so it is the
            // one that waits for the ledger.
            let recorded = ctx.audit.submit(switch_event(detector, on)).await;
            if !on && recorded.is_err() {
                return Response::Failed {
                    message: "The change was not made: it could not be written to the activity \
                              log, and turning a protection off without a record is not something \
                              this will do quietly."
                        .to_string(),
                };
            }
            apply(ctx, &mut cfg, |c| config::set_detector(c, &id, on))
        }
        Request::SetAlerts { id, mode } => {
            apply(ctx, &mut cfg, |c| config::set_alerts(c, &id, &mode))
        }
        Request::SetSensitivity { id, level } => {
            apply(ctx, &mut cfg, |c| config::set_sensitivity(c, &id, &level))
        }
        Request::FixPosture { surface } => fix(&surface).await,
    }
}

/// Run a change against the config and write it back.
fn apply(
    ctx: &Context,
    cfg: &mut Config,
    change: impl FnOnce(&mut Config) -> Result<(), config::Refused>,
) -> Response {
    if let Err(refused) = change(cfg) {
        return Response::Refused {
            message: refused.message(),
        };
    }
    match config::save(&ctx.config_path, cfg) {
        Ok(()) => Response::Done,
        Err(e) => Response::Failed {
            message: format!(
                "The change was not saved, so nothing about your detectors has changed: {e}"
            ),
        },
    }
}

/// Apply the one-click remediation for a surface.
///
/// Only the surfaces `readout::compose` marks fixable are acted on. The rest have
/// real remediations that lose something - a stored credential, a running service
/// - and those belong behind their own confirmation rather than one tap.
async fn fix(surface: &str) -> Response {
    let outcome = match surface {
        "bluetooth_discoverable" => host::stop_being_discoverable().await,
        "wifi_mac" | "saved_mac_policy" => host::randomize_saved_macs().await,
        other => {
            return Response::Refused {
                message: format!(
                    "There is no one-tap fix for {other}. Nothing was changed."
                ),
            }
        }
    };
    match outcome {
        Ok(()) => Response::Done,
        Err(message) => Response::Failed { message },
    }
}

/// Serve one connection: authenticate, read one request, answer, close.
pub async fn serve_connection(ctx: &Context, mut stream: UnixStream) {
    if PeerPidfd::from_socket(&stream, current_uid()).is_err() {
        tracing::warn!("peer auth refused; dropping");
        return;
    }
    let mut len = [0u8; 4];
    if stream.read_exact(&mut len).await.is_err() {
        return;
    }
    let len = u32::from_be_bytes(len) as usize;
    if len > MAX_FRAME {
        tracing::warn!("frame of {len} bytes refused before it was read");
        return;
    }
    let mut body = vec![0u8; len];
    if stream.read_exact(&mut body).await.is_err() {
        return;
    }
    let response = match serde_json::from_slice::<Request>(&body) {
        Ok(request) => handle(ctx, request).await,
        Err(e) => Response::Failed {
            message: format!("that is not a request this understands: {e}"),
        },
    };
    let Ok(out) = serde_json::to_vec(&response) else {
        return;
    };
    let _ = stream.write_all(&(out.len() as u32).to_be_bytes()).await;
    let _ = stream.write_all(&out).await;
    let _ = stream.flush().await;
}

/// Accept connections for the life of the daemon.
pub async fn run(listener: UnixListener, ctx: Arc<Context>) {
    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                let ctx = Arc::clone(&ctx);
                tokio::spawn(async move { serve_connection(&ctx, stream).await });
            }
            Err(e) => {
                tracing::warn!("accept failed: {e}");
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use audit_proto::MockAuditSink;

    fn ctx(dir: &Path) -> Context {
        Context {
            config_path: dir.join("arlen/sentinel.toml"),
            audit: Arc::new(MockAuditSink::accepting()),
        }
    }

    #[tokio::test]
    async fn a_switch_is_saved_and_reads_back() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ctx(dir.path());
        let r = handle(
            &ctx,
            Request::SetDetector {
                id: "tracker".into(),
                on: true,
            },
        )
        .await;
        assert_eq!(r, Response::Done);
        let (cfg, _) = config::load(&ctx.config_path);
        assert!(cfg.tracker.on);
    }

    #[tokio::test]
    async fn a_refused_change_says_what_still_holds_and_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ctx(dir.path());
        let r = handle(
            &ctx,
            Request::SetAlerts {
                id: "recording".into(),
                mode: "notify".into(),
            },
        )
        .await;
        match r {
            Response::Refused { message } => assert!(message.contains("stays quiet")),
            other => panic!("{other:?}"),
        }
        assert!(!ctx.config_path.exists(), "a refusal wrote no file");
    }

    #[tokio::test]
    async fn a_surface_with_no_one_tap_fix_is_refused_by_name() {
        let dir = tempfile::tempdir().unwrap();
        let r = handle(
            &ctx(dir.path()),
            Request::FixPosture {
                surface: "hidden_network".into(),
            },
        )
        .await;
        match r {
            Response::Refused { message } => {
                assert!(message.contains("hidden_network") && message.contains("Nothing"))
            }
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn a_detector_that_is_off_reports_no_readout() {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = Config::default();
        cfg.exposure.on = false;
        let state = current_state(&cfg).await;
        assert!(state.posture.is_empty());
        assert!(state.posture_incomplete.is_none());
        drop(dir);
    }

    #[tokio::test]
    async fn nothing_claims_to_know_whether_the_microphone_is_in_use() {
        let state = current_state(&Config::default()).await;
        assert_eq!(
            state.capture_active, None,
            "no portal answers this, so the page must not be told false"
        );
    }

    #[tokio::test]
    async fn a_nonsense_request_is_answered_rather_than_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ctx(dir.path());
        let (a, b) = UnixStream::pair().unwrap();
        let body = b"{\"op\":\"nope\"}";
        tokio::spawn(async move {
            let mut a = a;
            a.write_all(&(body.len() as u32).to_be_bytes()).await.unwrap();
            a.write_all(body).await.unwrap();
            let mut len = [0u8; 4];
            a.read_exact(&mut len).await.unwrap();
            let mut out = vec![0u8; u32::from_be_bytes(len) as usize];
            a.read_exact(&mut out).await.unwrap();
            let r: Response = serde_json::from_slice(&out).unwrap();
            assert!(matches!(r, Response::Failed { .. }));
        });
        serve_connection(&ctx, b).await;
    }
}

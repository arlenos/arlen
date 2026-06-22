//! `arlen-consent-broker`: the one daemon every system consent prompt routes
//! through (system-dialog-plan.md). It serves two Unix sockets:
//!
//! - the **intake** socket, where any app raises a [`RequestBody`]; the daemon
//!   resolves the caller from SO_PEERCRED (never the wire body, so an app cannot
//!   ask on another's behalf - the macOS TCC CVE-2025-31250 spoof is
//!   unrepresentable), classifies it, and either silent-grants it or parks the
//!   connection until the user decides;
//! - the **control** socket, where only the trusted shell fetches the front
//!   pending request and submits the user's decision (rendered on the approved
//!   `arlen-shell-overlay` `consent_*` surface).
//!
//! The deferred-reply correlation + classification live in [`daemon::SharedState`]
//! (unit-tested there); this binary is the socket transport + the attested-auth
//! gate. A resolved always-allow is audited fail-closed and then persisted into
//! the LCG Grant node (Option A) inside `SharedState::resolve`, through the
//! [`GraphGrantPersister`] this binary attaches via `with_persister`.

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use arlen_ai_core::capability::{AccessTier, ActionPermissions, BaselineMode, Capability};
use audit_proto::sink::LedgerAuditSink;
use arlen_consent_broker::daemon::{
    ControlReply, ControlRequest, GrantPersister, IntakeOutcome, IntakeResult, ResolveResult,
    SharedState,
};
use arlen_consent_broker::queue::RequestId;
use arlen_consent_broker::service::RequestBody;
use arlen_permissions::connection_auth::ConnectionAuth;
use os_sdk::UnixGraphClient;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};

/// The real [`GrantPersister`]: persists a remembered grant into the LCG Grant
/// node through the knowledge daemon's consent-grant write socket.
struct GraphGrantPersister {
    client: UnixGraphClient,
}

impl GraphGrantPersister {
    fn new(socket_path: String) -> Self {
        Self {
            client: UnixGraphClient::new(socket_path),
        }
    }
}

#[async_trait::async_trait]
impl GrantPersister for GraphGrantPersister {
    async fn persist(
        &self,
        recipient: &str,
        consent_class: &str,
        consent_scope: Option<&str>,
        revocation_handle: &str,
    ) -> Result<(), String> {
        self.client
            .persist_consent_grant(recipient, consent_class, consent_scope, revocation_handle)
            .await
            .map_err(|e| e.to_string())
    }
}

/// Resolve the knowledge daemon's write socket: `ARLEN_DAEMON_SOCKET`, else
/// `$XDG_RUNTIME_DIR/arlen/knowledge.sock`, else `/run/arlen/knowledge.sock`.
fn knowledge_socket() -> String {
    if let Some(s) = std::env::var_os("ARLEN_DAEMON_SOCKET") {
        return s.to_string_lossy().into_owned();
    }
    if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR") {
        let mut p = PathBuf::from(dir);
        p.push("arlen/knowledge.sock");
        return p.to_string_lossy().into_owned();
    }
    "/run/arlen/knowledge.sock".to_string()
}

/// Maximum wire frame, matching the intake-transport core's bound.
const MAX_FRAME: usize = 64 * 1024;

/// How long a peer has to send its request frame after connecting + being
/// authenticated, before the connection is dropped. This bounds a slow-loris: a
/// same-uid peer that connects and authenticates but withholds its request would
/// otherwise park a handler task indefinitely. It bounds ONLY the request read;
/// the intake decision await is deliberately open-ended (the user decides at
/// their own pace).
const REQUEST_READ_TIMEOUT: Duration = Duration::from_secs(30);

/// App ids permitted to drive the control socket (fetch pending + resolve).
/// Only the trusted shell renders the consent surface; everything else is
/// refused. In debug builds a `dev.`-prefixed id is also admitted (the dev /
/// test convention, mirroring the other daemons).
const CONTROL_ADMITTED: &[&str] = &["arlen-shell", "org.arlen.shell"];

fn control_caller_admitted(app_id: &str) -> bool {
    if CONTROL_ADMITTED.contains(&app_id) {
        return true;
    }
    #[cfg(debug_assertions)]
    if app_id.starts_with("dev.") {
        return true;
    }
    false
}

/// The runtime directory the broker's sockets live in
/// (`$XDG_RUNTIME_DIR/arlen`), created 0700 if absent.
fn runtime_dir() -> std::io::Result<PathBuf> {
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "XDG_RUNTIME_DIR is unset; cannot place the consent sockets",
            )
        })?;
    let dir = base.join("arlen");
    std::fs::create_dir_all(&dir)?;
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
    Ok(dir)
}

/// The current uid; cross-uid IPC is rejected by [`ConnectionAuth`].
fn current_uid() -> u32 {
    // SAFETY: getuid() never fails.
    unsafe { libc::getuid() }
}

/// Bind a Unix socket at `path` with 0600 perms, replacing any stale file.
fn bind_socket(path: &PathBuf) -> std::io::Result<UnixListener> {
    if path.exists() {
        let _ = std::fs::remove_file(path);
    }
    let listener = UnixListener::bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}

/// Read one length-prefixed frame (4-byte LE len + body), bounded by
/// [`MAX_FRAME`]. `Ok(None)` on a clean EOF before any byte.
async fn read_frame(stream: &mut UnixStream) -> std::io::Result<Option<Vec<u8>>> {
    let mut len_buf = [0u8; 4];
    match stream.read_exact(&mut len_buf).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let len = u32::from_le_bytes(len_buf) as usize;
    if len > MAX_FRAME {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "consent frame exceeds the maximum size",
        ));
    }
    let mut body = vec![0u8; len];
    stream.read_exact(&mut body).await?;
    Ok(Some(body))
}

/// Read one request frame, bounded by `timeout` (a withholding peer is dropped,
/// not parked forever). A timeout is surfaced as a `TimedOut` error.
async fn read_request_frame(
    stream: &mut UnixStream,
    timeout: Duration,
) -> std::io::Result<Option<Vec<u8>>> {
    match tokio::time::timeout(timeout, read_frame(stream)).await {
        Ok(result) => result,
        Err(_elapsed) => Err(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "peer did not send its request within the read timeout",
        )),
    }
}

/// Write one length-prefixed frame.
async fn write_frame(stream: &mut UnixStream, bytes: &[u8]) -> std::io::Result<()> {
    if bytes.len() > MAX_FRAME {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "consent reply exceeds the maximum size",
        ));
    }
    stream.write_all(&(bytes.len() as u32).to_le_bytes()).await?;
    stream.write_all(bytes).await?;
    stream.flush().await
}

/// Serve one intake connection: attest the peer, read one request, and reply
/// with the disposition (silent grant immediately, or the user's decision once
/// the shell resolves it - the connection is held open meanwhile).
async fn handle_intake_conn(state: Arc<SharedState>, mut stream: UnixStream, uid: u32) {
    let auth = match ConnectionAuth::extract_from(&stream, uid) {
        Ok(a) => a,
        Err(e) => {
            tracing::warn!(error = %e, "intake: peer authentication failed");
            return;
        }
    };
    if let Err(e) = auth.verify_alive() {
        tracing::warn!(error = %e, "intake: peer not alive");
        return;
    }
    let app_id = auth.app_id().to_string();

    let frame = match read_request_frame(&mut stream, REQUEST_READ_TIMEOUT).await {
        Ok(Some(f)) => f,
        Ok(None) => return,
        Err(e) => {
            tracing::warn!(error = %e, "intake: frame read failed");
            return;
        }
    };
    let body: RequestBody = match serde_json::from_slice(&frame) {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(error = %e, "intake: malformed request body");
            return;
        }
    };

    let result = match state.intake(body, &app_id) {
        IntakeOutcome::SilentGranted => IntakeResult::SilentGranted,
        IntakeOutcome::Pending { id, decision } => {
            tracing::info!(app_id = %app_id, id = id.get(), "intake: queued for a dialog");
            match decision.await {
                Ok(outcome) => IntakeResult::Decided { outcome },
                Err(_) => {
                    // The daemon dropped the sender (shutdown); nothing to reply.
                    tracing::warn!(id = id.get(), "intake: decision channel closed");
                    return;
                }
            }
        }
    };
    let bytes = match serde_json::to_vec(&result) {
        Ok(b) => b,
        Err(e) => {
            tracing::error!(error = %e, "intake: failed to encode reply");
            return;
        }
    };
    if let Err(e) = write_frame(&mut stream, &bytes).await {
        tracing::warn!(error = %e, "intake: reply write failed");
    }
}

/// Serve one control connection: attest the peer, require it be the trusted
/// shell, then service one fetch-or-resolve request.
async fn handle_control_conn(state: Arc<SharedState>, mut stream: UnixStream, uid: u32) {
    let auth = match ConnectionAuth::extract_from(&stream, uid) {
        Ok(a) => a,
        Err(e) => {
            tracing::warn!(error = %e, "control: peer authentication failed");
            return;
        }
    };
    if let Err(e) = auth.verify_alive() {
        tracing::warn!(error = %e, "control: peer not alive");
        return;
    }
    let app_id = auth.app_id().to_string();
    if !control_caller_admitted(&app_id) {
        tracing::warn!(app_id = %app_id, "control: caller not admitted");
        return;
    }

    let frame = match read_request_frame(&mut stream, REQUEST_READ_TIMEOUT).await {
        Ok(Some(f)) => f,
        Ok(None) => return,
        Err(e) => {
            tracing::warn!(error = %e, "control: frame read failed");
            return;
        }
    };
    let request: ControlRequest = match serde_json::from_slice(&frame) {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(error = %e, "control: malformed request");
            return;
        }
    };

    let reply = match request {
        ControlRequest::Fetch => ControlReply::Pending {
            view: state.front_view(),
        },
        ControlRequest::Resolve { id, outcome } => {
            match state.resolve(RequestId::from_raw(id), outcome).await {
                ResolveResult::Resolved { audited, grant, .. } => {
                    if !audited {
                        // S13: the decision could not be recorded, so the grant
                        // was failed closed to a denial. Surface the fault.
                        tracing::error!(
                            "control: decision audit failed; failed closed to a denial"
                        );
                    } else if let Some(grant) = grant {
                        // The grant was already recorded in the in-memory store
                        // and persisted into the LCG Grant node inside `resolve`
                        // (Option A, best-effort after the fail-closed decision
                        // audit). Content-free trace only: recipient + revocation
                        // handle, never the summary or scope.
                        tracing::info!(
                            recipient = %grant.recipient,
                            handle = %grant.revocation_handle,
                            "control: always-allow grant recorded"
                        );
                    }
                    ControlReply::Resolved { ok: true }
                }
                ResolveResult::Unknown => ControlReply::Resolved { ok: false },
            }
        }
        ControlRequest::ListGrants => ControlReply::Grants {
            grants: state.list_grants(),
        },
        ControlRequest::RevokeGrant { handle } => ControlReply::Revoked {
            ok: state.revoke_grant(&handle).await,
        },
    };
    let bytes = match serde_json::to_vec(&reply) {
        Ok(b) => b,
        Err(e) => {
            tracing::error!(error = %e, "control: failed to encode reply");
            return;
        }
    };
    if let Err(e) = write_frame(&mut stream, &bytes).await {
        tracing::warn!(error = %e, "control: reply write failed");
    }
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let dir = runtime_dir()?;
    let intake_path = dir.join("consent-intake.sock");
    let control_path = dir.join("consent-control.sock");
    let uid = current_uid();

    let intake = bind_socket(&intake_path)?;
    let control = bind_socket(&control_path)?;

    // Conservative default: Suggest baseline, no autonomous apps, so every
    // request needs a dialog (never fewer prompts than warranted). A
    // config-driven capability that marks specific apps autonomous is a later
    // slice.
    // Each resolved decision is recorded in the audit ledger before the grant is
    // released (S13 audit-before-act); the consent broker is an admitted producer
    // under the stable id `consent-broker`.
    // Durable grant persistence (Option A): an audited always-allow is also
    // persisted into the LCG Grant node, best-effort, so it survives a restart
    // and backs the Settings see+revoke panel. The in-memory store stays the live
    // fast path, so a persistence failure never breaks a resolve.
    let persister = Arc::new(GraphGrantPersister::new(knowledge_socket()));
    let state = Arc::new(
        SharedState::new(
            Capability::new(
                AccessTier::Minimal,
                ActionPermissions::new(BaselineMode::Suggest, Vec::<String>::new()),
            ),
            Arc::new(LedgerAuditSink::at_default_socket()),
        )
        .with_persister(persister),
    );

    tracing::info!(
        intake = %intake_path.display(),
        control = %control_path.display(),
        "consent broker listening"
    );

    let intake_state = Arc::clone(&state);
    let intake_loop = async move {
        loop {
            match intake.accept().await {
                Ok((stream, _)) => {
                    let st = Arc::clone(&intake_state);
                    tokio::spawn(handle_intake_conn(st, stream, uid));
                }
                Err(e) => tracing::warn!(error = %e, "intake: accept failed"),
            }
        }
    };
    let control_state = Arc::clone(&state);
    let control_loop = async move {
        loop {
            match control.accept().await {
                Ok((stream, _)) => {
                    let st = Arc::clone(&control_state);
                    tokio::spawn(handle_control_conn(st, stream, uid));
                }
                Err(e) => tracing::warn!(error = %e, "control: accept failed"),
            }
        }
    };

    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    tokio::select! {
        _ = intake_loop => {}
        _ = control_loop => {}
        _ = tokio::signal::ctrl_c() => tracing::info!("SIGINT, shutting down"),
        _ = sigterm.recv() => tracing::info!("SIGTERM, shutting down"),
    }

    let _ = std::fs::remove_file(&intake_path);
    let _ = std::fs::remove_file(&control_path);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn a_withholding_peer_times_out_rather_than_parking() {
        // One end of a connected pair never writes; the request read must return a
        // TimedOut error (so the handler drops the connection) instead of parking
        // forever. start_paused advances the clock instantly when idle.
        let (mut a, _b) = UnixStream::pair().unwrap();
        let err = read_request_frame(&mut a, REQUEST_READ_TIMEOUT)
            .await
            .expect_err("a silent peer must time out");
        assert_eq!(err.kind(), std::io::ErrorKind::TimedOut);
    }

    #[tokio::test(start_paused = true)]
    async fn a_prompt_peer_reads_its_frame_within_the_timeout() {
        // A peer that sends a framed payload promptly is read normally, not timed
        // out (the timeout bounds only a withholding peer).
        let (mut a, mut b) = UnixStream::pair().unwrap();
        let payload = b"{\"op\":\"fetch\"}";
        b.write_all(&(payload.len() as u32).to_le_bytes()).await.unwrap();
        b.write_all(payload).await.unwrap();
        b.flush().await.unwrap();
        let frame = read_request_frame(&mut a, REQUEST_READ_TIMEOUT)
            .await
            .expect("the read succeeds")
            .expect("a frame is present");
        assert_eq!(frame, payload);
    }
}

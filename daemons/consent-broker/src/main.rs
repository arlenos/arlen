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
        expires_at_micros: Option<i64>,
    ) -> Result<(), String> {
        self.client
            .persist_consent_grant(
                recipient,
                consent_class,
                consent_scope,
                revocation_handle,
                expires_at_micros,
            )
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

/// App ids permitted to drive EVERY control op, including rendering and resolving
/// consent prompts (`Fetch` / `Resolve`). Only the trusted shell renders the
/// consent surface. The control caller is resolved by `ConnectionAuth` from the
/// SO_PEERCRED-attested pid via `path_to_app_id`, so this is the shell's RESOLVED
/// app id: `/usr/bin/arlen-desktop-shell` -> `desktop-shell` (rule 2). The former
/// `arlen-shell` / `org.arlen.shell` entries were the pre-rename binary name and a
/// D-Bus-style name - NEITHER can be a `path_to_app_id` result, so the shell (which
/// resolves to `desktop-shell`) was refused in release and could not drive the
/// consent dialog at all. In debug builds a `dev.`-prefixed id is also admitted.
const CONTROL_ADMITTED: &[&str] = &["dev.arlen.desktop-shell"];

/// App ids permitted ONLY the grant-management ops (`ListGrants` / `RevokeGrant`)
/// - the App-access panel's "what you allowed" + release-a-grant surface.
/// `settings` is already the revoke authority (it drives the profile-scope revoke
/// 0x06 on the `is_settings_principal` anchor), so releasing a consent grant is a
/// strict subset of the power it already holds; it is deliberately NOT admitted to
/// `Fetch` / `Resolve` (rendering and answering prompts stays the trusted shell's).
const GRANT_MGMT_ADMITTED: &[&str] = &["dev.arlen.settings"];

/// The early gate: whether `app_id` may drive ANY control op at all (a cheap
/// refusal for outsiders before the request is read). The per-op restriction for a
/// grant-management-only caller is enforced by [`control_op_admitted`].
fn control_caller_admitted(app_id: &str) -> bool {
    CONTROL_ADMITTED.contains(&app_id)
        || GRANT_MGMT_ADMITTED.contains(&app_id)
        || (cfg!(debug_assertions)
            && (CONTROL_ADMITTED_DEV.contains(&app_id) || GRANT_MGMT_ADMITTED_DEV.contains(&app_id)))
}

/// Whether `app_id` may drive THIS specific control op. The shell (and a debug
/// `dev.` id) may drive all ops; a [`GRANT_MGMT_ADMITTED`] caller (`settings`) may
/// drive only `ListGrants` / `RevokeGrant`, never `Fetch` / `Resolve`.
/// The cargo-run ids of the admitted callers, accepted only in debug builds.
///
/// EXACT, never a `dev.` prefix match, which is what this used to be. Every
/// locally-built binary resolves to some `dev.<bin>`, so the prefix admitted all
/// of them - including, in a debug build, to ANSWER PROMPTS, which is the one
/// thing the split between these two lists exists to deny anything but the shell.
/// The audit ingest, the undo signer and the transfer daemon had each already
/// moved off this shape for the same reason.
#[cfg(debug_assertions)]
const CONTROL_ADMITTED_DEV: &[&str] = &["dev.arlen-desktop-shell"];

/// The same, for the grant-management half.
#[cfg(debug_assertions)]
const GRANT_MGMT_ADMITTED_DEV: &[&str] = &["dev.arlen-settings"];

fn control_op_admitted(app_id: &str, request: &ControlRequest) -> bool {
    if CONTROL_ADMITTED.contains(&app_id) {
        return true;
    }
    #[cfg(debug_assertions)]
    if CONTROL_ADMITTED_DEV.contains(&app_id) {
        return true;
    }
    #[cfg(debug_assertions)]
    if GRANT_MGMT_ADMITTED_DEV.contains(&app_id) {
        return matches!(
            request,
            ControlRequest::ListGrants | ControlRequest::RevokeGrant { .. }
        );
    }
    if GRANT_MGMT_ADMITTED.contains(&app_id) {
        return matches!(
            request,
            ControlRequest::ListGrants | ControlRequest::RevokeGrant { .. }
        );
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

/// Bind a Unix socket at `path` with 0600 perms, replacing a stale socket left by
/// a prior run.
///
/// Only an existing *socket* is removed. The previous form unlinked anything the
/// path led to, and it tested `path.exists()`, which follows symlinks - so both a
/// regular file and a symlink pointing at a live file triggered the removal. A
/// socket path should not be a way to delete a file the daemon did not create.
/// Matches the settings-broker and power-daemon sockets, which do the same job.
fn bind_socket(path: &PathBuf) -> std::io::Result<UnixListener> {
    if let Ok(meta) = std::fs::symlink_metadata(path) {
        use std::os::unix::fs::FileTypeExt;
        if meta.file_type().is_socket() {
            let _ = std::fs::remove_file(path);
        }
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
/// Best-effort peer identity for a diagnostic log: the SO_PEERCRED pid + comm
/// (the process name, readable via `/proc/<pid>/comm` even when `/proc/<pid>/exe`
/// is not, e.g. a non-dumpable WebKit process). Lets an operator see WHICH caller
/// was refused when exe-path identity resolution is denied.
fn peer_diag(stream: &UnixStream) -> String {
    use std::os::unix::io::AsRawFd;
    let mut cred: libc::ucred = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: getsockopt(SO_PEERCRED) fills `cred`; the fd is valid for the call
    // and `len` is initialised to the buffer size.
    let rc = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            std::ptr::addr_of_mut!(cred).cast(),
            &mut len,
        )
    };
    if rc != 0 {
        return "pid=? comm=?".to_string();
    }
    let comm = std::fs::read_to_string(format!("/proc/{}/comm", cred.pid))
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "?".to_string());
    // The owner uid of /proc/<pid> reveals the dumpable state: a non-dumpable
    // process (its exe unreadable by a same-uid non-root reader) has /proc/<pid>
    // owned by root (0); a dumpable one is owned by the process uid. So
    // procowner=0 here = the caller is non-dumpable (the cause of the exe EACCES).
    use std::os::unix::fs::MetadataExt;
    let procowner = std::fs::metadata(format!("/proc/{}", cred.pid))
        .map(|m| m.uid().to_string())
        .unwrap_or_else(|_| "?".to_string());
    // /proc/<pid>/status is readable even when exe is not, so grab the fields that
    // discriminate the remaining EACCES hypotheses when the peer is dumpable
    // (procowner=uid) yet the exe readlink still fails: a WebKit seccomp filter
    // (Seccomp), a capability the reader must dominate (CapEff), or a nested pid
    // namespace making /proc/<pid> a different task cross-ns (NSpid).
    let status = std::fs::read_to_string(format!("/proc/{}/status", cred.pid)).unwrap_or_default();
    let field = |name: &str| {
        status
            .lines()
            .find(|l| l.starts_with(name))
            .map(|l| l.split_whitespace().skip(1).collect::<Vec<_>>().join(","))
            .unwrap_or_else(|| "?".to_string())
    };
    format!(
        "pid={} uid={} comm={} procowner={} seccomp={} capeff={} nspid={} tracerpid={}",
        cred.pid,
        cred.uid,
        comm,
        procowner,
        field("Seccomp:"),
        field("CapEff:"),
        field("NSpid:"),
        field("TracerPid:")
    )
}

async fn handle_control_conn(state: Arc<SharedState>, mut stream: UnixStream, uid: u32) {
    let auth = match ConnectionAuth::extract_from(&stream, uid) {
        Ok(a) => a,
        Err(e) => {
            tracing::warn!(error = %e, peer = %peer_diag(&stream), "control: peer authentication failed");
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
    {
        // One-shot: confirm a caller successfully authenticated to the CONTROL
        // socket (the broker otherwise only logs failures), so a boot-verify can
        // see the shell get through.
        use std::sync::atomic::{AtomicBool, Ordering};
        static FIRST: AtomicBool = AtomicBool::new(true);
        if FIRST.swap(false, Ordering::Relaxed) {
            tracing::info!(app_id = %app_id, "control: first caller authenticated");
        }
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
    // A grant-management-only caller (settings) is refused Fetch / Resolve here;
    // the early gate admitted it for the grant ops, this enforces the op split.
    if !control_op_admitted(&app_id, &request) {
        tracing::warn!(app_id = %app_id, "control: caller not admitted for this op");
        return;
    }

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

    /// A restart has to clear the socket a previous run left, and the socket path
    /// must not double as a way to delete a file the daemon did not create. The
    /// old form tested `path.exists()`, which follows symlinks.
    #[tokio::test]
    async fn bind_clears_a_stale_socket_and_refuses_to_clobber_anything_else() {
        let dir = tempfile::tempdir().unwrap();

        let path = dir.path().join("consent.sock");
        drop(bind_socket(&path).expect("first bind"));
        drop(bind_socket(&path).expect("a stale socket must not block a restart"));
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600,
            "consent sockets are owner-only"
        );

        let file = dir.path().join("real-file");
        std::fs::write(&file, b"not a socket").unwrap();
        assert!(bind_socket(&file).is_err(), "binding over a regular file must fail");
        assert_eq!(std::fs::read(&file).unwrap(), b"not a socket", "and leave it alone");

        let target = dir.path().join("target-file");
        std::fs::write(&target, b"target").unwrap();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(bind_socket(&link).is_err(), "binding over a symlink must fail");
        assert!(link.symlink_metadata().is_ok(), "the symlink survives");
        assert_eq!(std::fs::read(&target).unwrap(), b"target", "and its target");
    }

    #[test]
    fn settings_may_manage_grants_but_not_answer_prompts() {
        use arlen_consent_contract::ConsentOutcome;
        let revoke = ControlRequest::RevokeGrant { handle: "h".into() };
        // The trusted shell drives every control op, under its SO_PEERCRED-resolved
        // app id `desktop-shell` (the id `path_to_app_id` returns for the shell's
        // canonical /usr/bin/arlen-desktop-shell binary).
        assert!(control_op_admitted("dev.arlen.desktop-shell", &ControlRequest::Fetch));
        assert!(control_op_admitted("dev.arlen.desktop-shell", &revoke));
        // The pre-rename `arlen-shell` name is NOT a resolvable app id, so it is
        // (correctly) no longer admitted - the shell resolves to `desktop-shell`.
        assert!(!control_caller_admitted("arlen-shell"));
        // settings may list + release remembered grants (the App-access surface)...
        assert!(control_op_admitted("dev.arlen.settings", &ControlRequest::ListGrants));
        assert!(control_op_admitted("dev.arlen.settings", &revoke));
        // ...but is refused rendering / answering consent prompts (shell-only).
        assert!(!control_op_admitted("dev.arlen.settings", &ControlRequest::Fetch));
        assert!(!control_op_admitted(
            "dev.arlen.settings",
            &ControlRequest::Resolve {
                id: 1,
                outcome: ConsentOutcome::AllowedOnce,
            }
        ));
        // A random app is refused every op, and the early gate rejects it too.
        assert!(!control_op_admitted("com.random", &ControlRequest::ListGrants));
        assert!(control_caller_admitted("dev.arlen.settings"));
        assert!(!control_caller_admitted("com.random"));
    }

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

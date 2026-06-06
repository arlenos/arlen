//! Clipboard IPC server.
//!
//! Brokers clipboard read/write/subscribe/history operations from
//! Lunaris-aware apps to the existing `ClipboardHistory` store.
//! Wire-protocol matches the rest of the SDK: 4-byte big-endian
//! length prefix, then a `ClipboardEnvelope` protobuf body.
//!
//! Permission enforcement is staged: phase 1 trusts caller-provided
//! intent (any app may read normal content, history reads bypass
//! the gate). Sensitive-content filtering still works because it
//! is label-based and decided at read time; the future hardening
//! adds caller-app-id authentication via SO_PEERCRED + cgroup so
//! the `read.sensitive` and `history` permission profile lookups
//! fire against a real identity rather than a self-declared one.
//!
//! See `docs/architecture/clipboard-api.md`.

use std::os::unix::io::AsRawFd;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::OnceLock;
use std::time::Duration;

use lunaris_permissions::{AuthError, ConnectionAuth};
use prost::Message;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::Semaphore;
use tokio::time::timeout;

use crate::clipboard_history::{ClipboardEntry as InternalEntry, ClipboardHistory, Label};

/// Generated protobuf types for the clipboard IPC. Compiled by
/// `build.rs` from `proto/clipboard_api.proto`.
mod proto {
    #![allow(dead_code, clippy::doc_markdown)]
    include!(concat!(env!("OUT_DIR"), "/lunaris.clipboard.rs"));
}

const MAX_FRAME_BYTES: usize = 1024 * 1024;
const SOCKET_NAME: &str = "clipboard.sock";

/// Cap on simultaneous in-flight broker connections. Mirrors the
/// search broker's hardening (Codex post-Sprint review MEDIUM-2).
/// Higher than search because clipboard subscribe-connections are
/// long-lived (a Lunaris-aware app holds one open across its
/// session); 64 covers a realistic 8-app workspace with 8x
/// safety margin. Excess accepts dropped at the listener.
const MAX_CONCURRENT_CONNS: usize = 64;

/// First-frame deadline: time-from-accept until the connection
/// has authenticated AND received its first complete envelope.
/// After the first frame, subscribe-connections idle indefinitely
/// (their long-lived nature is the whole point of subscribe), so
/// we cannot timeout the per-frame read in the loop below.
/// Stalled-attacker mitigation is therefore "first-frame only" —
/// enough to drop a peer that connects but never sends.
const FIRST_FRAME_TIMEOUT: Duration = Duration::from_secs(5);

fn semaphore() -> Arc<Semaphore> {
    static SEM: OnceLock<Arc<Semaphore>> = OnceLock::new();
    SEM.get_or_init(|| Arc::new(Semaphore::new(MAX_CONCURRENT_CONNS)))
        .clone()
}

/// Bind the IPC socket and spawn the accept loop. Idempotent: a
/// stale socket from a previous shell crash is removed before bind
/// so the daemon comes up clean.
pub fn start(history: Arc<ClipboardHistory>) {
    tauri::async_runtime::spawn(async move {
        match run(history).await {
            Ok(()) => log::info!("clipboard_ipc: shut down cleanly"),
            Err(e) => log::error!("clipboard_ipc: server exited: {e}"),
        }
    });
}

async fn run(history: Arc<ClipboardHistory>) -> Result<(), String> {
    let path = socket_path().map_err(|e| format!("derive socket path: {e}"))?;
    if path.exists() {
        let _ = std::fs::remove_file(&path);
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let listener = UnixListener::bind(&path).map_err(|e| format!("bind {}: {e}", path.display()))?;
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    log::info!("clipboard_ipc: listening on {}", path.display());

    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                // Connection cap (Codex review parity with search_ipc).
                // Try-acquire so we never block the accept loop: if
                // 64 connections are already in flight, drop the new
                // socket. The peer can retry; a flood-attacker gets
                // denial-by-default rather than task exhaustion.
                let permit = match semaphore().try_acquire_owned() {
                    Ok(p) => p,
                    Err(_) => {
                        log::warn!(
                            "clipboard_ipc: connection cap of {MAX_CONCURRENT_CONNS} reached, dropping accept"
                        );
                        drop(stream);
                        continue;
                    }
                };
                let history = history.clone();
                tokio::spawn(async move {
                    let _permit = permit;
                    if let Err(e) = connection_task(stream, history).await {
                        log::warn!("clipboard_ipc: connection task ended: {e}");
                    }
                });
            }
            Err(e) => {
                log::warn!("clipboard_ipc: accept failed: {e}");
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }
}

fn socket_path() -> Result<PathBuf, String> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .ok_or_else(|| "XDG_RUNTIME_DIR not set".to_string())?;
    let mut p = PathBuf::from(runtime);
    p.push("lunaris");
    p.push(SOCKET_NAME);
    Ok(p)
}

/// Per-connection driver. Reads framed `ClipboardEnvelope` messages
/// in a loop; each frame is dispatched via the oneof variant. The
/// connection lives until the peer closes or a malformed frame is
/// received.
async fn connection_task(
    stream: UnixStream,
    history: Arc<ClipboardHistory>,
) -> Result<(), String> {
    // Connection-time peer auth (Sprint D / peer-auth-system.md).
    // Resolves the connecting app's identity via SO_PEERCRED +
    // /proc/{pid}/exe, loads its permission profile, and stores
    // (pid, start_time) for per-request liveness checks.
    let caller_uid = unsafe { libc::getuid() };
    let raw_auth = match ConnectionAuth::extract_from(&stream, caller_uid) {
        Ok(a) => a,
        Err(e) => {
            log::warn!("clipboard_ipc: connection rejected at auth: {e}");
            return Err(format!("auth: {e}"));
        }
    };
    log::info!(
        "clipboard_ipc: connection from app_id={} pid={}",
        raw_auth.app_id(),
        raw_auth.pid()
    );
    let app_id = raw_auth.app_id().to_string();
    let auth = Arc::new(tokio::sync::Mutex::new(raw_auth));
    // RAII registration in the permission-revocation registry.
    // Dropping `_guard` at the end of this function (success, error,
    // panic — all paths) deregisters automatically.
    let _guard = crate::permission_watcher::register_connection(app_id, Arc::clone(&auth));

    let (mut reader, writer) = stream.into_split();
    let writer = Arc::new(tokio::sync::Mutex::new(writer));
    let mut buf = Vec::with_capacity(4096);
    let mut chunk = [0u8; 4096];

    // First-frame timeout: an attacker that completes accept+
    // SO_PEERCRED but never sends a frame would otherwise pin the
    // task indefinitely. Once the peer's first envelope decodes
    // successfully we drop the deadline — subscribe-style
    // connections legitimately idle for hours waiting for clipboard
    // events. (Codex-parity with search_ipc; first-frame-only
    // because clipboard is long-lived, unlike search's single-shot.)
    let mut got_first_frame = false;

    loop {
        let n = if !got_first_frame {
            match timeout(FIRST_FRAME_TIMEOUT, reader.read(&mut chunk)).await {
                Ok(Ok(n)) => n,
                Ok(Err(e)) => return Err(format!("read: {e}")),
                Err(_) => {
                    let g = auth.lock().await;
                    let (app, pid) = (g.app_id().to_string(), g.pid());
                    drop(g);
                    log::warn!(
                        "clipboard_ipc: first-frame timeout from app_id={app} pid={pid} after {FIRST_FRAME_TIMEOUT:?} — dropping"
                    );
                    return Ok(());
                }
            }
        } else {
            reader
                .read(&mut chunk)
                .await
                .map_err(|e| format!("read: {e}"))?
        };
        if n == 0 {
            return Ok(());
        }
        buf.extend_from_slice(&chunk[..n]);

        while let Some((consumed, envelope)) = decode_frame(&buf)? {
            buf.drain(..consumed);
            got_first_frame = true;
            // Per-request liveness check (PID-recycle guard).
            if let Err(e) = auth.lock().await.verify_alive() {
                log::info!("clipboard_ipc: peer no longer alive: {e}");
                return Ok(());
            }
            handle_envelope(envelope, history.clone(), writer.clone(), auth.clone()).await;
        }
    }
}

/// Decode a single envelope from the front of the buffer, returning
/// `Ok(Some((consumed, env)))` on success, `Ok(None)` when the
/// buffer is incomplete, `Err` on protocol violation (malformed
/// frame, oversize body — drop the connection in either case).
fn decode_frame(buf: &[u8]) -> Result<Option<(usize, proto::ClipboardEnvelope)>, String> {
    if buf.len() < 4 {
        return Ok(None);
    }
    let len_bytes: [u8; 4] = buf[..4].try_into().expect("checked len above");
    let len = u32::from_be_bytes(len_bytes) as usize;
    if len == 0 {
        return Err("empty frame".into());
    }
    if len > MAX_FRAME_BYTES {
        return Err(format!("frame too large: {len} > {MAX_FRAME_BYTES}"));
    }
    if buf.len() < 4 + len {
        return Ok(None);
    }
    let body = &buf[4..4 + len];
    let env = proto::ClipboardEnvelope::decode(body)
        .map_err(|e| format!("protobuf decode: {e}"))?;
    Ok(Some((4 + len, env)))
}

async fn handle_envelope(
    envelope: proto::ClipboardEnvelope,
    history: Arc<ClipboardHistory>,
    writer: Arc<tokio::sync::Mutex<tokio::net::unix::OwnedWriteHalf>>,
    auth: Arc<tokio::sync::Mutex<ConnectionAuth>>,
) {
    use proto::clipboard_envelope::Message as Msg;
    let Some(msg) = envelope.message else {
        return;
    };
    match msg {
        Msg::WriteRequest(req) => {
            // clipboard.write scope check.
            let auth_guard = auth.lock().await;
            if !auth_guard.profile().clipboard.write {
                audit_deny(&auth_guard, "write", "clipboard.write");
                drop(auth_guard);
                let resp = error_envelope(ProtoError {
                    kind: proto::ErrorKind::ErrorPermissionDenied,
                    detail: "missing clipboard.write scope".into(),
                });
                let _ = send_envelope(&writer, resp).await;
                return;
            }
            audit_grant(&auth_guard, "write", "clipboard.write");
            drop(auth_guard);

            let outcome = handle_write(&history, req).await;
            let resp = match outcome {
                Ok(entry) => proto::ClipboardEnvelope {
                    message: Some(Msg::WriteResponse(proto::WriteResponse { entry })),
                },
                Err(err) => error_envelope(err),
            };
            let _ = send_envelope(&writer, resp).await;
        }
        Msg::ReadRequest(_) => {
            // Three-policy gate (peer-auth-system.md):
            //   - no `clipboard.read` -> PermissionDenied
            //   - has `read` but not `read.sensitive` and entry
            //     is sensitive -> response shape unchanged but
            //     `content=None` (foundation §8.4.4 empty-string
            //     semantics; do not leak existence via denial).
            let auth_guard = auth.lock().await;
            if !auth_guard.profile().clipboard.read {
                audit_deny(&auth_guard, "read", "clipboard.read");
                drop(auth_guard);
                let resp = error_envelope(ProtoError {
                    kind: proto::ErrorKind::ErrorPermissionDenied,
                    detail: "missing clipboard.read scope".into(),
                });
                let _ = send_envelope(&writer, resp).await;
                return;
            }
            let allow_sensitive = auth_guard.profile().clipboard.read_sensitive;
            audit_grant(&auth_guard, "read", "clipboard.read");
            drop(auth_guard);

            let resp = handle_read(&history, allow_sensitive);
            let _ = send_envelope(&writer, resp).await;
        }
        Msg::SubscribeRequest(_) => {
            let auth_guard = auth.lock().await;
            if !auth_guard.profile().clipboard.read {
                audit_deny(&auth_guard, "subscribe", "clipboard.read");
                drop(auth_guard);
                let resp = error_envelope(ProtoError {
                    kind: proto::ErrorKind::ErrorPermissionDenied,
                    detail: "missing clipboard.read scope".into(),
                });
                let _ = send_envelope(&writer, resp).await;
                return;
            }
            let allow_sensitive = auth_guard.profile().clipboard.read_sensitive;
            audit_grant(&auth_guard, "subscribe", "clipboard.read");
            drop(auth_guard);

            // Acknowledge the subscribe handshake first so the
            // SDK's `subscribe()` future resolves; then start
            // streaming.
            let receiver = history.subscribe();
            let ack = proto::ClipboardEnvelope {
                message: Some(Msg::SubscribeResponse(proto::SubscribeResponse {})),
            };
            if send_envelope(&writer, ack).await.is_err() {
                return;
            }
            tokio::spawn(subscription_task(receiver, writer.clone(), allow_sensitive));
        }
        Msg::HistoryRequest(req) => {
            let auth_guard = auth.lock().await;
            if !auth_guard.profile().clipboard.history {
                audit_deny(&auth_guard, "history", "clipboard.history");
                drop(auth_guard);
                let resp = error_envelope(ProtoError {
                    kind: proto::ErrorKind::ErrorPermissionDenied,
                    detail: "missing clipboard.history scope".into(),
                });
                let _ = send_envelope(&writer, resp).await;
                return;
            }
            let allow_sensitive = auth_guard.profile().clipboard.read_sensitive;
            audit_grant(&auth_guard, "history", "clipboard.history");
            drop(auth_guard);

            let resp = handle_history(&history, req, allow_sensitive);
            let _ = send_envelope(&writer, resp).await;
        }
        // Response types should never arrive from the client; ignore.
        Msg::WriteResponse(_)
        | Msg::ReadResponse(_)
        | Msg::HistoryResponse(_)
        | Msg::SubscriptionEvent(_)
        | Msg::SubscribeResponse(_)
        | Msg::Error(_) => {}
    }
}

/// Read the current clipboard entry. Strips `content` from
/// sensitive entries when the caller lacks `read.sensitive`
/// (foundation §8.4.4 empty-string semantics).
fn handle_read(history: &ClipboardHistory, allow_sensitive: bool) -> proto::ClipboardEnvelope {
    use proto::clipboard_envelope::Message as Msg;
    let entry_opt = history.snapshot().into_iter().next();
    let entry = entry_opt.map(|e| {
        let mut p = entry_to_proto(&e);
        if matches!(e.label, Label::Sensitive) && !allow_sensitive {
            p.content = None;
        }
        p
    });
    proto::ClipboardEnvelope {
        message: Some(Msg::ReadResponse(proto::ReadResponse { entry })),
    }
}

/// Return up to `limit` history entries. Sensitive entries
/// have content stripped for callers without `read.sensitive`.
fn handle_history(
    history: &ClipboardHistory,
    req: proto::HistoryRequest,
    allow_sensitive: bool,
) -> proto::ClipboardEnvelope {
    use proto::clipboard_envelope::Message as Msg;
    if !history.is_enabled() {
        return proto::ClipboardEnvelope {
            message: Some(Msg::HistoryResponse(proto::HistoryResponse {
                entries: Vec::new(),
            })),
        };
    }
    let mut snap = history.snapshot();
    let limit = if req.limit == 0 {
        snap.len()
    } else {
        (req.limit as usize).min(snap.len())
    };
    snap.truncate(limit);
    let entries: Vec<proto::ClipboardEntry> = snap
        .iter()
        .map(|e| {
            let mut p = entry_to_proto(e);
            if matches!(e.label, Label::Sensitive) && !allow_sensitive {
                p.content = None;
            }
            p
        })
        .collect();
    proto::ClipboardEnvelope {
        message: Some(Msg::HistoryResponse(proto::HistoryResponse { entries })),
    }
}

/// Long-lived task: forwards every clipboard change to this
/// subscriber. `allow_sensitive` is captured at subscribe time;
/// permission.changed events refresh the parent connection's
/// auth but in-flight subscriptions keep their original
/// projection (acceptable; documented in
/// peer-auth-system.md).
async fn subscription_task(
    mut receiver: tokio::sync::broadcast::Receiver<InternalEntry>,
    writer: Arc<tokio::sync::Mutex<tokio::net::unix::OwnedWriteHalf>>,
    allow_sensitive: bool,
) {
    use proto::clipboard_envelope::Message as Msg;
    loop {
        let entry = match receiver.recv().await {
            Ok(e) => e,
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
            Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
        };
        let mut p = entry_to_proto(&entry);
        if matches!(entry.label, Label::Sensitive) && !allow_sensitive {
            p.content = None;
        }
        let envelope = proto::ClipboardEnvelope {
            message: Some(Msg::SubscriptionEvent(proto::SubscriptionEvent {
                entry: Some(p),
            })),
        };
        if send_envelope(&writer, envelope).await.is_err() {
            return;
        }
    }
}

fn entry_to_proto(e: &InternalEntry) -> proto::ClipboardEntry {
    proto::ClipboardEntry {
        id: e.id.to_string(),
        content: Some(e.content.as_bytes().to_vec()),
        mime: e.mime.clone(),
        label: label_to_proto(e.label) as i32,
        timestamp_ms: e.timestamp_ms,
        source_app_id: e.source_app_id.clone(),
    }
}

fn label_to_proto(label: Label) -> proto::Label {
    match label {
        Label::Normal => proto::Label::Normal,
        Label::Sensitive => proto::Label::Sensitive,
    }
}

/// Audit log emission (foundation §8.4.7). Per-(app, scope)
/// rate-limit applied via the ledger held in `audit_ledger`.
fn audit_grant(auth: &ConnectionAuth, surface: &str, scope: &'static str) {
    if audit_should_emit(auth.app_id(), scope) {
        log::info!(
            "[AUDIT] perm.grant app={} pid={} scope={} surface={} result=granted",
            auth.app_id(),
            auth.pid(),
            scope,
            surface
        );
    }
}

fn audit_deny(auth: &ConnectionAuth, surface: &str, scope: &'static str) {
    // Denials always log (no rate-limit) — a flood of denials
    // is a security event we want visibility on.
    log::warn!(
        "[AUDIT] perm.deny app={} pid={} scope={} surface={} result=denied reason=missing_scope",
        auth.app_id(),
        auth.pid(),
        scope,
        surface
    );
}

/// Per-(app, scope) ledger throttling grant emissions to max
/// 1/sec. Static because all clipboard connections share the
/// same audit flow; each (app, scope) tuple has its own
/// last-emit Instant.
fn audit_should_emit(app_id: &str, scope: &'static str) -> bool {
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    static LEDGER: std::sync::OnceLock<Mutex<std::collections::HashMap<(String, &'static str), Instant>>> =
        std::sync::OnceLock::new();
    let mut guard = LEDGER
        .get_or_init(|| Mutex::new(std::collections::HashMap::new()))
        .lock()
        .unwrap();
    let key = (app_id.to_string(), scope);
    let now = Instant::now();
    match guard.get(&key) {
        Some(&last) if now.duration_since(last) < Duration::from_secs(1) => false,
        _ => {
            guard.insert(key, now);
            true
        }
    }
}

async fn handle_write(
    history: &ClipboardHistory,
    req: proto::WriteRequest,
) -> Result<Option<proto::ClipboardEntry>, ProtoError> {
    if req.content.len() > crate::clipboard_history::MAX_ENTRY_BYTES {
        return Err(ProtoError {
            kind: proto::ErrorKind::ErrorContentTooLarge,
            detail: format!(
                "{} bytes exceeds the {}-byte limit",
                req.content.len(),
                crate::clipboard_history::MAX_ENTRY_BYTES
            ),
        });
    }
    if !req.mime.is_empty() && req.mime != "text/plain" {
        return Err(ProtoError {
            kind: proto::ErrorKind::ErrorUnsupportedMime,
            detail: format!("only text/plain is supported in phase 1, got {}", req.mime),
        });
    }
    let content = String::from_utf8(req.content.clone()).map_err(|_| ProtoError {
        kind: proto::ErrorKind::ErrorUnsupportedMime,
        detail: "content is not valid UTF-8 (text/plain requires UTF-8)".to_string(),
    })?;
    let label = label_from_proto(req.label);
    history
        .write_with_label(content.clone(), label, String::new())
        .map_err(|e| ProtoError {
            kind: proto::ErrorKind::ErrorSystem,
            detail: format!("wl-copy: {e}"),
        })?;
    // The wl-paste watcher will fire shortly and call `push()`; the
    // immediate write itself does not produce a stored entry to
    // return synchronously. Phase 2 will surface the watcher's
    // outcome here once that pathway becomes important.
    Ok(None)
}

async fn send_envelope(
    writer: &Arc<tokio::sync::Mutex<tokio::net::unix::OwnedWriteHalf>>,
    envelope: proto::ClipboardEnvelope,
) -> std::io::Result<()> {
    let body = envelope.encode_to_vec();
    let len = (body.len() as u32).to_be_bytes();
    let mut w = writer.lock().await;
    w.write_all(&len).await?;
    w.write_all(&body).await?;
    Ok(())
}

fn label_from_proto(value: i32) -> Label {
    match proto::Label::try_from(value).unwrap_or(proto::Label::Normal) {
        proto::Label::Sensitive => Label::Sensitive,
        proto::Label::Normal => Label::Normal,
    }
}

struct ProtoError {
    kind: proto::ErrorKind,
    detail: String,
}

fn error_envelope(err: ProtoError) -> proto::ClipboardEnvelope {
    use proto::clipboard_envelope::Message as Msg;
    proto::ClipboardEnvelope {
        message: Some(Msg::Error(proto::ErrorResponse {
            kind: err.kind as i32,
            detail: err.detail,
        })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lunaris_permissions::{
        AppTier, ClipboardPermissions, PermissionProfile, ProfileInfo,
    };
    use proto::clipboard_envelope::Message as Msg;
    use tokio::net::UnixStream;

    fn profile_with_clipboard(c: ClipboardPermissions) -> PermissionProfile {
        PermissionProfile {
            info: ProfileInfo {
                app_id: "test.app".into(),
                tier: AppTier::ThirdParty,
            },
            graph: Default::default(),
            event_bus: Default::default(),
            filesystem: Default::default(),
            network: Default::default(),
            notifications: Default::default(),
            clipboard: c,
            system: Default::default(),
            input: Default::default(),
            search: Default::default(),
            intents: Default::default(),
        }
    }

    /// Codex-parity with search_ipc: a stalled or malicious
    /// authenticated client cannot pin the broker task. Unlike
    /// search's per-frame deadline, clipboard's deadline is
    /// FIRST-FRAME ONLY because subscribe-connections legitimately
    /// idle for hours.
    #[test]
    fn first_frame_timeout_is_bounded() {
        assert!(FIRST_FRAME_TIMEOUT >= Duration::from_secs(1));
        assert!(FIRST_FRAME_TIMEOUT <= Duration::from_secs(30));
    }

    /// Codex-parity: connection cap protects against fd-exhaustion.
    /// 64 (vs search's 32) because clipboard's subscribe pattern
    /// keeps connections open across the app's session, so a
    /// realistic 8-app workspace already wants 8 slots.
    #[test]
    fn clipboard_connection_cap_is_finite() {
        assert!(MAX_CONCURRENT_CONNS > 0);
        assert!(MAX_CONCURRENT_CONNS <= 256);
    }

    /// Semaphore singleton — repeated calls return the same Arc.
    #[test]
    fn clipboard_semaphore_is_shared_singleton() {
        let s1 = semaphore();
        let s2 = semaphore();
        assert_eq!(Arc::as_ptr(&s1), Arc::as_ptr(&s2));
        assert!(s1.available_permits() <= MAX_CONCURRENT_CONNS);
    }

    /// Drives `handle_envelope` over a `UnixStream::pair()` with
    /// a test ConnectionAuth carrying the provided profile.
    /// Returns the single response envelope.
    async fn dispatch_with_profile(
        envelope: proto::ClipboardEnvelope,
        profile: PermissionProfile,
    ) -> proto::ClipboardEnvelope {
        let (a, b) = UnixStream::pair().expect("pair");
        let (mut a_read, _a_write) = a.into_split();
        let (_b_read, b_write) = b.into_split();
        let writer = Arc::new(tokio::sync::Mutex::new(b_write));

        let history = Arc::new(ClipboardHistory::new());
        let auth = Arc::new(tokio::sync::Mutex::new(
            ConnectionAuth::for_test("test.app", profile),
        ));
        handle_envelope(envelope, history, writer.clone(), auth).await;

        let mut len_buf = [0u8; 4];
        a_read.read_exact(&mut len_buf).await.expect("read len");
        let len = u32::from_be_bytes(len_buf) as usize;
        let mut body = vec![0u8; len];
        a_read.read_exact(&mut body).await.expect("read body");
        proto::ClipboardEnvelope::decode(body.as_slice()).expect("decode")
    }

    #[tokio::test]
    async fn read_without_scope_returns_permission_denied() {
        let resp = dispatch_with_profile(
            proto::ClipboardEnvelope {
                message: Some(Msg::ReadRequest(proto::ReadRequest {})),
            },
            profile_with_clipboard(ClipboardPermissions::default()),
        )
        .await;
        match resp.message {
            Some(Msg::Error(e)) => {
                assert_eq!(e.kind, proto::ErrorKind::ErrorPermissionDenied as i32);
                assert!(e.detail.contains("clipboard.read"));
            }
            other => panic!("expected Error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn read_with_scope_succeeds_on_empty_clipboard() {
        let resp = dispatch_with_profile(
            proto::ClipboardEnvelope {
                message: Some(Msg::ReadRequest(proto::ReadRequest {})),
            },
            profile_with_clipboard(ClipboardPermissions {
                read: true,
                ..Default::default()
            }),
        )
        .await;
        match resp.message {
            Some(Msg::ReadResponse(r)) => {
                // Empty clipboard: entry is None.
                assert!(r.entry.is_none());
            }
            other => panic!("expected ReadResponse, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn subscribe_without_scope_denied() {
        let resp = dispatch_with_profile(
            proto::ClipboardEnvelope {
                message: Some(Msg::SubscribeRequest(proto::SubscribeRequest {})),
            },
            profile_with_clipboard(ClipboardPermissions::default()),
        )
        .await;
        assert!(matches!(resp.message, Some(Msg::Error(_))));
    }

    #[tokio::test]
    async fn history_without_scope_denied_even_with_read() {
        // Foundation: clipboard.history is its own scope —
        // having clipboard.read does NOT grant history.
        let resp = dispatch_with_profile(
            proto::ClipboardEnvelope {
                message: Some(Msg::HistoryRequest(proto::HistoryRequest { limit: 10 })),
            },
            profile_with_clipboard(ClipboardPermissions {
                read: true,
                history: false,
                ..Default::default()
            }),
        )
        .await;
        match resp.message {
            Some(Msg::Error(e)) => {
                assert_eq!(e.kind, proto::ErrorKind::ErrorPermissionDenied as i32);
                assert!(e.detail.contains("clipboard.history"));
            }
            other => panic!("expected Error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn history_with_scope_returns_empty_when_disabled() {
        let resp = dispatch_with_profile(
            proto::ClipboardEnvelope {
                message: Some(Msg::HistoryRequest(proto::HistoryRequest { limit: 10 })),
            },
            profile_with_clipboard(ClipboardPermissions {
                history: true,
                ..Default::default()
            }),
        )
        .await;
        match resp.message {
            Some(Msg::HistoryResponse(r)) => {
                // History store is disabled by default; returns empty.
                assert!(r.entries.is_empty());
            }
            other => panic!("expected HistoryResponse, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn write_without_scope_denied() {
        let resp = dispatch_with_profile(
            proto::ClipboardEnvelope {
                message: Some(Msg::WriteRequest(proto::WriteRequest {
                    content: b"hello".to_vec(),
                    mime: "text/plain".into(),
                    label: 0,
                })),
            },
            profile_with_clipboard(ClipboardPermissions::default()),
        )
        .await;
        match resp.message {
            Some(Msg::Error(e)) => {
                assert_eq!(e.kind, proto::ErrorKind::ErrorPermissionDenied as i32);
                assert!(e.detail.contains("clipboard.write"));
            }
            other => panic!("expected Error, got {other:?}"),
        }
    }

    // Note on the missing "read strips sensitive content" test:
    //
    // The empty-content stripping in `handle_read` and
    // `handle_history` is defense-in-depth — but the current
    // `ClipboardHistory` architecture per FA12 in
    // `docs/architecture/clipboard-api.md` does NOT persist
    // sensitive entries in the history snapshot. Sensitive
    // entries are broadcast to live subscribers and dropped;
    // the snapshot only contains Normal entries.
    //
    // This means the empty-content stripping path on the
    // read+history surfaces is currently unreachable — there is
    // no way to construct a snapshot containing a sensitive
    // entry without changing FA12. The stripping logic is
    // retained as defense-in-depth in case FA12 is relaxed (e.g.
    // a future "sensitive-with-encrypted-history" mode), at
    // which point this test should be added back via direct
    // injection of a sensitive entry into the entries VecDeque.
    //
    // The subscribe-stream surface IS the path where sensitive
    // entries cross the wire today. A subscribe-stripping test
    // would require a broadcast-channel harness around
    // `subscription_task` (out of scope this sprint;
    // peer-auth-system.md FA refers to this as a follow-up).
}

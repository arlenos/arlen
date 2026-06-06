//! shell.search IPC broker.
//!
//! Single-shot Unix-socket broker for `shell.search.open(query, mode)`
//! requests from external Tauri apps. Mirror of `clipboard_ipc.rs`
//! with a much smaller surface — one envelope variant in, one out,
//! connection drops after.
//!
//! Permission gate: `[search] open = true` (foundation §7.3 explicit
//! grant). Auth via `sdk/permissions::ConnectionAuth` — same path as
//! clipboard (SO_PEERCRED + /proc/{pid}/exe + canonical-path match
//! + start_time PID-recycle guard).
//!
//! See `docs/architecture/peer-auth-system.md` for the broker
//! contract and `AUTH-CANONICAL.md` for the system canon. Long-
//! lived handler-registration goes through `arlen-modulesd` and
//! is **not** part of this broker.

use std::os::unix::io::AsRawFd;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::OnceLock;
use std::time::Duration;

use arlen_permissions::ConnectionAuth;
use prost::Message;
use tauri::{AppHandle, Manager};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::Semaphore;
use tokio::time::timeout;

/// Generated protobuf types for the search IPC. Compiled by
/// `build.rs` from `proto/search_api.proto`.
mod proto {
    #![allow(dead_code, clippy::doc_markdown)]
    include!(concat!(env!("OUT_DIR"), "/arlen.search.rs"));
}

const MAX_FRAME_BYTES: usize = 1024 * 1024;
const MAX_QUERY_BYTES: usize = 4096;
const SOCKET_NAME: &str = "search.sock";
const VALID_MODES: &[&str] = &["ai", "files", "apps"];

/// Per-connection read deadline. Single-shot search requests are
/// trivially fast (one envelope, < 4 KB body), so anything slower
/// than this is a stalled or malicious client. Codex post-Sprint
/// review MEDIUM-2 fix: previously connections without a frame
/// could pin a tokio task indefinitely.
const READ_TIMEOUT: Duration = Duration::from_secs(5);

/// Cap on simultaneous in-flight broker connections. Each open()
/// holds one slot for the duration of one envelope round-trip
/// (milliseconds). 32 is generous for the realistic single-app
/// burst case while bounded enough that a misbehaving client
/// cannot exhaust our task pool. Excess accepts are dropped at
/// the listener with a warn line.
const MAX_CONCURRENT_CONNS: usize = 32;

fn semaphore() -> Arc<Semaphore> {
    static SEM: OnceLock<Arc<Semaphore>> = OnceLock::new();
    SEM.get_or_init(|| Arc::new(Semaphore::new(MAX_CONCURRENT_CONNS)))
        .clone()
}

/// Bind the IPC socket and spawn the accept loop. Idempotent: a
/// stale socket from a previous shell crash is removed before bind.
pub fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        match run(app).await {
            Ok(()) => log::info!("search_ipc: shut down cleanly"),
            Err(e) => log::error!("search_ipc: server exited: {e}"),
        }
    });
}

async fn run(app: AppHandle) -> Result<(), String> {
    let path = socket_path().map_err(|e| format!("derive socket path: {e}"))?;
    if path.exists() {
        let _ = std::fs::remove_file(&path);
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let listener =
        UnixListener::bind(&path).map_err(|e| format!("bind {}: {e}", path.display()))?;
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    log::info!("search_ipc: listening on {}", path.display());

    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                // Connection cap (Codex post-Sprint review MEDIUM-2).
                // Try-acquire so we never block the accept loop: if
                // 32 connections are already in flight, drop the new
                // socket. The caller will retry; a flood-attacker
                // gets denial-by-default rather than task exhaustion.
                let permit = match semaphore().try_acquire_owned() {
                    Ok(p) => p,
                    Err(_) => {
                        log::warn!(
                            "search_ipc: connection cap of {MAX_CONCURRENT_CONNS} reached, dropping accept"
                        );
                        drop(stream);
                        continue;
                    }
                };
                let app = app.clone();
                tokio::spawn(async move {
                    let _permit = permit;
                    if let Err(e) = connection_task(stream, app).await {
                        log::warn!("search_ipc: connection task ended: {e}");
                    }
                });
            }
            Err(e) => {
                log::warn!("search_ipc: accept failed: {e}");
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }
}

fn socket_path() -> Result<PathBuf, String> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .ok_or_else(|| "XDG_RUNTIME_DIR not set".to_string())?;
    let mut p = PathBuf::from(runtime);
    p.push("arlen");
    p.push(SOCKET_NAME);
    Ok(p)
}

/// Per-connection driver. Reads one envelope, dispatches, writes
/// response, returns. Single-shot — search is stateless from the
/// app's side, so persistent connections add nothing.
async fn connection_task(stream: UnixStream, app: AppHandle) -> Result<(), String> {
    let caller_uid = unsafe { libc::getuid() };
    let auth = match ConnectionAuth::extract_from(&stream, caller_uid) {
        Ok(a) => a,
        Err(e) => {
            log::warn!("search_ipc: connection rejected at auth: {e}");
            return Err(format!("auth: {e}"));
        }
    };
    log::info!(
        "search_ipc: connection from app_id={} pid={}",
        auth.app_id(),
        auth.pid()
    );

    let (mut reader, writer) = stream.into_split();
    let writer = Arc::new(tokio::sync::Mutex::new(writer));
    let mut buf = Vec::with_capacity(4096);
    let mut chunk = [0u8; 4096];

    // Single-shot: read exactly one envelope, dispatch, return.
    // Wrapped in a read-deadline so a stalled client cannot pin
    // the task indefinitely (Codex review MEDIUM-2). The deadline
    // applies to the time-from-accept-to-first-complete-frame; an
    // open() request fits in one packet and finishes in
    // milliseconds, so 5s is generous.
    let task = async {
        loop {
            let n = reader
                .read(&mut chunk)
                .await
                .map_err(|e| format!("read: {e}"))?;
            if n == 0 {
                return Ok::<(), String>(());
            }
            buf.extend_from_slice(&chunk[..n]);

            if let Some((_, envelope)) = decode_frame(&buf)? {
                if let Err(e) = auth.verify_alive() {
                    log::info!("search_ipc: peer no longer alive: {e}");
                    return Ok(());
                }
                handle_envelope(envelope, &auth, &app, &writer).await;
                return Ok(());
            }
            // Incomplete frame: keep reading. In practice
            // 4-byte length + body fit in one packet.
        }
    };

    match timeout(READ_TIMEOUT, task).await {
        Ok(r) => r,
        Err(_) => {
            log::warn!(
                "search_ipc: read timeout from app_id={} pid={} after {:?} — dropping",
                auth.app_id(),
                auth.pid(),
                READ_TIMEOUT
            );
            Ok(())
        }
    }
}

fn decode_frame(buf: &[u8]) -> Result<Option<(usize, proto::SearchEnvelope)>, String> {
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
    let env = proto::SearchEnvelope::decode(body)
        .map_err(|e| format!("protobuf decode: {e}"))?;
    Ok(Some((4 + len, env)))
}

async fn handle_envelope(
    envelope: proto::SearchEnvelope,
    auth: &ConnectionAuth,
    app: &AppHandle,
    writer: &Arc<tokio::sync::Mutex<tokio::net::unix::OwnedWriteHalf>>,
) {
    use proto::search_envelope::Message as Msg;
    let Some(msg) = envelope.message else {
        return;
    };
    match msg {
        Msg::OpenRequest(req) => {
            handle_open(req, auth, app, writer).await;
        }
        // Server-side messages (response variants) are protocol
        // violations from the client direction. Drop silently.
        Msg::OpenResponse(_) | Msg::Error(_) => {
            log::warn!(
                "search_ipc: unexpected server-direction envelope from app_id={}",
                auth.app_id()
            );
        }
    }
}

async fn handle_open(
    req: proto::OpenRequest,
    auth: &ConnectionAuth,
    app: &AppHandle,
    writer: &Arc<tokio::sync::Mutex<tokio::net::unix::OwnedWriteHalf>>,
) {
    // 1. Permission check (foundation §7.3 explicit-grant).
    if !auth.profile().search.open {
        audit_deny(auth, "open", "search.open");
        let _ = send_error(writer, proto::ErrorKind::ErrorPermissionDenied,
            "missing search.open scope").await;
        return;
    }
    audit_grant(auth, "open", "search.open");

    // 2. Query validation (length + control-char strip). We do NOT
    // log the query content under any circumstance — it can carry
    // user-typed secrets that the audit trail must not capture.
    if req.query.len() > MAX_QUERY_BYTES {
        let _ = send_error(writer, proto::ErrorKind::ErrorQueryTooLarge,
            &format!("{} bytes > {MAX_QUERY_BYTES} max", req.query.len())).await;
        return;
    }
    let (sanitized_query, stripped) = sanitize_query(&req.query);

    // 3. Mode allowlist (silent strip on unknown — forward-compat).
    let mode = normalize_mode(&req.mode);

    // 4. Hand to the launcher. set_query_and_show is idempotent
    // (already-visible → update input; hidden → show with prefill).
    if app.get_webview_window("waypointer").is_none() {
        let _ = send_error(writer, proto::ErrorKind::ErrorWindowNotReady,
            "waypointer window not yet created").await;
        return;
    }
    let prefilled = build_prefilled(&sanitized_query, &mode);
    if let Err(e) = crate::waypointer::set_query_and_show(
        app.clone(),
        prefilled,
        String::new(),
    ) {
        let _ = send_error(writer, proto::ErrorKind::ErrorInternal, &e).await;
        return;
    }

    if stripped > 0 {
        // Audit-log the strip without leaking the original query.
        log::info!(
            "search_ipc: stripped {stripped} control char(s) from app_id={} query",
            auth.app_id()
        );
    }

    let _ = send_envelope(writer, proto::SearchEnvelope {
        message: Some(proto::search_envelope::Message::OpenResponse(
            proto::OpenResponse {},
        )),
    }).await;
}

/// Returns the input mode if it is on the allowlist (`ai`,
/// `files`, `apps`), otherwise an empty string. Foundation §6.4
/// allowlist; unknown modes are silent-mapped (forward-compat).
fn normalize_mode(input: &str) -> String {
    if VALID_MODES.contains(&input) {
        input.to_string()
    } else {
        String::new()
    }
}

/// Construct the prefilled launcher string from a sanitized query
/// + normalized mode. Mode-empty → query unchanged. Mode-set →
/// `"<mode>: <query>"` so plugins that opt in to mode-aware
/// matching can detect the prefix.
fn build_prefilled(query: &str, mode: &str) -> String {
    if mode.is_empty() {
        query.to_string()
    } else {
        format!("{mode}: {query}")
    }
}

/// Strip ASCII control characters (0x00-0x1F minus \t, plus 0x7F)
/// from the query. Returns the sanitised query and the count of
/// stripped characters. Newlines included — single-line input.
fn sanitize_query(input: &str) -> (String, usize) {
    let mut out = String::with_capacity(input.len());
    let mut stripped = 0usize;
    for c in input.chars() {
        let code = c as u32;
        let is_control = (code <= 0x1F && c != '\t') || code == 0x7F;
        if is_control {
            stripped += 1;
        } else {
            out.push(c);
        }
    }
    (out, stripped)
}

async fn send_envelope(
    writer: &Arc<tokio::sync::Mutex<tokio::net::unix::OwnedWriteHalf>>,
    envelope: proto::SearchEnvelope,
) -> std::io::Result<()> {
    let body = envelope.encode_to_vec();
    let len = (body.len() as u32).to_be_bytes();
    let mut w = writer.lock().await;
    w.write_all(&len).await?;
    w.write_all(&body).await?;
    Ok(())
}

async fn send_error(
    writer: &Arc<tokio::sync::Mutex<tokio::net::unix::OwnedWriteHalf>>,
    kind: proto::ErrorKind,
    detail: &str,
) -> std::io::Result<()> {
    let env = proto::SearchEnvelope {
        message: Some(proto::search_envelope::Message::Error(proto::SearchError {
            kind: kind as i32,
            detail: detail.to_string(),
        })),
    };
    send_envelope(writer, env).await
}

/// Audit log emission (foundation §8.4.7). Per-(app, scope)
/// rate-limit applied. Query content is **never** logged — it can
/// carry user-typed secrets.
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
    log::warn!(
        "[AUDIT] perm.deny app={} pid={} scope={} surface={} result=denied reason=missing_scope",
        auth.app_id(),
        auth.pid(),
        scope,
        surface
    );
}

fn audit_should_emit(app_id: &str, scope: &'static str) -> bool {
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    static LEDGER: std::sync::OnceLock<
        Mutex<std::collections::HashMap<(String, &'static str), Instant>>,
    > = std::sync::OnceLock::new();
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

// Suppress unused warning for AsRawFd import — used implicitly by
// ConnectionAuth::extract_from(&stream, ...) via the fd.
const _: fn() = || {
    fn _check<T: AsRawFd>(_: &T) {}
};

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_strips_null_byte() {
        let (out, n) = sanitize_query("foo\0bar");
        assert_eq!(out, "foobar");
        assert_eq!(n, 1);
    }

    #[test]
    fn sanitize_strips_newlines() {
        let (out, n) = sanitize_query("line1\nline2\rline3");
        assert_eq!(out, "line1line2line3");
        assert_eq!(n, 2);
    }

    #[test]
    fn sanitize_keeps_tab() {
        // Tab is intentionally allowed — shell-prefix UX uses it.
        let (out, n) = sanitize_query("foo\tbar");
        assert_eq!(out, "foo\tbar");
        assert_eq!(n, 0);
    }

    #[test]
    fn sanitize_strips_del() {
        let (out, n) = sanitize_query("foo\x7Fbar");
        assert_eq!(out, "foobar");
        assert_eq!(n, 1);
    }

    #[test]
    fn sanitize_keeps_unicode() {
        let (out, n) = sanitize_query("café 日本語 🎉");
        assert_eq!(out, "café 日本語 🎉");
        assert_eq!(n, 0);
    }

    #[test]
    fn valid_modes_does_not_include_unknown() {
        assert!(VALID_MODES.contains(&"ai"));
        assert!(VALID_MODES.contains(&"files"));
        assert!(VALID_MODES.contains(&"apps"));
        assert!(!VALID_MODES.contains(&"random"));
        assert!(!VALID_MODES.contains(&""));
    }

    #[test]
    fn normalize_mode_passes_known_values() {
        assert_eq!(normalize_mode("ai"), "ai");
        assert_eq!(normalize_mode("files"), "files");
        assert_eq!(normalize_mode("apps"), "apps");
    }

    #[test]
    fn normalize_mode_strips_unknown() {
        assert_eq!(normalize_mode(""), "");
        assert_eq!(normalize_mode("FILES"), ""); // case-sensitive
        assert_eq!(normalize_mode("zzz"), "");
        assert_eq!(normalize_mode("../../../etc"), "");
    }

    #[test]
    fn build_prefilled_no_mode_passes_through() {
        assert_eq!(build_prefilled("hello", ""), "hello");
        assert_eq!(build_prefilled("", ""), "");
    }

    #[test]
    fn build_prefilled_with_mode_prepends() {
        assert_eq!(build_prefilled("foo", "files"), "files: foo");
        assert_eq!(build_prefilled("", "ai"), "ai: ");
        assert_eq!(build_prefilled("café", "apps"), "apps: café");
    }

    /// Audit-log lines must NEVER include query content. The
    /// guard is a code-review checkpoint (audit_grant signature
    /// takes only `surface: &str` and `scope: &'static str`,
    /// never the user query). This test asserts the contract
    /// at the type level: if anyone changes audit_grant to take
    /// the query, this will fail to compile.
    #[test]
    fn audit_grant_signature_excludes_query() {
        // Marker test: audit_grant takes (auth, surface, scope).
        // No `query` argument exists. Adding one would break
        // this signature check.
        let _: fn(&ConnectionAuth, &str, &'static str) = audit_grant;
    }

    /// MAX_QUERY_BYTES is enforced at the byte level (post-UTF-8
    /// validation by prost). 4096 chosen to comfortably fit any
    /// reasonable Waypointer query while bounding plugin search
    /// memory.
    #[test]
    fn max_query_bytes_constant() {
        assert_eq!(MAX_QUERY_BYTES, 4096);
    }

    /// Codex review MEDIUM-2: a stalled or malicious authenticated
    /// client must not pin the broker task indefinitely. The
    /// deadline guards both partial-frame and idle-after-accept.
    #[test]
    fn read_timeout_is_bounded() {
        assert!(READ_TIMEOUT >= Duration::from_secs(1));
        assert!(READ_TIMEOUT <= Duration::from_secs(30));
    }

    /// Codex review MEDIUM-2: connection cap protects against
    /// fd-exhaustion + task-pool DoS from a single misbehaving
    /// client. 32 is high enough for legitimate burst use, low
    /// enough that the OS doesn't notice if every slot stalls.
    #[test]
    fn connection_cap_is_finite() {
        assert!(MAX_CONCURRENT_CONNS > 0);
        assert!(MAX_CONCURRENT_CONNS <= 256);
    }

    /// The semaphore is lazy-init'd via OnceLock — repeated calls
    /// must return the same instance so accept-loop and tests see
    /// the same permit count.
    #[test]
    fn semaphore_is_shared_singleton() {
        let s1 = semaphore();
        let s2 = semaphore();
        assert_eq!(Arc::as_ptr(&s1), Arc::as_ptr(&s2));
        // Initial available permits = MAX_CONCURRENT_CONNS minus
        // any taken by other tests in this module (none currently).
        assert!(s1.available_permits() <= MAX_CONCURRENT_CONNS);
    }
}

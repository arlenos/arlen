//! shell.intents IPC broker.
//!
//! Single-shot Unix-socket broker for `shell.intents.dispatch`
//! requests from external Tauri apps. Mirror of `clipboard_ipc.rs`
//! and `search_ipc.rs` patterns, with type-specific Phase-6
//! dispatch built-ins for url, file, text, email, project.
//!
//! Permission gate: `[intents] dispatch = true` (foundation §7.3
//! explicit grant). Auth via `sdk/permissions::ConnectionAuth` —
//! same path as clipboard / search (SO_PEERCRED + /proc/{pid}/exe
//! + canonical-path match + start_time PID-recycle guard).
//!
//! See `docs/architecture/intent-system.md` for the broker
//! contract, `peer-auth-system.md` for the system canon, and
//! `module-system.md` for the Phase-7 register-half cross-ref.

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

/// Generated protobuf types for the intent IPC. Compiled by
/// `build.rs` from `proto/intent_api.proto`.
mod proto {
    #![allow(dead_code, clippy::doc_markdown)]
    include!(concat!(env!("OUT_DIR"), "/arlen.intents.rs"));
}

const MAX_FRAME_BYTES: usize = 1024 * 1024;
const MAX_TEXT_BYTES: usize = 64 * 1024;
const SOCKET_NAME: &str = "intents.sock";
const READ_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_CONCURRENT_CONNS: usize = 32;
const VALID_TYPES: &[&str] = &["url", "file", "text", "email", "project"];

fn semaphore() -> Arc<Semaphore> {
    static SEM: OnceLock<Arc<Semaphore>> = OnceLock::new();
    SEM.get_or_init(|| Arc::new(Semaphore::new(MAX_CONCURRENT_CONNS)))
        .clone()
}

/// Bind the IPC socket and spawn the accept loop. Idempotent.
pub fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        match run(app).await {
            Ok(()) => log::info!("intent_ipc: shut down cleanly"),
            Err(e) => log::error!("intent_ipc: server exited: {e}"),
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
    log::info!("intent_ipc: listening on {}", path.display());

    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                let permit = match semaphore().try_acquire_owned() {
                    Ok(p) => p,
                    Err(_) => {
                        log::warn!(
                            "intent_ipc: connection cap of {MAX_CONCURRENT_CONNS} reached, dropping accept"
                        );
                        drop(stream);
                        continue;
                    }
                };
                let app = app.clone();
                tokio::spawn(async move {
                    let _permit = permit;
                    if let Err(e) = connection_task(stream, app).await {
                        log::warn!("intent_ipc: connection task ended: {e}");
                    }
                });
            }
            Err(e) => {
                log::warn!("intent_ipc: accept failed: {e}");
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

/// Per-connection driver. Reads exactly one envelope, dispatches,
/// returns. Single-shot — intents are stateless from the app's
/// perspective.
async fn connection_task(stream: UnixStream, app: AppHandle) -> Result<(), String> {
    let caller_uid = unsafe { libc::getuid() };
    let auth = match ConnectionAuth::extract_from(&stream, caller_uid) {
        Ok(a) => a,
        Err(e) => {
            log::warn!("intent_ipc: connection rejected at auth: {e}");
            return Err(format!("auth: {e}"));
        }
    };
    log::info!(
        "intent_ipc: connection from app_id={} pid={}",
        auth.app_id(),
        auth.pid()
    );

    let (mut reader, writer) = stream.into_split();
    let writer = Arc::new(tokio::sync::Mutex::new(writer));
    let mut buf = Vec::with_capacity(4096);
    let mut chunk = [0u8; 4096];

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
                    log::info!("intent_ipc: peer no longer alive: {e}");
                    return Ok(());
                }
                handle_envelope(envelope, &auth, &app, &writer).await;
                return Ok(());
            }
        }
    };

    match timeout(READ_TIMEOUT, task).await {
        Ok(r) => r,
        Err(_) => {
            log::warn!(
                "intent_ipc: read timeout from app_id={} pid={} after {READ_TIMEOUT:?} — dropping",
                auth.app_id(),
                auth.pid()
            );
            Ok(())
        }
    }
}

fn decode_frame(buf: &[u8]) -> Result<Option<(usize, proto::IntentEnvelope)>, String> {
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
    let env = proto::IntentEnvelope::decode(body)
        .map_err(|e| format!("protobuf decode: {e}"))?;
    Ok(Some((4 + len, env)))
}

async fn handle_envelope(
    envelope: proto::IntentEnvelope,
    auth: &ConnectionAuth,
    app: &AppHandle,
    writer: &Arc<tokio::sync::Mutex<tokio::net::unix::OwnedWriteHalf>>,
) {
    use proto::intent_envelope::Message as Msg;
    let Some(msg) = envelope.message else {
        return;
    };
    match msg {
        Msg::DispatchRequest(req) => {
            handle_dispatch(req, auth, app, writer).await;
        }
        Msg::DispatchResponse(_) | Msg::Error(_) => {
            log::warn!(
                "intent_ipc: unexpected server-direction envelope from app_id={}",
                auth.app_id()
            );
        }
    }
}

async fn handle_dispatch(
    req: proto::DispatchRequest,
    auth: &ConnectionAuth,
    app: &AppHandle,
    writer: &Arc<tokio::sync::Mutex<tokio::net::unix::OwnedWriteHalf>>,
) {
    if !auth.profile().intents.dispatch {
        audit_deny(auth, "dispatch", "intents.dispatch");
        let _ = send_error(
            writer,
            proto::ErrorKind::ErrorPermissionDenied,
            "missing intents.dispatch scope",
        )
        .await;
        return;
    }
    audit_grant(auth, "dispatch", "intents.dispatch");

    if !VALID_TYPES.contains(&req.r#type.as_str()) {
        let _ = send_error(
            writer,
            proto::ErrorKind::ErrorUnknownType,
            &format!("unknown intent type: {}", req.r#type),
        )
        .await;
        return;
    }

    let data_str = match std::str::from_utf8(&req.data) {
        Ok(s) => s,
        Err(_) => {
            let _ = send_error(
                writer,
                proto::ErrorKind::ErrorInvalidData,
                "data is not valid UTF-8",
            )
            .await;
            return;
        }
    };

    let (handler_id, outcome_or_err) = match req.r#type.as_str() {
        "url" => ("builtin.url", dispatch_url(data_str).await),
        "file" => ("builtin.file", dispatch_file(data_str).await),
        "text" => ("builtin.text", dispatch_text(data_str, app).await),
        "email" => ("builtin.email", dispatch_email(data_str).await),
        "project" => ("builtin.project", dispatch_project(data_str, app).await),
        _ => unreachable!("type guarded by VALID_TYPES check above"),
    };

    match outcome_or_err {
        Ok(outcome) => {
            crate::event_bus::emit_intent_dispatched(
                auth.app_id(),
                &req.action,
                &req.r#type,
                handler_id,
            );
            let _ = send_envelope(
                writer,
                proto::IntentEnvelope {
                    message: Some(proto::intent_envelope::Message::DispatchResponse(
                        proto::DispatchResponse {
                            handler: handler_id.to_string(),
                            outcome,
                        },
                    )),
                },
            )
            .await;
        }
        Err((kind, detail)) => {
            let _ = send_error(writer, kind, &detail).await;
        }
    }
}

// ---------------------------------------------------------------------------
// Built-in dispatchers
// ---------------------------------------------------------------------------

/// `url` intent → xdg-open via existing `shell_runner::open_url`.
/// Arlen-internal schemes rejected defensively; the OS-level
/// scheme allowlist is the portal's job (xdg-desktop-portal-
/// arlen OpenURI; see intent-system.md §5).
async fn dispatch_url(data: &str) -> Result<String, (proto::ErrorKind, String)> {
    if data.is_empty() {
        return Err((proto::ErrorKind::ErrorInvalidData, "url is empty".into()));
    }
    if is_rejected_scheme(data) {
        return Err((
            proto::ErrorKind::ErrorInvalidData,
            format!(
                "scheme rejected (defensive allowlist): {}",
                first_scheme(data)
            ),
        ));
    }
    crate::shell_runner::open_url(data.to_string());
    Ok(String::new())
}

/// System directories that are always rejected for `file` intents
/// regardless of the calling app's filesystem scope. These are
/// either Arlen-internal (`/run/arlen/`) or kernel pseudo-fs
/// (`/proc`, `/sys`, `/dev`) where opening with the user's default
/// app makes no sense and would surface internal state to the
/// xdg-mime handler resolution layer. Mirrors the eBPF normalizer's
/// system-path blocklist (`kernel-layer::normalizer::FILTER_PATHS`).
const FILE_DISPATCH_BLOCKLIST: &[&str] = &[
    "/proc/",
    "/sys/",
    "/dev/",
    "/run/arlen/",
];

/// `file` intent → xdg-open canonicalised path.
///
/// Phase-6 broker-side defenses (Codex post-Sprint review HIGH-1):
/// the original implementation called `xdg-open` on a raw user-
/// supplied path which bypassed the path-traversal guards the
/// spec promised. Phase 6 now does:
///
/// 1. absolute-path check (unchanged)
/// 2. `fs::canonicalize` to resolve `..` segments + symlinks; this
///    is what the portal would do for sandboxed callers
/// 3. system-directory blocklist (`/proc`, `/sys`, `/dev`,
///    `/run/arlen/`) — same blocklist eBPF uses
/// 4. existence check (free fallout from canonicalize)
/// 5. xdg-open the canonicalised path
///
/// Phase 7 will add filesystem-scope enforcement against the
/// caller's `[filesystem]` permission profile or the AppArmor
/// profile (whichever lands from the F3 bundle in
/// `identity-spoof-mitigation.md`). Until then, an app with
/// `[intents] dispatch = true` can open any user-readable file
/// outside the system blocklist — same blast as `xdg-open` direct
/// from any same-uid process.
async fn dispatch_file(data: &str) -> Result<String, (proto::ErrorKind, String)> {
    if data.is_empty() {
        return Err((
            proto::ErrorKind::ErrorInvalidData,
            "file path is empty".into(),
        ));
    }
    let raw = data.strip_prefix("file://").unwrap_or(data);
    if !raw.starts_with('/') {
        return Err((
            proto::ErrorKind::ErrorInvalidData,
            "file path must be absolute".into(),
        ));
    }

    // Canonicalize: resolves symlinks + `..` segments and returns
    // an Err for missing files. One call replaces the previous
    // separate exists() check, gives us a normalised path for the
    // blocklist comparison, and closes path-traversal vectors
    // (`/etc/foo/../passwd`, symlink-via-tmpfs into /etc, etc.).
    let canonical = std::fs::canonicalize(raw).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => {
            (proto::ErrorKind::ErrorNotFound, "file does not exist".into())
        }
        _ => (
            proto::ErrorKind::ErrorInvalidData,
            format!("canonicalize: {e}"),
        ),
    })?;
    let canonical_str = canonical.to_string_lossy();

    if FILE_DISPATCH_BLOCKLIST
        .iter()
        .any(|prefix| canonical_str.starts_with(prefix))
    {
        return Err((
            proto::ErrorKind::ErrorInvalidData,
            format!(
                "path resolves to system-internal directory (blocklist): {canonical_str}"
            ),
        ));
    }

    let _ = std::process::Command::new("xdg-open")
        .arg(canonical.as_os_str())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| {
            (
                proto::ErrorKind::ErrorNoHandler,
                format!("xdg-open spawn failed: {e}"),
            )
        })?;
    Ok(String::new())
}

/// `text` intent → write to clipboard via existing
/// `clipboard_history::write_with_label`. Returns empty outcome
/// (the API doesn't return an entry id; we don't fabricate one).
async fn dispatch_text(
    data: &str,
    app: &AppHandle,
) -> Result<String, (proto::ErrorKind, String)> {
    if data.len() > MAX_TEXT_BYTES {
        return Err((
            proto::ErrorKind::ErrorDataTooLarge,
            format!("text {} bytes > {MAX_TEXT_BYTES} max", data.len()),
        ));
    }
    let history: tauri::State<'_, std::sync::Arc<crate::clipboard_history::ClipboardHistory>> =
        app.state();
    history
        .write_with_label(
            data.to_string(),
            crate::clipboard_history::Label::Normal,
            String::new(),
        )
        .map_err(|e| {
            (
                proto::ErrorKind::ErrorInternal,
                format!("clipboard write failed: {e}"),
            )
        })?;
    Ok(String::new())
}

/// `email` intent → mailto: URL forwarded to xdg-open via
/// `shell_runner::open_url`. Defensive scheme check matches `url`.
async fn dispatch_email(data: &str) -> Result<String, (proto::ErrorKind, String)> {
    if !data.starts_with("mailto:") {
        return Err((
            proto::ErrorKind::ErrorInvalidData,
            "email data must be a mailto: URI".into(),
        ));
    }
    crate::shell_runner::open_url(data.to_string());
    Ok(String::new())
}

/// `project` intent → activate Focus Mode. Looks up project_id in
/// the graph, then calls the existing `projects::activate_focus`
/// path so the canonical `focus.activated` Event-Bus emission +
/// shell.toml persistence + accent-colour override all fire.
/// Returns the project name as outcome.
async fn dispatch_project(
    data: &str,
    app: &AppHandle,
) -> Result<String, (proto::ErrorKind, String)> {
    if data.is_empty() {
        return Err((
            proto::ErrorKind::ErrorInvalidData,
            "project_id is empty".into(),
        ));
    }
    let project = match crate::projects::get_project(data.to_string()).await {
        Ok(Some(p)) => p,
        Ok(None) => {
            return Err((
                proto::ErrorKind::ErrorNotFound,
                format!("project_id {data} not in graph"),
            ));
        }
        Err(e) => {
            return Err((
                proto::ErrorKind::ErrorInternal,
                format!("graph query failed: {e}"),
            ));
        }
    };
    let state: tauri::State<'_, std::sync::Arc<crate::projects::ProjectsState>> = app.state();
    let name = project.name.clone();
    crate::projects::activate_focus(
        project.id,
        project.name,
        project.root_path,
        project.accent_color,
        state,
        app.clone(),
    )
    .await
    .map_err(|e| (proto::ErrorKind::ErrorInternal, e))?;
    Ok(name)
}

fn first_scheme(s: &str) -> &str {
    match s.find(':') {
        Some(i) => &s[..i],
        None => s,
    }
}

/// Defensive scheme rejection. URI schemes are **case-insensitive
/// per RFC 3986** so the comparison must lowercase before matching;
/// otherwise `JaVaScRiPt:alert(1)` would silently bypass the
/// allowlist (Codex post-Sprint review HIGH-2). `file:` is also
/// rejected here because url callers must use the `file` intent
/// type for local paths — that path runs `fs::canonicalize` and
/// checks against the system-internal directory blocklist; routing
/// `file://` through `dispatch_url` would skip those guards.
fn is_rejected_scheme(s: &str) -> bool {
    let scheme = match s.find(':') {
        Some(i) => &s[..i],
        None => return false,
    };
    let lower = scheme.to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "arlen" | "javascript" | "data" | "vbscript" | "file"
    )
}

// ---------------------------------------------------------------------------
// Wire helpers
// ---------------------------------------------------------------------------

async fn send_envelope(
    writer: &Arc<tokio::sync::Mutex<tokio::net::unix::OwnedWriteHalf>>,
    envelope: proto::IntentEnvelope,
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
    let env = proto::IntentEnvelope {
        message: Some(proto::intent_envelope::Message::Error(proto::IntentError {
            kind: kind as i32,
            detail: detail.to_string(),
        })),
    };
    send_envelope(writer, env).await
}

// ---------------------------------------------------------------------------
// Audit log
// ---------------------------------------------------------------------------

/// **Signature deliberately excludes the `data` field** —
/// `data` carries user-supplied content (URLs with session tokens,
/// mailto bodies, project ids that may be sensitive). Compile-
/// time guard mirrors the search broker's signature test.
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

// AsRawFd marker — used implicitly by ConnectionAuth::extract_from.
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
    fn valid_types_full_set() {
        assert!(VALID_TYPES.contains(&"url"));
        assert!(VALID_TYPES.contains(&"file"));
        assert!(VALID_TYPES.contains(&"text"));
        assert!(VALID_TYPES.contains(&"email"));
        assert!(VALID_TYPES.contains(&"project"));
        assert!(!VALID_TYPES.contains(&""));
        assert!(!VALID_TYPES.contains(&"unknown"));
    }

    #[test]
    fn first_scheme_strips_at_first_colon() {
        assert_eq!(first_scheme("https://example.com"), "https");
        assert_eq!(first_scheme("javascript:alert(1)"), "javascript");
        assert_eq!(first_scheme("noscheme"), "noscheme");
    }

    #[test]
    fn is_rejected_scheme_blocks_dangerous_uris() {
        assert!(is_rejected_scheme("javascript:alert(1)"));
        assert!(is_rejected_scheme("data:text/html,<x>"));
        assert!(is_rejected_scheme("vbscript:msgbox"));
        assert!(is_rejected_scheme("arlen://shell-internal"));
        // file:// rejected on url path — must use `file` intent
        // type which runs canonicalize + system-dir blocklist.
        assert!(is_rejected_scheme("file:///tmp/x"));
        assert!(!is_rejected_scheme("https://example.com"));
        assert!(!is_rejected_scheme("mailto:foo@bar.com"));
    }

    /// Codex review HIGH-2: mixed-case URI schemes must be
    /// rejected. RFC 3986 §3.1 states schemes are
    /// case-insensitive, so the previous case-sensitive
    /// `starts_with` check let `JaVaScRiPt:alert(1)` silently
    /// pass through to xdg-open.
    #[test]
    fn is_rejected_scheme_handles_mixed_case() {
        assert!(is_rejected_scheme("JaVaScRiPt:alert(1)"));
        assert!(is_rejected_scheme("JAVASCRIPT:alert(1)"));
        assert!(is_rejected_scheme("Arlen://shell-internal"));
        assert!(is_rejected_scheme("LUNARIS://shell-internal"));
        assert!(is_rejected_scheme("DATA:text/html"));
        assert!(is_rejected_scheme("VbScRiPt:foo"));
        assert!(is_rejected_scheme("FILE:///tmp/x"));
    }

    /// No-scheme strings must not be flagged as rejected.
    #[test]
    fn is_rejected_scheme_no_colon() {
        assert!(!is_rejected_scheme(""));
        assert!(!is_rejected_scheme("noscheme"));
    }

    #[tokio::test]
    async fn url_rejects_mixed_case_javascript() {
        let r = dispatch_url("JaVaScRiPt:alert(1)").await;
        assert!(matches!(r, Err((proto::ErrorKind::ErrorInvalidData, _))));
    }

    #[tokio::test]
    async fn url_rejects_file_scheme_forcing_file_intent_type() {
        let r = dispatch_url("file:///tmp/x").await;
        assert!(matches!(r, Err((proto::ErrorKind::ErrorInvalidData, _))));
    }

    #[tokio::test]
    async fn url_rejects_javascript_scheme() {
        let r = dispatch_url("javascript:alert(1)").await;
        assert!(matches!(r, Err((proto::ErrorKind::ErrorInvalidData, _))));
    }

    #[tokio::test]
    async fn url_rejects_arlen_scheme() {
        let r = dispatch_url("arlen://shell-internal").await;
        assert!(matches!(r, Err((proto::ErrorKind::ErrorInvalidData, _))));
    }

    #[tokio::test]
    async fn url_rejects_empty() {
        let r = dispatch_url("").await;
        assert!(matches!(r, Err((proto::ErrorKind::ErrorInvalidData, _))));
    }

    #[tokio::test]
    async fn file_rejects_relative_path() {
        let r = dispatch_file("relative/path.txt").await;
        assert!(matches!(r, Err((proto::ErrorKind::ErrorInvalidData, _))));
    }

    #[tokio::test]
    async fn file_rejects_empty() {
        let r = dispatch_file("").await;
        assert!(matches!(r, Err((proto::ErrorKind::ErrorInvalidData, _))));
    }

    #[tokio::test]
    async fn file_rejects_nonexistent_absolute() {
        let r = dispatch_file("/this/path/should/not/exist/nope").await;
        assert!(matches!(r, Err((proto::ErrorKind::ErrorNotFound, _))));
    }

    /// Codex review HIGH-1: file dispatch must canonicalise the
    /// path before opening so `..` traversal + tmpfs-symlink
    /// rebasing cannot escape the user's intended target.
    #[tokio::test]
    async fn file_rejects_proc_self() {
        let r = dispatch_file("/proc/self/status").await;
        assert!(matches!(r, Err((proto::ErrorKind::ErrorInvalidData, _))));
        let detail = match r {
            Err((_, d)) => d,
            _ => unreachable!(),
        };
        assert!(
            detail.contains("blocklist"),
            "expected blocklist rejection, got: {detail}"
        );
    }

    /// Same blocklist for /sys/. Even if the kernel exposes a
    /// regular file there, opening it with the user's default
    /// app would surface internal kernel state.
    #[tokio::test]
    async fn file_rejects_sys_class() {
        // /sys/class/net/lo exists on Linux; canonicalise + blocklist.
        let path = "/sys/class/net/lo";
        if !std::path::Path::new(path).exists() {
            return; // skip on non-Linux or unusual systems
        }
        let r = dispatch_file(path).await;
        assert!(matches!(r, Err((proto::ErrorKind::ErrorInvalidData, _))));
    }

    /// Path-traversal via `..` segments: even if the user supplied
    /// `/tmp/foo/../etc/passwd`, canonicalize resolves to the
    /// real target before the blocklist check fires. /etc itself
    /// is NOT on the blocklist (xdg-open `/etc/hosts` is the same
    /// as user double-clicking it), but /proc IS.
    #[tokio::test]
    async fn file_traversal_via_dotdot_canonicalises() {
        let traversal = "/tmp/../proc/self/status";
        let r = dispatch_file(traversal).await;
        assert!(matches!(r, Err((proto::ErrorKind::ErrorInvalidData, _))));
    }

    #[test]
    fn file_dispatch_blocklist_constants() {
        assert!(FILE_DISPATCH_BLOCKLIST.contains(&"/proc/"));
        assert!(FILE_DISPATCH_BLOCKLIST.contains(&"/sys/"));
        assert!(FILE_DISPATCH_BLOCKLIST.contains(&"/dev/"));
        assert!(FILE_DISPATCH_BLOCKLIST.contains(&"/run/arlen/"));
    }

    #[tokio::test]
    async fn email_rejects_non_mailto() {
        let r = dispatch_email("https://example.com").await;
        assert!(matches!(r, Err((proto::ErrorKind::ErrorInvalidData, _))));
    }

    #[test]
    fn read_timeout_is_bounded() {
        assert!(READ_TIMEOUT >= Duration::from_secs(1));
        assert!(READ_TIMEOUT <= Duration::from_secs(30));
    }

    #[test]
    fn connection_cap_is_finite() {
        assert!(MAX_CONCURRENT_CONNS > 0);
        assert!(MAX_CONCURRENT_CONNS <= 256);
    }

    #[test]
    fn max_text_bytes_constant() {
        assert_eq!(MAX_TEXT_BYTES, 64 * 1024);
    }

    /// Audit grant signature must not accept the data field.
    /// Compile-time guard against future refactors that would
    /// re-introduce a data-leak path.
    #[test]
    fn audit_grant_signature_excludes_data() {
        let _: fn(&ConnectionAuth, &str, &'static str) = audit_grant;
    }

    /// Semaphore singleton — repeated calls return the same Arc.
    /// Codex parity check with search_ipc + clipboard_ipc.
    #[test]
    fn semaphore_is_shared_singleton() {
        let s1 = semaphore();
        let s2 = semaphore();
        assert_eq!(Arc::as_ptr(&s1), Arc::as_ptr(&s2));
        assert!(s1.available_permits() <= MAX_CONCURRENT_CONNS);
    }

    /// `text` intent enforces a 64 KB byte cap. Larger payloads
    /// would otherwise let an attacker fill clipboard memory via
    /// authenticated dispatch.
    #[tokio::test]
    async fn text_rejects_oversized_payload() {
        // Build a String just past MAX_TEXT_BYTES. Construct a real
        // AppHandle for unit tests is non-trivial, but the size
        // check happens before the AppHandle is touched.
        // We exercise the gate via a tiny helper that mirrors the
        // real check without the AppHandle dependency.
        fn size_check(data: &[u8]) -> Result<(), (proto::ErrorKind, String)> {
            if data.len() > MAX_TEXT_BYTES {
                Err((
                    proto::ErrorKind::ErrorDataTooLarge,
                    format!("text {} bytes > {MAX_TEXT_BYTES} max", data.len()),
                ))
            } else {
                Ok(())
            }
        }
        let oversized = vec![b'x'; MAX_TEXT_BYTES + 1];
        assert!(matches!(
            size_check(&oversized),
            Err((proto::ErrorKind::ErrorDataTooLarge, _))
        ));
        let undersized = vec![b'x'; MAX_TEXT_BYTES];
        assert!(size_check(&undersized).is_ok());
    }
}

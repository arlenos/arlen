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

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use prost::Message;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};

use crate::clipboard_history::{ClipboardHistory, Label};

/// Generated protobuf types for the clipboard IPC. Compiled by
/// `build.rs` from `proto/clipboard_api.proto`.
mod proto {
    #![allow(dead_code, clippy::doc_markdown)]
    include!(concat!(env!("OUT_DIR"), "/lunaris.clipboard.rs"));
}

const MAX_FRAME_BYTES: usize = 1024 * 1024;
const SOCKET_NAME: &str = "clipboard.sock";

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
                let history = history.clone();
                tokio::spawn(async move {
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
    let (mut reader, writer) = stream.into_split();
    let writer = Arc::new(tokio::sync::Mutex::new(writer));
    let mut buf = Vec::with_capacity(4096);
    let mut chunk = [0u8; 4096];

    loop {
        let n = reader
            .read(&mut chunk)
            .await
            .map_err(|e| format!("read: {e}"))?;
        if n == 0 {
            return Ok(());
        }
        buf.extend_from_slice(&chunk[..n]);

        while let Some((consumed, envelope)) = decode_frame(&buf)? {
            buf.drain(..consumed);
            handle_envelope(envelope, history.clone(), writer.clone()).await;
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
) {
    use proto::clipboard_envelope::Message as Msg;
    let Some(msg) = envelope.message else {
        return;
    };
    match msg {
        Msg::WriteRequest(req) => {
            let outcome = handle_write(&history, req).await;
            let resp = match outcome {
                Ok(entry) => proto::ClipboardEnvelope {
                    message: Some(Msg::WriteResponse(proto::WriteResponse { entry })),
                },
                Err(err) => error_envelope(err),
            };
            let _ = send_envelope(&writer, resp).await;
        }
        Msg::ReadRequest(_) | Msg::SubscribeRequest(_) | Msg::HistoryRequest(_) => {
            // Phase 1 hard-disables every read-side surface. The
            // socket has no peer authentication yet, and the
            // history layer cannot enforce permission scopes per
            // caller. Until Phase 6 ships SO_PEERCRED-based
            // identity + capability-token scope checks, returning
            // these handlers as `PermissionDenied` is the only
            // safe default — exposing them would (a) leak history
            // beyond what the bare Wayland clipboard reveals and
            // (b) broadcast `LABEL_SENSITIVE` content verbatim to
            // any unauthenticated subscriber, both of which break
            // the contract documented in
            // `docs/architecture/clipboard-api.md` (FA2, FA10,
            // FA12).
            let resp = error_envelope(ProtoError {
                kind: proto::ErrorKind::ErrorPermissionDenied,
                detail: "clipboard read/subscribe/history is disabled in phase 1; \
                         re-enabled in Phase 6 once peer authentication and \
                         capability-token enforcement are in place"
                    .to_string(),
            });
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
    use proto::clipboard_envelope::Message as Msg;
    use tokio::net::UnixStream;

    /// Drives `handle_envelope` over a `UnixStream::pair()` and
    /// returns the single response envelope.
    async fn dispatch_and_read(envelope: proto::ClipboardEnvelope) -> proto::ClipboardEnvelope {
        let (a, b) = UnixStream::pair().expect("pair");
        let (mut a_read, _a_write) = a.into_split();
        let (_b_read, b_write) = b.into_split();
        let writer = Arc::new(tokio::sync::Mutex::new(b_write));

        let history = Arc::new(ClipboardHistory::new());
        handle_envelope(envelope, history, writer.clone()).await;

        // Read one response frame from the peer side.
        let mut len_buf = [0u8; 4];
        a_read.read_exact(&mut len_buf).await.expect("read len");
        let len = u32::from_be_bytes(len_buf) as usize;
        let mut body = vec![0u8; len];
        a_read.read_exact(&mut body).await.expect("read body");
        proto::ClipboardEnvelope::decode(body.as_slice()).expect("decode")
    }

    #[tokio::test]
    async fn read_request_returns_permission_denied_in_phase_1() {
        let envelope = proto::ClipboardEnvelope {
            message: Some(Msg::ReadRequest(proto::ReadRequest {})),
        };
        let resp = dispatch_and_read(envelope).await;
        match resp.message {
            Some(Msg::Error(e)) => {
                assert_eq!(e.kind, proto::ErrorKind::ErrorPermissionDenied as i32);
                assert!(e.detail.contains("Phase 6"), "detail mentions Phase 6 deferral");
            }
            other => panic!("expected Error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn subscribe_request_returns_permission_denied_in_phase_1() {
        let envelope = proto::ClipboardEnvelope {
            message: Some(Msg::SubscribeRequest(proto::SubscribeRequest {})),
        };
        let resp = dispatch_and_read(envelope).await;
        match resp.message {
            Some(Msg::Error(e)) => {
                assert_eq!(e.kind, proto::ErrorKind::ErrorPermissionDenied as i32);
            }
            other => panic!("expected Error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn history_request_returns_permission_denied_in_phase_1() {
        let envelope = proto::ClipboardEnvelope {
            message: Some(Msg::HistoryRequest(proto::HistoryRequest { limit: 10 })),
        };
        let resp = dispatch_and_read(envelope).await;
        match resp.message {
            Some(Msg::Error(e)) => {
                assert_eq!(e.kind, proto::ErrorKind::ErrorPermissionDenied as i32);
            }
            other => panic!("expected Error, got {other:?}"),
        }
    }
}

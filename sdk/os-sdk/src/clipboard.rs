/// `shell.clipboard` — first-party app surface for the Lunaris
/// clipboard with explicit sensitivity labels.
///
/// Apps connect to the shell-side IPC socket at
/// `$XDG_RUNTIME_DIR/lunaris/clipboard.sock`. The shell is the
/// broker for all clipboard operations; this client never touches
/// Wayland directly. See `docs/architecture/clipboard-api.md` for
/// the full architecture.
///
/// The Rust API mirrors the foundation §6.4 spec:
/// - [`UnixClipboardClient::write`] places content with a
///   sensitivity label.
/// - [`UnixClipboardClient::read`] returns the current entry, with
///   content elided for `Sensitive` entries when the caller lacks
///   the `clipboard.read.sensitive` permission.
/// - [`UnixClipboardClient::subscribe`] streams future clipboard
///   changes.
/// - [`UnixClipboardClient::history`] returns the recorded ring
///   buffer (requires `clipboard.history`).

use std::path::PathBuf;

use prost::Message;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::Mutex;

use crate::proto_clipboard::{
    self, clipboard_envelope::Message as Envelope,
};

const MAX_FRAME_BYTES: usize = 1024 * 1024;
const SOCKET_NAME: &str = "clipboard.sock";

// ── Public types ───────────────────────────────────────────────

/// Sensitivity label attached to a clipboard entry.
///
/// Behaviour mirrors `desktop-shell::clipboard_history::Label`:
/// `Sensitive` content is filtered out of history at write time
/// and read-time delivery drops content for callers without
/// `clipboard.read.sensitive`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ClipboardLabel {
    Normal,
    Sensitive,
}

impl Default for ClipboardLabel {
    fn default() -> Self {
        ClipboardLabel::Normal
    }
}

/// One clipboard entry as the shell reports it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipboardEntry {
    /// Stable id assigned by the shell. Useful for delete or
    /// follow-up operations later.
    pub id: String,
    /// Entry content. `None` when the reader lacks the relevant
    /// permission to see sensitive content.
    pub content: Option<Vec<u8>>,
    pub mime: String,
    pub label: ClipboardLabel,
    /// Unix milliseconds at capture time.
    pub timestamp_ms: i64,
    /// App id inferred from the focused window at capture time.
    /// Empty when the shell could not determine a source.
    pub source_app_id: String,
}

/// Parameters for [`UnixClipboardClient::write`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WriteParams {
    pub content: Vec<u8>,
    /// Phase 1 supports `text/plain` only. Other MIME types are
    /// rejected by the shell.
    pub mime: String,
    #[serde(default)]
    pub label: ClipboardLabel,
}

/// Errors surfaced from the SDK to its caller.
#[derive(Debug, Error)]
pub enum ClipboardError {
    /// The shell is not reachable (socket missing, daemon down,
    /// or the user has not enabled the clipboard subsystem).
    #[error("connect to clipboard shell: {0}")]
    ConnectionFailed(String),
    /// A Tokio I/O error during a request or response.
    #[error("clipboard IPC I/O: {0}")]
    Io(#[from] std::io::Error),
    /// Wire-level protobuf decode/encode failure. Indicates either
    /// a buggy peer or a protocol-version mismatch.
    #[error("clipboard protocol error: {0}")]
    Protocol(String),
    /// Shell rejected the request because the caller's permission
    /// profile does not allow it.
    #[error("permission denied: {0}")]
    PermissionDenied(String),
    /// Content exceeded the shell's per-entry byte limit.
    #[error("content too large: {0}")]
    ContentTooLarge(String),
    /// Phase 1 supports text/plain only.
    #[error("unsupported MIME type: {0}")]
    UnsupportedMime(String),
    /// `wl-copy`/`wl-paste` shell-out failed inside the shell.
    #[error("system error: {0}")]
    System(String),
    /// Shell received a response we did not expect for the request
    /// we sent. Almost always a peer bug.
    #[error("unexpected response from shell")]
    UnexpectedResponse,
}

// ── Client ─────────────────────────────────────────────────────

/// Connected client over the shell's clipboard socket.
///
/// Cheap to clone (internal `Arc`); each clone shares the same
/// underlying connection. The wire is single-threaded, so concurrent
/// requests from multiple clones are serialised behind an internal
/// mutex.
#[derive(Clone)]
pub struct UnixClipboardClient {
    inner: std::sync::Arc<Inner>,
}

struct Inner {
    /// Single mutex covering the whole request/response cycle.
    /// Locking only the writer for the send and only the reader
    /// for the receive would let a second concurrent caller
    /// interleave its request between our send and our receive,
    /// and because the wire has no correlation IDs the second
    /// caller would steal our response. So we lock both halves
    /// together for the entire exchange.
    conn: Mutex<Conn>,
    socket_path: PathBuf,
}

struct Conn {
    writer: OwnedWriteHalf,
    reader: OwnedReadHalf,
}

impl UnixClipboardClient {
    /// Connect to the shell at the standard socket path.
    pub async fn connect() -> Result<Self, ClipboardError> {
        let path = socket_path()?;
        Self::connect_at(path).await
    }

    /// Connect to the shell at an arbitrary socket path. Primarily
    /// useful for integration tests that spin up an in-process
    /// fake shell on a tempdir socket.
    pub async fn connect_at(path: PathBuf) -> Result<Self, ClipboardError> {
        let stream = UnixStream::connect(&path)
            .await
            .map_err(|e| ClipboardError::ConnectionFailed(format!("{}: {e}", path.display())))?;
        let (reader, writer) = stream.into_split();
        Ok(Self {
            inner: std::sync::Arc::new(Inner {
                conn: Mutex::new(Conn { writer, reader }),
                socket_path: path,
            }),
        })
    }

    /// Place `params.content` on the clipboard with the given label.
    pub async fn write(&self, params: WriteParams) -> Result<(), ClipboardError> {
        let req = proto_clipboard::WriteRequest {
            content: params.content,
            mime: params.mime,
            label: label_to_proto(params.label).into(),
        };
        let envelope = proto_clipboard::ClipboardEnvelope {
            message: Some(Envelope::WriteRequest(req)),
        };
        let response = self.exchange(envelope).await?;
        match response.message {
            Some(Envelope::WriteResponse(_)) => Ok(()),
            Some(Envelope::Error(e)) => Err(error_to_sdk(e)),
            _ => Err(ClipboardError::UnexpectedResponse),
        }
    }

    /// Return the current clipboard entry, or `None` if the
    /// clipboard is empty.
    pub async fn read(&self) -> Result<Option<ClipboardEntry>, ClipboardError> {
        let envelope = proto_clipboard::ClipboardEnvelope {
            message: Some(Envelope::ReadRequest(proto_clipboard::ReadRequest {})),
        };
        let response = self.exchange(envelope).await?;
        match response.message {
            Some(Envelope::ReadResponse(r)) => Ok(r.entry.map(entry_from_proto)),
            Some(Envelope::Error(e)) => Err(error_to_sdk(e)),
            _ => Err(ClipboardError::UnexpectedResponse),
        }
    }

    /// Subscribe to clipboard changes. The returned receiver yields
    /// every change observed after the subscription was registered;
    /// to bootstrap with the current state, call [`Self::read`]
    /// separately first.
    ///
    /// Implementation note: the subscriber holds an internal
    /// background task that owns its own connection (so it does not
    /// block other operations on `self`). The task exits when the
    /// shell drops the connection.
    pub async fn subscribe(&self) -> Result<tokio::sync::mpsc::Receiver<ClipboardEntry>, ClipboardError> {
        // A fresh connection per subscriber keeps the response
        // pipeline simple — the main connection stays request/
        // response, the subscriber connection is one-way streaming.
        let path = self.inner.socket_path.clone();
        let stream = UnixStream::connect(&path)
            .await
            .map_err(|e| ClipboardError::ConnectionFailed(format!("{}: {e}", path.display())))?;
        let (mut reader, mut writer) = stream.into_split();
        let envelope = proto_clipboard::ClipboardEnvelope {
            message: Some(Envelope::SubscribeRequest(proto_clipboard::SubscribeRequest {})),
        };
        write_envelope(&mut writer, &envelope).await?;

        // Handshake: the shell must reply with `SubscribeResponse`
        // before any events. An `Error` here surfaces synchronously
        // to the caller so permission denials and broker failures
        // do not silently turn into "subscribed but no events".
        match read_envelope(&mut reader).await? {
            proto_clipboard::ClipboardEnvelope {
                message: Some(Envelope::SubscribeResponse(_)),
            } => {}
            proto_clipboard::ClipboardEnvelope {
                message: Some(Envelope::Error(e)),
            } => return Err(error_to_sdk(e)),
            _ => return Err(ClipboardError::UnexpectedResponse),
        }

        let (tx, rx) = tokio::sync::mpsc::channel::<ClipboardEntry>(64);
        tokio::spawn(async move {
            let mut buf = Vec::with_capacity(4096);
            let mut chunk = [0u8; 4096];
            loop {
                let n = match reader.read(&mut chunk).await {
                    Ok(0) => return,
                    Ok(n) => n,
                    Err(_) => return,
                };
                buf.extend_from_slice(&chunk[..n]);
                while let Some((consumed, env)) = decode_frame(&buf).unwrap_or(None) {
                    buf.drain(..consumed);
                    if let Some(Envelope::SubscriptionEvent(event)) = env.message {
                        if let Some(entry) = event.entry {
                            if tx.send(entry_from_proto(entry)).await.is_err() {
                                return;
                            }
                        }
                    }
                }
            }
        });
        Ok(rx)
    }

    /// Return up to `limit` recent clipboard entries, most-recent
    /// first. `0` asks the shell for as many as it will return,
    /// capped by the ring buffer size.
    pub async fn history(&self, limit: u32) -> Result<Vec<ClipboardEntry>, ClipboardError> {
        let envelope = proto_clipboard::ClipboardEnvelope {
            message: Some(Envelope::HistoryRequest(proto_clipboard::HistoryRequest { limit })),
        };
        let response = self.exchange(envelope).await?;
        match response.message {
            Some(Envelope::HistoryResponse(r)) => {
                Ok(r.entries.into_iter().map(entry_from_proto).collect())
            }
            Some(Envelope::Error(e)) => Err(error_to_sdk(e)),
            _ => Err(ClipboardError::UnexpectedResponse),
        }
    }

    async fn exchange(
        &self,
        envelope: proto_clipboard::ClipboardEnvelope,
    ) -> Result<proto_clipboard::ClipboardEnvelope, ClipboardError> {
        let mut conn = self.inner.conn.lock().await;
        write_envelope(&mut conn.writer, &envelope).await?;
        read_envelope(&mut conn.reader).await
    }
}

// ── Mock for tests ─────────────────────────────────────────────

/// In-memory mock client. Useful for unit-testing app code that
/// depends on the clipboard SDK without spinning up the shell.
#[derive(Default)]
pub struct MockClipboardClient {
    pub stored: tokio::sync::Mutex<Option<ClipboardEntry>>,
    pub history: tokio::sync::Mutex<Vec<ClipboardEntry>>,
}

impl MockClipboardClient {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn write(&self, params: WriteParams) -> Result<(), ClipboardError> {
        let entry = ClipboardEntry {
            id: format!("mock-{}", chrono_now_ms()),
            content: Some(params.content),
            mime: params.mime,
            label: params.label,
            timestamp_ms: chrono_now_ms(),
            source_app_id: "mock".to_string(),
        };
        *self.stored.lock().await = Some(entry.clone());
        if entry.label != ClipboardLabel::Sensitive {
            self.history.lock().await.insert(0, entry);
        }
        Ok(())
    }

    pub async fn read(&self) -> Result<Option<ClipboardEntry>, ClipboardError> {
        Ok(self.stored.lock().await.clone())
    }

    pub async fn history_snapshot(&self) -> Vec<ClipboardEntry> {
        self.history.lock().await.clone()
    }
}

// ── Helpers ────────────────────────────────────────────────────

fn socket_path() -> Result<PathBuf, ClipboardError> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").ok_or_else(|| {
        ClipboardError::ConnectionFailed("XDG_RUNTIME_DIR is not set".into())
    })?;
    let mut p = PathBuf::from(runtime);
    p.push("lunaris");
    p.push(SOCKET_NAME);
    Ok(p)
}

async fn write_envelope<W: AsyncWriteExt + Unpin>(
    writer: &mut W,
    envelope: &proto_clipboard::ClipboardEnvelope,
) -> Result<(), ClipboardError> {
    let body = envelope.encode_to_vec();
    if body.len() > MAX_FRAME_BYTES {
        return Err(ClipboardError::Protocol(format!(
            "outgoing frame {} bytes exceeds {} cap",
            body.len(),
            MAX_FRAME_BYTES
        )));
    }
    let len = (body.len() as u32).to_be_bytes();
    writer.write_all(&len).await?;
    writer.write_all(&body).await?;
    Ok(())
}

async fn read_envelope<R: AsyncReadExt + Unpin>(
    reader: &mut R,
) -> Result<proto_clipboard::ClipboardEnvelope, ClipboardError> {
    let mut len_buf = [0u8; 4];
    reader.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len == 0 || len > MAX_FRAME_BYTES {
        return Err(ClipboardError::Protocol(format!("frame length {len} out of range")));
    }
    let mut body = vec![0u8; len];
    reader.read_exact(&mut body).await?;
    proto_clipboard::ClipboardEnvelope::decode(&body[..])
        .map_err(|e| ClipboardError::Protocol(format!("decode: {e}")))
}

fn decode_frame(buf: &[u8]) -> Result<Option<(usize, proto_clipboard::ClipboardEnvelope)>, ClipboardError> {
    if buf.len() < 4 {
        return Ok(None);
    }
    let len = u32::from_be_bytes(buf[..4].try_into().expect("len bytes")) as usize;
    if len == 0 {
        return Err(ClipboardError::Protocol("empty frame".into()));
    }
    if len > MAX_FRAME_BYTES {
        return Err(ClipboardError::Protocol(format!(
            "incoming frame {len} bytes exceeds {MAX_FRAME_BYTES} cap"
        )));
    }
    if buf.len() < 4 + len {
        return Ok(None);
    }
    let env = proto_clipboard::ClipboardEnvelope::decode(&buf[4..4 + len])
        .map_err(|e| ClipboardError::Protocol(format!("decode: {e}")))?;
    Ok(Some((4 + len, env)))
}

fn label_to_proto(label: ClipboardLabel) -> proto_clipboard::Label {
    match label {
        ClipboardLabel::Normal => proto_clipboard::Label::Normal,
        ClipboardLabel::Sensitive => proto_clipboard::Label::Sensitive,
    }
}

fn label_from_proto(value: i32) -> ClipboardLabel {
    match proto_clipboard::Label::try_from(value).unwrap_or(proto_clipboard::Label::Normal) {
        proto_clipboard::Label::Sensitive => ClipboardLabel::Sensitive,
        proto_clipboard::Label::Normal => ClipboardLabel::Normal,
    }
}

fn entry_from_proto(p: proto_clipboard::ClipboardEntry) -> ClipboardEntry {
    ClipboardEntry {
        id: p.id,
        content: p.content,
        mime: p.mime,
        label: label_from_proto(p.label),
        timestamp_ms: p.timestamp_ms,
        source_app_id: p.source_app_id,
    }
}

fn error_to_sdk(e: proto_clipboard::ErrorResponse) -> ClipboardError {
    use proto_clipboard::ErrorKind as K;
    let kind = K::try_from(e.kind).unwrap_or(K::ErrorUnknown);
    match kind {
        K::ErrorPermissionDenied => ClipboardError::PermissionDenied(e.detail),
        K::ErrorContentTooLarge => ClipboardError::ContentTooLarge(e.detail),
        K::ErrorUnsupportedMime => ClipboardError::UnsupportedMime(e.detail),
        K::ErrorSystem => ClipboardError::System(e.detail),
        K::ErrorHistoryDisabled => ClipboardError::PermissionDenied(format!(
            "clipboard history is disabled in shell.toml: {}",
            e.detail
        )),
        K::ErrorUnknown => ClipboardError::Protocol(e.detail),
    }
}

fn chrono_now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mock_write_then_read_round_trips() {
        let mock = MockClipboardClient::new();
        mock.write(WriteParams {
            content: b"hello".to_vec(),
            mime: "text/plain".into(),
            label: ClipboardLabel::Normal,
        })
        .await
        .unwrap();
        let entry = mock.read().await.unwrap().expect("entry stored");
        assert_eq!(entry.content.as_deref(), Some(b"hello".as_slice()));
        assert_eq!(entry.label, ClipboardLabel::Normal);
    }

    #[tokio::test]
    async fn mock_sensitive_skips_history() {
        let mock = MockClipboardClient::new();
        mock.write(WriteParams {
            content: b"sk-ant-secret".to_vec(),
            mime: "text/plain".into(),
            label: ClipboardLabel::Sensitive,
        })
        .await
        .unwrap();
        // Sensitive entries are not persisted to history per spec.
        assert!(mock.history_snapshot().await.is_empty());
        // But the live read still returns the entry (a freshly-set
        // sensitive entry is still THE clipboard contents).
        let read = mock.read().await.unwrap().expect("entry stored");
        assert_eq!(read.label, ClipboardLabel::Sensitive);
    }

    /// Frame round-trip via the same encode/decode helpers the
    /// real client uses. Catches protocol drift at unit-test time.
    #[tokio::test]
    async fn frame_round_trips_through_helpers() {
        let envelope = proto_clipboard::ClipboardEnvelope {
            message: Some(Envelope::WriteRequest(proto_clipboard::WriteRequest {
                content: b"x".to_vec(),
                mime: "text/plain".into(),
                label: proto_clipboard::Label::Normal as i32,
            })),
        };
        let mut buf: Vec<u8> = Vec::new();
        write_envelope(&mut buf, &envelope).await.unwrap();
        let decoded = read_envelope(&mut buf.as_slice()).await.unwrap();
        match decoded.message {
            Some(Envelope::WriteRequest(r)) => assert_eq!(r.content, b"x"),
            other => panic!("unexpected variant: {other:?}"),
        }
    }
}

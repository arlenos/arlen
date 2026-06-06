//! Integration tests for `UnixClipboardClient` against an
//! in-process fake clipboard shell.
//!
//! The fake shell speaks the same length-prefixed protobuf
//! envelope as the real desktop-shell broker, but answers from a
//! local `Vec<ClipboardEntry>`. This exercises the full SDK
//! request/response loop, including subscribe streaming, without
//! depending on a live shell or Wayland session.

use os_sdk::{
    ClipboardEntry, ClipboardError, ClipboardLabel, UnixClipboardClient, WriteParams,
};
use prost::Message as _;
use std::path::PathBuf;
use std::time::Duration;
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

mod proto {
    #![allow(dead_code)]
    #![allow(clippy::doc_markdown)]
    include!(concat!(env!("OUT_DIR"), "/lunaris.clipboard.rs"));
}

use proto::clipboard_envelope::Message as Envelope;

/// In-process fake of the desktop-shell clipboard broker.
struct FakeShell {
    _tmp: TempDir,
    socket_path: PathBuf,
    accept_handle: JoinHandle<()>,
    subscribe_tx: mpsc::UnboundedSender<proto::ClipboardEntry>,
}

#[derive(Clone, Copy)]
enum SubscribeBehavior {
    Ack,
    DenyWithError,
}

impl FakeShell {
    async fn start() -> Self {
        Self::start_with(SubscribeBehavior::Ack).await
    }

    async fn start_with(subscribe_behavior: SubscribeBehavior) -> Self {
        let tmp = tempfile::tempdir().expect("tempdir");
        let socket_path = tmp.path().join("clipboard.sock");
        let listener = UnixListener::bind(&socket_path).expect("bind");
        let (subscribe_tx, mut subscribe_rx) =
            mpsc::unbounded_channel::<proto::ClipboardEntry>();

        // Shared state: most-recent entry on the clipboard.
        let current = std::sync::Arc::new(std::sync::Mutex::new(
            None::<proto::ClipboardEntry>,
        ));

        let current_clone = current.clone();

        let accept_handle = tokio::spawn(async move {
            // We accept many connections; per connection we may
            // be a request/response peer or a subscribe stream.
            // Subscribers register on a global broadcast channel
            // we forward `subscribe_rx` events to.
            let (broadcast_tx, _) =
                tokio::sync::broadcast::channel::<proto::ClipboardEntry>(64);
            let broadcast_for_fanout = broadcast_tx.clone();

            // Fanout: forward unbounded subscribe_rx into broadcast.
            tokio::spawn(async move {
                while let Some(entry) = subscribe_rx.recv().await {
                    let _ = broadcast_for_fanout.send(entry);
                }
            });

            loop {
                let (stream, _) = match listener.accept().await {
                    Ok(s) => s,
                    Err(_) => break,
                };
                let current = current_clone.clone();
                let broadcast_tx = broadcast_tx.clone();
                tokio::spawn(handle_conn(
                    stream,
                    current,
                    broadcast_tx,
                    subscribe_behavior,
                ));
            }
        });

        // Allow the listener loop to start.
        tokio::time::sleep(Duration::from_millis(20)).await;

        Self {
            _tmp: tmp,
            socket_path,
            accept_handle,
            subscribe_tx,
        }
    }

    fn path(&self) -> PathBuf {
        self.socket_path.clone()
    }

    /// Push an entry into all live subscriber streams.
    fn broadcast(&self, entry: proto::ClipboardEntry) {
        let _ = self.subscribe_tx.send(entry);
    }
}

impl Drop for FakeShell {
    fn drop(&mut self) {
        self.accept_handle.abort();
    }
}

async fn handle_conn(
    mut stream: UnixStream,
    current: std::sync::Arc<std::sync::Mutex<Option<proto::ClipboardEntry>>>,
    broadcast_tx: tokio::sync::broadcast::Sender<proto::ClipboardEntry>,
    subscribe_behavior: SubscribeBehavior,
) {
    loop {
        let envelope = match read_frame(&mut stream).await {
            Some(e) => e,
            None => return,
        };

        match envelope.message {
            Some(Envelope::WriteRequest(req)) => {
                let entry = proto::ClipboardEntry {
                    id: format!("entry-{}", chrono_now_ms()),
                    content: Some(req.content),
                    mime: req.mime,
                    label: req.label,
                    timestamp_ms: chrono_now_ms(),
                    source_app_id: "fake-shell-test".into(),
                };
                *current.lock().unwrap() = Some(entry.clone());
                let resp = proto::ClipboardEnvelope {
                    message: Some(Envelope::WriteResponse(proto::WriteResponse {
                        entry: Some(entry),
                    })),
                };
                if write_frame(&mut stream, &resp).await.is_err() {
                    return;
                }
            }
            Some(Envelope::ReadRequest(_)) => {
                let entry = current.lock().unwrap().clone();
                let resp = proto::ClipboardEnvelope {
                    message: Some(Envelope::ReadResponse(proto::ReadResponse {
                        entry,
                    })),
                };
                if write_frame(&mut stream, &resp).await.is_err() {
                    return;
                }
            }
            Some(Envelope::HistoryRequest(_)) => {
                let entries: Vec<proto::ClipboardEntry> =
                    current.lock().unwrap().clone().into_iter().collect();
                let resp = proto::ClipboardEnvelope {
                    message: Some(Envelope::HistoryResponse(proto::HistoryResponse {
                        entries,
                    })),
                };
                if write_frame(&mut stream, &resp).await.is_err() {
                    return;
                }
            }
            Some(Envelope::SubscribeRequest(_)) => {
                // Handshake: either ack and stream, or deny.
                match subscribe_behavior {
                    SubscribeBehavior::DenyWithError => {
                        let err = proto::ClipboardEnvelope {
                            message: Some(Envelope::Error(proto::ErrorResponse {
                                kind: proto::ErrorKind::ErrorPermissionDenied as i32,
                                detail: "test denies subscribe".into(),
                            })),
                        };
                        let _ = write_frame(&mut stream, &err).await;
                        return;
                    }
                    SubscribeBehavior::Ack => {}
                }
                let ack = proto::ClipboardEnvelope {
                    message: Some(Envelope::SubscribeResponse(
                        proto::SubscribeResponse {},
                    )),
                };
                if write_frame(&mut stream, &ack).await.is_err() {
                    return;
                }
                // Switch this connection into streaming mode:
                // forward broadcast channel events out.
                let mut rx = broadcast_tx.subscribe();
                while let Ok(entry) = rx.recv().await {
                    let event = proto::ClipboardEnvelope {
                        message: Some(Envelope::SubscriptionEvent(
                            proto::SubscriptionEvent {
                                entry: Some(entry),
                            },
                        )),
                    };
                    if write_frame(&mut stream, &event).await.is_err() {
                        return;
                    }
                }
                return;
            }
            _ => {
                // Unsupported in tests.
                return;
            }
        }
    }
}

async fn read_frame(stream: &mut UnixStream) -> Option<proto::ClipboardEnvelope> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await.ok()?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len == 0 || len > 1024 * 1024 {
        return None;
    }
    let mut body = vec![0u8; len];
    stream.read_exact(&mut body).await.ok()?;
    proto::ClipboardEnvelope::decode(body.as_slice()).ok()
}

async fn write_frame(
    stream: &mut UnixStream,
    envelope: &proto::ClipboardEnvelope,
) -> std::io::Result<()> {
    let mut buf = Vec::with_capacity(envelope.encoded_len());
    envelope.encode(&mut buf).expect("encode");
    let len = (buf.len() as u32).to_be_bytes();
    stream.write_all(&len).await?;
    stream.write_all(&buf).await?;
    Ok(())
}

fn chrono_now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

// ── Tests ────────────────────────────────────────────────────────

#[tokio::test]
async fn write_then_read_round_trips_through_socket() {
    let shell = FakeShell::start().await;
    let client = UnixClipboardClient::connect_at(shell.path())
        .await
        .expect("connect");

    client
        .write(WriteParams {
            content: b"hello".to_vec(),
            mime: "text/plain".into(),
            label: ClipboardLabel::Normal,
        })
        .await
        .expect("write");

    let entry = client.read().await.expect("read").expect("Some entry");
    assert_eq!(entry.content.as_deref(), Some(b"hello".as_ref()));
    assert_eq!(entry.mime, "text/plain");
    assert!(matches!(entry.label, ClipboardLabel::Normal));
}

#[tokio::test]
async fn read_returns_none_when_clipboard_empty() {
    let shell = FakeShell::start().await;
    let client = UnixClipboardClient::connect_at(shell.path())
        .await
        .expect("connect");

    let entry = client.read().await.expect("read");
    assert!(entry.is_none());
}

#[tokio::test]
async fn history_returns_current_entry() {
    let shell = FakeShell::start().await;
    let client = UnixClipboardClient::connect_at(shell.path())
        .await
        .expect("connect");

    client
        .write(WriteParams {
            content: b"abc".to_vec(),
            mime: "text/plain".into(),
            label: ClipboardLabel::Normal,
        })
        .await
        .expect("write");

    let entries = client.history(10).await.expect("history");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].mime, "text/plain");
}

#[tokio::test]
async fn subscribe_receives_broadcast_entries() {
    let shell = FakeShell::start().await;
    let client = UnixClipboardClient::connect_at(shell.path())
        .await
        .expect("connect");

    let mut rx = client.subscribe().await.expect("subscribe");

    // Allow the subscribe handshake to register on the server.
    tokio::time::sleep(Duration::from_millis(50)).await;

    let entry = proto::ClipboardEntry {
        id: "broadcast-1".into(),
        content: Some(b"streamed".to_vec()),
        mime: "text/plain".into(),
        label: 0,
        timestamp_ms: 1234,
        source_app_id: "test".into(),
    };
    shell.broadcast(entry);

    let received = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("receive in time")
        .expect("Some entry");

    assert_eq!(received.id, "broadcast-1");
    assert_eq!(received.content.as_deref(), Some(b"streamed".as_ref()));
}

#[tokio::test]
async fn connect_at_missing_socket_returns_connection_failed() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let path = tmp.path().join("does-not-exist.sock");
    match UnixClipboardClient::connect_at(path).await {
        Ok(_) => panic!("expected ConnectionFailed"),
        Err(ClipboardError::ConnectionFailed(_)) => {}
        Err(other) => panic!("expected ConnectionFailed, got {other:?}"),
    }
}

#[tokio::test]
async fn subscribe_returns_error_when_shell_denies() {
    let shell = FakeShell::start_with(SubscribeBehavior::DenyWithError).await;
    let client = UnixClipboardClient::connect_at(shell.path())
        .await
        .expect("connect");

    match client.subscribe().await {
        Ok(_) => panic!("expected PermissionDenied"),
        Err(ClipboardError::PermissionDenied(_)) => {}
        Err(other) => panic!("expected PermissionDenied, got {other:?}"),
    }
}

#[tokio::test]
async fn concurrent_requests_do_not_steal_each_others_responses() {
    // The server tags each HistoryResponse with the limit value
    // from the request as a single-entry id (`"limit-N"`). If the
    // SDK ever swapped a fast caller's response with a slow
    // caller's response, the assertions below would see the wrong
    // id come back.
    let tmp = tempfile::tempdir().expect("tempdir");
    let socket_path = tmp.path().join("clipboard.sock");
    let listener = UnixListener::bind(&socket_path).expect("bind");

    let task = tokio::spawn(async move {
        if let Ok((mut stream, _)) = listener.accept().await {
            loop {
                let env = match read_frame(&mut stream).await {
                    Some(e) => e,
                    None => return,
                };
                if let Some(Envelope::HistoryRequest(req)) = env.message {
                    // Stagger the slow request so the second
                    // arrives before the first reply goes out.
                    // This is the exact interleaving the SDK's
                    // single-mutex contract must prevent.
                    if req.limit == 1 {
                        tokio::time::sleep(Duration::from_millis(150)).await;
                    }
                    let tag = format!("limit-{}", req.limit);
                    let entry = proto::ClipboardEntry {
                        id: tag,
                        content: Some(b"x".to_vec()),
                        mime: "text/plain".into(),
                        label: 0,
                        timestamp_ms: 0,
                        source_app_id: "fifo-test".into(),
                    };
                    let resp = proto::ClipboardEnvelope {
                        message: Some(Envelope::HistoryResponse(proto::HistoryResponse {
                            entries: vec![entry],
                        })),
                    };
                    if write_frame(&mut stream, &resp).await.is_err() {
                        return;
                    }
                }
            }
        }
    });

    let client = std::sync::Arc::new(
        UnixClipboardClient::connect_at(socket_path)
            .await
            .expect("connect"),
    );

    // Fire two concurrent histories (limit=1 slow, limit=2 fast)
    // through the same client; the single-mutex exchange must
    // serialise them so each caller sees its own response.
    let c1 = client.clone();
    let c2 = client.clone();
    let h_slow = tokio::spawn(async move { c1.history(1).await });
    tokio::time::sleep(Duration::from_millis(20)).await;
    let h_fast = tokio::spawn(async move { c2.history(2).await });

    let slow_entries = h_slow.await.expect("join slow").expect("slow history");
    let fast_entries = h_fast.await.expect("join fast").expect("fast history");

    assert_eq!(slow_entries.len(), 1);
    assert_eq!(fast_entries.len(), 1);
    assert_eq!(slow_entries[0].id, "limit-1");
    assert_eq!(fast_entries[0].id, "limit-2");

    drop(client);
    let _ = task.await;
}

// Compile-time witness that the public type is what we expect.
#[allow(dead_code)]
fn _entry_field_types(entry: ClipboardEntry) {
    let _: String = entry.id;
    let _: Option<Vec<u8>> = entry.content;
    let _: String = entry.mime;
    let _: ClipboardLabel = entry.label;
    let _: i64 = entry.timestamp_ms;
    let _: String = entry.source_app_id;
}

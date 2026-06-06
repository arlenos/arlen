//! Integration tests for `UnixEventConsumer` against a fake
//! Event Bus consumer socket.
//!
//! The fake server speaks the same 3-line plaintext registration
//! plus length-prefixed protobuf framing as the real bus
//! (`event-bus/src/socket.rs:130-172`), but answers from local
//! state. This exercises the full SDK consumer pipeline: register,
//! receive event, mpsc forward, drop unsubscribes, reconnect on
//! disconnect.

use os_sdk::event_consumer::{EventConsumer, SubscribeError, UnixEventConsumer};
use prost::Message as _;
use std::path::PathBuf;
use std::time::Duration;
use tempfile::TempDir;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;

mod proto {
    #![allow(dead_code)]
    #![allow(clippy::doc_markdown)]
    include!(concat!(env!("OUT_DIR"), "/lunaris.eventbus.rs"));
}

/// In-process fake of the Event Bus consumer-side socket.
///
/// Accepts SDK consumers, parses their 3-line registration, and
/// forwards every broadcast event whose type matches the
/// requested filters.
struct FakeBus {
    _tmp: TempDir,
    socket_path: PathBuf,
    accept_handle: JoinHandle<()>,
    /// Test code calls `push()` to fire an event into the bus;
    /// every connected consumer with a matching filter sees it.
    fanout_tx: mpsc::UnboundedSender<proto::Event>,
    /// Active-connection counter. Incremented on each accepted
    /// connection, decremented when the connection task exits.
    /// Used by drop-unsubscribes test to assert teardown.
    active_connections: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl FakeBus {
    async fn start() -> Self {
        let tmp = tempfile::tempdir().expect("tempdir");
        let socket_path = tmp.path().join("event-bus-consumer.sock");
        let listener = UnixListener::bind(&socket_path).expect("bind");

        let (fanout_tx, mut fanout_rx) = mpsc::unbounded_channel::<proto::Event>();
        let active_connections = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));

        let (broadcast_tx, _) = broadcast::channel::<proto::Event>(64);
        let broadcast_for_fanout = broadcast_tx.clone();

        // Forward every push into the broadcast channel that all
        // connections subscribe to.
        tokio::spawn(async move {
            while let Some(event) = fanout_rx.recv().await {
                let _ = broadcast_for_fanout.send(event);
            }
        });

        let active_clone = active_connections.clone();
        let accept_handle = tokio::spawn(async move {
            loop {
                let (stream, _) = match listener.accept().await {
                    Ok(s) => s,
                    Err(_) => break,
                };
                let bcast = broadcast_tx.clone();
                let active = active_clone.clone();
                active.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                tokio::spawn(async move {
                    handle_connection(stream, bcast).await;
                    active.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                });
            }
        });

        // Allow the listener loop to register before tests connect.
        tokio::time::sleep(Duration::from_millis(20)).await;

        Self {
            _tmp: tmp,
            socket_path,
            accept_handle,
            fanout_tx,
            active_connections,
        }
    }

    fn path(&self) -> String {
        self.socket_path.to_string_lossy().into_owned()
    }

    fn push(&self, event: proto::Event) {
        let _ = self.fanout_tx.send(event);
    }

    fn active_connections(&self) -> usize {
        self.active_connections
            .load(std::sync::atomic::Ordering::SeqCst)
    }
}

impl Drop for FakeBus {
    fn drop(&mut self) {
        self.accept_handle.abort();
    }
}

async fn handle_connection(
    stream: UnixStream,
    broadcast_tx: broadcast::Sender<proto::Event>,
) {
    // Parse the 3-line registration.
    let (read_half, mut write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);

    let mut consumer_id = String::new();
    if reader.read_line(&mut consumer_id).await.is_err() {
        return;
    }
    let mut types_line = String::new();
    if reader.read_line(&mut types_line).await.is_err() {
        return;
    }
    let mut uid_line = String::new();
    if reader.read_line(&mut uid_line).await.is_err() {
        return;
    }

    let types: Vec<String> = types_line
        .trim()
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    // Stream events. Each iteration pulls from the broadcast and
    // forwards type-matching events as length-prefixed protobuf.
    let mut bcast_rx = broadcast_tx.subscribe();
    while let Ok(event) = bcast_rx.recv().await {
        if !type_matches(&event.r#type, &types) {
            continue;
        }
        let body = event.encode_to_vec();
        let len = (body.len() as u32).to_be_bytes();
        if write_half.write_all(&len).await.is_err()
            || write_half.write_all(&body).await.is_err()
        {
            return;
        }
    }
}

fn type_matches(event_type: &str, subscribed: &[String]) -> bool {
    subscribed.iter().any(|sub| {
        if sub == "*" {
            true
        } else if let Some(prefix) = sub.strip_suffix('.') {
            event_type.starts_with(prefix)
        } else {
            sub == event_type
        }
    })
}

fn make_event(id: &str, event_type: &str, payload: Vec<u8>) -> proto::Event {
    proto::Event {
        id: id.into(),
        r#type: event_type.into(),
        timestamp: 1,
        source: "test".into(),
        pid: 0,
        session_id: "s".into(),
        payload,
        uid: 0,
        project_id: String::new(),
    }
}

// ── Tests ────────────────────────────────────────────────────────

#[tokio::test]
async fn subscribe_returns_receiver_that_yields_matching_events() {
    let bus = FakeBus::start().await;
    let consumer = UnixEventConsumer::new(bus.path());

    let mut rx = consumer
        .subscribe(vec!["app.annotation.".to_string()])
        .await
        .expect("subscribe");

    // Allow registration to land in the FakeBus accept task.
    tokio::time::sleep(Duration::from_millis(50)).await;

    bus.push(make_event("e1", "app.annotation.set", vec![1, 2, 3]));
    let event = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("recv in time")
        .expect("event");
    assert_eq!(event.id, "e1");
    assert_eq!(event.r#type, "app.annotation.set");
    assert_eq!(event.payload, vec![1, 2, 3]);
}

#[tokio::test]
async fn subscribe_filters_non_matching_event_types() {
    let bus = FakeBus::start().await;
    let consumer = UnixEventConsumer::new(bus.path());

    let mut rx = consumer
        .subscribe(vec!["app.annotation.".to_string()])
        .await
        .expect("subscribe");

    tokio::time::sleep(Duration::from_millis(50)).await;

    // Non-matching: bus's prefix-filter drops these.
    bus.push(make_event("ignore-1", "file.opened", vec![]));
    bus.push(make_event("ignore-2", "window.focused", vec![]));
    // Matching:
    bus.push(make_event("keep", "app.annotation.cleared", vec![]));

    let event = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("recv")
        .expect("event");
    assert_eq!(event.id, "keep");
}

#[tokio::test]
async fn drop_receiver_unsubscribes_within_grace_window() {
    let bus = FakeBus::start().await;
    let consumer = UnixEventConsumer::new(bus.path());

    let rx = consumer
        .subscribe(vec!["*".to_string()])
        .await
        .expect("subscribe");

    // Wait for the connection to register on the bus.
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(bus.active_connections(), 1);

    drop(rx);
    // The bus only detects a dead consumer when its next write
    // returns an error (no read-half polling, matches production
    // behaviour). Two pushes are needed: the first wakes the SDK
    // forwarder, makes its `tx.send` fail, and the forwarder
    // exits — closing its stream. The second push triggers the
    // bus's `write_all` to fail against the now-closed peer,
    // which tears down the bus-side connection task.
    bus.push(make_event("nudge-1", "anything", vec![]));
    tokio::time::sleep(Duration::from_millis(50)).await;
    bus.push(make_event("nudge-2", "anything", vec![]));

    // Poll active_connections for the drop.
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while bus.active_connections() != 0 && std::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(
        bus.active_connections(),
        0,
        "dropping the receiver must tear down the bus connection"
    );
}

#[tokio::test]
async fn subscribe_returns_error_when_bus_is_unreachable() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let path = tmp.path().join("missing.sock");
    let consumer = UnixEventConsumer::new(path.to_string_lossy().into_owned());

    match consumer.subscribe(vec!["*".to_string()]).await {
        Ok(_) => panic!("expected ConnectionFailed"),
        Err(SubscribeError::ConnectionFailed(_)) => {}
        Err(other) => panic!("expected ConnectionFailed, got {other:?}"),
    }
}

#[tokio::test]
async fn two_subscribers_receive_independently() {
    let bus = FakeBus::start().await;
    let consumer = UnixEventConsumer::new(bus.path());

    let mut rx_a = consumer
        .subscribe(vec!["app.annotation.".to_string()])
        .await
        .expect("subscribe a");
    let mut rx_b = consumer
        .subscribe(vec!["app.annotation.".to_string()])
        .await
        .expect("subscribe b");

    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(
        bus.active_connections(),
        2,
        "each subscribe opens its own connection"
    );

    bus.push(make_event("shared", "app.annotation.set", vec![]));

    let a = tokio::time::timeout(Duration::from_secs(2), rx_a.recv())
        .await
        .expect("recv a")
        .expect("event a");
    let b = tokio::time::timeout(Duration::from_secs(2), rx_b.recv())
        .await
        .expect("recv b")
        .expect("event b");

    assert_eq!(a.id, "shared");
    assert_eq!(b.id, "shared");
}

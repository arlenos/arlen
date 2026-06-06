//! Integration test for `os-sdk::toolbar` against a fake Event
//! Bus producer socket. Verifies the wire is what the
//! desktop-shell consumer expects, end-to-end.

use os_sdk::{BreadcrumbItem, ProgressState, QuickAction, Toolbar, UnixEventEmitter};
use prost::Message as _;
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixListener;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

mod proto {
    #![allow(dead_code)]
    #![allow(clippy::doc_markdown)]
    include!(concat!(env!("OUT_DIR"), "/arlen.eventbus.rs"));
}

/// Fake of the bus producer socket. Records every event the
/// emitter sends, so tests can assert on type + decoded payload.
struct FakeProducer {
    _tmp: TempDir,
    socket_path: String,
    received: Arc<Mutex<Vec<proto::Event>>>,
    _accept_handle: JoinHandle<()>,
}

impl FakeProducer {
    async fn start() -> Self {
        let tmp = tempfile::tempdir().expect("tempdir");
        let socket_path = tmp.path().join("producer.sock");
        let listener = UnixListener::bind(&socket_path).expect("bind");
        let received: Arc<Mutex<Vec<proto::Event>>> =
            Arc::new(Mutex::new(Vec::new()));

        let received_clone = received.clone();
        let accept_handle = tokio::spawn(async move {
            loop {
                let (mut stream, _) = match listener.accept().await {
                    Ok(s) => s,
                    Err(_) => break,
                };
                let received = received_clone.clone();
                tokio::spawn(async move {
                    loop {
                        let mut len_buf = [0u8; 4];
                        if stream.read_exact(&mut len_buf).await.is_err() {
                            return;
                        }
                        let len = u32::from_be_bytes(len_buf) as usize;
                        if len == 0 || len > 1024 * 1024 {
                            return;
                        }
                        let mut body = vec![0u8; len];
                        if stream.read_exact(&mut body).await.is_err() {
                            return;
                        }
                        if let Ok(event) = proto::Event::decode(body.as_slice()) {
                            received.lock().await.push(event);
                        }
                    }
                });
            }
        });

        // Allow the listener loop to register before tests connect.
        tokio::time::sleep(Duration::from_millis(20)).await;

        Self {
            _tmp: tmp,
            socket_path: socket_path.to_string_lossy().into_owned(),
            received,
            _accept_handle: accept_handle,
        }
    }

    fn path(&self) -> &str {
        &self.socket_path
    }

    async fn wait_for(&self, count: usize, timeout: Duration) -> Vec<proto::Event> {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            {
                let r = self.received.lock().await;
                if r.len() >= count {
                    return r.clone();
                }
            }
            if std::time::Instant::now() >= deadline {
                return self.received.lock().await.clone();
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}

#[tokio::test]
async fn quick_actions_round_trip_through_bus() {
    let bus = FakeProducer::start().await;
    let emitter = UnixEventEmitter::new(bus.path().to_string());
    // Tokio's write-only producer connects on first emit.
    let tb = Toolbar::new(emitter, "com.example.editor");

    tb.set_quick_actions("main", vec![
        QuickAction {
            icon: "share".into(),
            action: "file.share".into(),
            tooltip: "Share".into(),
            toggle: None,
            active: None,
        },
        QuickAction {
            icon: "star".into(),
            action: "file.bookmark".into(),
            tooltip: "Bookmark".into(),
            toggle: Some(true),
            active: Some(false),
        },
    ])
    .await
    .expect("emit");

    let events = bus.wait_for(1, Duration::from_secs(2)).await;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].r#type, "app.toolbar.quick_actions");
    let p = proto::ToolbarQuickActionsPayload::decode(events[0].payload.as_slice())
        .expect("decode");
    assert_eq!(p.app_id, "com.example.editor");
    assert_eq!(p.actions.len(), 2);
    assert_eq!(p.actions[1].action, "file.bookmark");
    assert!(p.actions[1].toggle);
    assert!(!p.actions[1].active);
}

#[tokio::test]
async fn breadcrumb_round_trip() {
    let bus = FakeProducer::start().await;
    let emitter = UnixEventEmitter::new(bus.path().to_string());
    let tb = Toolbar::new(emitter, "com.example.files");

    tb.set_breadcrumb("main", vec![
        BreadcrumbItem {
            label: "Home".into(),
            action: "nav.home".into(),
        },
        BreadcrumbItem {
            label: "Documents".into(),
            action: "nav.docs".into(),
        },
    ])
    .await
    .unwrap();

    let events = bus.wait_for(1, Duration::from_secs(2)).await;
    let p = proto::ToolbarBreadcrumbPayload::decode(events[0].payload.as_slice())
        .expect("decode");
    assert_eq!(events[0].r#type, "app.toolbar.breadcrumb");
    assert_eq!(p.items[0].label, "Home");
    assert_eq!(p.items[1].action, "nav.docs");
}

#[tokio::test]
async fn progress_round_trip_clamps_value() {
    let bus = FakeProducer::start().await;
    let emitter = UnixEventEmitter::new(bus.path().to_string());
    let tb = Toolbar::new(emitter, "com.example.builder");

    tb.set_progress("main", ProgressState {
        value: 1.7, // out of range — must be clamped
        label: Some("Compiling".into()),
    })
    .await
    .unwrap();

    let events = bus.wait_for(1, Duration::from_secs(2)).await;
    let p = proto::ToolbarProgressPayload::decode(events[0].payload.as_slice())
        .expect("decode");
    assert_eq!(events[0].r#type, "app.toolbar.progress");
    assert!((p.value - 1.0).abs() < f32::EPSILON);
    assert_eq!(p.label, "Compiling");
}

#[tokio::test]
async fn clear_progress_emits_dedicated_event() {
    let bus = FakeProducer::start().await;
    let emitter = UnixEventEmitter::new(bus.path().to_string());
    let tb = Toolbar::new(emitter, "com.example.builder");

    tb.clear_progress("main").await.unwrap();
    let events = bus.wait_for(1, Duration::from_secs(2)).await;
    assert_eq!(events[0].r#type, "app.toolbar.progress_cleared");
}

#[tokio::test]
async fn clear_emits_blanket_cleared_event() {
    let bus = FakeProducer::start().await;
    let emitter = UnixEventEmitter::new(bus.path().to_string());
    let tb = Toolbar::new(emitter, "com.example.editor");

    tb.clear("main").await.unwrap();
    let events = bus.wait_for(1, Duration::from_secs(2)).await;
    assert_eq!(events[0].r#type, "app.toolbar.cleared");
}

#[tokio::test]
async fn action_invoked_decode_helper_round_trips() {
    let payload = proto::ToolbarActionInvokedPayload {
        app_id: "com.example.editor".into(),
        action: "file.share".into(),
        window_id: "doc-2".into(),
    };
    let encoded = payload.encode_to_vec();
    let decoded = os_sdk::decode_action_invoked(&encoded).expect("decode");
    assert_eq!(decoded.app_id, "com.example.editor");
    assert_eq!(decoded.action, "file.share");
    assert_eq!(decoded.window_id, "doc-2");

    // Malformed payload returns None.
    assert!(os_sdk::decode_action_invoked(b"not protobuf").is_none());
}

#[tokio::test]
async fn multi_window_emits_carry_distinct_window_ids() {
    // Sprint B-thin v2 / Codex F2.2 regression. Two windows of
    // the same app must emit toolbar state with their own
    // window_id so the shell can key state per-(app, window)
    // and not last-emit-wins one out.
    let bus = FakeProducer::start().await;
    let emitter = UnixEventEmitter::new(bus.path().to_string());
    let tb = Toolbar::new(emitter, "com.example.editor");

    // Window 1 sets a breadcrumb.
    tb.set_breadcrumb(
        "doc-1",
        vec![BreadcrumbItem {
            label: "Notes".into(),
            action: "nav.notes".into(),
        }],
    )
    .await
    .unwrap();

    // Window 2 sets quick actions.
    tb.set_quick_actions(
        "doc-2",
        vec![QuickAction {
            icon: "save".into(),
            action: "file.save".into(),
            tooltip: "Save".into(),
            toggle: None,
            active: None,
        }],
    )
    .await
    .unwrap();

    let events = bus.wait_for(2, Duration::from_secs(2)).await;
    assert_eq!(events.len(), 2);

    let bc = proto::ToolbarBreadcrumbPayload::decode(events[0].payload.as_slice())
        .expect("decode bc");
    assert_eq!(bc.window_id, "doc-1");

    let qa = proto::ToolbarQuickActionsPayload::decode(events[1].payload.as_slice())
        .expect("decode qa");
    assert_eq!(qa.window_id, "doc-2");

    // Distinct payloads, distinct window_ids — the shell-side
    // store keys per-(app, window) so doc-1's breadcrumb and
    // doc-2's quick actions coexist.
    assert_ne!(bc.window_id, qa.window_id);
}

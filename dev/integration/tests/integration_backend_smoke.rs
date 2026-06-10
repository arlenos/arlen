//! IT-1, first scenario: the unprivileged backend comes up hermetically.
//!
//! Brings up event-bus + the knowledge daemon in a fresh [`EphemeralStack`]
//! (its own runtime root, sockets, SQLite, and graph under a temp dir) and
//! asserts each binds its socket, i.e. the assembled backend actually starts
//! and wires together. This is the smallest end-to-end assertion above per-crate
//! tests; later scenarios extend it to emit-event -> KG-promotion -> query.
//!
//! `#[ignore]`d: it needs the daemon binaries built and a host where the
//! knowledge daemon's FUSE timeline mount can be created (the dev machine / VM).
//! Build first, then run:
//!   cargo build --manifest-path daemons/event-bus/Cargo.toml
//!   cargo build --manifest-path daemons/knowledge/Cargo.toml
//!   cargo test --manifest-path dev/integration/Cargo.toml \
//!     --test integration_backend_smoke -- --ignored
//! (a future `just integration-smoke` wraps this.)

use std::time::Duration;

use arlen_integration::EphemeralStack;
use os_sdk::{EventEmitter, UnixEventEmitter};
use prost::Message;
use sqlx::sqlite::SqlitePoolOptions;

mod proto {
    #![allow(dead_code)]
    include!(concat!(env!("OUT_DIR"), "/arlen.eventbus.rs"));
}

#[test]
#[ignore = "needs event-bus + knowledge binaries built and a FUSE-capable host"]
fn backend_stack_comes_up_hermetically() {
    let mut stack = EphemeralStack::new().expect("private runtime root");

    // Event bus: binds the producer + consumer sockets.
    stack
        .spawn("daemons/event-bus", "event-bus", &[])
        .expect("spawn event-bus");
    stack
        .wait_socket("event-bus-producer.sock", Duration::from_secs(20))
        .expect("event-bus producer socket appears");
    stack
        .wait_socket("event-bus-consumer.sock", Duration::from_secs(20))
        .expect("event-bus consumer socket appears");

    // Knowledge daemon: subscribes to the consumer socket and binds its query
    // socket. A longer timeout: it also opens SQLite + the graph + the FUSE
    // mount before the socket is up.
    stack
        .spawn("daemons/knowledge", "arlen-graph-daemon", &[])
        .expect("spawn knowledge");
    stack
        .wait_socket("knowledge.sock", Duration::from_secs(30))
        .expect("knowledge query socket appears");

    // The assembled backend is up and hermetic; dropping `stack` kills both
    // daemons and removes the runtime root (no /var/lib or $HOME write).
}

/// IT-1 data-flow: a `file.opened` event emitted to the bus lands in the
/// knowledge daemon's hermetic SQLite event store. The first real end-to-end
/// assertion (bus -> knowledge writer -> SQLite), one layer above "comes up".
/// Same `#[ignore]` rationale as above.
#[tokio::test]
#[ignore = "needs event-bus + knowledge binaries built and a FUSE-capable host"]
async fn a_file_opened_event_lands_in_sqlite() {
    let mut stack = EphemeralStack::new().expect("private runtime root");
    stack
        .spawn("daemons/event-bus", "event-bus", &[])
        .expect("spawn event-bus");
    stack
        .wait_socket("event-bus-producer.sock", Duration::from_secs(20))
        .expect("producer socket");
    stack
        .wait_socket("event-bus-consumer.sock", Duration::from_secs(20))
        .expect("consumer socket");
    stack
        .spawn("daemons/knowledge", "arlen-graph-daemon", &[])
        .expect("spawn knowledge");
    stack
        .wait_socket("knowledge.sock", Duration::from_secs(30))
        .expect("knowledge socket");

    // Let the knowledge writer register its `*` consumer subscription before we
    // emit, or the event would be delivered to no one (the bus fans out to the
    // consumers registered at emit time).
    tokio::time::sleep(Duration::from_millis(700)).await;

    // Emit one file.opened through the real SDK emitter (envelope + framing as a
    // production app would). The payload is the typed FileOpenedPayload bytes;
    // the emitter wraps them in the Event envelope under `file.opened`.
    let emitter = UnixEventEmitter::new(stack.producer_socket().to_string_lossy().into_owned());
    // The integration crate's proto copy omits the additive `cgroup_id` (field
    // 4); knowledge decodes its absence to 0, the documented sentinel.
    let payload = proto::FileOpenedPayload {
        path: "/tmp/it/main.rs".to_string(),
        app_id: "integration-test".to_string(),
        flags: 0,
    }
    .encode_to_vec();
    emitter
        .emit("file.opened", payload)
        .await
        .expect("emit file.opened");

    // The writer batches on a 500ms timer; wait it out with margin.
    tokio::time::sleep(Duration::from_millis(1200)).await;

    let db = stack.db_path();
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&format!("sqlite:{}", db.display()))
        .await
        .expect("open the hermetic events.db");
    let row: Option<(String,)> =
        sqlx::query_as("SELECT type FROM events WHERE type = 'file.opened' LIMIT 1")
            .fetch_optional(&pool)
            .await
            .expect("query events");
    assert!(
        row.is_some(),
        "the emitted file.opened event should have landed in the hermetic SQLite store"
    );
}

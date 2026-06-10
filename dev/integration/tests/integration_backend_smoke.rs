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
use os_sdk::{EventEmitter, QueryError, UnixEventEmitter, UnixGraphClient};
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

/// IT-1 read-scope enforcement (RS-R1): the knowledge read socket refuses an
/// authority-label query from a non-privileged caller. This test process
/// resolves (via SO_PEERCRED -> `/proc/self/exe`) to an unenrolled dev app id =
/// ThirdParty, not system-anchored, so a `Grant` query must be denied by the
/// `references_authority_label` pre-gate. A success would mean the gate failed:
/// the `Grant` table exists and the query is valid Cypher, so it would return
/// empty rows if allowed. Verifies the security boundary in the assembled
/// daemon, not just the unit test. Same `#[ignore]` rationale.
#[tokio::test]
#[ignore = "needs event-bus + knowledge binaries built and a FUSE-capable host"]
async fn the_read_socket_denies_an_unprivileged_authority_query() {
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

    let client = UnixGraphClient::new(stack.knowledge_socket().to_string_lossy().into_owned());
    let denied = client
        .query_rows("MATCH (g:Grant) RETURN g.id LIMIT 1")
        .await;
    assert!(
        denied.is_err(),
        "an unprivileged authority-label (Grant) query must be denied end-to-end (RS-R1)"
    );
}

/// IT-1 write-tier enforcement: the knowledge write socket refuses a relation
/// write from an unprivileged caller. The test process is ThirdParty (an
/// unenrolled dev app id), and the write path's least-privilege tier gate
/// rejects ThirdParty before any persistence, so `create_relation` must map to
/// `PermissionDenied`. Complements the RS-R1 read deny: a distinct boundary (the
/// write tier), verified end-to-end in the assembled daemon. Same `#[ignore]`.
#[tokio::test]
#[ignore = "needs event-bus + knowledge binaries built and a FUSE-capable host"]
async fn the_write_socket_refuses_an_unprivileged_relation_write() {
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

    let client = UnixGraphClient::new(stack.knowledge_socket().to_string_lossy().into_owned());
    let refused = client
        .create_relation(
            "system.File",
            "/tmp/it/x.rs",
            "system.Project",
            "/tmp/it",
            "FILE_PART_OF",
            "it-write-deny",
        )
        .await;
    assert!(
        matches!(refused, Err(QueryError::PermissionDenied)),
        "a ThirdParty relation write must be refused by the tier gate, got {refused:?}"
    );
}

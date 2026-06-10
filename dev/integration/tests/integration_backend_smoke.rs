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

use std::time::{Duration, Instant};

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

    // The knowledge writer registers its `*` consumer subscription concurrently
    // with binding its query socket (`tokio::select!`, no ordering guarantee), and
    // the bus drops an event with no consumer registered at emit time. So a single
    // emit after a fixed sleep races the subscription. Instead emit repeatedly
    // until the event lands: once the writer is subscribed a later emit is
    // delivered, and each emit carries a fresh envelope id so no idempotency check
    // suppresses the retries. This is robust to the subscription latency rather
    // than guessing it.
    let emitter = UnixEventEmitter::new(stack.producer_socket().to_string_lossy().into_owned());
    let db = stack.db_path();
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&format!("sqlite:{}", db.display()))
        .await
        .expect("open the hermetic events.db");

    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
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
        // The writer batches on a 500ms timer; give it margin before checking.
        tokio::time::sleep(Duration::from_millis(600)).await;
        let row: Option<(String,)> =
            sqlx::query_as("SELECT type FROM events WHERE type = 'file.opened' LIMIT 1")
                .fetch_optional(&pool)
                .await
                .expect("query events");
        if row.is_some() {
            return; // the emitted file.opened landed in the hermetic SQLite store
        }
        assert!(
            Instant::now() < deadline,
            "the emitted file.opened event never landed in the hermetic SQLite store within 20s"
        );
    }
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
    // The boundary is "the write is refused by the tier gate", not a specific
    // error variant: the daemon answers `ERROR: write mode not permitted for this
    // caller`, which the SDK's coarse `check_error` maps to `InvalidQuery` (it
    // keys `PermissionDenied` off the literal substring "permission", absent
    // here). Accept either denial shape, but exclude a transport error so the
    // assertion stays meaningful.
    let denied = match &refused {
        Err(QueryError::PermissionDenied) => true,
        Err(QueryError::InvalidQuery(msg)) => msg.contains("not permitted"),
        _ => false,
    };
    assert!(
        denied,
        "a ThirdParty relation write must be refused by the tier gate, got {refused:?}"
    );
}

/// IT-1 read-scope grant (the positive complement to the deny scenarios): a
/// caller WITH a seeded read profile may read its granted label. We seed a
/// `system.File` read grant for this test's own app id, then a `File` query is
/// ALLOWED where the unprivileged caller's was denied. `Ok` <=> allowed: a denied
/// query returns `Err`, and a fresh KG returns empty rows, so `Ok` proves the
/// seeded grant lifted the RS-R1 gate. Shows the gate is a real boundary that
/// both denies AND admits per the caller's scope (and that the daemon resolves
/// the connecting peer to the same app id the profile was seeded for). Same
/// `#[ignore]` rationale.
#[tokio::test]
#[ignore = "needs event-bus + knowledge binaries built and a FUSE-capable host"]
async fn a_scoped_caller_may_read_its_granted_label() {
    let mut stack = EphemeralStack::new().expect("private runtime root");
    // Seed the read grant before the daemon loads it for our connection.
    stack
        .seed_read_profile(&["system.File.id", "system.File.path"])
        .expect("seed read profile");
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
    let allowed = client
        .query_rows("MATCH (f:File) RETURN f.id LIMIT 1")
        .await;
    assert!(
        allowed.is_ok(),
        "a caller granted system.File read may run a File query (RS-R1 admits a granted label), got {allowed:?}"
    );
}

/// IT-1 capstone data-flow: a `file.opened` event promotes through to a graph
/// `File` node that an authorised caller can read back. Ties the whole pipeline
/// together: bus -> knowledge writer -> SQLite -> the ~30s promotion pass ->
/// graph node -> the authorised 0x01 typed read returning the node. The poll
/// (up to ~45s) absorbs the promotion timing rather than a brittle fixed sleep;
/// a non-empty result for `MATCH (f:File {id:'<our path>'})` proves OUR file was
/// promoted and is readable under the seeded scope. Same `#[ignore]` rationale
/// (and it is the slowest scenario, ~30s, so it sits last).
#[tokio::test]
#[ignore = "needs event-bus + knowledge binaries built and a FUSE-capable host (~30s)"]
async fn a_file_opened_promotes_to_a_readable_file_node() {
    let mut stack = EphemeralStack::new().expect("private runtime root");
    stack
        .seed_read_profile(&["system.File.id", "system.File.path"])
        .expect("seed read profile");
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

    let path = "/work/it/promoted.rs";
    let emitter = UnixEventEmitter::new(stack.producer_socket().to_string_lossy().into_owned());
    let client = UnixGraphClient::new(stack.knowledge_socket().to_string_lossy().into_owned());
    let query = format!("MATCH (f:File {{id: '{path}'}}) RETURN f.path LIMIT 1");
    // Re-emit each iteration so a `file.opened` dropped before the writer's
    // subscription registered does not doom the whole wait; promotion is
    // idempotent on the path (one File node), so repeated emits are harmless. The
    // deadline covers the writer-subscription race plus one full ~30s promotion
    // interval after the event first lands in SQLite.
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let payload = proto::FileOpenedPayload {
            path: path.to_string(),
            app_id: "integration-test".to_string(),
            flags: 0,
        }
        .encode_to_vec();
        emitter
            .emit("file.opened", payload)
            .await
            .expect("emit file.opened");
        if let Ok(rows) = client.query_rows(&query).await {
            if !rows.is_empty() {
                return; // promoted to a File node and readable under the seeded scope
            }
        }
        assert!(
            Instant::now() < deadline,
            "the file.opened event never promoted to a readable File node within 60s"
        );
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

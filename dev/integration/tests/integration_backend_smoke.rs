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

/// IT-1 retrieval (RRF, the 0x03 op): a promoted File is findable by keyword
/// through the LLM-free retrieval pipeline, scoped to the caller's read grant.
/// Exercises the whole read path no per-crate test covers end-to-end: promotion
/// synthesizes the File's fact text and populates the FTS5 index, then the 0x03
/// op runs BM25 keyword search + graph expansion -> RRF fusion -> validity
/// confirm -> the readable-label filter (RS-R1), returning ranked node ids. The
/// `system.File` grant is load-bearing twice: it admits the retrieve AND keeps
/// the File id past the readable-label filter (an unscoped caller's result is
/// dropped to empty). A distinctive basename token (`retrieval`) makes the match
/// unambiguous in an otherwise-empty hermetic graph. Same `#[ignore]` rationale
/// (promotion-dependent, ~30s).
#[tokio::test]
#[ignore = "needs event-bus + knowledge binaries built and a FUSE-capable host (~30s)"]
async fn a_promoted_file_is_retrievable_by_keyword_under_scope() {
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

    // A distinctive basename token so the keyword search is unambiguous; the File
    // node id is its path, so that is what the retrieve result must contain.
    let path = "/work/it/retrieval.rs";
    let emitter = UnixEventEmitter::new(stack.producer_socket().to_string_lossy().into_owned());
    let client = UnixGraphClient::new(stack.knowledge_socket().to_string_lossy().into_owned());
    // Promotion synthesizes the fact text and populates the FTS index in the same
    // pass that creates the node, so once the node exists the keyword search is
    // ready. Re-emit each iteration (subscription race) and poll the 0x03 op until
    // it returns OUR path among the ranked ids. The deadline covers the writer
    // subscription plus one full ~30s promotion interval.
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
        if let Ok(ids) = client.retrieve("retrieval", 10).await {
            if ids.iter().any(|id| id == path) {
                return; // found via BM25 + RRF + confirm, kept by the read-scope filter
            }
        }
        assert!(
            Instant::now() < deadline,
            "the promoted File was never retrievable by keyword via the 0x03 op within 60s"
        );
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

/// IT-1 provenance read (the 0x04 op) + the co-tenant no-leak filter: a File
/// opened by a foreign app is provenance-readable by a scoped caller, but the
/// foreign opener is collapsed to `accessed_by_others` and NEVER named. We emit a
/// `file.opened` with `app_id = "integration-test"` (an actor that is not the
/// caller's own id), let it promote (File + App + ACCESSED_BY), then read
/// provenance under the seeded `system.File` scope. The invariant checked on
/// every in-scope view is the security one: a co-tenant actor must never appear
/// in `actors`. The scenario succeeds once `accessed_by_others` is set, proving
/// the op surfaces foreign access without leaking the principal. `None` before
/// promotion is the no-oracle out-of-scope/absent shape. Same `#[ignore]`
/// rationale (promotion-dependent, ~30s).
#[tokio::test]
#[ignore = "needs event-bus + knowledge binaries built and a FUSE-capable host (~30s)"]
async fn provenance_read_flags_a_foreign_opener_without_naming_it() {
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

    let path = "/work/it/provenance.rs";
    let foreign_app = "integration-test";
    let emitter = UnixEventEmitter::new(stack.producer_socket().to_string_lossy().into_owned());
    let client = UnixGraphClient::new(stack.knowledge_socket().to_string_lossy().into_owned());
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let payload = proto::FileOpenedPayload {
            path: path.to_string(),
            app_id: foreign_app.to_string(),
            flags: 0,
        }
        .encode_to_vec();
        emitter
            .emit("file.opened", payload)
            .await
            .expect("emit file.opened");
        if let Ok(Some(view)) = client.read_provenance(path).await {
            // The security invariant, checked on every in-scope view regardless of
            // timing: the foreign opener is never named to the caller.
            assert!(
                !view.actors.iter().any(|a| a == foreign_app),
                "a co-tenant opener must never appear in the provenance actors, got {:?}",
                view.actors
            );
            if view.accessed_by_others {
                return; // in scope, foreign access flagged but the principal withheld
            }
        }
        assert!(
            Instant::now() < deadline,
            "the promoted File's provenance never flagged the foreign opener within 60s"
        );
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

/// IT-1 access-grants (the 0x05 op) + the LCG connect-time emit: a graph-accessing
/// app appears in its own caller-scoped capability browse, and sees ONLY its own
/// grants. On connect the daemon mints the caller's token from its profile and
/// emits a `Grant` node (living-capability-graph.md §4.1), awaited before the
/// request is served, so the caller's own grant exists by the time `access_grants`
/// reads. The op scopes to the kernel-attested app id (the request carries no
/// scope), so every returned grant must be the caller's own. This also exercises
/// the daemon profile resolver: the connect mint needs the seeded profile to load
/// (`ARLEN_PERMISSIONS_DIR`), so an empty result would mean the mint, hence the
/// resolver, failed. No promotion needed, so this is fast. Same `#[ignore]`
/// rationale (needs the assembled daemons).
#[tokio::test]
#[ignore = "needs event-bus + knowledge binaries built and a FUSE-capable host"]
async fn a_connecting_app_sees_only_its_own_capability_grant() {
    let mut stack = EphemeralStack::new().expect("private runtime root");
    let app_id = stack
        .seed_read_profile(&["system.File.id"])
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
    // The connect-emit is awaited before serving, so the first call should already
    // see the grant; poll briefly to absorb any reconnect interleaving.
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let grants = client.access_grants().await.expect("access_grants ok");
        if !grants.is_empty() {
            assert!(
                grants.iter().all(|g| g.app_id == app_id),
                "access_grants is caller-scoped: every grant must be the caller's own ({app_id}), got {:?}",
                grants.iter().map(|g| g.app_id.clone()).collect::<Vec<_>>()
            );
            return;
        }
        assert!(
            Instant::now() < deadline,
            "the connecting app's own capability grant never appeared in access_grants within 15s"
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// IT-1 Context Capsule materialize (the 0x07 op): a promoted File materializes
/// into a frozen slice rooted at it. Exercises the capsule read end-to-end: the
/// scope selector -> bounded-hop BFS manifest -> projected node load (the
/// `CAPSULE_LABELS` allowlist) -> canonical `FrozenSlice` over the wire. Unlike
/// the other read ops this is NOT RS-R1 gated (it reads the caller's own graph,
/// bounded by construction to File/Project + live FILE_PART_OF, hop- and
/// breadth-capped), so no read profile is seeded; the materialize poll itself is
/// the promotion check (an empty slice until the File node exists, then it
/// contains it). Asserts the root resolves to a `File`-labelled node, proving the
/// projected load picked the right label. Same `#[ignore]` rationale
/// (promotion-dependent, ~30s).
#[tokio::test]
#[ignore = "needs event-bus + knowledge binaries built and a FUSE-capable host (~30s)"]
async fn a_promoted_file_materializes_into_a_capsule_slice() {
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

    let path = "/work/it/capsule.rs";
    let emitter = UnixEventEmitter::new(stack.producer_socket().to_string_lossy().into_owned());
    let client = UnixGraphClient::new(stack.knowledge_socket().to_string_lossy().into_owned());
    // A single-node capsule rooted at the file (no expansion); the root is always
    // included in the manifest, so once the File node exists it is loaded.
    let scope = arlen_capsule::scope::CapsuleScope {
        roots: vec![path.to_string()],
        expand_hops: 0,
    };
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
        if let Ok(slice) = client.materialize_capsule(&scope).await {
            if slice.nodes.iter().any(|n| n.id == path && n.label == "File") {
                return; // the promoted File materialized into the frozen slice
            }
        }
        assert!(
            Instant::now() < deadline,
            "the promoted File never materialized into a capsule slice within 60s"
        );
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

/// IT-1 revoke (the 0x06 op) + the LCG strict-narrowing gate: a caller narrows a
/// target app's read profile, removing one of two field patterns. The daemon's
/// revoke caller-allowlist admits the canonical `settings` principal and, in a
/// debug build, the `dev.`-prefixed test caller, so this exercises the real
/// narrowing path end-to-end (not a deny). We seed a target profile with two read
/// patterns for a DIFFERENT app, revoke one, and assert `RevokeOutcome::Revoked`:
/// the strict-subset gate confirms the raw pattern set shrank (the prior sweep's
/// field-level fix, since both patterns share the `system.File` entity type), and
/// the on-disk profile is then narrowed. A `User` initiator is required (§6.3
/// refuses an agent-initiated revoke). The target lookup also depends on the
/// daemon resolving `ARLEN_PERMISSIONS_DIR`. Same `#[ignore]` rationale, and it
/// only narrows in a debug build (the dev. caller admission).
#[tokio::test]
#[ignore = "needs event-bus + knowledge binaries built (debug, for the dev. caller admission) and a FUSE-capable host"]
async fn revoke_narrows_a_target_profiles_read_scope() {
    use arlen_permissions::revoke::{RevokeInitiator, RevokeOutcome, RevokeReach, RevokedReach};

    let mut stack = EphemeralStack::new().expect("private runtime root");
    let target = "com.example.target";
    stack
        .seed_profile_for(target, &["system.File.id", "system.File.path"])
        .expect("seed target profile");
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
    let req = RevokeReach {
        target_app_id: target.to_string(),
        reach: RevokedReach::Read {
            entity_pattern: "system.File.path".to_string(),
        },
        initiator: RevokeInitiator::User,
    };
    let outcome = client.revoke(&req).await.expect("revoke round-trips");
    assert_eq!(
        outcome,
        RevokeOutcome::Revoked,
        "removing one of two read patterns is a strict narrowing the gate must accept"
    );

    // The on-disk target profile is narrowed: the revoked pattern is gone, the
    // other kept.
    let body = std::fs::read_to_string(stack.permissions_dir().join(format!("{target}.toml")))
        .expect("read the narrowed target profile");
    assert!(
        !body.contains("system.File.path"),
        "the revoked read pattern must be removed from the profile, got:\n{body}"
    );
    assert!(
        body.contains("system.File.id"),
        "the unrevoked read pattern must remain, got:\n{body}"
    );
}

/// IT-1 project detection: a directory bearing a project signal (a `.git` entry)
/// is detected by the knowledge daemon's project watcher and promoted to a graph
/// `Project` node. Exercises the detection pipeline end-to-end (watcher scan ->
/// signal detection -> `ProjectStore::create` -> graph node), driven by a
/// controlled fixture the hermetic `XDG_CONFIG_HOME` graph.toml points the watcher
/// at (never the dev's real repos). A `system.Project` read grant lets the caller
/// read the node back; the watcher scans on startup so the node appears shortly
/// after the socket binds. Same `#[ignore]` rationale.
#[tokio::test]
#[ignore = "needs event-bus + knowledge binaries built and a FUSE-capable host"]
async fn a_signal_bearing_directory_is_detected_as_a_project() {
    let mut stack = EphemeralStack::new().expect("private runtime root");
    // A fixture directory carrying a `.git` signal; the watcher must scan only it.
    let fixture = stack.runtime_dir().join("proj-fixture");
    std::fs::create_dir_all(fixture.join(".git")).expect("create .git fixture");
    stack
        .seed_project_watch_dir(&fixture)
        .expect("point the watcher at the fixture");
    stack
        .seed_read_profile(&["system.Project.id", "system.Project.name"])
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
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Ok(rows) = client
            .query_rows("MATCH (p:Project) RETURN p.id LIMIT 1")
            .await
        {
            if !rows.is_empty() {
                return; // the .git fixture was detected and promoted to a Project node
            }
        }
        assert!(
            Instant::now() < deadline,
            "the signal-bearing directory was never detected as a Project node within 20s"
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// IT-1 audit subsystem: the audit daemon comes up hermetically and binds both
/// its sockets. The foundation for the audit-chain scenarios (an admitted
/// component writing an entry that a reader verifies): it proves `arlen-auditd`
/// starts under the harness env contract, persisting its HMAC key + ledger under
/// the private `XDG_DATA_HOME` (not the dev's real `~/.local/share`) and binding
/// the ingest + read sockets under `$XDG_RUNTIME_DIR/arlen/`. The daemon
/// `create_dir_all`s its data dir and socket parents itself, so the harness only
/// points the env at the temp root. Same `#[ignore]` rationale.
#[tokio::test]
#[ignore = "needs event-bus + audit-daemon binaries built and a per-user data dir"]
async fn the_audit_daemon_comes_up_hermetically() {
    let mut stack = EphemeralStack::new().expect("private runtime root");
    // The event bus first: the audit daemon opens a producer client for its
    // `audit.tampered` alert (lazily, so a late bus is fine, but spawn it for a
    // realistic assembled context).
    stack
        .spawn("daemons/event-bus", "event-bus", &[])
        .expect("spawn event-bus");
    stack
        .wait_socket("event-bus-producer.sock", Duration::from_secs(20))
        .expect("producer socket");

    stack
        .spawn("daemons/audit-daemon", "arlen-auditd", &[])
        .expect("spawn audit-daemon");
    // The sockets live under the `arlen/` subdir of the runtime root; the daemon
    // creates that dir when it binds.
    stack
        .wait_socket("arlen/audit-ingest.sock", Duration::from_secs(20))
        .expect("audit ingest socket appears");
    stack
        .wait_socket("arlen/audit-read.sock", Duration::from_secs(20))
        .expect("audit read socket appears");

    // Both sockets bound: the audit daemon is up, hermetic (key + ledger under the
    // temp XDG_DATA_HOME). Dropping `stack` tears it down and removes the root.
    assert!(stack.audit_ingest_socket().exists());
    assert!(stack.audit_read_socket().exists());
}

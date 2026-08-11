/// Graph Daemon: Unix socket server for Cypher queries with token auth.
///
/// Phase 1A: Read-only queries, no authentication.
/// Phase 3.2: Token-based authentication added. Clients receive a
///   CapabilityToken at connection time; each query must pass token
///   verification and scope checks.
///
/// Protocol:
///   Client sends:  4-byte BE length + UTF-8 Cypher string
///   Server replies: 4-byte BE length + UTF-8 result string
///
/// See `docs/architecture/DAEMON-COMMUNICATION.md` Section 8.

use std::collections::HashMap;
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use audit_proto::{AuditSink, LedgerAuditSink};
use prost::Message;
use serde::Deserialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::Mutex;
use tracing::{debug, error, info, warn};

use crate::auth::Authenticator;
use crate::events::{self, GraphEvent};
use crate::graph::GraphHandle;
use crate::identity::{app_id_from_pid, pid_start_time, process_alive};
use crate::proto::Event;
use crate::quota::{AppTier, QuotaConfig, RateLimiter};
use crate::schema::SchemaRegistry;
use crate::utils::{content_merge_key, escape_cypher};
use crate::write::{create_relation, retract_relation, RelationResult};

/// One `graph.rate_limited` event per app at most this often, so a
/// query flood does not hammer the Event Bus producer.
const EMIT_THROTTLE: Duration = Duration::from_secs(5);

/// Upper bound on a write request's `op_id` (the agent's operation digest is a
/// fixed-length hex string; this bounds an abusive caller's literal).
const MAX_OP_ID_LEN: usize = 128;

/// Per-identity rate-limit state shared across all query connections,
/// so a caller's many connections share one token bucket (per
/// *identity*, not per connection).
struct RateState {
    limiter: RateLimiter,
    last_emit: HashMap<String, Instant>,
}

impl RateState {
    fn new() -> Self {
        Self {
            limiter: RateLimiter::new(QuotaConfig::arlen_default()),
            last_emit: HashMap::new(),
        }
    }

    /// Whether a violation event should be emitted for `app_id` now
    /// (edge-throttled to one per [`EMIT_THROTTLE`]).
    fn should_emit(&mut self, app_id: &str) -> bool {
        let now = Instant::now();
        match self.last_emit.get(app_id) {
            Some(&t) if now.duration_since(t) < EMIT_THROTTLE => false,
            _ => {
                self.last_emit.insert(app_id.to_string(), now);
                true
            }
        }
    }
}

/// Fire-and-forget emitter for `graph.rate_limited` Event Bus events.
struct RateLimitEmitter {
    socket_path: PathBuf,
}

impl RateLimitEmitter {
    fn new() -> Self {
        let path = crate::utils::socket_path("ARLEN_PRODUCER_SOCKET", "event-bus-producer.sock");
        Self {
            socket_path: PathBuf::from(path),
        }
    }

    /// Emit `graph.rate_limited`; the payload is the offending app_id.
    /// Consumed by the Anomaly Detector (foundation §8.4). Best-effort.
    fn emit(&self, app_id: &str) {
        let event = Event {
            id: uuid::Uuid::now_v7().to_string(),
            r#type: "graph.rate_limited".to_string(),
            timestamp: chrono::Utc::now().timestamp_micros(),
            source: "knowledge".to_string(),
            pid: std::process::id(),
            // The Event Bus rejects an empty session_id; a daemon has
            // no user session, so a stable daemon identifier is used.
            // The daemon itself, in no user session: a named system source.
            origin: "system:knowledge-daemon".to_string(),
            payload: app_id.as_bytes().to_vec(),
            uid: unsafe { libc::getuid() },
            project_id: String::new(),
        };
        let encoded = event.encode_to_vec();
        let len = (encoded.len() as u32).to_be_bytes();
        if let Ok(mut stream) = std::os::unix::net::UnixStream::connect(&self.socket_path) {
            use std::io::Write;
            let _ = stream
                .write_all(&len)
                .and_then(|_| stream.write_all(&encoded))
                .and_then(|_| stream.flush());
        }
    }
}

/// `SO_PEERCRED` → `(pid, uid)` of the peer that opened `fd`.
fn so_peercred(fd: std::os::unix::io::RawFd) -> std::io::Result<(i32, u32)> {
    let mut cred = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: cred + len are valid for the call; fd is a live socket.
    let r = unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            &mut cred as *mut _ as *mut libc::c_void,
            &mut len,
        )
    };
    if r != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok((cred.pid, cred.uid))
}

/// Sleep a small random jitter (0–10 ms) before a reply so two
/// *equivalent* queries are not timing-distinguishable (foundation
/// §8.4: "a small random noise value added"). Uses `getrandom`
/// (already a dependency).
///
/// Scope: this is additive noise, the mechanism the foundation
/// describes. It makes equivalent queries indistinguishable, but it
/// does not pad cost-dependent runtime to a fixed floor, so the cost
/// *difference* between two structurally-different queries can still
/// be recovered by an attacker who samples within the rate limit.
/// Bucketed/fixed response deadlines per query class are the stronger
/// follow-up; additive noise is the §8.4 baseline.
async fn timing_noise() {
    let mut b = [0u8; 1];
    let jitter = if getrandom::getrandom(&mut b).is_ok() {
        (b[0] % 11) as u64
    } else {
        5
    };
    tokio::time::sleep(Duration::from_millis(jitter)).await;
}

/// Start the Graph Daemon listener and event subscriber.
///
/// Spawns two concurrent tasks:
/// 1. Socket listener for client queries.
/// 2. Event Bus subscriber for permission/schema change events.
pub async fn listen(
    socket_path: &str,
    graph: GraphHandle,
    pool: sqlx::SqlitePool,
    gate: crate::activity_delete::PromotionGate,
) -> Result<()> {
    let auth = Arc::new(Mutex::new(Authenticator::new()));
    info!("graph daemon: HMAC key generated");

    // LCG-R3 (living-capability-graph.md §4.2): the restart liveness sweep. The
    // HMAC key was just regenerated, so every persisted Grant node's projected
    // token is provably dead. Mark them all not-live here, after the fresh
    // Authenticator is built and BEFORE the socket binds (below), so no node
    // claims a live token that cannot verify and no re-mint races the sweep; apps
    // re-mint live one at a time as they reconnect. Best-effort: a failed sweep
    // only degrades the browse projection (the read command re-checks liveness
    // with process_alive), so it must not block startup.
    if let Err(e) = graph
        .write("MATCH (g:Grant) WHERE g.live = true SET g.live = false".to_string())
        .await
    {
        warn!("LCG liveness sweep failed (browse projection may show stale live): {e}");
    }

    // Per-identity rate limiting + the violation emitter are shared
    // across all query connections.
    let rate = Arc::new(Mutex::new(RateState::new()));
    let emitter = Arc::new(RateLimitEmitter::new());
    // Grant exercises, tallied in memory and flushed coarsely (living-capability-graph.md
    // §4). Shared across connections because the counter is per (app, capability),
    // not per connection or per token.
    let uses = Arc::new(Mutex::new(crate::lcg::UseTally::new(crate::time::now().0)));

    // Schema registry for write-mode relation validation. Built with the
    // compiled-in system entity types only; that is sufficient for the agent's
    // built-in system relations (the only write op today), since
    // Load app-declared entity schemas (foreign-app-bridges.md): without this
    // the registry is empty, so an app can declare custom entity types but
    // never write instances of them. `load_all` is fail-soft - a missing
    // schema dir is a no-op and a bad file warns and is skipped - so a schema
    // problem never blocks daemon startup.
    let registry = {
        let mut reg = SchemaRegistry::new(vec![]);
        if let Err(e) = reg.load_all() {
            warn!("failed to load entity schemas at startup: {e}");
        }
        Arc::new(reg)
    };

    // The audit sink for the app-tier entity-write path (foreign-app-bridges):
    // every upsert is recorded fail-closed before it persists (S13). The actor
    // is set by the audit daemon from this daemon's SO_PEERCRED (`knowledge`);
    // the calling app goes in the record as a coarse identifier.
    let audit: Arc<dyn AuditSink> = Arc::new(LedgerAuditSink::at_default_socket());

    tokio::try_join!(
        listen_queries(
            socket_path,
            graph.clone(),
            pool,
            gate,
            auth.clone(),
            rate,
            emitter,
            registry,
            audit,
            uses,
        ),
        listen_events(auth, graph),
    )?;

    Ok(())
}

/// Accept and handle client connections.
#[allow(clippy::too_many_arguments)]
async fn listen_queries(
    socket_path: &str,
    graph: GraphHandle,
    pool: sqlx::SqlitePool,
    gate: crate::activity_delete::PromotionGate,
    auth: Arc<Mutex<Authenticator>>,
    rate: Arc<Mutex<RateState>>,
    emitter: Arc<RateLimitEmitter>,
    registry: Arc<SchemaRegistry>,
    audit: Arc<dyn AuditSink>,
    uses: Arc<Mutex<crate::lcg::UseTally>>,
) -> Result<()> {
    if Path::new(socket_path).exists() {
        std::fs::remove_file(socket_path)?;
    }
    if let Some(parent) = Path::new(socket_path).parent() {
        std::fs::create_dir_all(parent)?;
    }

    let listener = UnixListener::bind(socket_path)?;

    // SAFETY: getuid() has no preconditions and cannot fail.
    let our_uid = unsafe { libc::getuid() };

    // Optional owner-uid restriction (ARLEN_OWNER_UID). When set - the desktop
    // session user, which a multi-user deployment supplies - a CROSS-uid
    // first-party/system peer is served only if it IS that owner, so a different
    // human user cannot reach this user's graph even via a canonical binary.
    // Unset = single-user default: any first-party/system cross-uid peer (the
    // local user's AI layer) is served.
    // A name is preferred over a number because the unit that needs this most is
    // the image's system-service drop-in, where the socket sits at the shared
    // /run/arlen and the desktop user's uid is whatever `useradd` picked. A unit
    // hardcoding 1000 is a guess that fails silently in the wrong direction: it
    // would refuse the real user and leave the graph unreachable.
    let uid_env = std::env::var("ARLEN_OWNER_UID").ok();
    let user_env = std::env::var("ARLEN_OWNER_USER").ok();
    let owner_uid = crate::utils::resolve_owner_uid(
        uid_env.as_deref(),
        user_env.as_deref(),
        crate::utils::uid_for_user,
    );
    if owner_uid.is_none() && std::env::var_os("ARLEN_OWNER_USER").is_some() {
        // Naming an owner that cannot be resolved is a deployment mistake, and
        // silently serving every cross-uid peer is the opposite of what was asked
        // for. Say so; the operator sees it in the journal on the first boot.
        tracing::warn!("ARLEN_OWNER_USER is set but does not resolve to a user; serving any first-party cross-uid peer");
    }

    apply_socket_exposure(socket_path, socket_exposure(owner_uid, our_uid))?;
    info!(socket = socket_path, "graph daemon listening");

    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                let graph = graph.clone();
                let pool = pool.clone();
                let gate = gate.clone();
                let auth = auth.clone();
                let rate = rate.clone();
                let emitter = emitter.clone();
                let registry = registry.clone();
                let audit = audit.clone();
                let uses = uses.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_client(
                        stream, graph, pool, gate, auth, rate, emitter, registry, our_uid,
                        owner_uid, audit, uses,
                    )
                    .await
                    {
                        error!("graph daemon client error: {e}");
                    }
                });
            }
            Err(e) => error!("graph daemon accept error: {e}"),
        }
    }
}

/// Subscribe to Event Bus and process permission/schema events.
async fn listen_events(auth: Arc<Mutex<Authenticator>>, graph: GraphHandle) -> Result<()> {
    let uid = unsafe { libc::getuid() };
    let consumer_id = format!("graph-daemon-{uid}");

    // Event Bus connection is optional -- daemon works without it.
    let mut stream = match events::connect(&consumer_id, uid).await {
        Ok(s) => {
            info!("graph daemon: connected to event bus");
            s
        }
        Err(e) => {
            warn!("graph daemon: event bus not available ({e}), running without live updates");
            // Block forever so try_join doesn't exit.
            std::future::pending::<()>().await;
            return Ok(());
        }
    };

    loop {
        match events::recv_event(&mut stream).await {
            Some(event) => {
                handle_graph_event(&auth, &graph, event).await;
            }
            None => {
                warn!("graph daemon: event bus disconnected, attempting reconnect");
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                match events::connect(&consumer_id, uid).await {
                    Ok(s) => {
                        stream = s;
                        info!("graph daemon: reconnected to event bus");
                    }
                    Err(_) => {
                        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    }
                }
            }
        }
    }
}

/// Mark an app's live Grant nodes stale (living-capability-graph.md §4.2): a
/// token invalidation means the projected reach no longer verifies. Best-effort,
/// like the restart sweep: a failed update only degrades the browse projection
/// (the read command re-checks liveness with `process_alive`).
async fn mark_app_grants_stale(graph: &GraphHandle, app_id: &str) {
    if let Err(e) = graph
        .write(format!(
            "{} WHERE g.live SET g.live = false",
            crate::cypher::match_node_by_field("g", "Grant", "app_id", app_id)
        ))
        .await
    {
        warn!("marking {app_id} grants stale failed: {e}");
    }
}

/// Remove an app's Grant nodes entirely (living-capability-graph.md §13, LCG-R6
/// uninstall cleanup): when the profile is gone, the grants are orphaned
/// projection (the app will never re-mint and the durable authority history lives
/// in the audit ledger, not here), so a `DETACH DELETE` removes them and their
/// `GRANTS`/`USED_BY` edges rather than leaving them as misleading "dormant"
/// rows. Best-effort like the stale-mark.
async fn remove_app_grants(graph: &GraphHandle, app_id: &str) {
    if let Err(e) = graph
        .write(format!(
            "{} DETACH DELETE g",
            crate::cypher::match_node_by_field("g", "Grant", "app_id", app_id)
        ))
        .await
    {
        warn!("removing {app_id} grants on uninstall failed: {e}");
    }
}

/// Whether the app still has a profile on disk. A `permission.changed` for an app
/// whose profile is gone is an uninstall (the projection must be cleaned up), not
/// a narrowing (the projection stays stale and re-mints).
fn profile_exists(app_id: &str) -> bool {
    // Both tiers: this decides whether a permission change is an EDIT (reproject
    // the declared grants) or an UNINSTALL (delete them). A system-tier-only app,
    // which is what an apt-installed one looks like, has no user file at all, so
    // asking that tier alone would read every edit to it as an uninstall and drop
    // the grants of an app that is still installed.
    arlen_permissions::profile_exists(app_id)
}

/// Process a graph-relevant event.
async fn handle_graph_event(
    auth: &Arc<Mutex<Authenticator>>,
    graph: &GraphHandle,
    event: GraphEvent,
) {
    match event {
        GraphEvent::PermissionChanged { app_id } => {
            info!("permission changed for {app_id}");
            if profile_exists(&app_id) {
                // A narrowing/edit: the prior live reach no longer verifies, so
                // mark it stale; the next connection re-mints at the new ceiling.
                mark_app_grants_stale(graph, &app_id).await;
                // Project the profile's DECLARED grants now, driven from the
                // profile alone (no running pid). An installed-but-never-run app,
                // or one that only ever touches non-graph dimensions, would else
                // have a profile on disk but zero Grant nodes until its first
                // graph connect (E1: install-time projection). Idempotent - the
                // declared emit MERGEs and its revoke-preserving ON MATCH never
                // resurrects a user-revoked grant. Best-effort.
                if let Ok(profile) = arlen_permissions::load_profile(&app_id) {
                    if let Err(e) =
                        crate::lcg::emit_all_declared_grants(graph, &app_id, &profile).await
                    {
                        warn!(app_id = %app_id, "enroll-time declared grants emit failed: {e}");
                    }
                }
            } else {
                // The profile is gone (uninstall): the grants are orphaned, remove
                // them so the browse surface does not show a dead app as dormant.
                remove_app_grants(graph, &app_id).await;
            }
        }
        GraphEvent::AiLevelChanged => {
            info!("AI level changed");
            mark_app_grants_stale(graph, "ai-daemon").await;
        }
        GraphEvent::SchemaRegistered { app_id } => {
            info!("schema registered: {app_id}");
            // Schema loading comes in Phase 3.3.
        }
        GraphEvent::SchemaRemoved { app_id } => {
            info!("schema removed: {app_id}");
        }
    }
}

/// The kernel-attested write peer: the SO_PEERCRED pid plus that pid's start
/// time captured at connection. The start time is the PID-reuse guard (E7): a
/// write re-reads it and refuses if it changed, so a recycled pid (the original
/// peer exited and the number was reused by another process) cannot borrow the
/// connection's authority. `start_time` is `None` only if it could not be read
/// at connection, in which case a write fails closed (reuse is unguardable).
#[derive(Clone, Copy)]
struct WritePeer {
    pid: u32,
    /// The peer's uid, carried because it selects WHOSE permission profile the
    /// token is minted from. The daemon is root, so its own uid names the wrong
    /// set; the profile that applies is filed under the connecting user's.
    uid: u32,
    start_time: Option<u64>,
}

/// A structured graph write request, sent with a leading `0x02` byte (the
/// write-mode prefix, beside the legacy raw-Cypher text query and the `0x01`
/// typed-rows query). The body is JSON, tagged by `op`.
///
/// The boundary is deliberately narrow: the only ops are creating a built-in
/// graph relation between two existing nodes, and retracting (compensating) a
/// relation the caller previously created, keyed by its operation id. There is
/// no free-form Cypher write path. The agent's executor is the only intended
/// caller, and the authorisation primitives (`create_relation` /
/// `retract_relation`) refuse anything outside the declared relation allowlist
/// regardless of what is sent.
#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum WriteRequest {
    /// Create `from -[relation_type]-> to` between two existing nodes,
    /// identified by their namespaced entity type and concrete id.
    CreateRelation {
        from_type: String,
        from_id: String,
        to_type: String,
        to_id: String,
        relation_type: String,
        /// Durable operation identity: the caller's stable id for this logical
        /// write, persisted on the edge so a lost-response retry can reconcile
        /// by asking whether *this* operation's edge exists. Optional; an empty
        /// id is not persisted (the edge's `op_id` stays NULL, as for the
        /// promotion pipeline). Only `FILE_PART_OF` carries the column today.
        #[serde(default)]
        op_id: String,
    },
    /// Retract (compensate) a relation the caller previously created, deleting
    /// only the edge that carries this exact `op_id`. The `op_id` is mandatory
    /// and non-empty: a retract is always a precise undo of the caller's own
    /// write, never a bare-edge delete. Idempotent (a no-match is success).
    RetractRelation {
        from_type: String,
        from_id: String,
        to_type: String,
        to_id: String,
        relation_type: String,
        op_id: String,
    },
    /// Create a node of a bounded built-in type at a caller-supplied id, guarded
    /// so it can only ever create, never overwrite (bitemporal-knowledge-graph.md
    /// §5.3). The id is the caller's own (e.g. a deterministic UUIDv5), checked
    /// label-agnostically so a foreign-label id collision is refused. The only
    /// creatable types are the consolidation node types ([`CREATABLE_NODES`]);
    /// node fields and the transaction-time stamp are a later increment.
    CreateNode {
        /// The namespaced node type (e.g. `system.Summary`).
        node_type: String,
        /// The caller-supplied node id.
        id: String,
    },
    /// Upsert (create-or-update) an instance of the CALLER'S OWN declared entity
    /// type, keyed by a stable `external_key` so a re-sync strengthens the same
    /// node rather than duplicating (foreign-app-bridges piece 1). The type must
    /// be in the caller's namespace and registered; `system.*`/`shared.*` are
    /// structurally unwritable. This is the general app-tier instance-write path
    /// that the declare-but-cannot-write gap needs.
    UpsertEntity {
        /// The caller's namespaced entity type (e.g. `md.obsidian.Note`).
        qualified_type: String,
        /// The bridge's stable idempotency key for this instance.
        external_key: String,
        /// The instance's field values, validated against the registered schema.
        #[serde(default)]
        fields: std::collections::HashMap<String, serde_json::Value>,
        /// Optional upstream provenance for this write: which sync run produced
        /// it and what it was in the source system. Stamped by the daemon into
        /// reserved `_`-prefixed columns, so a caller cannot forge it through
        /// `fields` (schema validation refuses a reserved name there).
        #[serde(default)]
        origin: Option<WireOrigin>,
    },
    /// Link two instances of the CALLER'S OWN declared entity types with an edge
    /// (foreign-app-bridges piece 2). Both endpoints must be in the caller's
    /// namespace, registered, and token-writable; the edge is idempotent (a
    /// re-sync never duplicates it). Endpoints are addressed by their stable
    /// external keys; the daemon owns the deterministic id scheme.
    LinkEntities {
        /// The edge (relation) label to create.
        edge_type: String,
        /// The source node's namespaced entity type.
        from_type: String,
        /// The source node's external key.
        from_key: String,
        /// The target node's namespaced entity type.
        to_type: String,
        /// The target node's external key.
        to_key: String,
    },
    /// Merge a duplicate instance into a canonical one (SHARED-ENTITIES.md §Merge
    /// Flow steps 3-4): re-point every edge of the duplicate onto the canonical,
    /// then delete the duplicate. Only a `shared.*` type's declared owner app may
    /// merge its instances (a non-shared type stays in the caller's namespace);
    /// the field-data merge (steps 1-2) is the owner's separate write. Both keys
    /// address instances of the same `qualified_type`.
    MergeEntities {
        /// The namespaced entity type of both instances.
        qualified_type: String,
        /// The external key of the duplicate to fold in and delete.
        duplicate_key: String,
        /// The external key of the canonical instance that survives.
        canonical_key: String,
    },
    /// Delete the user's own recorded activity at or after `from`, for good
    /// (`bitemporal-knowledge-graph.md`, ruled 9 August). The one place the
    /// close-never-delete graph is genuinely destroyed, because the audit
    /// guarantee exists so the user can hold the SYSTEM to account and lives in a
    /// different store, which this never touches. Only the knowledge app may ask.
    DeleteActivity {
        /// Unix seconds; everything recorded at or after this instant goes.
        from: i64,
    },
    /// Persist a consent grant into the shared LCG Grant node (system-dialog-
    /// plan.md, Option A): the durable half of the consent lifecycle, surfaced by
    /// the `access_grants` read in the same see+revoke place. Only the consent
    /// broker may call it; the grant is keyed by its revocation handle so a
    /// re-consent strengthens the same node.
    PersistConsentGrant {
        /// The app the grant authorises (the Grant node's `app_id`).
        recipient: String,
        /// The consent class.
        consent_class: String,
        /// The concrete scope, when there is one.
        #[serde(default)]
        consent_scope: Option<String>,
        /// The stable revocation handle = the Grant node id.
        revocation_handle: String,
        /// When a time-boxed consent stops authorising, in epoch microseconds.
        /// Absent for a grant that lasts until it is revoked.
        #[serde(default)]
        expires_at_micros: Option<i64>,
    },
    /// File a produced meeting note as a `Meeting` node with its `ActionItem`
    /// children (agent-work-surfaces). The summary + action items are AI-derived
    /// from the transcript; only the meetings app / AI engine may file. Idempotent
    /// on `id`, so a re-file updates the note in place rather than duplicating.
    FileMeeting {
        /// The app-minted meeting id.
        id: String,
        /// The note title.
        title: String,
        /// The prose summary.
        summary: String,
        /// Participant display names, in listing order.
        #[serde(default)]
        participants: Vec<String>,
        /// The extracted action items.
        #[serde(default)]
        action_items: Vec<MeetingActionItemInput>,
        /// The recording start, microseconds since epoch.
        started_at: i64,
    },
}

/// The upstream provenance a bridge may attach to an entity upsert: the sync run
/// that produced the write and the ref it came from in the source system.
///
/// Deliberately a separate wire struct rather than two loose fields, so a caller
/// cannot send half of it; `WriteOrigin::new` then rejects an empty or oversized
/// half before the write is planned.
#[derive(Debug, Deserialize)]
struct WireOrigin {
    /// The sink's id for this sync run, opaque to the daemon.
    run_id: String,
    /// What this row was upstream, opaque to the daemon.
    origin_ref: String,
}

/// One action item in a [`WriteRequest::FileMeeting`] request.
#[derive(Debug, Deserialize)]
struct MeetingActionItemInput {
    /// The task text.
    text: String,
    /// The owner, when the extractor attributed one.
    #[serde(default)]
    owner: Option<String>,
}

/// The node types creatable via the `0x02` write socket: the consolidation node
/// types only (§5.3). A narrow allowlist, like `BUILTIN_RELATIONS` for edges, so
/// a token write-scope alone cannot create an arbitrary node label.
const CREATABLE_NODES: &[&str] = &["system.Summary"];

/// The reserved canary id namespace (canary-honeytools.md §3). No node may be
/// created whose id mentions this token, so the agent's structural-canary operand
/// check stays zero-false-positive: an honest node id can never contain it, so an
/// operand (or an embedded read-query id) that does is provably injected. The
/// `CreateNode` op is the only node-create path that takes a caller-supplied id
/// (promotion ids are derived paths, entity ids are server-minted UUIDv7), so the
/// reservation lives at its persistence primitive. With the native agent retired
/// (pi is the loop now), this ingestion boundary is the sole canary authority: the
/// reservation here refuses the write AND fires the content-free trip audit (see
/// `canary_trip_event`) the anomaly detector surfaces. A future pi-side operand
/// tripwire must reuse this exact token string; there is no shared dependency, so
/// any such duplication is deliberate.
const RESERVED_CANARY_TOKEN: &str = "__canary:";

/// An LLM-free retrieval request, sent with a leading `0x03` byte. The body is
/// JSON; the response is a JSON array of ranked node ids.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RetrieveRequest {
    /// The keyword query.
    query: String,
    /// The maximum number of results, clamped to `[1, MAX_RETRIEVE_LIMIT]`.
    #[serde(default = "default_retrieve_limit")]
    limit: i64,
}

/// The default retrieve result cap when the request omits `limit`.
fn default_retrieve_limit() -> i64 {
    20
}

/// The hard ceiling on a retrieve request's result count, so a caller cannot ask
/// for an unbounded fused/confirmed set.
const MAX_RETRIEVE_LIMIT: i64 = 100;

/// A prep-for-this request, sent with a leading `0x09` byte (agent-work-surfaces-
/// plan.md surface 3). The body is JSON `{subject_id, limit}`; the response is a
/// JSON array of ranked [`crate::prep::PrepItem`] (live-and-important first, noise
/// dropped), or the plaintext `ERROR:` form. A pure read of the caller's own graph
/// (like retrieve), so it is rate-limited and its returned ids are scoped to the
/// caller's readable labels for a non-system caller.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PrepRequest {
    /// The entity to prep for (a project / file / ... node id).
    subject_id: String,
    /// The maximum number of prep items, clamped to `[1, MAX_PREP_LIMIT]`.
    #[serde(default = "default_prep_limit")]
    limit: i64,
}

/// The default prep item cap when the request omits `limit`.
fn default_prep_limit() -> i64 {
    20
}

/// The hard ceiling on a prep view's item count.
const MAX_PREP_LIMIT: i64 = 100;

/// A live-working-set request, sent with a leading `0x0A` byte (agent-work-surfaces-
/// plan.md surface 2, the briefing engine). The body is JSON `{limit}`; the response
/// is a JSON array of ranked [`crate::prep::PrepItem`] across the caller's graph
/// (the briefing digest's ranked foundation). A pure read scoped to the caller's
/// readable labels, like prep.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkingSetRequest {
    /// The maximum number of items, clamped to `[1, MAX_PREP_LIMIT]`.
    #[serde(default = "default_prep_limit")]
    limit: i64,
}

/// A request to list pending shared-entity merge suggestions, sent with a leading
/// `0x0F` byte (SHARED-ENTITIES.md). An optional `entity_type` narrows to one
/// shared type (e.g. `"shared.Person"`); `limit` is clamped to `[1, MAX_PREP_LIMIT]`.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ListSuggestionsRequest {
    /// Narrow to one shared entity type, or all pending suggestions if absent.
    #[serde(default)]
    entity_type: Option<String>,
    /// The maximum number of suggestions, clamped to `[1, MAX_PREP_LIMIT]`.
    #[serde(default = "default_prep_limit")]
    limit: i64,
}

/// The hard ceiling on a capsule scope's hop expansion, so a request cannot walk
/// the whole graph from a high-degree root (the relation-type / count over-share
/// controls are the CC-R6 mint preview; this is the coarse DoS bound). The manifest
/// breadth is separately capped in `capsule::capsule_expand`.
const MAX_CAPSULE_HOPS: u32 = 6;

/// The wall-clock bound on a capsule materialize. Unlike the single-probe read ops
/// (a 500ms timeout), the materialize does O(manifest) graph reads, so it gets a
/// few seconds; the manifest cap keeps that bounded.
const CAPSULE_MATERIALIZE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// A caller-scoped provenance read request, sent with a leading `0x04` byte
/// (provenance-halo.md §5). The body is JSON; the response is the object's
/// scoped provenance or a uniform out-of-scope denial.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProvenanceRequest {
    /// The graph node id whose provenance is requested.
    object_id: String,
}

/// The built-in observation-graph node labels a token may read, derived from its
/// read scopes (provenance-halo.md §5). A read scope's entity type is namespaced
/// (`system.File`); the observation graph's node labels are the unprefixed system
/// names (`File`), so only `system.*` scopes map to a probeable label and the
/// prefix is stripped. App-namespaced types (`com.x.Y`) are not built-in
/// observation nodes and contribute no label. The result is validated to safe
/// identifiers, since it is interpolated into the per-label existence probe that
/// makes an out-of-scope object indistinguishable from an absent one.
/// Whether `name` is a graph identifier safe to interpolate into a query.
///
/// A Cypher label or relationship type is a letter or underscore followed by
/// letters, digits and underscores, and this accepts exactly that: it must
/// exclude anything that could end the identifier and start something else, and
/// it must not exclude a name the graph really uses.
///
/// **Underscores are the point.** The previous rule was alphanumeric-only, which
/// silently dropped every name carrying one - no node label uses one today, so
/// nothing was visibly broken, but every relationship type does (`FILE_PART_OF`,
/// `ACCESSED_BY`), and a filter that quietly rejects valid names is a defect
/// whether or not something is currently hitting it. It would have met any
/// traversal work head-on: the first edge type to be authorised would have been
/// dropped by the predicate rather than by a decision.
fn is_safe_graph_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn readable_system_labels(read_scopes: &[crate::token::EntityScope]) -> Vec<String> {
    read_scopes
        .iter()
        .filter_map(|s| s.entity_type.strip_prefix("system."))
        .filter(|l| is_safe_graph_identifier(l))
        .map(str::to_string)
        .collect()
}

/// The uniform denial for the provenance read op. Every failure mode (malformed
/// request, no or changed peer, out-of-scope, absent object) returns this one
/// shape so a spoofed reference cannot become a file-existence oracle by shape;
/// the caller pairs it with `timing_noise()` so timing cannot either.
const PROVENANCE_OUT_OF_SCOPE: &str = "ERROR: OutOfScope";

/// The uniform denial for the code-analysis op (CG-R5): a caller that is not
/// system-anchored may not run the whole-codebase analysis, and any failure
/// returns the same shape (no oracle).
const CODE_ANALYSIS_DENIED: &str = "ERROR: code analysis is not permitted for this caller";

/// CG-R5: the whole-codebase analysis read — god-symbols (degree-centrality
/// hubs) and surprises (sole cross-module call bridges) over the
/// `CodeSymbol`/`CALLS` graph, token-free (no LLM; the AI explains on top).
///
/// Unlike the caller-scoped grant/provenance reads, this is an AGGREGATE over
/// the entire code index, so it is gated to **system-anchored** callers (a
/// resolved, non-`unknown` app id whose quota tier is above ThirdParty — the
/// agent, the Knowledge app, Settings). That is the conservative default: a
/// whole-codebase structural view (symbol ids are file paths) exceeds a
/// ThirdParty app's per-label read scope, and denying it is always safe while
/// widening later is the reversible direction. The result is the serialised
/// [`crate::code_analysis::CodeAnalysis`]; every failure is the uniform denial.
async fn handle_code_analysis(app_id: &str, graph: &GraphHandle) -> String {
    let system_anchored = app_id != "unknown"
        && QuotaConfig::arlen_default().tier_for_app(app_id) != AppTier::ThirdParty;
    if !system_anchored {
        return CODE_ANALYSIS_DENIED.to_string();
    }
    match crate::code_analysis::analyze_code_graph(
        graph,
        crate::code_analysis::DEFAULT_GOD_MIN_DEGREE,
    )
    .await
    {
        Ok(analysis) => {
            serde_json::to_string(&analysis).unwrap_or_else(|_| CODE_ANALYSIS_DENIED.to_string())
        }
        Err(_) => CODE_ANALYSIS_DENIED.to_string(),
    }
}

/// Uniform denial for the code-symbol-context op (CG-R6).
const CODE_SYMBOL_DENIED: &str = "ERROR: code symbol context is not permitted for this caller";

/// A code-symbol-context request: the symbol id, and an optional bitemporal
/// as-of (µs since epoch; absent = live/now). `deny_unknown_fields` so a
/// malformed request is refused, not silently widened.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct CodeSymbolRequest {
    symbol_id: String,
    #[serde(default)]
    as_of_micros: Option<i64>,
}

/// CG-R6: a code symbol's activity-layer context — its defining file, the
/// project that file belongs to (bitemporal, optionally as-of a timestamp), and
/// the apps that accessed it. Gated to **system-anchored** callers like the
/// whole-codebase analysis (the file/project/provenance join over symbol ids
/// that are file paths exceeds a ThirdParty's per-label read scope, so denying
/// is the safe default); uniform denial on every failure.
async fn handle_code_symbol_context(app_id: &str, graph: &GraphHandle, body: &[u8]) -> String {
    let system_anchored = app_id != "unknown"
        && QuotaConfig::arlen_default().tier_for_app(app_id) != AppTier::ThirdParty;
    if !system_anchored {
        return CODE_SYMBOL_DENIED.to_string();
    }
    let req: CodeSymbolRequest = match serde_json::from_slice(body) {
        Ok(r) => r,
        Err(_) => return CODE_SYMBOL_DENIED.to_string(),
    };
    if req.symbol_id.is_empty() {
        return CODE_SYMBOL_DENIED.to_string();
    }
    match crate::code_analysis::code_symbol_context(graph, &req.symbol_id, req.as_of_micros).await {
        Ok(ctx) => serde_json::to_string(&ctx).unwrap_or_else(|_| CODE_SYMBOL_DENIED.to_string()),
        Err(_) => CODE_SYMBOL_DENIED.to_string(),
    }
}

/// Co-tenant filter (provenance-halo.md §5): from an object's actor set, name
/// only the caller's own access and collapse every foreign actor to a single
/// "accessed by others" signal, so the provenance of a shared object never names
/// a co-tenant. Returns `(visible_actors, accessed_by_others)`.
fn co_tenant_filter(actors: &[String], caller: &str) -> (Vec<String>, bool) {
    let caller_accessed = actors.iter().any(|a| a == caller);
    let accessed_by_others = actors.iter().any(|a| a != caller);
    let visible = if caller_accessed { vec![caller.to_string()] } else { Vec::new() };
    (visible, accessed_by_others)
}

/// Handle a caller-scoped provenance read (provenance-halo.md §5, the shared
/// read-scope op). Serves an object's provenance only to a caller whose read
/// scope covers it; every other outcome is the uniform [`PROVENANCE_OUT_OF_SCOPE`]
/// denial.
///
/// The caller's token (and its read scopes) is resolved from the kernel-attested
/// peer pid with the same PID-reuse guard the write path uses. The object's label
/// is found by probing only the labels the caller may read, so an object under an
/// unreadable label is indistinguishable from an absent one (no existence
/// oracle). The actor set is co-tenant-filtered. The caller routes the result
/// through `timing_noise()`.
async fn handle_provenance_read(
    body: &[u8],
    peer: &Option<WritePeer>,
    auth: &Arc<Mutex<Authenticator>>,
    graph: &GraphHandle,
    uses: &Arc<Mutex<crate::lcg::UseTally>>,
) -> String {
    let Ok(req) = serde_json::from_slice::<ProvenanceRequest>(body) else {
        return PROVENANCE_OUT_OF_SCOPE.to_string();
    };

    // Resolve the caller's token from a live, unchanged peer process (the write
    // path's guard: a reused pid cannot borrow the original peer's scope).
    let Some(peer) = peer else {
        return PROVENANCE_OUT_OF_SCOPE.to_string();
    };
    let Some(captured_start) = peer.start_time else {
        return PROVENANCE_OUT_OF_SCOPE.to_string();
    };
    match pid_start_time(peer.pid) {
        Ok(now) if now == captured_start => {}
        _ => return PROVENANCE_OUT_OF_SCOPE.to_string(),
    }
    let token = match auth.lock().await.issue_token_for_pid(peer.uid, peer.pid) {
        Ok(t) => t,
        Err(_) => return PROVENANCE_OUT_OF_SCOPE.to_string(),
    };

    // Probe the labels the caller may read for the object. Every readable label
    // is probed (no early break) so the query count, and thus the timing, does
    // not reveal which label the object lives under, nor distinguish an
    // out-of-scope object from an absent one: both cost exactly N probes and no
    // actor query. The FIRST label the object is found under is the one bound in
    // the actor query, so provenance is read for exactly a label the caller may
    // read, never the label-free union across tables that share an id (a Kuzu
    // primary key is per-table, so one id can exist under several labels). The
    // labels are safe identifiers (see `readable_system_labels`); the id is
    // escaped.
    let mut found_label: Option<String> = None;
    for label in readable_system_labels(&token.read_scopes) {
        let cypher = format!(
            "{} RETURN n.id LIMIT 1",
            crate::cypher::match_node("n", &label, &req.object_id)
        );
        if let Ok(rs) = graph.query_rows(cypher).await {
            if !rs.rows.is_empty() && found_label.is_none() {
                found_label = Some(label);
            }
        }
    }
    let Some(label) = found_label else {
        return PROVENANCE_OUT_OF_SCOPE.to_string();
    };

    // In scope: the object's actor set, bound to the label that satisfied the
    // scope check (ACCESSED_BY is File -> App, so only a File yields actors),
    // co-tenant-filtered so a co-tenant is never named. The filter keys on the
    // token's own app id, the identity the read scope was resolved for, not the
    // connection-time id.
    let cypher =
        format!(
            "{}-[:ACCESSED_BY]->(a) RETURN a.id AS id",
            crate::cypher::match_node("n", &label, &req.object_id)
        );
    let actors: Vec<String> = match graph.query_rows(cypher).await {
        Ok(rs) => rs
            .rows
            .iter()
            .filter_map(|r| r.first().map(|c| c.as_str().to_string()))
            .collect(),
        Err(_) => return PROVENANCE_OUT_OF_SCOPE.to_string(),
    };
    // The read completed under exactly one label - the one the scope check bound -
    // so that is the capability exercised, not the whole readable set that was
    // probed to find it.
    record_capability_uses(graph, uses, &token.app_id, &[format!("system.{label}")]).await;

    let (visible, accessed_by_others) = co_tenant_filter(&actors, &token.app_id);
    serde_json::json!({ "actors": visible, "accessed_by_others": accessed_by_others }).to_string()
}

/// Handle a structured typed read (RS-R2). The ONLY path that serves sensitive
/// labels, because the daemon owns the entire query shape: the label is checked
/// against the caller's readable allowlist, every field is identifier-checked,
/// every value is `encode_literal`-escaped, and the built Cypher carries a mandatory
/// anchoring `WHERE` and a `LIMIT`. Every failure is the single uniform denial (no
/// existence oracle), routed through `timing_noise` by the caller.
///
/// The owner axis is the v1 anchor-requirement approximation: the live observation
/// nodes carry no `_owner` column, so a non-empty filter set is required (an
/// unanchored sensitive read is the wholesale-harvest shape this op prevents). The
/// `n._owner = caller` predicate lands with that column (RS-R3); the filter core
/// does not change when it does.
async fn handle_typed_read(
    body: &[u8],
    peer: Option<&WritePeer>,
    auth: &Arc<Mutex<Authenticator>>,
    graph: &GraphHandle,
    uses: &Arc<Mutex<crate::lcg::UseTally>>,
) -> String {
    let Ok(req) = serde_json::from_slice::<crate::typed_read::TypedReadRequest>(body) else {
        return PROVENANCE_OUT_OF_SCOPE.to_string();
    };
    // Resolve the token from a live, unchanged peer process (the write path's
    // PID-reuse guard: a reused pid cannot borrow the original peer's scope).
    let Some(peer) = peer else {
        return PROVENANCE_OUT_OF_SCOPE.to_string();
    };
    let Some(captured_start) = peer.start_time else {
        return PROVENANCE_OUT_OF_SCOPE.to_string();
    };
    match pid_start_time(peer.pid) {
        Ok(now) if now == captured_start => {}
        _ => return PROVENANCE_OUT_OF_SCOPE.to_string(),
    }
    let token = match auth.lock().await.issue_token_for_pid(peer.uid, peer.pid) {
        Ok(t) => t,
        Err(_) => return PROVENANCE_OUT_OF_SCOPE.to_string(),
    };
    let readable = readable_system_labels(&token.read_scopes);
    let validated = match crate::typed_read::validate_typed_read(req, &readable) {
        Ok(v) => v,
        Err(reason) => {
            warn!(app_id = %token.app_id, reason, "typed read denied");
            return PROVENANCE_OUT_OF_SCOPE.to_string();
        }
    };
    let Some(cypher) = crate::typed_read::build_cypher(&validated) else {
        return PROVENANCE_OUT_OF_SCOPE.to_string();
    };
    // The validated label carries the GRANTED casing (validation binds it from the
    // readable set), so the capability recorded here is the same string the write
    // and raw-read paths record.
    let exercised = format!("system.{}", validated.label);
    match graph.query_rows_json(cypher).await {
        Ok(json) => {
            record_capability_uses(graph, uses, &token.app_id, &[exercised]).await;
            json
        }
        Err(_) => PROVENANCE_OUT_OF_SCOPE.to_string(),
    }
}

/// One Grant row for the browse surface (living-capability-graph.md §5). The
/// `declared_ceiling` is the faithful scope JSON; `reach` is the queryable type
/// projection; `live` is resolved fresh at read time (see below).
// The five bools mirror the Grant node's lifecycle + caveat flags (§3.1), which
// the browse surface must each distinguish; they are not a collapsible state.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, serde::Serialize)]
struct GrantView {
    id: String,
    app_id: String,
    declared_ceiling: String,
    required: bool,
    identity_verified: bool,
    live: bool,
    revoked: bool,
    superseded: bool,
    issued_at: i64,
    /// When a time-boxed grant stops authorising, or `0` for one that lasts
    /// until revoked. Sent so a surface can say why a grant is not live: a
    /// closed window is the only one of expired / process-gone / revoked the
    /// user can do something about.
    expires_at: i64,
    reach: Vec<String>,
    /// The grant kind: `capability-token` (an empty/null source reads the same)
    /// or `consent` (a remembered consent grant, system-dialog-plan.md Option A).
    source: String,
    /// The consent class, when `source == "consent"` (else empty).
    consent_class: String,
    /// The concrete consent scope, when `source == "consent"` (else empty).
    consent_scope: String,
    /// When one of this grant's reachable capabilities was last actually
    /// exercised, or `0` for never. Read off `CapabilityUse`, which is keyed on
    /// (app, capability) rather than on a token instance, so it survives the
    /// re-mint that replaces a Grant node on every request.
    ///
    /// `0` means NOT MEASURED as much as it means unused: nothing recorded uses
    /// before this was built, and the tally flushes coarsely, so a grant
    /// exercised seconds ago can still read zero. A surface that offers to revoke
    /// what is unused must say "no use recorded", never "you have never used
    /// this".
    last_used_at: i64,
    /// How many of this grant's reachable capabilities have ever been exercised.
    /// With `reach.len()` it answers the question the revoke prompt asks: this
    /// grant reaches five things and you have used one.
    used_capability_count: usize,
}

/// The uniform `access_grants` failure shape.
const ACCESS_GRANTS_ERROR: &str = "ERROR: access_grants failed";

/// Cap on (grant, type) rows returned by `access_grants`, bounding the response
/// so one app's accumulated grant history cannot grow it without limit. Generous
/// for a real app's live + dormant + revoked grants and their type reach.
const ACCESS_GRANTS_ROW_CAP: usize = 5000;

/// LCG-R4 (living-capability-graph.md §5): the caller-scoped grant browse read.
///
/// Scopes by the **kernel-attested** `app_id` (resolved from `SO_PEERCRED` at
/// connect, never a request field): a normal caller receives only its own grants
/// (`WHERE app_id = caller`), the privileged Knowledge/Settings principal the
/// whole-system view. Two principals see whole-system: `is_privileged_authority_reader`
/// (false for every caller until F3) and the canonical Settings management principal
/// (`is_settings_principal`, the Settings app that also owns revoke) so the App-access
/// capability browser can render every app's grants. Every other caller sees only its
/// own grants (keyed on the attested app_id), so no ordinary app can enumerate
/// another's authority through this op (the §5 leak the dedicated reader exists to
/// prevent). Settings reaches authority data only through this curated projection and
/// `revoke`; the general read path's `Grant`-label deny still holds for it.
///
/// `live` is recomputed fresh from `process_alive(g.pid)` and from the stored
/// expiry: a node stored live but whose process is gone, or whose window has
/// closed, renders not-live, so the flag never lies beyond the read instant
/// (§4.2). The general read path already denies the `Grant` label, so this
/// is the only way these nodes are ever served.
async fn handle_access_grants(app_id: &str, graph: &GraphHandle) -> String {
    let privileged = is_privileged_authority_reader(app_id) || is_settings_principal(app_id);
    // The same scoping the grant query uses, kept as a value so the use overlay
    // below cannot drift from it: whatever this list is scoped to, the use
    // numbers on it are scoped to as well.
    let scope_app = (!privileged).then(|| app_id.to_string());
    let scope = if privileged {
        // Whole-system view (gated false until F3).
        String::new()
    } else {
        // Own grants only; the filter keys on the attested app_id.
        let app_esc = escape_cypher(app_id);
        format!(" {{app_id: '{app_esc}'}}")
    };

    // One query, one consistent snapshot: a row per (grant, reachable type),
    // grouped by grant id here. A single query (vs a separate reach query) makes
    // the reach join atomic, and returning the label per row (not a collected
    // LIST) keeps every cell a scalar the typed row path can represent. Superseded
    // nodes are excluded (they accumulate on every reconnect and are collapsed in
    // the surface anyway, §3.1), and the row count is bounded so one app's grant
    // history cannot grow the response without limit.
    let cypher = format!(
        "MATCH (g:Grant{scope}) WHERE NOT g.superseded \
         OPTIONAL MATCH (g)-[:GRANTS]->(t:EntityType) \
         RETURN g.id, g.app_id, g.declared_ceiling, g.required, g.identity_verified, \
         g.live, g.revoked, g.superseded, g.pid, g.issued_at, t.label, \
         g.source, g.consent_class, g.consent_scope, g.expires_at, t.id \
         LIMIT {ACCESS_GRANTS_ROW_CAP}"
    );
    let rows = match graph.query_rows(cypher).await {
        Ok(rs) => rs.rows,
        Err(_) => return ACCESS_GRANTS_ERROR.to_string(),
    };

    let mut views: Vec<GrantView> = Vec::new();
    // Parallel to `views`: the namespaced types behind each view's reach labels.
    let mut reach_types: Vec<Vec<String>> = Vec::new();
    let mut index: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for row in rows {
        if row.len() < 16 {
            continue;
        }
        let id = row[0].as_str().to_string();
        let slot = *index.entry(id.clone()).or_insert_with(|| {
            let revoked = row[6].as_bool();
            let superseded = row[7].as_bool();
            let stored_live = row[5].as_bool();
            let pid = row[8].as_i64();
            let source = row[11].as_str();
            // Fresh liveness. Only a capability-token grant is process-bound: it
            // renders live only if its minting process is still alive (death caught
            // at read time). A `consent` grant (runtime user-allowed) and a
            // `declared` grant (a profile declaration projected into the graph, e.g.
            // a NetworkAccess reach) have NO process (pid 0) and persist independent
            // of one, so their liveness is the stored flag minus revoke/supersede -
            // they must not be failed by a process_alive(0) check. All require
            // not-revoked and not-superseded (defensive: the active flag is correct
            // in the reader, not only by emitter discipline). `try_from` keeps a bad
            // stored pid from wrapping.
            let process_independent = source == "consent" || source == "declared";
            // An expiry that has passed makes a grant not-live regardless of every
            // other flag. It belongs in the reader for exactly the reason the
            // others do: it is a pure function of the clock, so no emitter can
            // keep it current, and a time-boxed grant that renders live after its
            // window is the one failure this whole surface exists to prevent. `0`
            // is the stored "no expiry" (a token without one projects zero), not
            // an expiry at the epoch.
            let expires_at = row[14].as_i64();
            let expired = expires_at != 0 && expires_at <= crate::time::now().0;
            let live = stored_live
                && !revoked
                && !superseded
                && !expired
                && (process_independent
                    || u32::try_from(pid).map(process_alive).unwrap_or(false));
            views.push(GrantView {
                id: id.clone(),
                app_id: row[1].as_str().to_string(),
                declared_ceiling: row[2].as_str().to_string(),
                required: row[3].as_bool(),
                identity_verified: row[4].as_bool(),
                live,
                revoked,
                superseded,
                issued_at: row[9].as_i64(),
                expires_at,
                reach: Vec::new(),
                source: source.to_string(),
                consent_class: row[12].as_str().to_string(),
                consent_scope: row[13].as_str().to_string(),
                last_used_at: 0,
                used_capability_count: 0,
            });
            reach_types.push(Vec::new());
            views.len() - 1
        });
        // Append this row's reach label (OPTIONAL MATCH yields an empty string
        // when the grant reaches no type).
        let label = row[10].as_str();
        if !label.is_empty() {
            let reach = &mut views[slot].reach;
            if !reach.iter().any(|l| l == label) {
                reach.push(label.to_string());
            }
        }
        // The FULL type behind that label, kept beside the view for the use
        // overlay only. `reach` stays labels because that is what a surface
        // shows; the tally is keyed on the namespaced type, and reconstructing
        // one from the other works for `system.*` and silently fails for anything
        // else - a bridge's `md.obsidian.Note` has the label `Note`, so a
        // `system.` prefix would look up a capability that does not exist and
        // report a used grant as unused.
        let full_type = row[15].as_str();
        if !full_type.is_empty() {
            let types = &mut reach_types[slot];
            if !types.iter().any(|s| s == full_type) {
                types.push(full_type.to_string());
            }
        }
    }
    // Overlay the use tally. A SEPARATE query rather than a join: the grant reach
    // is keyed on `EntityType.id` (the full `system.File`) while the row above
    // returns `t.label` for display, and `CapabilityUse.id` is `<app>:<capability>`
    // - joining those inside Cypher means string concatenation in a MATCH, which
    // buys nothing over folding two bounded reads in Rust. The overlay is
    // advisory: a failed read leaves the counts at zero, which the field's own
    // contract already covers ("no use recorded"), so a hiccup in a projection
    // never fails the grant list itself.
    overlay_capability_use(graph, &scope_app, &mut views, &reach_types).await;

    serde_json::to_string(&views).unwrap_or_else(|_| "[]".to_string())
}

/// Fill each view's use fields from `CapabilityUse`, scoped exactly as the grant
/// query was: one app for an ordinary caller, every app for the privileged
/// whole-machine reader (which no caller is yet).
async fn overlay_capability_use(
    graph: &GraphHandle,
    scope_app: &Option<String>,
    views: &mut [GrantView],
    reach_types: &[Vec<String>],
) {
    if views.is_empty() {
        return;
    }
    let filter = match scope_app {
        Some(app) => format!(" {{app_id: '{}'}}", crate::utils::escape_cypher(app)),
        None => String::new(),
    };
    let cypher = format!(
        "MATCH (u:CapabilityUse{filter}) \
         RETURN u.app_id, u.capability, u.last_observed \
         LIMIT {ACCESS_GRANTS_ROW_CAP}"
    );
    let Ok(rs) = graph.query_rows(cypher).await else {
        return;
    };
    // (app, capability) -> last observed.
    let mut used: std::collections::HashMap<(String, String), i64> =
        std::collections::HashMap::new();
    for row in &rs.rows {
        if row.len() < 3 {
            continue;
        }
        used.insert(
            (row[0].as_str().to_string(), row[1].as_str().to_string()),
            row[2].as_i64(),
        );
    }
    for (i, view) in views.iter_mut().enumerate() {
        for capability in reach_types.get(i).into_iter().flatten() {
            if let Some(last) = used.get(&(view.app_id.clone(), capability.clone())) {
                view.used_capability_count += 1;
                view.last_used_at = view.last_used_at.max(*last);
            }
        }
    }
}

/// The app ids permitted to invoke a revoke (living-capability-graph.md §6.2,
/// Option A). Revoke is user-initiated through Settings, narrowing-only, so only
/// the canonical `settings` principal is admitted; in debug the exact cargo-run
/// id `dev.arlen-settings` is also admitted for a locally-run Settings (the
/// audit-daemon convention). The match is exact, not a broad `dev.` prefix,
/// which would let any locally-built crate narrow another app's scope.
fn revoke_caller_admitted(app_id: &str) -> bool {
    is_settings_principal(app_id)
}

/// May this caller delete the user's recorded activity?
///
/// Only the knowledge app, which is the surface that owns the timeline and its
/// Delete control. This is the most destructive thing the write socket can do -
/// it is the one operation that permanently removes graph nodes - so the gate is
/// an exact match on one principal rather than a tier check: FirstParty is a
/// dozen binaries, and none of the others has any business clearing someone's
/// history.
///
/// `dev.arlen.knowledge` is what the resolver's rule (3) makes of the app's
/// install path, `/usr/lib/arlen/apps/dev.arlen.knowledge/bin/arlen-knowledge`,
/// which the image now stages. The debug arm accepts the exact cargo-run id and
/// the harness extra-admit, never a broad `dev.` prefix - that would let any
/// locally built crate wipe the timeline.
fn activity_delete_caller_admitted(app_id: &str) -> bool {
    if app_id == "dev.arlen.knowledge" {
        return true;
    }
    #[cfg(debug_assertions)]
    {
        app_id == "dev.arlen-knowledge-app" || revoke_extra_admit(app_id)
    }
    #[cfg(not(debug_assertions))]
    {
        false
    }
}

/// The canonical Settings management principal: the only app trusted to browse
/// the whole-system grant list (`access_grants`) and to issue revokes (`revoke`),
/// living-capability-graph.md §6.2. Root-anchored app_id `settings` (identity.rs
/// resolves `/usr/lib/arlen/apps/settings/bin/arlen-settings` to it); in debug the
/// exact cargo-run id `dev.arlen-settings` (and the harness extra-admit) are also
/// accepted. The match is exact, never a broad `dev.` prefix, which would let any
/// locally-built crate read or narrow another app's scope. This lifts ONLY the
/// curated `access_grants` browse to whole-system; it does NOT touch
/// `is_privileged_authority_reader`, so the general read path's authority-label
/// deny still holds for Settings (it reaches authority data only through the
/// curated `access_grants`/`revoke` ops, never arbitrary Cypher).
fn is_settings_principal(app_id: &str) -> bool {
    if app_id == "dev.arlen.settings" {
        return true;
    }
    #[cfg(debug_assertions)]
    {
        // The integration harness submits as its own hash-suffixed `dev.<test-bin>`
        // id, which the exact `dev.arlen-settings` match below rejects. A debug-only
        // env names ONE extra exact id to admit (the audit-daemon
        // `ARLEN_AUDIT_EXTRA_ADMIT` convention), set by the harness to its own
        // resolved id. Off by default, so the exact-match tightening holds for any
        // real caller.
        app_id == "dev.arlen-settings" || revoke_extra_admit(app_id)
    }
    #[cfg(not(debug_assertions))]
    {
        false
    }
}

/// Whether `app_id` matches the debug-only `ARLEN_REVOKE_EXTRA_ADMIT` exact
/// extra-admit (the integration harness's own dev id). Exact match, debug only.
#[cfg(debug_assertions)]
fn revoke_extra_admit(app_id: &str) -> bool {
    std::env::var("ARLEN_REVOKE_EXTRA_ADMIT").is_ok_and(|v| v == app_id)
}

/// Whether `app_id` is a safe single path component for use as a profile-file
/// name: non-empty, only `[A-Za-z0-9._-]` (excludes every path separator and NUL,
/// so it cannot escape the permissions directory), and not a `.`/`..` directory
/// reference. Reverse-DNS ids (`com.example.app`) pass; a traversal or absolute
/// path does not.
fn is_safe_app_id(app_id: &str) -> bool {
    !app_id.is_empty()
        && app_id != "."
        && app_id != ".."
        && app_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'_' || b == b'-')
}

/// LCG-R5 (living-capability-graph.md §6): the caller-scoped, narrowing-only
/// revoke. Admits only the `settings` principal (kernel-attested), refuses a
/// system-tier target (managed by the system, not revocable here), and applies
/// the narrowing to the target's user-tier profile through the strict-subset
/// gate, writing nothing unless authority strictly shrank. The closed
/// `RevokedReach` cannot express a widening; the gate proves narrowing on the
/// re-derived scopes. Outcomes are `OK:` tokens; only an auth/parse/tier/io
/// failure is an `ERROR:`.
/// The capability-change records for `target_app` from a page of audit views, as
/// `(outcome, reach)` pairs in the views' (chain) order: keep only records whose
/// kind is `CapabilityChange` with the fixed capability-change subject and
/// `target_app` among the coarse `node_types`, whose typed reach is present, and
/// convert each stored reach back to a [`crate::revoke::RevokedReach`]. Pure over
/// the views so the filter is unit-tested without the audit socket; the async
/// reader pages the ledger and folds these (in chain order) into the removal
/// ledger the restore op gates on.
fn capability_change_pairs(
    target_app: &str,
    views: &[audit_proto::StructuralView],
) -> Vec<(String, crate::revoke::RevokedReach)> {
    views
        .iter()
        .filter(|v| {
            v.kind == audit_proto::AuditKind::CapabilityChange
                && v.structural.subject == crate::audit::CAPABILITY_CHANGE_SUBJECT
                && v.structural.node_types.iter().any(|n| n == target_app)
        })
        .filter_map(|v| {
            v.structural
                .capability_change
                .as_ref()
                .map(|reach| {
                    (v.structural.outcome.clone(), crate::audit::audit_to_reach(reach))
                })
        })
        .collect()
}

/// How many audit entries to request per read page while scanning for a target's
/// capability-change records. Capability changes are sparse (user-initiated), so a
/// few pages cover any real app; the scan is bounded by the ledger head captured at
/// the first page.
const CAPABILITY_READ_PAGE: u64 = 200;

/// Reconstruct a target app's removal ledger from the durable audit ledger: page
/// the Structural-tier records via the audit read socket, keep the app's
/// capability-change records, and fold them (in chain order) into the removal
/// ledger the restore op gates on. The ledger head is fixed at the first page so
/// concurrent appends cannot extend the scan. Fail-closed: an unreachable or
/// failing audit read yields an EMPTY ledger, so with no readable record nothing is
/// restorable (the safe direction for the authority-growth path).
async fn removal_ledger_for(target_app: &str) -> crate::revoke::RemovalLedger {
    let client = audit_proto::ReadClient::new(audit_proto::read_socket_path());
    let mut pairs: Vec<(String, crate::revoke::RevokedReach)> = Vec::new();
    let mut from = 0u64;
    let mut target_head = u64::MAX;
    loop {
        match client.read(from, u64::MAX, CAPABILITY_READ_PAGE, None).await {
            Ok(page) => {
                if target_head == u64::MAX {
                    target_head = page.head;
                }
                let n = page.entries.len() as u64;
                pairs.extend(capability_change_pairs(target_app, &page.entries));
                from += n;
                if n == 0 || from >= target_head {
                    break;
                }
            }
            Err(_) => return crate::revoke::RemovalLedger::default(),
        }
    }
    crate::revoke::fold_removal_ledger(pairs.iter().map(|(o, r)| (o.as_str(), r)))
}

fn handle_revoke(app_id: &str, body: &[u8]) -> String {
    if !revoke_caller_admitted(app_id) {
        return "ERROR: revoke not permitted for this caller".to_string();
    }
    let req: crate::revoke::RevokeReach = match serde_json::from_slice(body) {
        Ok(r) => r,
        Err(e) => return format!("ERROR: invalid revoke request: {e}"),
    };
    // §6.3: there is no agent path that auto-applies. The agent may only *propose*
    // a revoke into the pull-review timeline; a user confirming it replays the
    // proposal as `User`. A literal `Agent` initiator arriving at the apply site is
    // therefore never a confirmed revoke, so it is refused, not applied. The
    // caller-allowlist (only `settings`) already blocks the agent from connecting,
    // but that guards *who* connects; this guards *what semantics* may apply, so
    // the invariant survives admitting another caller later (a confused deputy
    // forwarding a raw suggestion cannot turn it into an auto-applied revoke).
    if matches!(req.initiator, crate::revoke::RevokeInitiator::Agent { .. }) {
        return "ERROR: an agent-initiated revoke is a proposal, not a confirmed revoke".to_string();
    }
    // The target app id becomes a filesystem path (`profile_path` interpolates it
    // into `~/.config/permissions/{id}.toml`), so it MUST be a single safe path
    // component. Without this an attacker-influenced id like `/etc/x` (absolute,
    // discards the base) or `../../etc/x` (lexical escape) would let revoke
    // read+rewrite an arbitrary graph-profile-shaped TOML anywhere on disk. The
    // charset excludes every path separator, so the result can only ever name a
    // file inside the permissions directory.
    if !is_safe_app_id(&req.target_app_id) {
        return "ERROR: invalid target app id".to_string();
    }
    // System-tier targets are refused, not faked (§6.2): their authority is
    // managed by the system, and the user-tier `~/.config` write would not be
    // re-read for them.
    if !crate::revoke::tier_allows_revoke(&req.target_app_id) {
        return "ERROR: SystemTier: this app is managed by the system".to_string();
    }
    let path = match arlen_permissions::profile_path(&req.target_app_id) {
        Ok(p) => p,
        Err(e) => return format!("ERROR: {e}"),
    };
    match crate::revoke::revoke_at(&path, &req.reach) {
        Ok(crate::revoke::RevokeOutcome::Revoked) => "OK: revoked".to_string(),
        Ok(crate::revoke::RevokeOutcome::NoChange) => "OK: no-change".to_string(),
        Ok(crate::revoke::RevokeOutcome::NotNarrowing) => "OK: not-narrowing".to_string(),
        Ok(crate::revoke::RevokeOutcome::NotFound) => "OK: not-found".to_string(),
        Ok(crate::revoke::RevokeOutcome::Required) => "OK: required".to_string(),
        Err(e) => format!("ERROR: {e}"),
    }
}

/// Authorise and apply a restore (re-widen), the reverse of [`handle_revoke`] and
/// the ONE authority-growth path. It carries the same gates as revoke - admitted
/// only for the Settings principal, an `Agent` initiator refused (§6.3), the target
/// id charset-checked before it becomes a path, system-tier targets refused - and
/// adds the load-bearing bound: the reach is re-added ONLY if the durable audit
/// ledger records this user having revoked it (`removal_ledger_for` +
/// `RemovalLedger::contains`), so a restore can only un-do a specific prior revoke,
/// never grant fresh authority. The change is recorded to the audit ledger AFTER a
/// confirmed re-widen and best-effort (like revoke): the `restored` record clears
/// the reach from the removal set, so it is written ONLY on an actual `Restored`
/// outcome, never for a no-op or a failed write (which would fold a still-removed
/// reach out of the ledger and deny a legitimate future restore). A dropped record
/// only leaves the reach showing as still-removed - a re-restore is then a safe
/// no-op - and the grant is already bounded to a recorded removal, authorised, and
/// reversible, so a missing provenance record is not an escalation.
async fn handle_restore(app_id: &str, body: &[u8], audit: &Arc<dyn AuditSink>) -> String {
    if !revoke_caller_admitted(app_id) {
        return "ERROR: restore not permitted for this caller".to_string();
    }
    let req: crate::revoke::RestoreReach = match serde_json::from_slice(body) {
        Ok(r) => r,
        Err(e) => return format!("ERROR: invalid restore request: {e}"),
    };
    // §6.3: an agent may only propose; a literal `Agent` initiator at the apply site
    // is never a confirmed restore, so it is refused (the caller-allowlist already
    // blocks the agent, this guards the semantics regardless).
    if matches!(req.initiator, crate::revoke::RevokeInitiator::Agent { .. }) {
        return "ERROR: an agent-initiated restore is a proposal, not a confirmed restore"
            .to_string();
    }
    if !is_safe_app_id(&req.target_app_id) {
        return "ERROR: invalid target app id".to_string();
    }
    if !crate::revoke::tier_allows_revoke(&req.target_app_id) {
        return "ERROR: SystemTier: this app is managed by the system".to_string();
    }
    let path = match arlen_permissions::profile_path(&req.target_app_id) {
        Ok(p) => p,
        Err(e) => return format!("ERROR: {e}"),
    };
    if !path.exists() {
        return crate::revoke::RestoreOutcome::NotFound.wire_token().to_string();
    }
    // The authority-growth ceiling: reconstruct what this user actually revoked from
    // the durable audit ledger. A reach the ledger does not record as removed is not
    // restorable - refused here before anything is audited or written.
    let ledger = removal_ledger_for(&req.target_app_id).await;
    if !ledger.contains(&req.reach) {
        return crate::revoke::RestoreOutcome::NotPermitted.wire_token().to_string();
    }
    let outcome = match crate::revoke::restore_at(&path, &req.reach, &ledger) {
        Ok(o) => o,
        Err(e) => return format!("ERROR: {e}"),
    };
    // Record the restore ONLY on a confirmed write (`Restored`), best-effort: the
    // `restored` record clears the reach from the removal set, so writing it for a
    // `NoChange` / `NotPermitted` / error that left the profile untouched would fold
    // a still-removed reach out of the ledger and deny a legitimate future restore.
    if outcome == crate::revoke::RestoreOutcome::Restored {
        if let Err(e) = audit
            .submit(crate::audit::capability_change_event(
                &req.target_app_id,
                &req.reach,
                crate::revoke::OUTCOME_RESTORED,
            ))
            .await
        {
            warn!("restore audit failed (removal ledger not cleared for this reach): {e}");
        }
    }
    outcome.wire_token().to_string()
}

/// Authorise and persist a structured write request, returning the plaintext
/// response (`OK` / `ERROR: ...`).
///
/// Fail-closed at every layer: the request must come from a live, non-recycled
/// peer process (PID-reuse guard) whose permission profile grants the relation
/// (token issuance), the relation must pass `create_relation` (scope +
/// anchor/privilege + declared + known types), and persistence is a *checked*
/// MATCH/MERGE that reports not-found when an endpoint instance is absent rather
/// than a silent no-op success.
///
/// SECURITY BOUNDARY (same-uid): the peer's identity comes from
/// `app_id_from_pid`, and both it and the permission profile under
/// `~/.config/permissions/` are user-writable. A same-uid process can therefore
/// squat a privileged app id and grant itself the relation scope, so this write
/// mode does not defend against a same-uid attacker. That is the documented F3
/// gap shared across the daemon (the read rate-limiter, the audit daemon's
/// ingest admission), closed only by the installd inode-keyed identity registry
/// and root-owned profiles (`docs/architecture/identity-spoof-mitigation.md`).
/// It is *sharper* here than on the read path because this authorises a graph
/// mutation, so enabling this socket for real first-party-only use is gated on
/// that hardening (canonical-executable provenance as the interim step); a hard
/// path gate is not applied now because the agent runs from a dev tree during
/// development.
///
/// This used to end "Cross-uid peers are already rejected at connection", which
/// promises more than the connection does and would let a reader of this function
/// assume every caller here shares its uid. What `cross_uid_admitted` actually
/// does: a cross-uid peer below FirstParty is refused, and above it the answer
/// turns on configuration - with `ARLEN_OWNER_UID`/`ARLEN_OWNER_USER` set, only
/// that owner; with neither set, ANY local uid at first-party or system tier. The
/// image sets `ARLEN_OWNER_USER=arlen`, so the admitted cross-uid peer there is
/// the desktop user this daemon serves, which is the intended one - but that is a
/// property of the deployment, not of the connection, and it is the same reasoning
/// the block above the check spells out at length.
async fn handle_write_request(
    body: &[u8],
    peer: Option<WritePeer>,
    registry: &SchemaRegistry,
    graph: &GraphHandle,
    // The raw event store, because a delete that leaves the source is a delayed
    // failure: the promotion pass would put the range straight back.
    pool: &sqlx::SqlitePool,
    // Taken for the whole delete, so no pass is mid-flight with a batch it read
    // before the rows went.
    gate: &crate::activity_delete::PromotionGate,
    auth: &Arc<Mutex<Authenticator>>,
    audit: Option<&Arc<dyn AuditSink>>,
    uses: Option<&Arc<Mutex<crate::lcg::UseTally>>>,
) -> String {
    let req: WriteRequest = match serde_json::from_slice(body) {
        Ok(r) => r,
        Err(e) => return format!("ERROR: malformed write request: {e}"),
    };

    // A write must be attributable to a live peer process.
    let Some(peer) = peer else {
        return "ERROR: write requires a resolvable peer process".to_string();
    };

    // PID-reuse guard (E7): the pid's start time must be readable now and match
    // the value captured at connection. If it changed, the original peer exited
    // and another process inherited the pid number, so it must not borrow this
    // connection's write authority. Re-checked immediately before token
    // issuance, which itself re-resolves the app_id from the same pid.
    let Some(captured_start) = peer.start_time else {
        return "ERROR: write requires a verifiable peer process".to_string();
    };
    match pid_start_time(peer.pid) {
        Ok(now) if now == captured_start => {}
        _ => return "ERROR: peer process changed since connection".to_string(),
    }

    // The token is issued from the pid's permission profile and fails closed if
    // it has no graph access or no matching relation scope.
    let token = match auth.lock().await.issue_token_for_pid(peer.uid, peer.pid) {
        Ok(t) => t,
        Err(e) => return format!("ERROR: {e}"),
    };

    // Living Capability Graph (living-capability-graph.md §4.1): project this
    // graph-access peer's minted token into the browse graph. The auth lock is
    // already released; failure is logged and swallowed so a graph hiccup never
    // fails a validly-issued token (the projection degrades, the write does not).
    match crate::lcg::emit_grant_node(graph, &token).await {
        // A NEW or CHANGED grant is an issuance and belongs in the ledger; a
        // repeat of a reach the process already holds is not an event. Without
        // this, granting was the one capability action with no audit record at
        // all - only revoke and restore were written.
        Ok(true) => {
            if let Some(sink) = audit {
                let _ = sink
                    .submit(crate::audit::grant_issued_event(
                        &token.app_id,
                        &crate::lcg::reach_summary(&token),
                    ))
                    .await;
            }
        }
        Ok(false) => {}
        Err(e) => {
            tracing::warn!("emit_grant_node failed (capability projection degraded): {e}");
        }
    }

    // What this request exercises, in the same vocabulary the grant's reach is
    // projected in (`GRANTS` -> `EntityType`), captured before the match consumes
    // the request. Counted only if the outcome below is not an error, so an
    // authorisation refusal or a failed persist never counts as use: a denial is
    // not use, and counting it would make an unused grant look used and hide it
    // from the revoke prompt.
    let exercised: Vec<String> = match &req {
        WriteRequest::CreateRelation { from_type, to_type, .. }
        | WriteRequest::RetractRelation { from_type, to_type, .. } => {
            let mut v = vec![from_type.clone(), to_type.clone()];
            v.sort();
            v.dedup();
            v
        }
        WriteRequest::CreateNode { node_type, .. } => vec![node_type.clone()],
        _ => Vec::new(),
    };
    let exercising_app = token.app_id.clone();

    let outcome = match req {
        WriteRequest::CreateRelation {
            from_type,
            from_id,
            to_type,
            to_id,
            relation_type,
            op_id,
        } => {
            let rel = match create_relation(
                registry,
                &from_type,
                &from_id,
                &to_type,
                &to_id,
                &relation_type,
                &token,
            ) {
                Ok(r) => r,
                Err(e) => return format!("ERROR: {e}"),
            };
            persist_relation(graph, &rel, &op_id).await
        }
        WriteRequest::RetractRelation {
            from_type,
            from_id,
            to_type,
            to_id,
            relation_type,
            op_id,
        } => {
            let rel = match retract_relation(
                registry,
                &from_type,
                &from_id,
                &to_type,
                &to_id,
                &relation_type,
                &op_id,
                &token,
            ) {
                Ok(r) => r,
                Err(e) => return format!("ERROR: {e}"),
            };
            persist_retract(graph, &rel, &op_id).await
        }
        WriteRequest::CreateNode { node_type, id } => {
            // Token write scope (the same check entity create uses), then the
            // narrow creatable-node allowlist and a non-empty id. All fail-closed.
            if !token.can_write(&node_type) {
                return format!("ERROR: permission denied for {node_type}");
            }
            if !CREATABLE_NODES.contains(&node_type.as_str()) {
                return format!("ERROR: node type {node_type} is not creatable via this socket");
            }
            if id.is_empty() {
                return "ERROR: create node requires an id".to_string();
            }
            // Structural-canary tripwire (canary-honeytools.md §3). An id bearing
            // the reserved token can only reach this caller-supplied-id path if
            // external content injected it, so it is proof of a hijacked proposal.
            // `persist_create_node` refuses it below regardless (the containment);
            // emit the content-free trip audit here so the anomaly detector can
            // surface the hijack attempt. Best-effort: the write is already refused,
            // so a down ledger must not change the outcome, only lose the signal.
            if id.contains(RESERVED_CANARY_TOKEN) {
                if let Some(sink) = audit {
                    if let Err(e) = sink
                        .submit(crate::audit::canary_trip_event(&token.app_id))
                        .await
                    {
                        warn!("canary trip audit failed (write still refused): {e}");
                    }
                }
            }
            let label = node_type.strip_prefix("system.").unwrap_or(&node_type);
            persist_create_node(graph, label, &id).await
        }
        WriteRequest::DeleteActivity { from } => {
            // One principal, exact. The gate comes before anything reads the
            // graph, so a refused caller cannot even learn how much is there.
            if !activity_delete_caller_admitted(&token.app_id) {
                return "ERROR: permission denied for activity delete".to_string();
            }
            // Taken before the COUNT, not just before the destruction.
            //
            // Deleting the raw events closes the case where a pass runs after the
            // delete. It does nothing for a pass already running: those read their
            // batch out of SQLite first and write the nodes second, so a delete in
            // that gap removes rows the pass is holding in memory and the pass
            // writes them anyway. On a booted run that happened two times in three,
            // with the surviving stamp sitting 28 seconds past the cut-off.
            //
            // Before the count because `count_activity_since` promises the numbers
            // describe the range the delete then removes. Lock only the destructive
            // part and a pass can slip in between, so the audit records a size the
            // act did not have - a small lie, in the one record that exists to be
            // trusted. The audit round-trip happens under the lock as a result: it
            // is a local socket and a delete is rare, so a pass waits briefly at
            // worst.
            let _pass = gate.lock().await;

            let planned = match crate::activity_delete::count_activity_since(graph, from).await {
                Ok(c) => c,
                Err(e) => return format!("ERROR: {e}"),
            };
            // Audit BEFORE the act, fail-closed, the same order the executor
            // keeps for a destructive write. The ruling asks that a user who
            // clears their history can still see that they did; if the ledger
            // cannot record it, the honest move is to refuse rather than to
            // delete unrecorded. Nothing is destroyed at this point, so refusing
            // costs the user a retry and never their data.
            if let Some(sink) = audit {
                if let Err(e) = sink
                    .submit(crate::audit::activity_delete_event(
                        &token.app_id,
                        from,
                        planned.total(),
                    ))
                    .await
                {
                    warn!("activity delete refused: audit unavailable: {e}");
                    return "ERROR: audit unavailable, nothing was deleted".to_string();
                }
            }
            // Raw events FIRST, then the graph, and the order is the reverse of
            // the obvious one.
            //
            // The promotion pass turns raw events into nodes every thirty seconds.
            // Delete the graph first and a pass can run in the window before the
            // raw events go, putting the range straight back - the race this whole
            // ruling exists to close. Delete the raw side first and any pass that
            // runs in between has nothing left to promote.
            //
            // The two stores cannot share a transaction, so a failure between them
            // is possible and the order decides which failure. Raw gone with nodes
            // remaining is a delete the user can retry to finish; nodes gone with
            // raw remaining is a delete the system silently reverses. Only one of
            // those is recoverable, and it is this one.
            let raw = match crate::activity_delete::delete_raw_events_since(pool, from).await {
                Ok(n) => n,
                Err(e) => return format!("ERROR: {e}"),
            };
            match crate::activity_delete::run_deletion(graph, from).await {
                // The reported count stays the graph's. The raw events are the
                // same activity counted a second way, so adding them would tell
                // the user their range was twice the size it was; it is logged
                // instead, where it answers "did the source go too".
                Ok(()) => {
                    info!(raw_events = raw, nodes = planned.total(), "activity deleted");
                    format!("OK: deleted {}", planned.total())
                }
                Err(e) => format!("ERROR: {e}"),
            }
        }
        WriteRequest::UpsertEntity {
            qualified_type,
            external_key,
            fields,
            origin,
        } => {
            // For a shared-entity write, keep a copy of the fields so duplicate
            // detection can run AFTER the write (plan_entity_upsert consumes the
            // map). Only shared types carry a DuplicateConfig, so the capture is
            // gated on `shared.` - a non-shared app write skips it entirely.
            let dedup_data: Option<serde_json::Map<String, serde_json::Value>> =
                qualified_type
                    .starts_with("shared.")
                    .then(|| fields.clone().into_iter().collect());
            // Authorise + validate against the registered schema and build the
            // (ensure-table, upsert) plan; all fail-closed in `plan_entity_upsert`
            // (namespace bound, system.* refused, shared.* owner-gated, fields
            // type-checked).
            // Validate the origin before anything else touches it: a malformed
            // one refuses the write rather than landing a row with half a
            // provenance record on it.
            let origin = match origin
                .map(|o| crate::write::WriteOrigin::new(&o.run_id, &o.origin_ref))
                .transpose()
            {
                Ok(o) => o,
                Err(e) => return format!("ERROR: {e}"),
            };
            let (ddl, cypher) = match crate::write::plan_entity_upsert(
                registry,
                &token,
                &qualified_type,
                &external_key,
                fields,
                origin.as_ref(),
            ) {
                Ok(p) => p,
                Err(e) => return format!("ERROR: {e}"),
            };
            // Audit-before-persist, fail-closed (S13): a third-party bridge's
            // entity write is recorded before it touches the graph, so a down
            // or unreachable ledger refuses the write rather than letting it
            // land unaudited. The record is content-free (the app id + the
            // qualified type, never the key or field bodies).
            let Some(sink) = audit else {
                return "ERROR: audit unavailable".to_string();
            };
            if let Err(e) = sink
                .submit(crate::audit::entity_upsert_event(
                    &token.app_id,
                    &qualified_type,
                    "ok",
                ))
                .await
            {
                warn!("entity upsert audit failed, refusing write: {e}");
                return "ERROR: audit unavailable".to_string();
            }
            // Ensure the app's dynamic entity table exists (idempotent), then
            // run the keyed MERGE so a re-sync never duplicates.
            if let Err(e) = graph.query(ddl).await {
                return format!("ERROR: ensure entity table: {e}");
            }
            match graph.query_rows(cypher).await {
                Ok(_) => {
                    // Advisory (SHARED-ENTITIES.md write-path producer): after the
                    // owner writes a shared entity, detect whether it duplicates an
                    // existing one and record a pending merge suggestion for the
                    // owner to confirm. Best-effort - it only ever WRITES a
                    // MergeSuggestion node (never mutates the entity), so a dedup
                    // hiccup is logged but never fails the successful write.
                    if let Some(data) = dedup_data {
                        let new_id = crate::write::entity_node_id(&qualified_type, &external_key);
                        if let Err(e) = crate::shared::dedup_shared_entity_on_write(
                            graph,
                            &qualified_type,
                            &new_id,
                            &data,
                            &token.app_id,
                        )
                        .await
                        {
                            warn!("shared-entity dedup after upsert failed (advisory): {e}");
                        }
                    }
                    "OK: upserted".to_string()
                }
                Err(e) => format!("ERROR: {e}"),
            }
        }
        WriteRequest::LinkEntities {
            edge_type,
            from_type,
            from_key,
            to_type,
            to_key,
        } => {
            // Authorise + validate both endpoints + build the (rel-table DDL,
            // edge MERGE) plan; all fail-closed in `plan_entity_link` (both
            // endpoints namespace-bound + registered, system.*/shared.* refused,
            // edge type a safe identifier, ids escaped).
            let (ddl, cypher) = match crate::write::plan_entity_link(
                registry,
                &token,
                &edge_type,
                &from_type,
                &from_key,
                &to_type,
                &to_key,
            ) {
                Ok(p) => p,
                Err(e) => return format!("ERROR: {e}"),
            };
            // Audit-before-persist, fail-closed (S13): content-free (the app id +
            // the two endpoint types + the edge label, never the keys).
            let Some(sink) = audit else {
                return "ERROR: audit unavailable".to_string();
            };
            if let Err(e) = sink
                .submit(crate::audit::entity_link_event(
                    &token.app_id,
                    &edge_type,
                    &from_type,
                    &to_type,
                    "ok",
                ))
                .await
            {
                warn!("entity link audit failed, refusing write: {e}");
                return "ERROR: audit unavailable".to_string();
            }
            // Ensure the dynamic REL TABLE exists (idempotent), then the keyed
            // MERGE. A `linked` count of 0 means an endpoint was not present (a
            // forward reference to a not-yet-synced node); the bridge's re-sync
            // resolves it, so it is surfaced as an error rather than a false OK.
            if let Err(e) = graph.query(ddl).await {
                return format!("ERROR: ensure entity rel table: {e}");
            }
            match graph.query_rows(cypher).await {
                Ok(rows) => {
                    let linked = rows
                        .rows
                        .first()
                        .and_then(|r| r.first())
                        .map(|v| v.as_i64())
                        .unwrap_or(0);
                    if linked >= 1 {
                        "OK: linked".to_string()
                    } else {
                        "ERROR: link endpoints not found".to_string()
                    }
                }
                Err(e) => format!("ERROR: {e}"),
            }
        }
        WriteRequest::MergeEntities {
            qualified_type,
            duplicate_key,
            canonical_key,
        } => {
            // Owner-gated: only a shared.* type's declared owner may merge its
            // instances; a non-shared type stays in the caller's namespace.
            // Fail-closed in `authorise_entity_merge`.
            if let Err(e) = crate::write::authorise_entity_merge(&token, &qualified_type) {
                return format!("ERROR: {e}");
            }
            // The rel tables whose edges must be re-pointed (a catalog read).
            let table = crate::write::entity_table_name(&qualified_type);
            let involved = match graph.rel_tables_involving(&table).await {
                Ok(t) => t,
                Err(e) => return format!("ERROR: enumerate relations: {e}"),
            };
            // The re-point + delete plan (fail-closed: non-empty, distinct keys).
            let stmts = match crate::write::plan_entity_merge(
                &qualified_type,
                &duplicate_key,
                &canonical_key,
                &involved,
            ) {
                Ok(s) => s,
                Err(e) => return format!("ERROR: {e}"),
            };
            // Both instances must exist before a destructive merge. The delete is
            // canonical-anchored so a missing canonical no-ops rather than
            // destroys, but a merge that would move nothing is a caller error
            // (a mistyped key), so surface it plainly instead of a false OK.
            let dup_id = crate::write::entity_node_id(&qualified_type, &duplicate_key);
            let canon_id = crate::write::entity_node_id(&qualified_type, &canonical_key);
            let exist_q = format!(
                "MATCH (n:{table}) WHERE n.id IN ['{}', '{}'] RETURN count(*) AS c",
                crate::utils::escape_cypher(&dup_id),
                crate::utils::escape_cypher(&canon_id),
            );
            match graph.query_rows(exist_q).await {
                Ok(rows) => {
                    let present = rows
                        .rows
                        .first()
                        .and_then(|r| r.first())
                        .map(|v| v.as_i64())
                        .unwrap_or(0);
                    if present < 2 {
                        return "ERROR: merge requires both the duplicate and the canonical to exist"
                            .to_string();
                    }
                }
                Err(e) => return format!("ERROR: check merge instances: {e}"),
            }
            // Audit-before-persist, fail-closed (S13): content-free (app id + the
            // merged type, never the duplicate/canonical keys).
            let Some(sink) = audit else {
                return "ERROR: audit unavailable".to_string();
            };
            if let Err(e) = sink
                .submit(crate::audit::entity_merge_event(&token.app_id, &qualified_type, "ok"))
                .await
            {
                warn!("entity merge audit failed, refusing write: {e}");
                return "ERROR: audit unavailable".to_string();
            }
            // Re-point every edge then delete the duplicate ATOMICALLY: a partial
            // merge (some edges moved, the duplicate left, or vice versa) would
            // corrupt the graph, so the whole plan runs in one transaction.
            match graph.transaction(stmts).await {
                Ok(()) => "OK: merged".to_string(),
                Err(e) => format!("ERROR: merge: {e}"),
            }
        }
        WriteRequest::PersistConsentGrant {
            recipient,
            consent_class,
            consent_scope,
            revocation_handle,
            expires_at_micros,
        } => {
            // Only the consent broker may write a consent Grant node (the Grant
            // label is otherwise daemon-internal). Authorised by the caller's
            // attested app_id, so a third party cannot forge an authority grant.
            if !consent_grant_writer_admitted(&token.app_id) {
                return format!(
                    "ERROR: permission denied for consent-grant write by {}",
                    token.app_id
                );
            }
            if recipient.trim().is_empty()
                || consent_class.trim().is_empty()
                || revocation_handle.trim().is_empty()
            {
                return "ERROR: consent grant requires recipient, class and handle".to_string();
            }
            // Audit-before-persist, fail-closed (S13): content-free (the broker +
            // the recipient + the class, never the scope).
            let Some(sink) = audit else {
                return "ERROR: audit unavailable".to_string();
            };
            if let Err(e) = sink
                .submit(crate::audit::consent_grant_event(
                    &token.app_id,
                    &recipient,
                    &consent_class,
                ))
                .await
            {
                warn!("consent grant audit failed, refusing write: {e}");
                return "ERROR: audit unavailable".to_string();
            }
            match crate::lcg::persist_consent_grant(
                graph,
                &recipient,
                &consent_class,
                consent_scope.as_deref(),
                &revocation_handle,
                expires_at_micros,
            )
            .await
            {
                Ok(()) => "OK: persisted".to_string(),
                Err(e) => format!("ERROR: {e}"),
            }
        }
        WriteRequest::FileMeeting {
            id,
            title,
            summary,
            participants,
            action_items,
            started_at,
        } => {
            // The Meeting label is app-owned content, but filing is gated to the
            // meetings app / AI engine so a third-party app cannot write arbitrary
            // meeting nodes. The summary/participants/action text are AI/human
            // content stored as node fields (escaped), never Cypher.
            if !meeting_caller_admitted(&token.app_id) {
                return format!("ERROR: permission denied for meeting write by {}", token.app_id);
            }
            if id.trim().is_empty() || title.trim().is_empty() {
                return "ERROR: meeting requires id and title".to_string();
            }
            let items: Vec<crate::meeting::ActionItemRecord> = action_items
                .iter()
                .map(|a| crate::meeting::ActionItemRecord {
                    text: &a.text,
                    owner: a.owner.as_deref(),
                })
                .collect();
            let record = crate::meeting::MeetingRecord {
                title: &title,
                summary: &summary,
                participants: &participants,
                action_items: &items,
            };
            match crate::meeting::file_meeting(graph, &id, record, started_at).await {
                Ok(()) => "OK: filed".to_string(),
                Err(e) => format!("ERROR: {e}"),
            }
        }
    };

    // Successful exercise only. Best-effort: the tally is a projection, so a graph
    // hiccup must never turn a completed write into a failed one.
    if !outcome.starts_with("ERROR:") {
        if let Some(uses) = uses {
            record_capability_uses(graph, uses, &exercising_app, &exercised).await;
        }
    }
    outcome
}

/// App ids permitted to file OR read a meeting note (the meetings app
/// orchestrates capture -> summarize -> file and renders the recent-meetings
/// home; the AI engine may file directly and read for a briefing). Meeting nodes
/// are the user's own content, so read and write share the same principals. In
/// debug a `dev.`-prefixed id is admitted (the dev/test convention the other
/// gates use).
fn meeting_caller_admitted(app_id: &str) -> bool {
    // `dev.arlen.meetings` is what the app actually resolves to: the image
    // installs it at `/usr/lib/arlen/apps/dev.arlen.meetings/`, and rule (3)
    // reads that directory name. The bare `meetings` beside it is what this gate
    // was written against and nothing produces it under any current install path
    // - left rather than removed, because whether the short id or the namespaced
    // one is canonical for an APP is a naming decision, not mine to settle at the
    // point where it blocks a shipped feature.
    if app_id == "dev.arlen.meetings" || app_id == "meetings" || app_id == "ai-agent" {
        return true;
    }
    // EXACT, never a `dev.` prefix: every locally-built binary resolves to some
    // `dev.<bin>`, so a prefix match admits all of them to another app's meeting
    // notes. The sibling gates in this file, the audit ingest and the transfer
    // daemon all key on an exact list for exactly this reason.
    #[cfg(debug_assertions)]
    if matches!(app_id, "dev.arlen-meetings" | "dev.arlen-ai-engine-daemon" | "dev.arlen-ai-agent") {
        return true;
    }
    false
}

/// A meeting read request over the `0x0C` socket op: list the recent meetings for
/// the home, or fetch one note by id. Tagged JSON, gated to the meetings app / AI
/// engine like the write op.
#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum MeetingReadRequest {
    /// The recent meetings, newest first (the recent-meetings home).
    List,
    /// One meeting note by id (its summary metadata + action items).
    Get {
        /// The meeting id.
        id: String,
    },
}

/// Handle a `0x0C` meeting read: list or get, scoped to the attested caller. The
/// full note document (with its transcript) lives app-side; this returns the KG's
/// list/link metadata (summary + action items) as JSON. A get for an unknown id
/// answers `null`.
async fn handle_meetings_read(app_id: &str, body: &[u8], graph: &GraphHandle) -> String {
    if !meeting_caller_admitted(app_id) {
        return format!("ERROR: permission denied for meeting read by {app_id}");
    }
    let req: MeetingReadRequest = match serde_json::from_slice(body) {
        Ok(r) => r,
        Err(e) => return format!("ERROR: malformed meeting read request: {e}"),
    };
    match req {
        MeetingReadRequest::List => match crate::meeting::list_meetings(graph).await {
            Ok(rows) => serde_json::to_string(&rows).unwrap_or_else(|e| format!("ERROR: {e}")),
            Err(e) => format!("ERROR: {e}"),
        },
        MeetingReadRequest::Get { id } => match crate::meeting::get_meeting(graph, &id).await {
            Ok(Some(detail)) => {
                serde_json::to_string(&detail).unwrap_or_else(|e| format!("ERROR: {e}"))
            }
            Ok(None) => "null".to_string(),
            Err(e) => format!("ERROR: {e}"),
        },
    }
}

/// App ids permitted to write a consent Grant node (the Grant label is otherwise
/// daemon-internal). Only the consent broker; in debug a `dev.`-prefixed id is
/// also admitted (the dev/test convention the audit producers use).
fn consent_grant_writer_admitted(app_id: &str) -> bool {
    if app_id == "consent-broker" {
        return true;
    }
    // EXACT: the Grant label is daemon-internal, and a prefix match would let any
    // locally-built binary write consent grants for any app.
    #[cfg(debug_assertions)]
    if app_id == "dev.arlen-consent-broker" {
        return true;
    }
    false
}

/// Persist an authorised relation with an **atomic conditional create** that
/// reports whether it actually created the edge.
///
/// The endpoint types were validated as built-in system types, so their graph
/// table name is the type minus the `system.` prefix and the relation label is
/// a known identifier from the allowlist; none of those are attacker-controlled.
/// Only the endpoint ids are caller-supplied, so they are escaped into the
/// Cypher string literals.
///
/// The create is a single statement (`OPTIONAL MATCH ... WHERE r IS NULL CREATE
/// ... RETURN count`) on the dedicated, serial graph thread, so create-only-
/// if-absent cannot race a concurrent create: a second creator's statement runs
/// after the first and sees the edge, so it creates nothing. `created` is 1 iff
/// THIS statement created the edge, so a single attempt can distinguish a create
/// from a no-op and never double-creates. Three outcomes: `OK: created`,
/// `OK: exists` (idempotent no-op), or `ERROR: relation endpoints not found`.
///
/// The signal is per-*statement*, not per logical operation: if a create commits
/// but its response is lost and the call is retried, the retry sees the edge and
/// reports `exists`. Which logical *operation* created the edge is recorded
/// separately by the `op_id` set on the edge (see below): a caller that loses
/// the response reconciles by reading whether *its* `op_id` edge exists, a
/// causally-tied verdict the bare `created`/`exists` flag cannot give.
///
/// Row-level ownership/visibility on the matched endpoints is intentionally not
/// enforced here. The authorisation gate in `create_relation` already requires a
/// privileged `InstanceScope::All` token for a relation between nodes the caller
/// does not own, and the only write caller today is the agent, a first-party
/// component curating the global graph. Enforcing per-row `_owner`/`_deleted`
/// filters becomes load-bearing when an unprivileged app links its own
/// (anchored) nodes; that is the documented follow-up alongside app-relation
/// support.
async fn persist_relation(graph: &GraphHandle, rel: &RelationResult, op_id: &str) -> String {
    let from_label = rel
        .from_type
        .strip_prefix("system.")
        .unwrap_or(&rel.from_type);
    let to_label = rel.to_type.strip_prefix("system.").unwrap_or(&rel.to_type);
    let rel_type = &rel.relation_type;
    let from_id = escape_cypher(&rel.from_id);
    let to_id = escape_cypher(&rel.to_id);

    // Durable operation identity (idempotency key): persisted on the edge so a
    // lost-response retry can reconcile by reading whether *this* op's edge
    // exists. Only `FILE_PART_OF` carries the `op_id` column today, so it is set
    // only there; a missing/empty id leaves it NULL (as the promotion pipeline's
    // own creates do). The caller-supplied id is bounded and escaped into the
    // literal (it is untrusted; the agent derives a fixed-length digest).
    if op_id.len() > MAX_OP_ID_LEN {
        return "ERROR: op_id too long".to_string();
    }
    // FILE_PART_OF is the bi-temporal assertion edge: a file belongs to one
    // project at a time, so a new membership is a single-statement close-then-
    // append (§4.5) rather than a plain create. Other relations carry no
    // temporal columns and get the simple conditional create below.
    if rel_type == "FILE_PART_OF" {
        // The content-addressed merge identity of this membership fact, derived
        // from the RAW content tuple (not the Cypher-escaped literal, so the key
        // is independent of the escaping scheme and stays stable across devices).
        // graph-drift.md §2 / GD-R1.
        let merge_key =
            content_merge_key(from_label, &rel.from_id, rel_type, to_label, &rel.to_id);
        return persist_file_part_of(graph, from_label, to_label, &from_id, &to_id, op_id, &merge_key)
            .await;
    }

    // Atomic conditional create for a non-temporal relation. `created` = 1 only
    // if this statement created the edge; 0 if the edge already existed (the
    // `WHERE r IS NULL` filters its row out) OR an endpoint is missing (the MATCH
    // binds nothing). The write awaits its definitive result with no client-side
    // timeout: the graph worker is not cancellable, so a timeout would unblock
    // the caller while the queued CREATE could still commit, mis-reporting it.
    let create_cypher = format!(
        "MATCH (a:{from_label} {{id: '{from_id}'}}), (b:{to_label} {{id: '{to_id}'}}) \
         OPTIONAL MATCH (a)-[r:{rel_type}]->(b) WITH a, b, r WHERE r IS NULL \
         CREATE (a)-[:{rel_type}]->(b) RETURN count(*) AS created"
    );
    let created = match graph.query_rows(create_cypher).await {
        Ok(rs) => row_count(&rs),
        Err(e) => return format!("ERROR: {e}"),
    };
    if created > 0 {
        return "OK: created".to_string();
    }

    // created == 0 means either the edge already existed or an endpoint was
    // missing at create time. Disambiguate by checking the EDGE itself, never
    // merely the endpoints: an endpoint that a concurrent writer adds *after*
    // the create matched nothing must not be mistaken for a successful link.
    let edge_cypher = format!(
        "MATCH (a:{from_label} {{id: '{from_id}'}})-[r:{rel_type}]->(b:{to_label} {{id: '{to_id}'}}) \
         RETURN count(*) AS edge"
    );
    match graph.query_rows(edge_cypher).await {
        Ok(rs) if row_count(&rs) > 0 => "OK: exists".to_string(),
        Ok(_) => "ERROR: relation endpoints not found".to_string(),
        Err(e) => format!("ERROR: {e}"),
    }
}

/// Persist a FILE_PART_OF membership as a single-statement **close-then-append**
/// (bitemporal-knowledge-graph.md §4.5).
///
/// Because a file has a single live membership, this closes any currently-live
/// FILE_PART_OF to a *different* project (the single-membership contradiction),
/// then appends a freshly stamped edge — but only if no live edge for this exact
/// `(from, to)` pair already exists (the idempotent re-assert). It is one Cypher
/// statement, dispatched as one request the serial graph thread runs
/// uninterrupted, so it is race-free precisely because it is one statement.
///
/// Outcome strings are unchanged so the os-sdk client is not broken: a
/// supersession still returns `OK: created` (a new live edge was created; the
/// close is internal). The new edge records `superseded = old.op_id` (the closed
/// edge's id) so a later one-unit compensation can re-open what it replaced
/// (§4.6). `created_at`/`valid_at` are the server clock at persist; the
/// caller-supplied `valid_at`/`origin`/`prov_beh` are a protocol follow-up, so
/// `origin` defaults to the agent write socket's `agent` for now. The
/// `exists`-disambiguation checks the LIVE edge: a closed edge is retained but is
/// not a current membership.
async fn persist_file_part_of(
    graph: &GraphHandle,
    from_label: &str,
    to_label: &str,
    from_id: &str,
    to_id: &str,
    op_id: &str,
    merge_key: &str,
) -> String {
    let now = crate::time::now().0;
    let op_prop = if op_id.is_empty() {
        String::new()
    } else {
        format!("op_id: '{}', ", escape_cypher(op_id))
    };
    // `merge_key` is a fixed-length lowercase-hex digest (no escaping needed; it
    // contains no quote or backslash by construction), the content identity that
    // makes two devices' assertion of the same fact converge (GD-R1).
    let create_cypher = format!(
        "MATCH (a:{from_label} {{id: '{from_id}'}}), (b:{to_label} {{id: '{to_id}'}}) \
         OPTIONAL MATCH (a)-[old:FILE_PART_OF]->(c:Project) \
           WHERE c.id <> '{to_id}' AND old.invalid_at IS NULL AND old.expired_at IS NULL \
         SET old.invalid_at = {now}, old.expired_at = {now} \
         WITH a, b, old \
         OPTIONAL MATCH (a)-[live:FILE_PART_OF]->(b) \
           WHERE live.invalid_at IS NULL AND live.expired_at IS NULL \
         WITH a, b, old, live WHERE live IS NULL \
         CREATE (a)-[:FILE_PART_OF {{ {op_prop}valid_at: {now}, invalid_at: NULL, \
           created_at: {now}, expired_at: NULL, origin: 'agent', prov_beh: '', \
           merge_key: '{merge_key}', \
           superseded: CASE WHEN old IS NULL THEN NULL ELSE old.op_id END }}]->(b) \
         RETURN count(*) AS created"
    );
    let created = match graph.query_rows(create_cypher).await {
        Ok(rs) => row_count(&rs),
        Err(e) => return format!("ERROR: {e}"),
    };
    if created > 0 {
        return "OK: created".to_string();
    }
    // created == 0: a live membership for this exact pair already exists (the
    // append was skipped) or an endpoint is missing. Disambiguate on the LIVE
    // edge; a closed edge is retained but is not a current membership.
    let edge_cypher = format!(
        "MATCH (a:{from_label} {{id: '{from_id}'}})-[r:FILE_PART_OF]->(b:{to_label} {{id: '{to_id}'}}) \
         WHERE r.invalid_at IS NULL AND r.expired_at IS NULL RETURN count(*) AS edge"
    );
    match graph.query_rows(edge_cypher).await {
        Ok(rs) if row_count(&rs) > 0 => "OK: exists".to_string(),
        Ok(_) => "ERROR: relation endpoints not found".to_string(),
        Err(e) => format!("ERROR: {e}"),
    }
}

/// Persist a node-create with the atomic single-statement id probe (§5.3).
///
/// The label-agnostic existence check and the `CREATE` are ONE Cypher statement
/// on the serial graph thread, so the foreign-label id-collision check and the
/// create are evaluated together and cannot race (an earlier "probe then create"
/// framing is two statements and would). It creates only, never overwrites: a
/// node already carrying this id under ANY label refuses the create. `created` 1
/// is `OK: created`; 0 means a node with this id already exists (`OK: exists`,
/// the guarded no-op). `label` is a validated identifier from [`CREATABLE_NODES`]
/// (not attacker-controlled); only `id` is caller-supplied and escaped.
async fn persist_create_node(graph: &GraphHandle, label: &str, id: &str) -> String {
    // Ingestion-boundary canary reservation (canary-honeytools.md §3): refuse to
    // create any node whose id mentions the reserved token, so a producer can
    // never seed the canary namespace and the agent's operand tripwire keeps its
    // zero-false-positive property. Enforced at the persistence primitive so any
    // caller is covered, not only the current dispatch.
    if id.contains(RESERVED_CANARY_TOKEN) {
        return "ERROR: id is in the reserved canary namespace".to_string();
    }
    let id_lit = escape_cypher(id);
    let cypher = format!(
        "OPTIONAL MATCH (existing {{id: '{id_lit}'}}) WITH existing WHERE existing IS NULL \
         CREATE (:{label} {{id: '{id_lit}'}}) RETURN count(*) AS created"
    );
    match graph.query_rows(cypher).await {
        Ok(rs) if row_count(&rs) > 0 => "OK: created".to_string(),
        Ok(_) => "OK: exists".to_string(),
        Err(e) => format!("ERROR: {e}"),
    }
}

/// Persist an authorised relation *retract* (compensation): temporally close the
/// live edge that carries this `op_id`, restoring the membership it superseded if
/// any by appending a fresh live edge (the one-unit inverse of supersession,
/// §4.6; the restore is an append, never a stamp-clear, graph-drift.md §2).
///
/// The match is keyed by the `op_id` property AND the liveness predicate, so this
/// closes exactly the live edge the caller's own create stamped, never a bare
/// edge it did not write and never an already-closed one. Only `FILE_PART_OF`
/// carries the `op_id` column, so a retract of any other relation is refused
/// fail-closed (there is no precise key). The `op_id` non-emptiness was already
/// enforced by `retract_relation`; it is re-checked and length-bounded here.
///
/// Closing is **idempotent**: a match-nothing run (no live edge with this op_id,
/// already closed or never existed) is `OK: absent`, a successful no-op; a run
/// that closed the edge is `OK: retracted`. Both guarantee the same post-state
/// (no *live* edge with this op_id), which is what compensation needs, and the
/// closed edge is retained for audit (bitemporal-knowledge-graph.md §4.7). The
/// single statement runs on the serial graph thread, so a concurrent retract of
/// the same edge cannot double-close: the second sees no live edge and reports
/// `absent`.
async fn persist_retract(graph: &GraphHandle, rel: &RelationResult, op_id: &str) -> String {
    if op_id.is_empty() {
        return "ERROR: retract requires an op_id".to_string();
    }
    if op_id.len() > MAX_OP_ID_LEN {
        return "ERROR: op_id too long".to_string();
    }
    let rel_type = &rel.relation_type;
    // Only relations that carry the `op_id` column can be precisely retracted.
    // Anything else has no per-operation key, so deleting it would be a
    // bare-edge delete; refuse it rather than risk removing an untracked edge.
    if rel_type != "FILE_PART_OF" {
        return "ERROR: relation does not support op-id retract".to_string();
    }
    let from_label = rel
        .from_type
        .strip_prefix("system.")
        .unwrap_or(&rel.from_type);
    let to_label = rel.to_type.strip_prefix("system.").unwrap_or(&rel.to_type);
    let from_id = escape_cypher(&rel.from_id);
    let to_id = escape_cypher(&rel.to_id);
    let op = escape_cypher(op_id);

    // Retract is a temporal *close*, not a delete (bitemporal-knowledge-graph.md
    // §4.7): the edge is retained, its two intervals set, so an undone action
    // leaves a closed edge whose history corroborates the audit ledger rather
    // than vanishing. The close `MATCH` carries the liveness predicate, so it
    // mutates only an edge that was live (a `SET` over an already-closed edge is
    // a no-op), which keeps the retract idempotent.
    //
    // Endpoint labels and the relation type are validated identifiers; only the
    // ids and op_id are caller-supplied and escaped into the literals. `now` is
    // the server clock at the close (a server-computed i64, safe to interpolate).
    let now = crate::time::now().0;

    // The retracted/absent verdict is read FIRST, before the mutation, so it can
    // never report a half-applied state: was there a live edge for this op_id?
    // The probe and the transaction below are serialised on the graph thread, so
    // no other writer interleaves between them. `OK: retracted` is reported only
    // when the transaction that observed a live edge also commits; a transaction
    // error leaves the edge live (rollback), which the executor's reconcile then
    // reads as not-yet-retracted (Indeterminate) rather than a false success.
    let probe_cypher = format!(
        "MATCH (a:{from_label} {{id: '{from_id}'}})-[r:{rel_type} {{op_id: '{op}'}}]->(b:{to_label} {{id: '{to_id}'}}) \
         WHERE r.invalid_at IS NULL AND r.expired_at IS NULL \
         RETURN count(*) AS live"
    );
    let was_live = match graph.query_rows(probe_cypher).await {
        Ok(rs) => row_count(&rs) > 0,
        Err(e) => return format!("ERROR: {e}"),
    };

    // A retract is the inverse of supersession as ONE unit (§4.6): close the live
    // op_id edge, and if it superseded an earlier membership (it carries
    // `superseded = old.op_id`), restore that membership so the file returns to
    // its pre-supersession project rather than landing in neither. The restore
    // APPENDS a fresh live edge (graph-drift.md §2, GD-R4) instead of clearing the
    // superseded edge's close stamps: the old edge keeps its history (close-never-
    // delete §4.7), the reopen is a new event (its own `created_at`), and a
    // future merge cannot un-close what another device closed.
    //
    // The close and the reopen run as ONE atomic transaction, so a failure (or a
    // crash) of either rolls back BOTH and never leaves the file in neither
    // project; a retry redoes the whole unit cleanly. The reopen's append is
    // guarded against a duplicate (the `OPTIONAL MATCH ... live ... WHERE live IS
    // NULL` mirrors `persist_file_part_of`) and keys off the now-closed edge's
    // stable `op_id`/`superseded` (no liveness predicate on `r`), so it is
    // idempotent under retry. FOREACH (the openCypher conditional-create idiom)
    // is unavailable on Kuzu, so the conditional is the non-optional `MATCH
    // (a)-[old]->(p) WHERE old.op_id = r.superseded`: the CREATE fires for the
    // matched (r, old) pair and not at all when nothing was superseded. op_id is
    // the per-operation key (unique by construction); the `WITH ... LIMIT 1`
    // bounds the restore to a single membership defensively even if that
    // invariant were ever violated. The reopen carries a derived `reopen:<op>` id
    // (itself retractable, deterministic so the retry stays idempotent), no
    // `superseded` back-ref (it chains no further), the prior edge's `origin`
    // (defaulting to the agent's when the superseded edge carried none), and the
    // prior edge's `merge_key` (GD-R1): the reopen restores the SAME membership
    // (a -> p), so it must carry that membership's content identity, and `old`'s
    // key is exactly it — copying keeps the restored live edge content-identical
    // to the rest of the membership's history (a future merge sees one fact).
    //
    // Bitemporal axes: the reopen restores the prior edge's `valid_at` (the
    // membership was true from then, so inverting the supersession leaves no hole
    // in the valid-time line over the superseded period), while `created_at = now`
    // records that the system re-believed it at the undo instant. The superseded
    // edge keeps its own closed intervals, so a transaction-time read still shows
    // the move was believed until the undo.
    let close_stmt = format!(
        "MATCH (a:{from_label} {{id: '{from_id}'}})-[r:{rel_type} {{op_id: '{op}'}}]->(b:{to_label} {{id: '{to_id}'}}) \
         WHERE r.invalid_at IS NULL AND r.expired_at IS NULL \
         SET r.invalid_at = {now}, r.expired_at = {now}"
    );
    let reopen_stmt = format!(
        "MATCH (a:{from_label} {{id: '{from_id}'}})-[r:{rel_type} {{op_id: '{op}'}}]->(:Project) \
         WHERE r.superseded IS NOT NULL \
         MATCH (a)-[old:FILE_PART_OF]->(p:Project) WHERE old.op_id = r.superseded \
         WITH a, old, p LIMIT 1 \
         OPTIONAL MATCH (a)-[live:FILE_PART_OF]->(p) \
           WHERE live.invalid_at IS NULL AND live.expired_at IS NULL \
         WITH a, p, old, live WHERE live IS NULL \
         CREATE (a)-[:FILE_PART_OF {{ op_id: 'reopen:{op}', valid_at: old.valid_at, invalid_at: NULL, \
           created_at: {now}, expired_at: NULL, \
           origin: CASE WHEN old.origin IS NULL THEN 'agent' ELSE old.origin END, \
           prov_beh: '', merge_key: old.merge_key, superseded: NULL }}]->(p)"
    );
    if let Err(e) = graph.transaction(vec![close_stmt, reopen_stmt]).await {
        return format!("ERROR: {e}");
    }
    if was_live {
        "OK: retracted".to_string()
    } else {
        "OK: absent".to_string()
    }
}

/// Extract the first cell of the first row as an i64 (a `count(*)` result),
/// defaulting to 0 for an empty result.
fn row_count(rs: &crate::graph::RowSet) -> i64 {
    rs.rows
        .first()
        .and_then(|r| r.first())
        .map(|c| c.as_i64())
        .unwrap_or(0)
}

/// The app id for a SAME-uid peer whose `/proc/<pid>/exe` could not be read for
/// identity resolution.
///
/// The deployed knowledge daemon runs as root (for the eBPF sensor) and reads
/// any peer's exe, so it always resolves a real id. A NON-root daemon (the
/// integration harness and `just dev`, which run every daemon as the developer
/// uid) cannot read a same-uid peer's `/proc/<pid>/exe` (`__ptrace_may_access`
/// denies the cross-process read), so the peer resolves to the `unknown`
/// sentinel and the read-scope label gate then denies every seeded read.
///
/// In a debug build ONLY, honor a caller id declared by the launcher
/// (`ARLEN_KNOWLEDGE_DEV_SELF_ID`, set by the integration harness to the test's
/// own resolved app id) so its seeded read profile applies. A release daemon
/// never consults this env (it resolves via `/proc` as root); it is a
/// debug-only test/dev accommodation, the same shape as the audit daemon's
/// `ARLEN_AUDIT_EXTRA_ADMIT`. Same-uid only: a cross-uid peer keeps the strict
/// resolve-or-reject path above, so this cannot widen cross-user access.
fn same_uid_unresolved_id() -> String {
    #[cfg(debug_assertions)]
    {
        if let Ok(id) = std::env::var("ARLEN_KNOWLEDGE_DEV_SELF_ID") {
            if !id.is_empty() {
                return id;
            }
        }
    }
    "unknown".to_string()
}

/// Who the kernel will let connect to the read socket at all.
///
/// The peer check at accept time is the authority boundary and stays the
/// authority boundary. This is the door in front of it, and it exists because the
/// two disagreed: on the image the socket is the shared `/run/arlen/knowledge.sock`
/// under a `RuntimeDirectory` any uid can traverse, and it was mode 0666
/// unconditionally, so every uid on the host could connect and reach the framing,
/// the length caps and the rate-limiter state before being refused. The deployment
/// that names an owner has already said who is allowed; the socket should say it
/// too.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SocketExposure {
    /// No owner named. Anyone may connect and the peer check decides, which is the
    /// single-user model the per-user runtime dir already scopes by being 0700.
    Shared,
    /// An owner is named: only that uid may connect. Root still can, having
    /// CAP_DAC_OVERRIDE, which is what lets the root-run timeline client keep
    /// working.
    OwnerOnly(u32),
}

/// Pure so the rule is testable without a socket.
fn socket_exposure(owner_uid: Option<u32>, our_uid: u32) -> SocketExposure {
    match owner_uid {
        Some(owner) => SocketExposure::OwnerOnly(owner),
        // Running as the user with no owner configured is the per-user topology,
        // where the runtime directory is the boundary.
        None => {
            let _ = our_uid;
            SocketExposure::Shared
        }
    }
}

/// Apply the exposure to the bound socket.
///
/// Narrowing is best-effort in one direction only: a chown that fails leaves the
/// socket connectable and says so, because refusing to start would take the graph
/// down over a hardening step, and silently presenting a socket as narrowed when
/// it is not is the failure this whole change is about. The mode is set before the
/// owner so there is never a moment where a third uid may connect.
fn apply_socket_exposure(socket_path: &str, exposure: SocketExposure) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    match exposure {
        SocketExposure::Shared => {
            // 0666 because a cross-uid first-party client must be able to connect
            // and systemd's 0022 umask leaves `bind` owner-only-write.
            std::fs::set_permissions(socket_path, std::fs::Permissions::from_mode(0o666))?;
        }
        SocketExposure::OwnerOnly(owner) => {
            std::fs::set_permissions(socket_path, std::fs::Permissions::from_mode(0o600))?;
            let c_path = std::ffi::CString::new(socket_path)
                .map_err(|e| anyhow::anyhow!("socket path: {e}"))?;
            // SAFETY: a valid NUL-terminated path; -1 leaves the group unchanged.
            let rc = unsafe { libc::chown(c_path.as_ptr(), owner, u32::MAX) };
            if rc != 0 {
                let err = std::io::Error::last_os_error();
                tracing::warn!(
                    error = %err, owner,
                    "could not hand the read socket to its owner; widening it back so the \
                     named owner can still connect - the peer check is then the only boundary"
                );
                std::fs::set_permissions(socket_path, std::fs::Permissions::from_mode(0o666))?;
            }
        }
    }
    Ok(())
}

/// Whether a CROSS-uid peer (uid `peer_uid`, resolved to `tier`) may be served.
/// It must be a first-party/system client (a canonical /usr binary another user
/// cannot plant), AND - when `owner_uid` is configured (ARLEN_OWNER_UID, the
/// desktop session user) - it must be that owner. With no owner configured the
/// gate is single-user: any first-party/system cross-uid peer (the local user's
/// AI layer) is served. Same-uid peers do not consult this (always served).
///
/// The owner restriction is what makes a MULTI-user host safe: without it, the
/// FirstParty/System tier only blocks PLANTING a privileged binary, not running
/// code under that identity (the canonical binaries are world-executable +
/// non-setuid, so any uid can LD_PRELOAD one - see `handle_client`); pinning the
/// owner uid means only the data owner can do so, which is not an escalation.
fn cross_uid_admitted(peer_uid: u32, owner_uid: Option<u32>, tier: AppTier) -> bool {
    if !matches!(tier, AppTier::FirstParty | AppTier::System) {
        return false;
    }
    match owner_uid {
        Some(owner) => peer_uid == owner,
        None => true,
    }
}

/// Handle a single client connection.
/// Request to decide a pending merge suggestion (0x10). The caller names ONLY the
/// suggestion id and the decision; the merged pair (source/target) comes from the
/// stored suggestion, never the request, so a caller cannot direct a merge of an
/// arbitrary pair.
#[derive(serde::Deserialize)]
struct MergeDecisionRequest {
    suggestion_id: String,
    /// `true` = accept (merge: fold source into target); `false` = reject (keep both).
    accept: bool,
}

/// Decide a merge suggestion (0x10, SHARED-ENTITIES.md §4 "owner confirms"). Only
/// the OWNER app of the suggestion's entity type may act, and only on a `Pending`
/// suggestion. An accept runs `merge_node` (delete the duplicate source, keep the
/// canonical target, re-point the source's edges across the dynamic rel tables)
/// then marks it Accepted; a reject marks it Rejected and touches no entity. The
/// merged pair is read from the stored suggestion, not the request. Audited before
/// any mutation, fail-closed.
async fn handle_merge_accept(
    graph: &GraphHandle,
    audit: &Arc<dyn AuditSink>,
    token: Option<&crate::token::CapabilityToken>,
    app_id: &str,
    body: &[u8],
    uses: &Arc<Mutex<crate::lcg::UseTally>>,
) -> String {
    let req: MergeDecisionRequest = match serde_json::from_slice(body) {
        Ok(r) => r,
        Err(e) => return format!("ERROR: invalid request: {e}"),
    };
    let core = match crate::shared::fetch_suggestion(graph, &req.suggestion_id).await {
        Ok(Some(c)) => c,
        Ok(None) => return "ERROR: no such suggestion".to_string(),
        Err(e) => return format!("ERROR: {e}"),
    };
    // Owner-confirms: only the declared owner app of this shared type may decide
    // (the same peer-attested owner gate the shared-entity write uses).
    match crate::shared::shared_owner(&core.entity_type) {
        Some(owner) if owner == app_id => {}
        _ => {
            return "ERROR: only the owner app may decide a suggestion for this entity type"
                .to_string()
        }
    }
    // Bind the destructive decision to the owner's LIVE capability, not only its
    // compiled-in identity: a freshly minted token must resolve to the SAME owner
    // identity AND grant write to the type. So a revoked / profile-less owner (or a
    // pid-reuse race between connect and now) cannot drive a merge - the same
    // defense-in-depth the entity-write path applies via `can_write`.
    if !token.is_some_and(|t| t.app_id == app_id && t.can_write(&core.entity_type)) {
        return format!("ERROR: permission denied for {}", core.entity_type);
    }
    // Only a Pending suggestion is actionable; a terminal one is a no-op refusal
    // (so a merge is never re-applied through a stale suggestion id).
    if core.status != crate::shared::SuggestionStatus::Pending {
        return "ERROR: suggestion is not pending".to_string();
    }
    if req.accept {
        // Re-validate the stored pair is STILL a live duplicate before the
        // destructive fold. Entity ids are natural-key-derived and reusable after
        // deletion, so a stale suggestion could otherwise fold an unrelated
        // reused-id entity; refuse if either is gone or they no longer match.
        match crate::shared::suggestion_still_valid(
            graph,
            &core.entity_type,
            &core.source_id,
            &core.target_id,
        )
        .await
        {
            Ok(true) => {}
            Ok(false) => {
                return "ERROR: suggestion is stale; the entities no longer match, not merging"
                    .to_string()
            }
            Err(e) => return format!("ERROR: {e}"),
        }
        // Audit-before-act, fail-closed (S13): content-free (app id + type + the
        // decision, never the ids or field bodies).
        if let Err(e) = audit
            .submit(crate::audit::merge_decision_event(
                app_id,
                &core.entity_type,
                "merge-accepted",
            ))
            .await
        {
            warn!("merge-decision audit failed, refusing: {e}");
            return "ERROR: audit unavailable".to_string();
        }
        let table = crate::write::entity_table_name(&core.entity_type);
        if let Err(e) =
            crate::shared::merge_node(graph, &table, &core.source_id, &core.target_id).await
        {
            return format!("ERROR: merge: {e}");
        }
        // A merge is a write under the caller's grant for this entity type, and it
        // has now happened, so it counts. The refusal above (`can_write`) returns
        // before here, which is what keeps a denial out of the number.
        record_capability_uses(graph, uses, app_id, &[core.entity_type.clone()]).await;
        // The merge is applied; mark accepted best-effort (a status-write hiccup
        // only leaves the suggestion listable, and a re-accept is now re-validated
        // and either re-runs the idempotent merge_node harmlessly or is refused as
        // stale).
        if let Err(e) = crate::shared::update_suggestion_status(
            graph,
            &req.suggestion_id,
            crate::shared::SuggestionStatus::Accepted,
        )
        .await
        {
            warn!("merge accepted but status update failed (advisory): {e}");
        }
        "OK: merged".to_string()
    } else {
        // Reject: audit, then mark Rejected. No entity is touched.
        if let Err(e) = audit
            .submit(crate::audit::merge_decision_event(
                app_id,
                &core.entity_type,
                "merge-rejected",
            ))
            .await
        {
            warn!("merge-decision audit failed, refusing: {e}");
            return "ERROR: audit unavailable".to_string();
        }
        if let Err(e) = crate::shared::update_suggestion_status(
            graph,
            &req.suggestion_id,
            crate::shared::SuggestionStatus::Rejected,
        )
        .await
        {
            return format!("ERROR: {e}");
        }
        "OK: rejected".to_string()
    }
}

///
/// Phase 3.2 adds token awareness, but for backward compatibility the
/// daemon still accepts raw Cypher queries. Full token enforcement
/// (token on every request) is deferred to when the Request/Response
/// protobuf protocol replaces the current plaintext protocol.
#[allow(clippy::too_many_arguments)]
async fn handle_client(
    mut stream: UnixStream,
    graph: GraphHandle,
    pool: sqlx::SqlitePool,
    gate: crate::activity_delete::PromotionGate,
    auth: Arc<Mutex<Authenticator>>,
    rate: Arc<Mutex<RateState>>,
    emitter: Arc<RateLimitEmitter>,
    registry: Arc<SchemaRegistry>,
    our_uid: u32,
    owner_uid: Option<u32>,
    audit: Arc<dyn AuditSink>,
    uses: Arc<Mutex<crate::lcg::UseTally>>,
) -> Result<()> {
    // Resolve the peer identity once at connection for per-identity
    // rate limiting (foundation §8.4). The socket is per-user, so a
    // cross-uid peer is rejected; an unresolvable binary is treated as
    // the strictest tier via the `unknown` sentinel (ThirdParty).
    //
    // Known limitation (same-uid, F3): the tier is derived from the
    // resolved `app_id`, and `arlen-permissions` maps a user-
    // installed `~/.local/share/arlen/apps/{id}/` binary to `{id}`.
    // So a same-uid attacker could squat a reserved id (`system` →
    // unlimited, `ai-daemon` → FirstParty) to escape ThirdParty. This
    // does not regress vs. pre-S15 (which rate-limited no one) and is
    // the same gap as the audit daemon's ingest admission; the global
    // fix is the installd inode-keyed identity registry
    // (`docs/architecture/identity-spoof-mitigation.md`). A
    // provenance check (privileged tiers only from canonical /usr
    // paths) is the interim hardening when that lands.
    // `peer` is the kernel-attested write peer (pid + start time), captured
    // once for write-mode token issuance and the PID-reuse guard; `None` when
    // it cannot be trusted, so a write fails closed.
    let (app_id, peer) = match so_peercred(stream.as_raw_fd()) {
        Ok((pid, uid)) => {
            let cross_uid = uid != our_uid;
            if pid <= 0 {
                // No pid to resolve. A same-uid peer is the local trusted case
                // (served as unknown/ThirdParty); a cross-uid peer cannot be
                // identified as a first-party client, so reject it rather than
                // serve it as `unknown` (keeps the cross-uid gate absolute).
                if cross_uid {
                    warn!(peer_uid = uid, "graph daemon: rejecting unidentifiable cross-uid client");
                    return Ok(());
                }
                ("unknown".to_string(), None)
            } else {
                let pid = pid as u32;
                // Resolve the peer's identity. As a system service this daemon
                // runs as root, so it can read a cross-uid peer's
                // `/proc/<pid>/exe` even when that peer is process-hardened
                // (non-dumpable) - the resolution that lets the per-user AI layer
                // reach the system Knowledge Graph.
                let id = match app_id_from_pid(pid) {
                    Ok(id) => id,
                    Err(e) => {
                        // A hardened, non-dumpable cross-uid peer's /proc/exe is
                        // EACCES even to root (the VM boot proved the old "root can
                        // always read it" assumption false), and there is no longer
                        // a second source to ask. The peer's cgroup unit used to
                        // answer here, until it turned out the peer owns the
                        // directory names under its own unit: naming one after the
                        // AI daemon made an unreadable identity into that daemon's,
                        // which is what the tier and the read scope key on. An
                        // unresolvable caller is unresolved, and recorded as such.
                        if cross_uid {
                            warn!(
                                peer_uid = uid,
                                pid,
                                error = %e,
                                "graph daemon: cross-uid app_id resolution failed (peer served as unknown)"
                            );
                            "unknown".to_string()
                        } else {
                            same_uid_unresolved_id()
                        }
                    }
                };
                // A SAME-uid peer is served, scoped by `id` (other root services).
                // A CROSS-uid peer is served only when it resolves FirstParty/
                // System: the documented deployment where this root daemon serves
                // the SESSION USER's AI layer (ai-agent/ai-daemon run as the user;
                // this daemon as root for the eBPF sensor).
                //
                // SECURITY (honest scope, per adversarial review): the FirstParty/
                // System tiers require canonical /usr install paths, which blocks a
                // cross-uid peer from PLANTING a privileged binary - but NOT from
                // RUNNING code under that identity. The canonical binaries are
                // world-executable and not setuid, so any local uid can
                // `LD_PRELOAD=evil.so /usr/lib/arlen/libexec/arlen-ai-engine-daemon`
                // (or ptrace it) and present `ai-agent` as its /proc/self/exe. With no
                // owner configured this gate therefore grants ANY local uid a full
                // read of the system graph (a FirstParty read is `system_anchored`,
                // bypassing the label gate + sensitive-column scrub) - acceptable
                // under Arlen's single-user-desktop model, where the one human user
                // already has broad local visibility and IS the served session.
                // Setting ARLEN_OWNER_UID closes the MULTI-user case: a cross-uid
                // peer is then served only if it is the owner, so a different human
                // user cannot reach this user's graph (the LD_PRELOAD vector is then
                // only the owner reading their own data, not an escalation).
                if cross_uid
                    && !cross_uid_admitted(
                        uid,
                        owner_uid,
                        QuotaConfig::arlen_default().tier_for_app(&id),
                    )
                {
                    warn!(
                        peer_uid = uid,
                        app_id = %id,
                        "graph daemon: rejecting untrusted cross-uid client"
                    );
                    return Ok(());
                }
                let start_time = pid_start_time(pid).ok();
                (id, Some(WritePeer { pid, uid, start_time }))
            }
        }
        Err(e) => {
            warn!("graph daemon: SO_PEERCRED failed ({e}); treating peer as untrusted");
            ("unknown".to_string(), None)
        }
    };
    debug!(app_id = %app_id, "new graph daemon client");

    // LCG §4.1: project the connecting app's capability at connect, so an app
    // with graph access appears in the browse surface even when it only ever
    // reads (the typed-query path mints no per-request token, so a pure reader
    // would otherwise have no Grant node). Best-effort and off the request path:
    // mint from the peer's profile and emit; an app with no graph access (the
    // mint fails closed) gets no node, and a graph hiccup degrades the projection
    // never the connection. An `unknown` peer (SO_PEERCRED unresolved) is skipped
    // so its grants never pool under one shared id.
    if app_id != "unknown" {
        if let Some(p) = &peer {
            let minted = auth.lock().await.issue_token_for_pid(p.uid, p.pid).ok();
            if let Some(token) = minted {
                match crate::lcg::emit_grant_node(&graph, &token).await {
                    Ok(true) => {
                        let _ = audit
                            .submit(crate::audit::grant_issued_event(
                                &token.app_id,
                                &crate::lcg::reach_summary(&token),
                            ))
                            .await;
                    }
                    Ok(false) => {}
                    Err(e) => {
                        warn!(app_id = %app_id, "connect-time grant emit failed: {e}");
                    }
                }
            }
        }
        // Also project the app's DECLARED reach across ALL profile dimensions as
        // `declared`-source LCG grants (LCG §11b), so the App-access page shows +
        // revokes the app's full reach (network, event_bus, filesystem, clipboard,
        // ...). Independent of the token (a declaration, not a graph capability):
        // load the full profile, emit a grant per dimension whose reach is declared.
        // Best-effort; a profile-less app gets no declared grants, and the emit's
        // revoke-preserving ON MATCH means this per-connect refresh never resurrects
        // a grant the user revoked.
        //
        // Deliberately NOT audited, unlike the token grant above, and the asymmetry
        // is the point: a token grant is an ISSUANCE - authority minted now, from a
        // profile, for this process - while a declared grant is a projection of what
        // the profile has said all along. Auditing it would write one entry per
        // dimension per connect for a fact that did not change, and a ledger that
        // records non-events is a ledger nobody reads. The declaration's own change
        // is audited where it happens: the profile write, and revoke.
        if let Ok(profile) = arlen_permissions::load_profile(&app_id) {
            if let Err(e) = crate::lcg::emit_all_declared_grants(&graph, &app_id, &profile).await {
                warn!(app_id = %app_id, "connect-time declared grants emit failed: {e}");
            }
        }
    }

    loop {
        // Read query length.
        let mut len_buf = [0u8; 4];
        match stream.read_exact(&mut len_buf).await {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                debug!("graph daemon client disconnected");
                return Ok(());
            }
            Err(e) => return Err(e.into()),
        }

        let len = u32::from_be_bytes(len_buf) as usize;
        if len == 0 || len > 64 * 1024 {
            warn!(len, "invalid query length");
            return Ok(());
        }

        // Read the request body; its leading byte selects the mode.
        let mut buf = vec![0u8; len];
        stream.read_exact(&mut buf).await?;

        // Write mode: a leading 0x02 byte selects a structured graph write
        // request (JSON body) instead of a Cypher read query. It is rate-
        // limited on the per-identity *write* bucket and authorised + persisted
        // by `handle_write_request`; the read path below is untouched.
        if buf.first() == Some(&0x02) {
            let body = &buf[1..];

            // Least-privilege dispatch gate: only first-party / system callers
            // may mutate the graph; a ThirdParty (or unresolved `unknown`) peer
            // is refused before any token work. This does not by itself defeat
            // same-uid app_id spoofing (see `handle_write_request`), but it
            // keeps the system-relation write path off-limits to ordinary apps.
            let tier = QuotaConfig::arlen_default().tier_for_app(&app_id);
            if tier == AppTier::ThirdParty {
                warn!(app_id = %app_id, "graph write refused for non-first-party caller");
                let response = "ERROR: write mode not permitted for this caller";
                let response_len = (response.len() as u32).to_be_bytes();
                stream.write_all(&response_len).await?;
                stream.write_all(response.as_bytes()).await?;
                continue;
            }

            let violation = {
                let mut rs = rate.lock().await;
                match rs.limiter.check_write(&app_id) {
                    Ok(()) => None,
                    Err(e) => Some((e.to_string(), rs.should_emit(&app_id))),
                }
            };

            let (response, emit_violation) = if let Some((reason, emit)) = violation {
                warn!(app_id = %app_id, "graph write rate limit exceeded");
                (format!("ERROR: RateLimited: {reason}"), emit)
            } else {
                (
                    handle_write_request(
                        body,
                        peer,
                        &registry,
                        &graph,
                        &pool,
                        &gate,
                        &auth,
                        Some(&audit),
                        Some(&uses),
                    )
                    .await,
                    false,
                )
            };

            if emit_violation {
                let emitter = emitter.clone();
                let app_id = app_id.clone();
                tokio::task::spawn_blocking(move || emitter.emit(&app_id));
            }

            timing_noise().await;

            let response_bytes = response.as_bytes();
            let response_len = u32::try_from(response_bytes.len())
                .expect("response too large")
                .to_be_bytes();
            stream.write_all(&response_len).await?;
            stream.write_all(response_bytes).await?;
            continue;
        }

        // Retrieve mode: a leading 0x03 byte selects the LLM-free retrieval op
        // (§7). The body is a JSON `{query, limit}`; the response is a JSON array
        // of ranked node ids (best first), or the plaintext `ERROR: ...` form on
        // failure (a client detects the `ERROR:` prefix before parsing JSON). It
        // is a read, so it is query-rate-limited and open to every tier; the
        // §7.6 read-tier label/time-window gating on the returned set is a
        // follow-up, shared with the coarse gating on the typed query path.
        if buf.first() == Some(&0x03) {
            let violation = {
                let mut rs = rate.lock().await;
                rs.limiter.check_query(&app_id).err().map(|e| e.to_string())
            };
            let response = if let Some(reason) = violation {
                format!("ERROR: RateLimited: {reason}")
            } else {
                match serde_json::from_slice::<RetrieveRequest>(&buf[1..]) {
                    Ok(req) => {
                        let limit = req.limit.clamp(1, MAX_RETRIEVE_LIMIT);
                        match crate::retrieval::retrieve(&pool, &graph, &req.query, limit).await {
                            Ok(ids) => {
                                // RS-R1: scope the returned ids to the caller's
                                // readable labels (a system caller sees all).
                                let system_anchored = app_id != "unknown"
                                    && QuotaConfig::arlen_default().tier_for_app(&app_id)
                                        != AppTier::ThirdParty;
                                let scoped = if system_anchored {
                                    ids
                                } else {
                                    let readable = caller_readable_labels(peer.as_ref(), &auth).await;
                                    filter_ids_to_readable_labels(&graph, &ids, &readable, Some((&uses, &app_id))).await
                                };
                                serde_json::to_string(&scoped).unwrap_or_else(|_| "[]".to_string())
                            }
                            Err(e) => format!("ERROR: {e}"),
                        }
                    }
                    Err(e) => format!("ERROR: invalid retrieve request: {e}"),
                }
            };
            timing_noise().await;
            let response_bytes = response.as_bytes();
            let response_len = u32::try_from(response_bytes.len())
                .expect("response too large")
                .to_be_bytes();
            stream.write_all(&response_len).await?;
            stream.write_all(response_bytes).await?;
            continue;
        }

        // Live-working-set mode: a leading 0x0A byte selects the briefing engine's
        // ranked-live-set aggregation (agent-work-surfaces-plan.md surface 2). The
        // body is a JSON `{limit}`; the response is a JSON array of ranked items
        // across the caller's graph. A pure read, rate-limited + scoped to the
        // caller's readable labels for a non-system caller, exactly like prep.
        if buf.first() == Some(&0x0A) {
            let violation = {
                let mut rs = rate.lock().await;
                rs.limiter.check_query(&app_id).err().map(|e| e.to_string())
            };
            let response = if let Some(reason) = violation {
                format!("ERROR: RateLimited: {reason}")
            } else {
                match serde_json::from_slice::<WorkingSetRequest>(&buf[1..]) {
                    Ok(req) => {
                        let limit = req.limit.clamp(1, MAX_PREP_LIMIT) as usize;
                        match crate::working_set::gather_working_set(&graph, crate::time::now().0, limit)
                            .await
                        {
                            Ok(items) => {
                                let system_anchored = app_id != "unknown"
                                    && QuotaConfig::arlen_default().tier_for_app(&app_id)
                                        != AppTier::ThirdParty;
                                let scoped = if system_anchored {
                                    items
                                } else {
                                    let ids: Vec<String> =
                                        items.iter().map(|i| i.id.clone()).collect();
                                    let readable =
                                        caller_readable_labels(peer.as_ref(), &auth).await;
                                    let allowed: std::collections::HashSet<String> =
                                        filter_ids_to_readable_labels(
                                            &graph,
                                            &ids,
                                            &readable,
                                            Some((&uses, &app_id)),
                                        )
                                        .await
                                            .into_iter()
                                            .collect();
                                    items.into_iter().filter(|i| allowed.contains(&i.id)).collect()
                                };
                                serde_json::to_string(&scoped).unwrap_or_else(|_| "[]".to_string())
                            }
                            Err(e) => format!("ERROR: {e}"),
                        }
                    }
                    Err(e) => format!("ERROR: invalid working-set request: {e}"),
                }
            };
            timing_noise().await;
            let response_bytes = response.as_bytes();
            let response_len = u32::try_from(response_bytes.len())
                .expect("response too large")
                .to_be_bytes();
            stream.write_all(&response_len).await?;
            stream.write_all(response_bytes).await?;
            continue;
        }

        // List merge-suggestions mode: a leading 0x0F byte lists the pending
        // shared-entity merge suggestions for review (SHARED-ENTITIES.md). It is
        // system-internal review data (about first-party-managed shared entities),
        // so only a system-anchored (non-ThirdParty) caller sees them; a ThirdParty
        // caller gets an empty list, never a denial oracle. Query-rate-limited.
        if buf.first() == Some(&0x0F) {
            let violation = {
                let mut rs = rate.lock().await;
                rs.limiter.check_query(&app_id).err().map(|e| e.to_string())
            };
            let response = if let Some(reason) = violation {
                format!("ERROR: RateLimited: {reason}")
            } else {
                match serde_json::from_slice::<ListSuggestionsRequest>(&buf[1..]) {
                    Ok(req) => {
                        let system_anchored = app_id != "unknown"
                            && QuotaConfig::arlen_default().tier_for_app(&app_id)
                                != AppTier::ThirdParty;
                        if !system_anchored {
                            "[]".to_string()
                        } else {
                            let limit = req.limit.clamp(1, MAX_PREP_LIMIT) as usize;
                            let cypher = crate::shared::pending_suggestions_query(
                                req.entity_type.as_deref(),
                                limit,
                            );
                            match graph.query_rows_json(cypher).await {
                                Ok(json) => json,
                                Err(e) => format!("ERROR: {e}"),
                            }
                        }
                    }
                    Err(e) => format!("ERROR: invalid list-suggestions request: {e}"),
                }
            };
            timing_noise().await;
            let response_bytes = response.as_bytes();
            let response_len = u32::try_from(response_bytes.len())
                .expect("response too large")
                .to_be_bytes();
            stream.write_all(&response_len).await?;
            stream.write_all(response_bytes).await?;
            continue;
        }

        // Merge-decision mode: a leading 0x10 byte confirms/dismisses a pending
        // merge suggestion (SHARED-ENTITIES.md §4 owner-confirms). The body is a
        // JSON `{suggestion_id, accept}`. It MUTATES the graph on accept (folds a
        // duplicate + deletes it), gated to the entity type's owner app; query-
        // rate-limited (user-initiated, infrequent).
        if buf.first() == Some(&0x10) {
            let violation = {
                let mut rs = rate.lock().await;
                rs.limiter.check_query(&app_id).err().map(|e| e.to_string())
            };
            let response = if let Some(reason) = violation {
                format!("ERROR: RateLimited: {reason}")
            } else {
                // Mint the caller's live token from the kernel-attested peer pid so
                // the merge is bound to the owner's CURRENT capability (revocation-
                // honouring), the same way the entity-write path mints it.
                let token = if let Some(p) = &peer {
                    auth.lock().await.issue_token_for_pid(p.uid, p.pid).ok()
                } else {
                    None
                };
                handle_merge_accept(&graph, &audit, token.as_ref(), &app_id, &buf[1..], &uses).await
            };
            timing_noise().await;
            let response_bytes = response.as_bytes();
            let response_len = u32::try_from(response_bytes.len())
                .expect("response too large")
                .to_be_bytes();
            stream.write_all(&response_len).await?;
            stream.write_all(response_bytes).await?;
            continue;
        }

        // Prep-for-this mode: a leading 0x09 byte selects the prep aggregation
        // (agent-work-surfaces-plan.md surface 3). The body is a JSON
        // `{subject_id, limit}`; the response is a JSON array of ranked prep items,
        // or the plaintext `ERROR:` form. It is a pure read of the caller's own
        // graph, so it is query-rate-limited and, like retrieve, its returned ids
        // are scoped to the caller's readable labels for a non-system caller
        // (so a third-party app cannot prep-harvest the user's project structure).
        if buf.first() == Some(&0x09) {
            let violation = {
                let mut rs = rate.lock().await;
                rs.limiter.check_query(&app_id).err().map(|e| e.to_string())
            };
            let response = if let Some(reason) = violation {
                format!("ERROR: RateLimited: {reason}")
            } else {
                match serde_json::from_slice::<PrepRequest>(&buf[1..]) {
                    Ok(req) => {
                        let limit = req.limit.clamp(1, MAX_PREP_LIMIT) as usize;
                        match crate::prep::gather_prep_candidates(&graph, &req.subject_id).await {
                            Ok(candidates) => {
                                let system_anchored = app_id != "unknown"
                                    && QuotaConfig::arlen_default().tier_for_app(&app_id)
                                        != AppTier::ThirdParty;
                                let scoped = if system_anchored {
                                    candidates
                                } else {
                                    // Keep only candidates whose id is readable by
                                    // the caller (mirrors the retrieve scoping).
                                    let ids: Vec<String> =
                                        candidates.iter().map(|c| c.id.clone()).collect();
                                    let readable =
                                        caller_readable_labels(peer.as_ref(), &auth).await;
                                    let allowed: std::collections::HashSet<String> =
                                        filter_ids_to_readable_labels(
                                            &graph,
                                            &ids,
                                            &readable,
                                            Some((&uses, &app_id)),
                                        )
                                        .await
                                            .into_iter()
                                            .collect();
                                    candidates
                                        .into_iter()
                                        .filter(|c| allowed.contains(&c.id))
                                        .collect()
                                };
                                let view = crate::prep::rank_prep(scoped, crate::time::now().0, limit);
                                serde_json::to_string(&view).unwrap_or_else(|_| "[]".to_string())
                            }
                            Err(e) => format!("ERROR: {e}"),
                        }
                    }
                    Err(e) => format!("ERROR: invalid prep request: {e}"),
                }
            };
            timing_noise().await;
            let response_bytes = response.as_bytes();
            let response_len = u32::try_from(response_bytes.len())
                .expect("response too large")
                .to_be_bytes();
            stream.write_all(&response_len).await?;
            stream.write_all(response_bytes).await?;
            continue;
        }

        // Capsule materialize mode: a leading 0x07 byte selects the Context
        // Capsule frozen-slice read (context-capsule.md §4, loader option (b)). The
        // body is a JSON `CapsuleScope`; the response is the canonical JSON of the
        // materialized `FrozenSlice`, or the plaintext `ERROR: ...` form (a client
        // detects the `ERROR:` prefix before parsing the JSON object). It reads the
        // caller's OWN graph; its reach is bounded NOT by the 0x01 read-only/
        // authority-label deny but by construction — the materializer loads only
        // `File`/`Project` (the `CAPSULE_LABELS` allowlist) and follows only live
        // `FILE_PART_OF`, so it never touches authority nodes, and the manifest is
        // hop- AND breadth-capped. Query-rate-limited and wrapped in a wall-clock
        // timeout (the materialize does O(manifest) reads, so it gets a longer
        // bound than the single-probe sibling ops). The capsule access control (the
        // signed grant, the audience, the human-gated mint) lives in `capsuled`.
        if buf.first() == Some(&0x07) {
            let violation = {
                let mut rs = rate.lock().await;
                rs.limiter.check_query(&app_id).err().map(|e| e.to_string())
            };
            let response = if let Some(reason) = violation {
                format!("ERROR: RateLimited: {reason}")
            } else {
                match serde_json::from_slice::<arlen_capsule::scope::CapsuleScope>(&buf[1..]) {
                    Ok(mut scope) => {
                        scope.expand_hops = scope.expand_hops.min(MAX_CAPSULE_HOPS);
                        let materialize = crate::capsule::materialize_slice(&graph, &scope);
                        match tokio::time::timeout(CAPSULE_MATERIALIZE_TIMEOUT, materialize).await {
                            Ok(Ok(slice)) => String::from_utf8(slice.canonical_bytes())
                                .unwrap_or_else(|_| "ERROR: non-utf8 slice".to_string()),
                            Ok(Err(e)) => format!("ERROR: {e}"),
                            Err(_) => "ERROR: capsule materialize timed out".to_string(),
                        }
                    }
                    Err(e) => format!("ERROR: invalid capsule scope: {e}"),
                }
            };
            timing_noise().await;
            let response_bytes = response.as_bytes();
            let response_len = u32::try_from(response_bytes.len())
                .expect("response too large")
                .to_be_bytes();
            stream.write_all(&response_len).await?;
            stream.write_all(response_bytes).await?;
            continue;
        }

        // Provenance read mode: a leading 0x04 byte selects the caller-scoped
        // provenance op (provenance-halo.md §5, the shared read-scope op). Like
        // the retrieve op it is a read (query-rate-limited), but every outcome is
        // bounded and routed through `timing_noise()` with a single denial shape,
        // so a spoofed object reference is not a file-existence oracle. The 500 ms
        // bound (as on the typed path) caps the per-label scope probe.
        if buf.first() == Some(&0x04) {
            let violation = {
                let mut rs = rate.lock().await;
                rs.limiter.check_query(&app_id).err().map(|e| e.to_string())
            };
            let response = if let Some(reason) = violation {
                format!("ERROR: RateLimited: {reason}")
            } else {
                match tokio::time::timeout(
                    Duration::from_millis(500),
                    handle_provenance_read(&buf[1..], &peer, &auth, &graph, &uses),
                )
                .await
                {
                    Ok(r) => r,
                    Err(_elapsed) => PROVENANCE_OUT_OF_SCOPE.to_string(),
                }
            };
            timing_noise().await;
            let response_bytes = response.as_bytes();
            let response_len = u32::try_from(response_bytes.len())
                .expect("response too large")
                .to_be_bytes();
            stream.write_all(&response_len).await?;
            stream.write_all(response_bytes).await?;
            continue;
        }

        // Structured typed read mode: a leading 0x08 byte selects RS-R2, the only
        // bypass-proof read for sensitive labels - the daemon owns the entire query
        // shape (the caller supplies no Cypher), so a value cannot smuggle clause
        // structure and a field cannot launder past the projection. It mirrors the
        // 0x04 provenance op: query-rate-limited, 500 ms-bounded, every failure the
        // single uniform denial routed through `timing_noise()` (no existence
        // oracle). NB 0x07 is the capsule materialize op; 0x08 is the next free byte.
        if buf.first() == Some(&0x08) {
            let violation = {
                let mut rs = rate.lock().await;
                rs.limiter.check_query(&app_id).err().map(|e| e.to_string())
            };
            let response = if let Some(reason) = violation {
                format!("ERROR: RateLimited: {reason}")
            } else {
                match tokio::time::timeout(
                    Duration::from_millis(500),
                    handle_typed_read(&buf[1..], peer.as_ref(), &auth, &graph, &uses),
                )
                .await
                {
                    Ok(r) => r,
                    Err(_elapsed) => PROVENANCE_OUT_OF_SCOPE.to_string(),
                }
            };
            timing_noise().await;
            let response_bytes = response.as_bytes();
            let response_len = u32::try_from(response_bytes.len())
                .expect("response too large")
                .to_be_bytes();
            stream.write_all(&response_len).await?;
            stream.write_all(response_bytes).await?;
            continue;
        }

        // Code-analysis mode: a leading 0x0D byte selects the CG-R5 whole-codebase
        // analysis (god-symbols + surprises over the CodeSymbol/CALLS graph,
        // token-free). The handler gates it to system-anchored callers (the
        // aggregate exceeds a ThirdParty's per-label read scope); query-rate-
        // limited and 500 ms-bounded like the other read ops.
        //
        // This op previously claimed 0x09, which the prep-for-this read ALREADY
        // held further up this chain - so every code-analysis request was
        // swallowed by prep (which then failed parsing the absent
        // `{subject_id, limit}` body) and CG-R5 could never run at all, including
        // through its knowledge-mcp tool. 0x0D is the first genuinely free byte:
        // 0x01-0x0C are taken. See `mode_bytes_are_uniquely_assigned`.
        if buf.first() == Some(&0x0D) {
            let violation = {
                let mut rs = rate.lock().await;
                rs.limiter.check_query(&app_id).err().map(|e| e.to_string())
            };
            let response = if let Some(reason) = violation {
                format!("ERROR: RateLimited: {reason}")
            } else {
                match tokio::time::timeout(
                    Duration::from_millis(500),
                    handle_code_analysis(&app_id, &graph),
                )
                .await
                {
                    Ok(r) => r,
                    Err(_elapsed) => CODE_ANALYSIS_DENIED.to_string(),
                }
            };
            timing_noise().await;
            let response_bytes = response.as_bytes();
            let response_len = u32::try_from(response_bytes.len())
                .expect("response too large")
                .to_be_bytes();
            stream.write_all(&response_len).await?;
            stream.write_all(response_bytes).await?;
            continue;
        }

        // Code-symbol-context mode: a leading 0x0E byte selects the CG-R6 fusion
        // read — a symbol's defining file, its project (bitemporal, optionally
        // as-of), and its accessing apps. The body is a JSON CodeSymbolRequest.
        // Same system-anchored gate + rate-limit + 500 ms bound as 0x09.
        if buf.first() == Some(&0x0E) {
            let violation = {
                let mut rs = rate.lock().await;
                rs.limiter.check_query(&app_id).err().map(|e| e.to_string())
            };
            let response = if let Some(reason) = violation {
                format!("ERROR: RateLimited: {reason}")
            } else {
                match tokio::time::timeout(
                    Duration::from_millis(500),
                    handle_code_symbol_context(&app_id, &graph, &buf[1..]),
                )
                .await
                {
                    Ok(r) => r,
                    Err(_elapsed) => CODE_SYMBOL_DENIED.to_string(),
                }
            };
            timing_noise().await;
            let response_bytes = response.as_bytes();
            let response_len = u32::try_from(response_bytes.len())
                .expect("response too large")
                .to_be_bytes();
            stream.write_all(&response_len).await?;
            stream.write_all(response_bytes).await?;
            continue;
        }

        // Access-grants mode: a leading 0x05 byte selects the caller-scoped grant
        // browse read (living-capability-graph.md §5). The caller's own grants
        // only (scoped by the attested app_id resolved at connect, never a request
        // field), with live recomputed fresh. It is a read, so query-rate-limited
        // and open to every tier; the general read path's label-denial keeps these
        // nodes off the raw-Cypher path, so this is their only reader.
        if buf.first() == Some(&0x05) {
            let violation = {
                let mut rs = rate.lock().await;
                rs.limiter.check_query(&app_id).err().map(|e| e.to_string())
            };
            let response = if let Some(reason) = violation {
                format!("ERROR: RateLimited: {reason}")
            } else {
                // Bounded wait, as the typed and provenance read ops have, so a
                // slow graph cannot pin the connection without a client deadline.
                match tokio::time::timeout(
                    Duration::from_millis(500),
                    handle_access_grants(&app_id, &graph),
                )
                .await
                {
                    Ok(r) => r,
                    Err(_elapsed) => ACCESS_GRANTS_ERROR.to_string(),
                }
            };
            timing_noise().await;
            let response_bytes = response.as_bytes();
            let response_len = u32::try_from(response_bytes.len())
                .expect("response too large")
                .to_be_bytes();
            stream.write_all(&response_len).await?;
            stream.write_all(response_bytes).await?;
            continue;
        }

        // Revoke mode: a leading 0x06 byte selects the LCG narrowing-only revoke
        // (living-capability-graph.md §6). The body is a JSON `RevokeReach`. It
        // mutates the target's user-tier permission profile, admitted only for the
        // `settings` principal; query-rate-limited (infrequent, user-initiated).
        if buf.first() == Some(&0x06) {
            let violation = {
                let mut rs = rate.lock().await;
                rs.limiter.check_query(&app_id).err().map(|e| e.to_string())
            };
            let response = if let Some(reason) = violation {
                format!("ERROR: RateLimited: {reason}")
            } else {
                handle_revoke(&app_id, &buf[1..])
            };
            // Record the narrowing as a capability-change provenance event: the
            // durable "what was removed" the profile-first restore reads back as its
            // ceiling (living-capability-graph.md §6). Best-effort AFTER the narrow -
            // a revoke is a tightening the user must always be able to make, so a
            // down audit must never block it; a dropped record only means that reach
            // cannot later be restored (fail-closed on the loosening direction). The
            // body parsed cleanly inside handle_revoke, so it re-parses here.
            if response == crate::revoke::RevokeOutcome::Revoked.wire_token() {
                if let Ok(req) =
                    serde_json::from_slice::<crate::revoke::RevokeReach>(&buf[1..])
                {
                    if let Err(e) = audit
                        .submit(crate::audit::capability_change_event(
                            &req.target_app_id,
                            &req.reach,
                            crate::revoke::OUTCOME_REVOKED,
                        ))
                        .await
                    {
                        warn!("capability-change audit failed (reach not recorded for restore): {e}");
                    }
                }
            }
            timing_noise().await;
            let response_bytes = response.as_bytes();
            let response_len = u32::try_from(response_bytes.len())
                .expect("response too large")
                .to_be_bytes();
            stream.write_all(&response_len).await?;
            stream.write_all(response_bytes).await?;
            continue;
        }

        // Restore mode: a leading 0x0B byte selects the LCG restore (re-widen), the
        // reverse of revoke and the one authority-growth path. 0x01-0x0A are all
        // taken (0x08 is the RS-R2 typed read, 0x09/0x0A the code-analysis ops), so
        // restore is 0x0B. The body is a JSON `RestoreReach`. Admitted only for the
        // Settings principal; the reach is bounded to a recorded removal in the
        // durable audit ledger, all inside `handle_restore`. Rate-limited like revoke.
        if buf.first() == Some(&0x0B) {
            let violation = {
                let mut rs = rate.lock().await;
                rs.limiter.check_query(&app_id).err().map(|e| e.to_string())
            };
            let response = if let Some(reason) = violation {
                format!("ERROR: RateLimited: {reason}")
            } else {
                handle_restore(&app_id, &buf[1..], &audit).await
            };
            timing_noise().await;
            let response_bytes = response.as_bytes();
            let response_len = u32::try_from(response_bytes.len())
                .expect("response too large")
                .to_be_bytes();
            stream.write_all(&response_len).await?;
            stream.write_all(response_bytes).await?;
            continue;
        }

        // Meetings read mode: a leading 0x0C byte selects the meeting note store
        // read (agent-work-surfaces), the list/get counterpart to the 0x02
        // FileMeeting write. The body is a JSON `MeetingReadRequest`; gated to the
        // meetings app / AI engine and rate-limited like the other read ops. 0x0C
        // is the next free byte after 0x0B (restore).
        if buf.first() == Some(&0x0C) {
            let violation = {
                let mut rs = rate.lock().await;
                rs.limiter.check_query(&app_id).err().map(|e| e.to_string())
            };
            let response = if let Some(reason) = violation {
                format!("ERROR: RateLimited: {reason}")
            } else {
                handle_meetings_read(&app_id, &buf[1..], &graph).await
            };
            timing_noise().await;
            let response_bytes = response.as_bytes();
            let response_len = u32::try_from(response_bytes.len())
                .expect("response too large")
                .to_be_bytes();
            stream.write_all(&response_len).await?;
            stream.write_all(response_bytes).await?;
            continue;
        }

        // Read mode. A leading 0x01 byte selects the structured (typed JSON
        // RowSet) response; without it the request is a legacy raw-Cypher text
        // query, so existing clients are unaffected.
        let typed_rows = buf.first() == Some(&0x01);
        let cypher_bytes = if typed_rows { &buf[1..] } else { &buf[..] };
        let cypher = String::from_utf8(cypher_bytes.to_vec())?;

        debug!(cypher = %cypher, typed_rows, "received query");

        // Per-identity rate limit, before any work.
        let violation = {
            let mut rs = rate.lock().await;
            match rs.limiter.check_query(&app_id) {
                Ok(()) => None,
                Err(e) => Some((e.to_string(), rs.should_emit(&app_id))),
            }
        };

        // Per-caller read scope (RS-R1). A system-anchored caller (the AI daemon,
        // agent, terminal, Settings, Files - tier != ThirdParty, non-`unknown`)
        // bypasses the label gate; the system tier is non-same-uid-forgeable for the
        // root-owned identity rules 1-3 and, since F3 Rung B, inode-attested for
        // rule-4 user apps once installd has enrolled them (a same-uid copy to a
        // different path is rejected). An unenrolled rule-4 app stays cooperative
        // until it is recorded, and an in-place inode-preserving rewrite of the
        // user-owned binary is the documented A-2 residual only AppArmor closes
        // (`docs/architecture/identity-spoof-mitigation.md`). A non-system caller is
        // held to its readable label set, so its
        // readable labels are minted lazily here (only for that caller) via the
        // write path's PID-reuse guard; an unprovisioned or recycled peer mints the
        // empty scope (not an error), so a label-less no-op read still works while
        // any labelled read is denied - the correct fail-closed behaviour.
        let system_anchored = app_id != "unknown"
            && QuotaConfig::arlen_default().tier_for_app(&app_id) != AppTier::ThirdParty;
        let readable_labels: Vec<String> = if system_anchored {
            Vec::new()
        } else {
            caller_readable_labels(peer.as_ref(), &auth).await
        };

        // Failure responses are the plaintext `ERROR: ...` form in both
        // modes; a typed client detects the `ERROR:` prefix before parsing
        // JSON (the SDK does). Wrapping every typed failure in a structured
        // JSON envelope (stable code + message) so a typed client can tell
        // RateLimited from QueryTimeout is a follow-up; today the sole typed
        // consumer fails closed on any error, so the category is not needed.
        let (response, emit_violation) = if let Some((reason, emit)) = violation {
            warn!(app_id = %app_id, "graph query rate limit exceeded");
            (format!("ERROR: RateLimited: {reason}"), emit)
        } else if is_write_query(&cypher) {
            // Reject write queries (Phase 1A constraint, relaxed in 3.4).
            (
                "ERROR: write queries are not permitted via the query interface"
                    .to_string(),
                false,
            )
        } else if references_authority_label(&cypher) && !is_privileged_authority_reader(&app_id) {
            // The authority and provenance labels (Grant, CapabilityUse,
            // EntityType) are served only through the dedicated, caller-scoped
            // read op, never the general query path: a raw query like
            // `MATCH (g:Grant)-[:USED_BY]->(a:App) RETURN a.id, g.declared_ceiling`
            // would otherwise harvest the whole machine's authority-and-behaviour
            // map, the most sensitive data the system holds (living-capability-
            // graph.md §5, provenance-halo.md §5). This is the first per-query
            // read-scope enforcement on the daemon, scoped to exactly these
            // labels. The privileged whole-machine exemption is gated on F3
            // (installd's inode-keyed identity registry) and not yet granted, so
            // `is_privileged_authority_reader` denies every caller today.
            warn!(app_id = %app_id, "graph query referencing an authority label denied");
            (
                "ERROR: queries referencing authority labels are not permitted via the query interface"
                    .to_string(),
                false,
            )
        } else if let Some(denial) = raw_read_label_gate(&cypher, &readable_labels, system_anchored) {
            // RS-R1 per-caller read scope: a non-system caller may only read labels
            // in its readable set (a label-less or out-of-scope/sensitive read is
            // refused before execution). A real boundary for coarse labels and
            // defense-in-depth for the rest; sensitive content is served only by the
            // structured read op. The authority gate above is a strict superset
            // denial left as-is.
            warn!(app_id = %app_id, "graph read denied by the read-scope label gate");
            (format!("ERROR: {denial}"), false)
        } else {
            // Bounded *client wait*: the connection returns QueryTimeout
            // after 500 ms (foundation §8.4) so the caller is never
            // stuck. Because the graph enqueue now yields under
            // backpressure rather than blocking a worker (see graph.rs),
            // this deadline also covers queue admission. NB it unblocks
            // the caller; it does not abort the graph worker's in-flight
            // query, which runs to completion — a true execution deadline
            // needs an interruptible graph API and is a follow-up.
            // What this read exercises, captured before `cypher` is moved into the
            // query. Empty for a system-anchored caller, which bypasses the gate
            // and holds no read scope to exercise.
            let exercised = if system_anchored {
                Vec::new()
            } else {
                exercised_read_types(&cypher, &readable_labels)
            };
            let r = if typed_rows {
                // The Ladybug thread serialises the rows to JSON, so this
                // deadline bounds the query AND its serialisation together
                // (the text branch below serialises on that thread too).
                match tokio::time::timeout(
                    Duration::from_millis(500),
                    graph.query_rows_json(cypher),
                )
                .await
                {
                    // Defense-in-depth: scrub known-sensitive columns for a
                    // non-system caller (the label gate is the real boundary).
                    Ok(Ok(json)) if !system_anchored => scrub_sensitive_columns_json(&json),
                    Ok(Ok(json)) => json,
                    Ok(Err(e)) => format!("ERROR: {e}"),
                    Err(_elapsed) => "ERROR: QueryTimeout".to_string(),
                }
            } else {
                match tokio::time::timeout(Duration::from_millis(500), graph.query(cypher)).await {
                    Ok(Ok(result)) => result,
                    Ok(Err(e)) => format!("ERROR: {e}"),
                    Err(_elapsed) => "ERROR: QueryTimeout".to_string(),
                }
            };
            // Successful exercise only: a query that timed out or errored did not
            // use the grant. Best-effort, and never in the way of the reply.
            if !r.starts_with("ERROR:") {
                record_capability_uses(&graph, &uses, &app_id, &exercised).await;
            }
            (r, false)
        };

        // Best-effort violation telemetry for the Anomaly Detector,
        // scheduled BEFORE the reply. `spawn_blocking` returns
        // immediately (the blocking Event Bus socket I/O runs on the
        // blocking pool, so the reply is never delayed), and scheduling
        // it here means a client that disconnects mid-write cannot
        // suppress the signal whose throttle slot it already consumed.
        if emit_violation {
            let emitter = emitter.clone();
            let app_id = app_id.clone();
            tokio::task::spawn_blocking(move || emitter.emit(&app_id));
        }

        // Small random delay before replying, so two equivalent
        // queries are not timing-distinguishable.
        timing_noise().await;

        // Write response.
        let response_bytes = response.as_bytes();
        let response_len = u32::try_from(response_bytes.len())
            .expect("response too large")
            .to_be_bytes();

        stream.write_all(&response_len).await?;
        stream.write_all(response_bytes).await?;
    }
}

/// Check if a Cypher query contains write operations.
///
/// A write clause can appear anywhere, not only at the start (`MATCH (n)
/// DELETE n`, `MATCH (a) MERGE (b)`), so this scans whole-word tokens rather
/// than the leading keyword, skipping single-quoted string literals so a
/// value that merely contains a keyword (e.g. a path with `DELETE` in it) is
/// not mistaken for a write. It over-rejects rather than under-rejects (a
/// read whose identifier collides with a keyword is refused, never a write
/// let through), which is the safe direction for a read-only socket.
///
/// The blocklist covers not only the graph-mutation clauses but the
/// data-definition, attach, copy, and extension verbs, because on this socket
/// those are the dangerous ones: `LOAD EXTENSION`/`INSTALL` run native code,
/// `COPY` does filesystem I/O, `ATTACH`/`USE` reach another database, and
/// `ALTER`/`EXPORT`/`IMPORT` change or dump the schema/data. None appear in the
/// agent's read queries (`MATCH`/`WHERE`/`WITH`/`RETURN`/`ORDER`/`LIMIT`).
///
/// This is a lexical guard, not a parser. Engine-level read-only enforcement
/// (a read-only `lbug` connection for the read path) was investigated and is
/// NOT viable with the current engine: a read-only `Database` handle is a
/// snapshot at open time and does not observe writes committed through the
/// read-write handle, so the agent would read stale data (its own writes
/// invisible), and opening a fresh handle per query is far too costly. Until
/// the engine exposes a per-statement / per-transaction read-only flag, this
/// expanded blocklist is the ceiling; it closes the known privilege-escalation
/// verbs while keeping the over-reject-not-under-reject safety direction.
fn is_write_query(cypher: &str) -> bool {
    // CALL drives Kuzu's runtime config (`CALL threads=N`), metadata, and
    // extension procedures, and FOREACH iterates writes; both can mutate or
    // reconfigure the database, so they are write barriers on the read path
    // alongside the mutation verbs. This matches ai-core's own cypher
    // blocklist, and no read consumer issues either on the query socket.
    const WRITE_KEYWORDS: [&str; 17] = [
        "CREATE", "MERGE", "DELETE", "SET", "REMOVE", "DROP", "DETACH", "ALTER",
        "ATTACH", "USE", "COPY", "LOAD", "INSTALL", "EXPORT", "IMPORT", "CALL",
        "FOREACH",
    ];
    cypher_references_any(cypher, &WRITE_KEYWORDS)
}

/// The graph labels whose projection is the machine's authority-and-behaviour
/// map (living-capability-graph.md §5, provenance-halo.md §5). The general query
/// path denies any reference to them; they are served only through the dedicated
/// caller-scoped read op, so they can never be harvested wholesale.
const AUTHORITY_LABELS: [&str; 3] = ["GRANT", "CAPABILITYUSE", "ENTITYTYPE"];

/// Whether `cypher` references one of the protected authority labels. Fail-closed:
/// an unusual identifier matching a label name (case-insensitively) is denied
/// rather than risk a leak, which is safe because these are reserved
/// authority-label identifiers a normal query never names.
fn references_authority_label(cypher: &str) -> bool {
    cypher_references_any(cypher, &AUTHORITY_LABELS)
}

/// Whether `app_id` may reference the authority labels on the general query path
/// (the privileged whole-machine reader). Gated on F3 (installd's inode-keyed
/// identity registry): until a caller's identity is hardware/installer-attested,
/// a same-uid process could squat the privileged app id and become a
/// whole-machine authority harvester, so no caller is privileged yet and the
/// general path denies the labels for everyone. When F3 lands, the privileged
/// Knowledge/Settings principal held to a dedicated capability is admitted here.
///
/// This is what SX-5 (the unified apps + modules + bridges management view) is
/// waiting on, and it is worth knowing that the DATA side of SX-5 already exists:
/// modules get a `PermissionProfile` at install (`installd::module_permissions`),
/// bridges get their write capability through a profile's delegated namespaces,
/// and the shell's profile watcher fires `permission.changed` for any file in the
/// permissions directory regardless of what kind of principal it belongs to - so
/// `emit_all_declared_grants` already projects all three into the LCG. The
/// missing piece is not an aggregation across three daemons; it is one read of
/// every principal's grants, which IS the whole-machine authority view this gate
/// denies. Open the gate and the query is small.
fn is_privileged_authority_reader(_app_id: &str) -> bool {
    false
}

/// Token-scan `cypher` for any of `needles` (compared uppercased), skipping
/// single-quoted string literals so a keyword or label name inside a string value
/// is a value, not a clause. The shared scan behind the write-query and
/// authority-label read guards.
fn cypher_references_any(cypher: &str, needles: &[&str]) -> bool {
    let mut in_string = false;
    let mut escaped = false;
    let mut token = String::new();
    for ch in cypher.chars() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '\'' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '\'' => in_string = true,
            c if c.is_ascii_alphanumeric() || c == '_' => token.push(c.to_ascii_uppercase()),
            _ => {
                if needles.contains(&token.as_str()) {
                    return true;
                }
                token.clear();
            }
        }
    }
    needles.contains(&token.as_str())
}

/// The labels whose content is sensitive enough that the raw-Cypher read path can
/// never serve them to a non-system caller: a name-keyed field filter cannot
/// soundly scope them (the read-scope notes' bypass classes B1-B7 - label-less
/// `MATCH (n)`, traversal, whole-node `RETURN n`, `properties()`/alias laundering,
/// aggregation oracles), so they are denied here and served only through the
/// structured read op (RS-R2). Compared uppercased, like [`AUTHORITY_LABELS`].
///
/// Empty today: the observation graph holds no command-history / clipboard /
/// credential / PII node label yet (the authority labels are denied separately by
/// [`references_authority_label`]); when the terminal/clipboard KG-promotion writes
/// such a label, add it here and RS-R2 becomes its only reader. The allowlist
/// pre-gate ([`raw_read_label_gate`]) is the active boundary in the meantime.
const SENSITIVE_RAW_LABELS: [&str; 0] = [];

/// Relationship types that carry more than their endpoints do, and so need an
/// explicit grant rather than following from the labels they connect.
///
/// **Empty, and honestly so.** The list is for an edge that says something the
/// endpoints do not - one tying a person to a document, or one person to another.
/// The observation graph has no `Person` node today, and every edge it does have
/// (`FILE_PART_OF`, `ACCESSED_BY`, `ACTIVE_IN`, `DERIVED_FROM`) relates things a
/// caller reading both ends can already see. The authority edges are not here
/// because they are already refused a step earlier, by the label deny on `Grant`,
/// `CapabilityUse` and `EntityType`.
///
/// Adding an entry is cheap; the standard it has to meet is that someone reading
/// this file can see why that edge carries more than its ends.
const RESTRICTED_RELATIONS: [&str; 0] = [];

/// The uppercased label/relationship-type tokens a Cypher query references: every
/// identifier that immediately follows a `:` (a node label `(n:File)` or a rel type
/// `-[:FILE_PART_OF]->`), skipping single-quoted string literals so a name inside a
/// value is not mistaken for a label. A token must start with a letter, so a map
/// literal's numeric value (`{count:5}`) is not captured. Used by the read-scope
/// pre-gate to check every referenced label against the caller's readable set.
fn cypher_label_tokens(cypher: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut in_string = false;
    let mut escaped = false;
    let mut token = String::new();
    let mut token_after_colon = false;
    let mut prev_colon = false;
    // Inside a map literal a colon separates a key from its value, and neither is
    // a label - Cypher has no way to write one there. Without this, an unquoted
    // alpha value hard against the colon is collected: `{app_id:true}` produced
    // the token `TRUE`, so the gate refused a query for naming a label nobody
    // wrote, while the quoted `{app_id:'x'}` right beside it passed. That refuses
    // the more precise form and lets the vaguer one through, which teaches
    // callers to narrow less. A map key was never a label.
    let mut map_depth = 0usize;
    let flush = |token: &mut String, after_colon: bool, out: &mut Vec<String>| {
        if after_colon
            && token
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic())
        {
            out.push(token.clone());
        }
        token.clear();
    };
    for ch in cypher.chars() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '\'' {
                in_string = false;
            }
            continue;
        }
        if ch.is_ascii_alphanumeric() || ch == '_' {
            if token.is_empty() {
                token_after_colon = prev_colon && map_depth == 0;
            }
            token.push(ch.to_ascii_uppercase());
        } else {
            flush(&mut token, token_after_colon, &mut out);
            prev_colon = ch == ':';
            if ch == '{' {
                map_depth += 1;
            } else if ch == '}' {
                map_depth = map_depth.saturating_sub(1);
            } else if ch == '\'' {
                in_string = true;
            }
        }
    }
    flush(&mut token, token_after_colon, &mut out);
    out
}

/// Whether every node pattern names a label and every relationship names a type.
///
/// **An omitted constraint is the widest constraint, not an absent one.** An
/// unlabelled `(x)` means "a node of ANY label" and an untyped `-[r]->` means "an
/// edge of ANY type, to anywhere". The label allowlist is a check on what a query
/// NAMES, so leaving the name out walked straight past it: measured on this
/// daemon, `MATCH (f:File)-[r]->(x)` produced the single token `FILE` and was
/// allowed for a caller scoped to `File` alone, which is every node adjacent to a
/// file. Naming the edge got you refused; omitting it got you through. Endpoint
/// authorisation only means something when the endpoints are constrained.
///
/// A node pattern is `(` NOT preceded by an identifier - `count(*)` and
/// `toLower(n.x)` are calls, not patterns, and must not be read as unlabelled
/// nodes. A relationship is `[` preceded by `-`, which is the only way Cypher
/// spells one; a bare `[` is a list literal (`WHERE n.id IN ['a']`).
///
/// Returns the offending construct's name for the refusal, so the message can say
/// which one rather than failing obscurely.
struct PatternScan {
    /// The construct that named no constraint, if any.
    unconstrained: Option<&'static str>,
    /// Uppercased labels named on node patterns.
    node_labels: Vec<String>,
    /// Uppercased types named on relationship patterns.
    rel_types: Vec<String>,
}

fn scan_patterns(cypher: &str) -> PatternScan {
    let chars: Vec<char> = cypher.chars().collect();
    let mut in_string = false;
    let mut escaped = false;
    // (kind, saw_colon) for each open bracket that is a PATTERN, innermost last.
    let mut open: Vec<(&'static str, bool)> = Vec::new();
    let mut prev_significant: Option<char> = None;
    let mut prev_char: Option<char> = None;
    let mut unconstrained: Option<&'static str> = None;
    let mut node_labels: Vec<String> = Vec::new();
    let mut rel_types: Vec<String> = Vec::new();
    // An identifier being collected because a `:` opened it, and the bracket kind
    // it belongs to. A token must start with a letter, so a map literal's numeric
    // value (`{count:5}`) is not read as a name.
    let mut pending: Option<(String, &'static str)> = None;

    for (i, &ch) in chars.iter().enumerate() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '\'' {
                in_string = false;
            }
            continue;
        }
        if pending.is_some() && !(ch.is_ascii_alphanumeric() || ch == '_') {
            let (tok, kind) = pending.take().expect("checked above");
            if tok.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) {
                match kind {
                    "relationship" => rel_types.push(tok),
                    "node" => node_labels.push(tok),
                    // A map key or a list/call element names nothing the gate
                    // scopes, so it is neither a label nor an edge type.
                    _ => {}
                }
            }
        } else if let Some((tok, _)) = pending.as_mut() {
            tok.push(ch.to_ascii_uppercase());
            continue;
        }
        match ch {
            '\'' => in_string = true,
            '(' => {
                // The IMMEDIATELY preceding character, whitespace included: a call
                // is `count(` with nothing between, while `MATCH (` has a space.
                // Skipping whitespace here read every pattern after a keyword as a
                // call, so no node label was collected at all - and the tests
                // still passed, because each query they refused was refused by the
                // unlabelled node later in it. A verdict can be right for the
                // wrong reason; the assertions now name the labels.
                let is_call = prev_char
                    .is_some_and(|c: char| c.is_ascii_alphanumeric() || c == '_');
                if !is_call {
                    open.push(("node", false));
                } else {
                    open.push(("call", true));
                }
            }
            '[' => {
                if prev_significant == Some('-') {
                    open.push(("relationship", false));
                } else {
                    open.push(("list", true));
                }
            }
            // A property map inside a pattern: `(f:File {app_id:'x'})`. Its keys
            // follow a `:` exactly as a label does, and reading them as labels
            // refused the more PRECISE query while the vague one passed - an
            // allowlist that rewards saying less. Opened as satisfied so it never
            // reports unconstrained; its tokens are dropped below.
            '{' => open.push(("map", true)),
            ')' | ']' | '}' => {
                if let Some((kind, saw_colon)) = open.pop() {
                    if !saw_colon && (kind == "node" || kind == "relationship") {
                        unconstrained = unconstrained.or(Some(kind));
                    }
                }
            }
            ':' => {
                if let Some(last) = open.last_mut() {
                    last.1 = true;
                    pending = Some((String::new(), last.0));
                }
            }
            _ => {}
        }
        let _ = i;
        prev_char = Some(ch);
        if !ch.is_whitespace() {
            prev_significant = Some(ch);
        }
    }
    if let Some((tok, kind)) = pending {
        if tok.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) {
            match kind {
                "relationship" => rel_types.push(tok),
                "node" => node_labels.push(tok),
                _ => {}
            }
        }
    }
    PatternScan { unconstrained, node_labels, rel_types }
}

/// Whether `cypher` references a label outside the caller's readable label set, OR
/// a sensitive label a non-system caller may never read on the raw path. Returns
/// the denial reason, or `None` to allow.
///
/// Fail-closed by design (the notes' over-reject stance): a `system_anchored` caller
/// bypasses the allowlist entirely (the system tier is non-same-uid-forgeable for
/// the root-owned identity rules 1-3, cooperative-only for rule-4 user apps until
/// F3 - the same boundary-vs-cooperative split [`is_privileged_authority_reader`]
/// documents). A non-system caller is denied a label-less query (nothing to scope,
/// B1), any sensitive label (RS-R2 only), and any label/rel-type outside its
/// readable set (so a traversal to an out-of-scope neighbour, B2, is refused). Only
/// a query whose every referenced label is in the readable, non-sensitive set runs.
/// The read scopes a successful query actually exercised, as the namespaced type
/// names the grant projection uses (`system.File`), so a count here joins the
/// `GRANTS` -> `EntityType` reach on the card.
///
/// Rebuilding the type by putting `system.` back in front of the label is only
/// sound HERE, and it is worth saying why, because the same move is a bug on the
/// reading side. `readable_system_labels` accepts a scope only if it starts with
/// `system.`, and makes the label by stripping exactly that, so every label in a
/// readable set has a `system.` type behind it and no other. The `access_grants`
/// overlay cannot assume that - a grant may reach `md.obsidian.Note`, whose label
/// is `Note` - so it carries the full type instead of reconstructing it.
///
/// The GRANTED casing is bound, never the caller's: the gate matches labels
/// case-insensitively, so a caller asking for `file` must not mint a second
/// capability row distinct from the `system.File` it was granted. Same rule the
/// structured read op follows when it interpolates a label.
///
/// Only meaningful for a gated caller. A system-anchored one bypasses the gate
/// and mints no read scope at all, so it is not exercising a grant and the caller
/// does not ask.
fn exercised_read_types(cypher: &str, readable_labels: &[String]) -> Vec<String> {
    let mut out: Vec<String> = cypher_label_tokens(cypher)
        .iter()
        .filter_map(|t| {
            readable_labels
                .iter()
                .find(|l| l.eq_ignore_ascii_case(t))
                .map(|granted| format!("system.{granted}"))
        })
        .collect();
    out.sort();
    out.dedup();
    out
}

fn raw_read_label_gate(
    cypher: &str,
    readable_labels: &[String],
    system_anchored: bool,
) -> Option<&'static str> {
    if system_anchored {
        return None;
    }
    // A backtick quotes an identifier (label, rel type, property) that the token
    // scanner cannot positively account for: `cypher_label_tokens` resets on the
    // backtick, so a backtick-quoted label `(b:`Session`)` is invisible to the
    // allowlist and would fall open. The scanner can never be a sound parser, so
    // an unmodellable construct is treated as denial (the over-reject stance).
    // Arlen labels are plain identifiers, so a scoped read never needs a backtick.
    if cypher.contains('`') {
        return Some("read denied: backtick-quoted identifiers cannot be scoped to the caller");
    }
    // An omitted constraint is the widest one. A scoped caller must say which
    // labels and which edge types it means, because "any" is not a scope it holds.
    let scan = scan_patterns(cypher);
    match scan.unconstrained {
        Some("node") => {
            return Some(
                "read denied: every node in the pattern must name a label; an unlabelled node means any label",
            )
        }
        Some("relationship") => {
            return Some(
                "read denied: every relationship must name a type; an untyped edge means any edge",
            )
        }
        Some(_) | None => {}
    }
    if scan.node_labels.is_empty() {
        return Some("read denied: a label-less query cannot be scoped to the caller");
    }
    let readable = |t: &String| readable_labels.iter().any(|l| l.eq_ignore_ascii_case(t));
    for t in &scan.node_labels {
        if SENSITIVE_RAW_LABELS.contains(&t.as_str()) {
            return Some("read denied: sensitive label served only through the structured read op");
        }
        if !readable(t) {
            return Some("read denied: label outside the caller's read scope");
        }
    }
    // A traversal is authorised by its ENDPOINTS: both node labels were just
    // checked, so the edge between them adds little a caller could not infer from
    // the properties and paths it may already read. Enumerating every type in a
    // profile buys a longer grant, not a tighter one - it becomes "list them all"
    // through ceremony. Named types on the restricted list are the exception and
    // need the explicit grant.
    for t in &scan.rel_types {
        if RESTRICTED_RELATIONS.contains(&t.as_str()) && !readable(t) {
            return Some("read denied: this relationship type needs an explicit grant");
        }
    }
    None
}

/// Null every cell of a typed `RowSet` JSON (`{columns, rows}`) whose column names a
/// sensitive field. The column name's last dotted segment is compared (so
/// `n.email`/`p.email` match `email`), case-insensitively, against
/// [`crate::shared::all_sensitive_field_names`].
///
/// This is a defense-in-depth scrub on the raw read path, NOT a boundary: a query
/// that aliases or transforms a sensitive field past its column name escapes it
/// (the notes' B4). The boundary is the label pre-gate (which denies a sensitive or
/// out-of-scope label outright for a non-system caller, so the row never returns)
/// and the structured read op. Best-effort: a non-`RowSet` or an `ERROR:` string is
/// returned unchanged. Applied only for callers without sensitive read scope.
fn scrub_sensitive_columns_json(json: &str) -> String {
    let Ok(mut value) = serde_json::from_str::<serde_json::Value>(json) else {
        return json.to_string();
    };
    let Some(columns) = value.get("columns").and_then(|c| c.as_array()) else {
        return json.to_string();
    };
    let sensitive = crate::shared::all_sensitive_field_names();
    let sensitive_idx: Vec<usize> = columns
        .iter()
        .enumerate()
        .filter_map(|(i, col)| {
            let name = col.as_str()?;
            let leaf = name.rsplit('.').next().unwrap_or(name);
            sensitive
                .iter()
                .any(|s| leaf.eq_ignore_ascii_case(s))
                .then_some(i)
        })
        .collect();
    if sensitive_idx.is_empty() {
        return json.to_string();
    }
    if let Some(rows) = value
        .as_object_mut()
        .and_then(|o| o.get_mut("rows"))
        .and_then(|r| r.as_array_mut())
    {
        for row in rows.iter_mut() {
            if let Some(cells) = row.as_array_mut() {
                for &i in &sensitive_idx {
                    if let Some(cell) = cells.get_mut(i) {
                        *cell = serde_json::Value::Null;
                    }
                }
            }
        }
    }
    value.to_string()
}

/// The caller's readable system labels, minted lazily from a live, unchanged peer
/// process via the write path's PID-reuse guard. An unprovisioned or recycled peer
/// (or one with no graph access) yields the empty set, NOT an error - the
/// fail-closed default, so a caller with no scope can read nothing labelled.
async fn caller_readable_labels(
    peer: Option<&WritePeer>,
    auth: &Arc<Mutex<Authenticator>>,
) -> Vec<String> {
    match peer {
        Some(p) => match (p.start_time, pid_start_time(p.pid).ok()) {
            (Some(captured), Some(now)) if now == captured => {
                match auth.lock().await.issue_token_for_pid(p.uid, p.pid) {
                    Ok(token) => readable_system_labels(&token.read_scopes),
                    Err(_) => Vec::new(),
                }
            }
            _ => Vec::new(),
        },
        None => Vec::new(),
    }
}

/// Keep only the `ids` that exist under one of the caller's `readable_labels`,
/// preserving the input ranking order (RS-R1 for the `0x03` retrieve op). One
/// batched probe per readable label (`MATCH (n:Label) WHERE n.id IN [...]`), so the
/// cost is the readable-label count, not the id count; the ids are bounded by the
/// retrieve limit and escaped, the labels are safe identifiers. An empty readable
/// set drops everything (fail-closed). Unlike the raw path this IS a real boundary
/// for `0x03`: it returns only ids, so there is no field-laundering bypass.
async fn filter_ids_to_readable_labels(
    graph: &GraphHandle,
    ids: &[String],
    readable_labels: &[String],
    tally: Option<(&Arc<Mutex<crate::lcg::UseTally>>, &str)>,
) -> Vec<String> {
    if ids.is_empty() || readable_labels.is_empty() {
        return Vec::new();
    }
    let in_list = crate::cypher::id_list(ids.iter());
    let mut allowed = std::collections::HashSet::new();
    // The labels that actually resolved something. A label that matched nothing
    // was probed but not exercised: the caller learned no fact through it, so
    // counting it would mark a grant used on the strength of an empty answer.
    let mut exercised: Vec<String> = Vec::new();
    for label in readable_labels {
        let cypher = format!("MATCH (n:{label}) WHERE n.id IN [{in_list}] RETURN n.id");
        if let Ok(rs) = graph.query_rows(cypher).await {
            let mut matched = false;
            for row in &rs.rows {
                if let Some(cell) = row.first() {
                    allowed.insert(cell.as_str().to_string());
                    matched = true;
                }
            }
            if matched {
                exercised.push(format!("system.{label}"));
            }
        }
    }
    if let Some((uses, app_id)) = tally {
        record_capability_uses(graph, uses, app_id, &exercised).await;
    }
    ids.iter().filter(|id| allowed.contains(*id)).cloned().collect()
}

/// Tally successful grant exercises and flush them if the interval has passed.
///
/// One place, because every op that scopes a read reaches it: the raw-read path
/// captures its own labels, and the id-filtering ops (retrieve, working set,
/// prep) come through `filter_ids_to_readable_labels`. Best-effort throughout -
/// the tally is a projection, so a graph hiccup must never change what the caller
/// gets back.
async fn record_capability_uses(
    graph: &GraphHandle,
    uses: &Arc<Mutex<crate::lcg::UseTally>>,
    app_id: &str,
    exercised: &[String],
) {
    if exercised.is_empty() {
        return;
    }
    let now = crate::time::now().0;
    // The lock is dropped before the flush is awaited: a slow graph write must
    // not hold the counter against every other connection.
    let batch = {
        let mut tally = uses.lock().await;
        let mut due = false;
        for capability in exercised {
            due |= tally.record(app_id, capability, now);
        }
        due.then(|| tally.drain(now))
    };
    if let Some(batch) = batch {
        if let Err(e) = crate::lcg::flush_capability_use(graph, &batch).await {
            warn!("capability-use flush failed (effective-use projection stale): {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every socket op must own its mode byte.
    ///
    /// The ops are dispatched by a long `if buf.first() == Some(&0xNN)` chain,
    /// so a duplicate byte does not conflict - the FIRST arm simply wins and the
    /// later op becomes unreachable, silently. That happened: code-analysis
    /// (CG-R5) was given 0x09 with a comment calling it "the next free byte",
    /// but prep-for-this already held 0x09 higher up, so every code-analysis
    /// request was swallowed by prep and the op could never run - including
    /// through the knowledge-mcp tool that exposes it to the AI.
    ///
    /// Scanning the source is deliberate: the dispatch is an if-chain, not a
    /// table, so there is no runtime value to assert over. This catches the
    /// duplicate at the only place it is expressible.
    #[test]
    fn mode_bytes_are_uniquely_assigned() {
        use std::collections::BTreeMap;
        let src = include_str!("daemon.rs");
        let mut seen: BTreeMap<String, usize> = BTreeMap::new();
        for line in src.lines() {
            let Some(rest) = line.split_once("buf.first() == Some(&0x") else {
                continue;
            };
            let byte: String = rest
                .1
                .chars()
                .take_while(|c| c.is_ascii_hexdigit())
                .collect::<String>()
                .to_uppercase();
            if !byte.is_empty() {
                *seen.entry(byte).or_default() += 1;
            }
        }
        assert!(!seen.is_empty(), "the scan found no dispatch arms at all");
        let dupes: Vec<_> = seen.iter().filter(|(_, n)| **n > 1).collect();
        assert!(
            dupes.is_empty(),
            "these mode bytes are claimed by more than one op, so the later one is \
             unreachable: {dupes:?}"
        );
    }

    #[test]
    fn cross_uid_admits_only_first_party_and_system() {
        // No owner configured (single-user default): serve any first-party/system
        // cross-uid peer (the local AI layer); reject ThirdParty/unknown.
        assert!(cross_uid_admitted(1000, None, AppTier::FirstParty));
        assert!(cross_uid_admitted(1000, None, AppTier::System));
        assert!(!cross_uid_admitted(1000, None, AppTier::ThirdParty));
    }

    #[test]
    fn cross_uid_owner_restriction_pins_the_session_user() {
        // Owner configured (multi-user host): serve a first-party/system peer only
        // if it is the owner; a different uid is rejected even at FirstParty, and
        // ThirdParty is rejected regardless.
        assert!(cross_uid_admitted(1000, Some(1000), AppTier::FirstParty));
        assert!(cross_uid_admitted(1000, Some(1000), AppTier::System));
        assert!(!cross_uid_admitted(1001, Some(1000), AppTier::FirstParty));
        assert!(!cross_uid_admitted(1000, Some(1000), AppTier::ThirdParty));
    }

    #[test]
    fn label_tokens_extracts_labels_and_rel_types() {
        assert_eq!(cypher_label_tokens("MATCH (f:File) RETURN f.path"), ["FILE"]);
        assert_eq!(
            cypher_label_tokens("MATCH (f:File)-[:FILE_PART_OF]->(p:Project) RETURN p"),
            ["FILE", "FILE_PART_OF", "PROJECT"]
        );
        // No label: label-less MATCH yields nothing.
        assert!(cypher_label_tokens("MATCH (n) RETURN n").is_empty());
        // A colon inside a string literal is not a label.
        assert!(cypher_label_tokens("MATCH (n) WHERE n.x = 'a:b' RETURN n").is_empty());
        // A map-literal numeric value (`{c:5}`) is not a label (must start alpha).
        assert!(cypher_label_tokens("MATCH (n {c:5}) RETURN n").is_empty());
        // Nor is an ALPHA one. `{app_id:true}` used to yield `TRUE` and get the
        // query refused for a label nobody wrote, while the quoted `{app_id:'x'}`
        // beside it passed - refusing the more precise form and admitting the
        // vaguer. Cypher cannot write a label inside a map, so nothing in there is
        // one, whatever it starts with.
        assert!(cypher_label_tokens("MATCH (n {app_id:true}) RETURN n").is_empty());
        assert!(cypher_label_tokens("MATCH (n {app_id:x}) RETURN n").is_empty());
        assert_eq!(
            cypher_label_tokens("MATCH (f:File {app_id:x}) RETURN f.id"),
            ["FILE"]
        );
        // The label still lands when the pattern follows a closed map.
        assert_eq!(
            cypher_label_tokens("MATCH (f:File {a:1})-[:FILE_PART_OF]->(p:Project) RETURN p"),
            ["FILE", "FILE_PART_OF", "PROJECT"]
        );
    }

    #[test]
    fn read_gate_denies_label_less_query() {
        assert!(raw_read_label_gate("MATCH (n) RETURN n", &[], false).is_some());
    }

    #[test]
    fn read_gate_allows_an_in_scope_label() {
        assert!(raw_read_label_gate("MATCH (f:File) RETURN f.path", &["File".into()], false).is_none());
    }

    #[test]
    fn read_gate_denies_an_out_of_scope_label() {
        assert!(
            raw_read_label_gate("MATCH (s:Session) RETURN s.id", &["File".into()], false).is_some()
        );
    }

    #[test]
    fn read_gate_system_anchored_caller_bypasses() {
        // The same out-of-scope query is allowed for a system-anchored caller.
        assert!(
            raw_read_label_gate("MATCH (s:Session) RETURN s.id", &["File".into()], true).is_none()
        );
    }

    #[test]
    fn read_gate_denies_a_backtick_quoted_label() {
        // A backtick-quoted label is invisible to the token scanner, so it
        // would otherwise smuggle an out-of-scope label past the allowlist.
        // The raw read path denies any backtick for a non-system caller, even
        // when a readable label is also present.
        assert!(raw_read_label_gate(
            "MATCH (f:File)-[:FILE_PART_OF]->(b:`Session`) RETURN b",
            &["File".into()],
            false
        )
        .is_some());
        assert!(
            raw_read_label_gate("MATCH (b:`File`) RETURN b", &["File".into()], false).is_some()
        );
        // A system-anchored caller still bypasses the whole gate.
        assert!(
            raw_read_label_gate("MATCH (b:`Session`) RETURN b", &["File".into()], true).is_none()
        );
    }

    #[test]
    fn scrub_nulls_sensitive_columns_keeps_others() {
        let json = r#"{"columns":["p.email","p.name","phone"],"rows":[["a@b.com","Alice","555"],["c@d.com","Bob","666"]]}"#;
        let scrubbed = scrub_sensitive_columns_json(json);
        let v: serde_json::Value = serde_json::from_str(&scrubbed).unwrap();
        let rows = v["rows"].as_array().unwrap();
        // email (col 0) and phone (col 2) nulled; name (col 1) kept.
        assert!(rows[0][0].is_null());
        assert_eq!(rows[0][1], "Alice");
        assert!(rows[0][2].is_null());
        assert!(rows[1][0].is_null());
        assert_eq!(rows[1][1], "Bob");
    }

    #[test]
    fn scrub_passes_through_non_rowset_and_errors() {
        assert_eq!(scrub_sensitive_columns_json("ERROR: QueryTimeout"), "ERROR: QueryTimeout");
        // No sensitive column: unchanged.
        let plain = r#"{"columns":["f.path"],"rows":[["/x"]]}"#;
        let out = scrub_sensitive_columns_json(plain);
        assert_eq!(serde_json::from_str::<serde_json::Value>(&out).unwrap()["rows"][0][0], "/x");
    }

    #[test]
    fn read_gate_denies_a_traversal_to_an_unreadable_type() {
        // File is readable, but the rel type and Project are not: fail-closed.
        assert!(raw_read_label_gate(
            "MATCH (f:File)-[:FILE_PART_OF]->(p:Project) RETURN p",
            &["File".into()],
            false
        )
        .is_some());
    }

    #[test]
    fn detects_write_queries() {
        assert!(is_write_query("CREATE (n:File)"));
        assert!(is_write_query("MERGE (n:App)"));
        assert!(is_write_query("DELETE n"));
        assert!(is_write_query("SET n.name = 'x'"));
        assert!(is_write_query("  create (n)"));
    }

    #[test]
    fn detects_writes_that_do_not_start_with_the_keyword() {
        // The leading-token check missed these; the token scan catches them.
        assert!(is_write_query("MATCH (n:File) DELETE n"));
        assert!(is_write_query("MATCH (a:App) MERGE (b:Session)"));
        assert!(is_write_query("MATCH (n) SET n.name = 'x' RETURN n"));
        assert!(is_write_query("MATCH (n) DETACH DELETE n"));
    }

    #[test]
    fn rejects_dangerous_non_mutation_verbs() {
        // Code execution, file I/O, cross-database, and schema/dump verbs are
        // refused on the read socket even though they are not graph mutations.
        assert!(is_write_query("LOAD EXTENSION 'evil.so'"));
        assert!(is_write_query("INSTALL httpfs"));
        assert!(is_write_query("COPY File TO '/tmp/exfil.csv'"));
        assert!(is_write_query("ATTACH '/other/db' AS x (dbtype kuzu)"));
        assert!(is_write_query("ALTER TABLE File ADD col STRING"));
        assert!(is_write_query("USE other_db"));
        assert!(is_write_query("EXPORT DATABASE '/tmp/dump'"));
        assert!(is_write_query("IMPORT DATABASE '/tmp/dump'"));
        // CALL reconfigures/runs procedures, FOREACH iterates writes.
        assert!(is_write_query("CALL threads=4"));
        assert!(is_write_query("MATCH (n) CALL show_tables() RETURN *"));
        assert!(is_write_query(
            "MATCH (n) FOREACH (x IN [1] | SET n.y = x)"
        ));
    }

    #[test]
    fn allows_read_queries() {
        assert!(!is_write_query("MATCH (n:File) RETURN n"));
        assert!(!is_write_query("MATCH (a:App) WHERE a.id = 'x' RETURN a.name"));
        // A write/admin keyword inside a string literal is a value, not a clause.
        assert!(!is_write_query(
            "MATCH (f:File) WHERE f.path = '/home/tim/DELETE/x' RETURN f.id"
        ));
        assert!(!is_write_query(
            "MATCH (f:File) WHERE f.path = '/var/COPY/load' RETURN f.id"
        ));
    }

    #[test]
    fn detects_authority_label_references() {
        assert!(references_authority_label("MATCH (g:Grant) RETURN g"));
        assert!(references_authority_label(
            "MATCH (g:Grant)-[:USED_BY]->(a:App) RETURN a.id, g.declared_ceiling"
        ));
        assert!(references_authority_label("MATCH (c:CapabilityUse) RETURN c"));
        assert!(references_authority_label("MATCH (e:EntityType) RETURN e"));
        // Fail-closed across case (a label is case-sensitive, but denying any
        // case variant is safe and robust).
        assert!(references_authority_label("MATCH (g:grant) RETURN g"));
    }

    #[test]
    fn allows_non_authority_queries() {
        assert!(!references_authority_label("MATCH (n:File) RETURN n"));
        assert!(!references_authority_label("MATCH (a:App) RETURN a.id"));
        // A property whose name merely contains a label word is a distinct token.
        assert!(!references_authority_label("MATCH (a:App) RETURN a.grant_total"));
        // A label name inside a string literal is a value, not a label.
        assert!(!references_authority_label(
            "MATCH (f:File) WHERE f.path = '/home/tim/Grant/x' RETURN f.id"
        ));
    }

    #[test]
    fn no_caller_is_a_privileged_authority_reader_pre_f3() {
        // The privileged whole-machine exemption is gated on F3; until it lands,
        // the general query path denies the authority labels for every caller.
        assert!(!is_privileged_authority_reader("desktop-shell"));
        assert!(!is_privileged_authority_reader("knowledge-app"));
        assert!(!is_privileged_authority_reader("unknown"));
    }

    #[test]
    fn settings_browses_whole_system_grants_but_not_the_general_authority_path() {
        // The Settings management principal sees whole-system grants (the App-access
        // capability browser) through the curated access_grants op.
        assert!(is_settings_principal("dev.arlen.settings"));
        // Every ordinary caller sees only its own grants (scoped by attested id).
        for other in ["desktop-shell", "ai-agent", "com.x", "unknown", ""] {
            assert!(!is_settings_principal(other), "{other} is not the Settings principal");
        }
        // Admitting Settings to the grant browse must NOT lift the general read
        // path's authority-label deny for it: is_privileged_authority_reader stays
        // false, so Settings reaches Grant/CapabilityUse/EntityType only through the
        // curated access_grants/revoke ops, never arbitrary Cypher.
        assert!(!is_privileged_authority_reader("dev.arlen.settings"));
    }

    #[test]
    fn an_exercised_read_binds_the_granted_casing_not_the_callers() {
        // The gate matches case-insensitively, so `file` passes against a
        // `File` grant. The counter must still name the granted capability, or
        // one grant would accrue two rows nothing can join.
        let granted = vec!["File".to_string(), "Project".to_string()];
        assert_eq!(
            exercised_read_types("MATCH (f:file) RETURN f.path", &granted),
            vec!["system.File".to_string()]
        );
    }

    #[test]
    fn an_exercised_read_names_every_label_once() {
        let granted = vec!["File".to_string(), "Project".to_string()];
        assert_eq!(
            exercised_read_types(
                "MATCH (f:File)-[:FILE_PART_OF]->(p:Project) MATCH (g:File) RETURN f, p, g",
                &granted
            ),
            vec!["system.File".to_string(), "system.Project".to_string()],
            "sorted and deduped, and the relation type is not a read scope"
        );
    }

    #[test]
    fn a_label_outside_the_read_scope_is_never_counted() {
        // It cannot happen (the gate refuses the query first), but the counter
        // must not depend on that: an ungranted label is not an exercise.
        assert!(exercised_read_types("MATCH (s:Session) RETURN s", &["File".to_string()]).is_empty());
        assert!(exercised_read_types("MATCH (n) RETURN n", &["File".to_string()]).is_empty());
    }

    /// The measurement that produced this rule: an unlabelled node and an untyped
    /// edge are "any label" and "any edge", and the allowlist never saw them
    /// because it only checks what a query NAMES.
    /// A property map's keys follow a `:` exactly as a label does, and reading
    /// them as labels refused `MATCH (f:File {app_id:'x'})` while the vaguer
    /// `MATCH (f:File)` passed - the allowlist rewarding the caller who says
    /// less. A map key was never a label, so this narrows nothing: the node still
    /// has to name a readable one.
    #[test]
    fn a_property_map_key_is_not_a_label() {
        let readable = ["File".to_string()];
        assert!(raw_read_label_gate(
            "MATCH (f:File {app_id:'com.example'}) RETURN f.path",
            &readable,
            false
        )
        .is_none());
        // The numeric case already worked and still does.
        assert!(raw_read_label_gate("MATCH (f:File {count:5}) RETURN f.path", &readable, false).is_none());
        // The node's own label is still checked: a map cannot smuggle one past.
        assert!(raw_read_label_gate(
            "MATCH (p:Project {app_id:'x'}) RETURN p.name",
            &readable,
            false
        )
        .is_some());
        // And an unlabelled node with a map is still an unlabelled node.
        assert!(raw_read_label_gate("MATCH (x {app_id:'y'}) RETURN x.id", &readable, false).is_some());
    }

    #[test]
    fn an_omitted_constraint_is_refused_as_the_widest_one() {
        let readable = ["File".to_string()];
        // The scan names what it found, so a refusal cannot be credited to the
        // wrong construct: this is what caught a call-detection bug that made
        // every pattern after a keyword look like a function call, collecting no
        // node label at all while the verdicts stayed correct.
        let scan = scan_patterns("MATCH (f:File)-[r:FILE_PART_OF]->(p:Project) RETURN p.id");
        assert_eq!(scan.node_labels, ["FILE", "PROJECT"]);
        assert_eq!(scan.rel_types, ["FILE_PART_OF"]);
        assert_eq!(scan.unconstrained, None);
        // A call is not a node, and its argument is not a label.
        let calls = scan_patterns("MATCH (f:File) RETURN count(*), toLower(f.path)");
        assert_eq!(calls.node_labels, ["FILE"]);
        assert_eq!(calls.unconstrained, None);

        // The exact query that was allowed before this rule.
        assert!(
            raw_read_label_gate("MATCH (f:File)-[r]->(x) RETURN x.id", &readable, false).is_some()
        );
        // Each half on its own.
        assert!(raw_read_label_gate("MATCH (x) RETURN x.id", &readable, false).is_some());
        assert!(
            raw_read_label_gate(
                "MATCH (f:File)-[r]->(p:Project) RETURN p.id",
                &["File".to_string(), "Project".to_string()],
                false
            )
            .is_some(),
            "an untyped edge between two named labels is still any edge"
        );
        // Fully constrained and in scope: allowed. The edge type is NOT in the
        // readable set - it does not need to be, because both of its endpoints
        // are. That is the endpoint model: an edge between two nodes a caller may
        // read adds little it could not infer from their properties and paths,
        // and enumerating types in a profile buys length rather than tightness.
        assert!(raw_read_label_gate(
            "MATCH (f:File)-[r:FILE_PART_OF]->(p:Project) RETURN p.id",
            &["File".to_string(), "Project".to_string()],
            false
        )
        .is_none());
        // An endpoint outside the scope still refuses, so the edge cannot be used
        // to reach a label the caller may not read.
        assert!(raw_read_label_gate(
            "MATCH (f:File)-[r:FILE_PART_OF]->(p:Project) RETURN p.id",
            &["File".to_string()],
            false
        )
        .is_some());
        // A function call is not an unlabelled node, and a list is not an edge.
        assert!(raw_read_label_gate(
            "MATCH (f:File) WHERE f.id IN ['a','b'] RETURN count(*)",
            &readable,
            false
        )
        .is_none());
        // A system-anchored caller still bypasses, as before.
        assert!(raw_read_label_gate("MATCH (x) RETURN x.id", &[], true).is_none());
    }

    #[test]
    fn readable_system_labels_maps_only_safe_system_scopes() {
        use crate::token::EntityScope;
        let scope = |t: &str| EntityScope {
            entity_type: t.to_string(),
            fields: None,
            exclude_fields: vec![],
        };
        // system.* scopes map to the unprefixed graph label; an app type does not.
        assert_eq!(
            readable_system_labels(&[scope("system.File"), scope("system.App"), scope("com.anki.Card")]),
            vec!["File".to_string(), "App".to_string()],
        );
        // A type that would break the per-label probe is dropped (only safe
        // identifiers reach the interpolated existence query).
        assert!(readable_system_labels(&[scope("system.File; DROP")]).is_empty());
        assert!(readable_system_labels(&[scope("system.")]).is_empty());
        assert!(readable_system_labels(&[]).is_empty());
        // An underscore is part of a valid identifier, not a reason to drop the
        // name: every relationship type has one, and the old alphanumeric-only
        // rule rejected them all without saying so.
        assert_eq!(
            readable_system_labels(&[scope("system.FILE_PART_OF"), scope("system._x1")]),
            vec!["FILE_PART_OF".to_string(), "_x1".to_string()],
        );
        // A leading digit is not an identifier and stays out.
        assert!(readable_system_labels(&[scope("system.1File")]).is_empty());
    }

    #[test]
    fn only_the_knowledge_app_may_delete_activity() {
        // The exact principal the image stages, and nothing else. A tier check
        // would admit every FirstParty binary; a `dev.` prefix would admit any
        // locally built crate. Both were considered and both are too wide for the
        // one operation that permanently destroys the user's records.
        assert!(activity_delete_caller_admitted("dev.arlen.knowledge"));
        for other in [
            "dev.arlen.settings",
            "dev.arlen.files",
            "knowledge",
            "ai-agent",
            "dev.arlen.knowledge.evil",
            "",
        ] {
            assert!(
                !activity_delete_caller_admitted(other),
                "{other} must not be able to clear the timeline"
            );
        }
    }

    #[test]
    fn revoke_caller_admitted_only_settings() {
        assert!(revoke_caller_admitted("dev.arlen.settings"));
        for other in ["ai-agent", "ai-daemon", "com.x", "knowledge", "unknown", ""] {
            assert!(!revoke_caller_admitted(other), "{other} must not be allowed to revoke");
        }
        // Only Settings' exact cargo-run id is admitted in debug, never
        // in release; an arbitrary `dev.*` crate is always refused.
        assert_eq!(
            revoke_caller_admitted("dev.arlen-settings"),
            cfg!(debug_assertions)
        );
        assert!(!revoke_caller_admitted("dev.evil"));
        assert!(!revoke_caller_admitted("dev.arlen-knowledge"));
    }

    #[test]
    fn meeting_caller_admitted_only_meetings_and_engine() {
        assert!(meeting_caller_admitted("meetings"));
        assert!(meeting_caller_admitted("ai-agent"));
        for other in ["settings", "com.x", "knowledge", "unknown", ""] {
            assert!(!meeting_caller_admitted(other), "{other} must not access meetings");
        }
        // A `dev.`-prefixed id is admitted in debug only, never in release.
        assert_eq!(meeting_caller_admitted("dev.arlen-meetings"), cfg!(debug_assertions));
    }

    #[tokio::test]
    async fn meetings_read_lists_and_gets_filed_notes_and_gates_the_caller() {
        let tmp = tempfile::TempDir::new().unwrap();
        let graph = crate::graph::spawn(tmp.path().join("graph").to_str().unwrap()).unwrap();

        // File a note directly through the store, then read it back over the op.
        let items = [crate::meeting::ActionItemRecord { text: "ship it", owner: Some("Tim") }];
        let record = crate::meeting::MeetingRecord {
            title: "Sync",
            summary: "we shipped",
            participants: &["Tim".to_string()],
            action_items: &items,
        };
        crate::meeting::file_meeting(&graph, "m-1", record, 42).await.unwrap();

        // A non-admitted caller is refused before any read.
        let denied = handle_meetings_read("com.x", br#"{"op":"list"}"#, &graph).await;
        assert!(denied.starts_with("ERROR: permission denied"), "{denied}");

        // List surfaces the filed note.
        let list = handle_meetings_read("meetings", br#"{"op":"list"}"#, &graph).await;
        assert!(list.contains("\"id\":\"m-1\"") && list.contains("\"title\":\"Sync\""), "{list}");

        // Get returns the detail with its action item.
        let got = handle_meetings_read("meetings", br#"{"op":"get","id":"m-1"}"#, &graph).await;
        assert!(got.contains("\"summary\"") && got.contains("ship it"), "{got}");

        // Get for an unknown id answers null.
        let none = handle_meetings_read("meetings", br#"{"op":"get","id":"nope"}"#, &graph).await;
        assert_eq!(none.trim(), "null");

        // A malformed body is a clean error, not a panic.
        let bad = handle_meetings_read("meetings", b"not json", &graph).await;
        assert!(bad.starts_with("ERROR: malformed"), "{bad}");
    }

    #[test]
    fn handle_revoke_refuses_a_non_settings_caller() {
        // Even a well-formed body is refused before parsing if the caller is not
        // the admitted Settings principal.
        let r = handle_revoke("com.attacker", b"{\"target_app_id\":\"com.x\",\"reach\":{\"InstanceAll\":null},\"initiator\":{\"User\":null}}");
        assert!(r.starts_with("ERROR: revoke not permitted"), "got {r}");
    }

    #[test]
    fn capability_change_pairs_filters_to_the_target_apps_records() {
        use audit_proto::{AuditKind, CapabilityReach, StructuralRecord, StructuralView};
        let view = |target: &str, outcome: &str, kind: AuditKind, subject: &str| StructuralView {
            index: 0,
            timestamp_micros: 0,
            kind,
            actor: "knowledge".into(),
            structural: StructuralRecord {
                subject: subject.into(),
                node_types: vec![target.into()],
                relations: vec![],
                result_count: None,
                duration_ms: None,
                outcome: outcome.into(),
                depth: None,
                capability_change: Some(CapabilityReach::Read {
                    entity_pattern: "system.File".into(),
                }),
            },
            call_chain_id: None,
            project_id: None,
            entry_hash_hex: String::new(),
        };
        let subj = crate::audit::CAPABILITY_CHANGE_SUBJECT;
        let views = vec![
            view("com.a", "revoked", AuditKind::CapabilityChange, subj),
            view("com.b", "revoked", AuditKind::CapabilityChange, subj), // a different app
            view("com.a", "ok", AuditKind::GraphAccess, subj),          // wrong kind
            view("com.a", "revoked", AuditKind::CapabilityChange, "other"), // wrong subject
        ];
        let pairs = capability_change_pairs("com.a", &views);
        assert_eq!(pairs.len(), 1, "only com.a's capability-change record survives");
        assert_eq!(pairs[0].0, "revoked");
        assert_eq!(
            pairs[0].1,
            crate::revoke::RevokedReach::Read { entity_pattern: "system.File".into() }
        );
    }

    #[tokio::test]
    async fn handle_restore_refuses_a_non_settings_caller() {
        // A non-Settings caller is refused at the caller gate, before any ledger
        // read or audit - the same authority-growth guard as revoke.
        let audit: std::sync::Arc<dyn audit_proto::AuditSink> =
            std::sync::Arc::new(audit_proto::LedgerAuditSink::at_default_socket());
        let r = handle_restore(
            "com.attacker",
            b"{\"target_app_id\":\"com.x\",\"reach\":{\"InstanceAll\":null},\"initiator\":{\"User\":null}}",
            &audit,
        )
        .await;
        assert!(r.starts_with("ERROR: restore not permitted"), "got {r}");
    }

    #[tokio::test]
    async fn handle_restore_refuses_an_agent_initiator() {
        // Even as the admitted Settings principal, an agent-initiated restore is a
        // proposal, never a confirmed grant (§6.3), refused before any write.
        let audit: std::sync::Arc<dyn audit_proto::AuditSink> =
            std::sync::Arc::new(audit_proto::LedgerAuditSink::at_default_socket());
        let r = handle_restore(
            "dev.arlen-settings",
            b"{\"target_app_id\":\"com.x\",\"reach\":{\"InstanceAll\":null},\"initiator\":{\"Agent\":{\"suggestion_id\":\"s1\"}}}",
            &audit,
        )
        .await;
        assert!(r.starts_with("ERROR: an agent-initiated restore"), "got {r}");
    }

    #[test]
    fn handle_revoke_rejects_a_malformed_request() {
        let r = handle_revoke("dev.arlen.settings", b"not json");
        assert!(r.starts_with("ERROR: invalid revoke request"), "got {r}");
    }

    #[test]
    fn handle_revoke_refuses_an_agent_initiated_revoke() {
        // §6.3: a literal `Agent` initiator at the apply site is a proposal, never a
        // confirmed revoke, so it is refused even from the admitted Settings caller
        // (a confirmed revoke is replayed as `User`). The refusal lands before the
        // target is turned into a path, so no profile is touched.
        let body = b"{\"target_app_id\":\"com.x\",\"reach\":{\"InstanceAll\":null},\"initiator\":{\"Agent\":{\"suggestion_id\":\"s1\"}}}";
        let r = handle_revoke("dev.arlen.settings", body);
        assert!(r.starts_with("ERROR: an agent-initiated revoke"), "got {r}");
    }

    #[test]
    fn handle_revoke_rejects_a_traversal_target() {
        // A target app id that would escape the permissions dir is refused before
        // it becomes a path, so revoke can never read+rewrite an arbitrary file.
        for target in ["../../../etc/cron.d/evil", "/etc/passwd", "..", "a/b", "x\0y"] {
            let body = format!(
                "{{\"target_app_id\":\"{}\",\"reach\":{{\"InstanceAll\":null}},\"initiator\":{{\"User\":null}}}}",
                target.escape_default()
            );
            let r = handle_revoke("dev.arlen.settings", body.as_bytes());
            assert!(
                r.starts_with("ERROR: invalid target app id") || r.starts_with("ERROR: invalid revoke request"),
                "traversal target {target:?} must be refused, got {r}"
            );
        }
    }

    #[test]
    fn is_safe_app_id_accepts_reverse_dns_and_rejects_traversal() {
        for ok in ["settings", "com.example.app", "ai-agent", "a_b.c-d"] {
            assert!(is_safe_app_id(ok), "{ok} should be safe");
        }
        for bad in ["", ".", "..", "../x", "/etc/x", "a/b", "a\\b", "x\0y"] {
            assert!(!is_safe_app_id(bad), "{bad:?} must be rejected");
        }
    }

    #[test]
    fn co_tenant_filter_names_only_the_caller() {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        // The caller and a co-tenant: name the caller, flag the other.
        assert_eq!(co_tenant_filter(&s(&["me", "other"]), "me"), (vec!["me".to_string()], true));
        // Only the caller: named, no others.
        assert_eq!(co_tenant_filter(&s(&["me"]), "me"), (vec!["me".to_string()], false));
        // Only foreign actors: none named, others flagged (the co-tenant fix).
        assert_eq!(co_tenant_filter(&s(&["a", "b"]), "me"), (Vec::<String>::new(), true));
        // No actors at all.
        assert_eq!(co_tenant_filter(&s(&[]), "me"), (Vec::<String>::new(), false));
    }

    #[tokio::test]
    async fn test_handle_graph_event_permission_changed() {
        let auth = Arc::new(Mutex::new(Authenticator::new()));
        let tmp = tempfile::TempDir::new().unwrap();
        let graph = crate::graph::spawn(tmp.path().join("graph").to_str().unwrap()).unwrap();
        handle_graph_event(
            &auth,
            &graph,
            GraphEvent::PermissionChanged {
                app_id: "com.test".into(),
            },
        )
        .await;
        // Should not panic; cache invalidation is internal and the stale-mark is
        // a no-op on an empty graph.
    }

    #[tokio::test]
    async fn test_handle_graph_event_ai_level() {
        let auth = Arc::new(Mutex::new(Authenticator::new()));
        let tmp = tempfile::TempDir::new().unwrap();
        let graph = crate::graph::spawn(tmp.path().join("graph").to_str().unwrap()).unwrap();
        handle_graph_event(&auth, &graph, GraphEvent::AiLevelChanged).await;
    }

    #[tokio::test]
    async fn typed_read_fails_closed_to_the_uniform_denial() {
        let auth = Arc::new(Mutex::new(Authenticator::new()));
        let tmp = tempfile::TempDir::new().unwrap();
        let graph = crate::graph::spawn(tmp.path().join("graph").to_str().unwrap()).unwrap();
        // Past its interval, so a recorded use would flush and be visible.
        let uses = Arc::new(Mutex::new(crate::lcg::UseTally::new(
            crate::time::now().0 - crate::lcg::USE_FLUSH_INTERVAL_MICROS,
        )));
        // A malformed body never reaches the token guard: uniform OutOfScope.
        assert_eq!(
            handle_typed_read(b"not json", None, &auth, &graph, &uses).await,
            PROVENANCE_OUT_OF_SCOPE
        );
        // A well-formed request with no attested peer is also the uniform denial
        // (no oracle distinguishing it from an out-of-scope or absent read).
        let body = br#"{"label":"CommandHistory","filters":[{"field":"session_id","value":"s1"}],"select":["command"]}"#;
        assert_eq!(
            handle_typed_read(body, None, &auth, &graph, &uses).await,
            PROVENANCE_OUT_OF_SCOPE
        );

        // A denied read is not use, on this op as on every other.
        let rows = graph
            .query_rows_json("MATCH (u:CapabilityUse) RETURN count(*)".to_string())
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&rows).unwrap();
        assert_eq!(parsed["rows"][0][0], 0, "a refused typed read counted nothing");
    }

    /// A grant says whether its reach has actually been exercised, so a surface
    /// can offer to revoke what is unused. The count is per (app, capability) on
    /// `CapabilityUse`, which is why it survives the Grant node being replaced on
    /// every request.
    #[tokio::test]
    async fn access_grants_reports_which_reach_has_been_used() {
        let tmp = tempfile::TempDir::new().unwrap();
        let graph = crate::graph::spawn(tmp.path().join("graph").to_str().unwrap()).unwrap();

        let token = crate::token::CapabilityToken::new(
            "com.use".to_string(),
            std::process::id(),
            vec![
                crate::token::EntityScope {
                    entity_type: "system.File".into(),
                    fields: Some(vec!["path".into()]),
                    exclude_fields: vec![],
                },
                crate::token::EntityScope {
                    entity_type: "system.Project".into(),
                    fields: Some(vec!["name".into()]),
                    exclude_fields: vec![],
                },
            ],
            vec![],
            vec![],
            crate::token::InstanceScope::Own,
        );
        crate::lcg::emit_grant_node(&graph, &token).await.unwrap();

        // Before any use: reach of two, nothing exercised.
        let before: serde_json::Value =
            serde_json::from_str(&handle_access_grants("com.use", &graph).await).unwrap();
        assert_eq!(before[0]["reach"].as_array().unwrap().len(), 2);
        assert_eq!(before[0]["used_capability_count"], 0);
        assert_eq!(before[0]["last_used_at"], 0);

        // One capability exercised and flushed.
        crate::lcg::flush_capability_use(
            &graph,
            &[("com.use".into(), "system.File".into(), 3, 4_242)],
        )
        .await
        .unwrap();

        let after: serde_json::Value =
            serde_json::from_str(&handle_access_grants("com.use", &graph).await).unwrap();
        assert_eq!(
            after[0]["used_capability_count"], 1,
            "one of the two reachable types has been exercised"
        );
        assert_eq!(after[0]["last_used_at"], 4_242);
    }

    /// The overlay keys on the FULL type, not on the display label with `system.`
    /// put back in front of it. A bridge type has the label `Note` and the
    /// capability `md.obsidian.Note`, so reconstructing the key would look up
    /// `system.Note`, find nothing, and report a used grant as never used - the
    /// exact direction that hides a live grant from the revoke prompt.
    #[tokio::test]
    async fn a_non_system_capability_is_matched_by_its_full_type() {
        let tmp = tempfile::TempDir::new().unwrap();
        let graph = crate::graph::spawn(tmp.path().join("graph").to_str().unwrap()).unwrap();

        let token = crate::token::CapabilityToken::new(
            "com.bridge".to_string(),
            std::process::id(),
            vec![crate::token::EntityScope {
                entity_type: "md.obsidian.Note".into(),
                fields: Some(vec!["title".into()]),
                exclude_fields: vec![],
            }],
            vec![],
            vec![],
            crate::token::InstanceScope::Own,
        );
        crate::lcg::emit_grant_node(&graph, &token).await.unwrap();
        crate::lcg::flush_capability_use(
            &graph,
            &[("com.bridge".into(), "md.obsidian.Note".into(), 1, 777)],
        )
        .await
        .unwrap();

        let view: serde_json::Value =
            serde_json::from_str(&handle_access_grants("com.bridge", &graph).await).unwrap();
        assert_eq!(view[0]["reach"][0], "Note", "the label is what a surface shows");
        assert_eq!(
            view[0]["used_capability_count"], 1,
            "and the use is found by the type behind it"
        );
        assert_eq!(view[0]["last_used_at"], 777);
    }

    #[tokio::test]
    async fn access_grants_is_caller_scoped_with_fresh_liveness() {
        use crate::token::{CapabilityToken, EntityScope, InstanceScope};
        let tmp = tempfile::TempDir::new().unwrap();
        let graph = crate::graph::spawn(tmp.path().join("graph").to_str().unwrap()).unwrap();

        let mk = |app: &str, ty: &str| {
            CapabilityToken::new(
                app.into(),
                999_999, // a pid that is not alive, so fresh liveness renders not-live
                vec![EntityScope {
                    entity_type: ty.into(),
                    fields: None,
                    exclude_fields: vec![],
                }],
                vec![],
                vec![],
                InstanceScope::Own,
            )
        };
        crate::lcg::emit_grant_node(&graph, &mk("com.a", "system.File")).await.unwrap();
        crate::lcg::emit_grant_node(&graph, &mk("com.b", "system.Project")).await.unwrap();

        // com.a sees only its own grant, never com.b's (the §5 scoping).
        let json = handle_access_grants("com.a", &graph).await;
        let views: serde_json::Value = serde_json::from_str(&json).unwrap();
        let arr = views.as_array().unwrap();
        assert_eq!(arr.len(), 1, "caller sees exactly its own grant: {json}");
        assert_eq!(arr[0]["app_id"], "com.a");
        assert_eq!(arr[0]["reach"][0], "File");
        // Stored live=true, but pid 999999 is not alive, so it renders not-live.
        assert_eq!(arr[0]["live"], false, "dead process renders not-live: {json}");
        assert_eq!(arr[0]["revoked"], false);

        // An app with no grants gets an empty array, never an error or a leak.
        let none = handle_access_grants("com.c", &graph).await;
        assert_eq!(none, "[]", "no grants -> empty, not a leak: {none}");
    }

    #[tokio::test]
    async fn access_grants_surfaces_consent_grants_alongside_capability_tokens() {
        use crate::token::{CapabilityToken, EntityScope, InstanceScope};
        let tmp = tempfile::TempDir::new().unwrap();
        let graph = crate::graph::spawn(tmp.path().join("graph").to_str().unwrap()).unwrap();

        // A capability-token grant for com.app (a real, scoped token).
        let token = CapabilityToken::new(
            "com.app".into(),
            999_999, // not alive -> capability tokens render not-live
            vec![EntityScope {
                entity_type: "system.File".into(),
                fields: None,
                exclude_fields: vec![],
            }],
            vec![],
            vec![],
            InstanceScope::Own,
        );
        crate::lcg::emit_grant_node(&graph, &token).await.unwrap();

        // A consent grant for the same principal (pid-free, user-confirmed).
        crate::lcg::persist_consent_grant(
            &graph,
            "com.app",
            "contacts.read",
            Some("self"),
            "rev-handle-1",
            None,
        )
        .await
        .unwrap();

        let json = handle_access_grants("com.app", &graph).await;
        let views: serde_json::Value = serde_json::from_str(&json).unwrap();
        let arr = views.as_array().unwrap();
        assert_eq!(arr.len(), 2, "both grants surface for the caller: {json}");

        let token_view = arr
            .iter()
            .find(|v| v["source"] == "capability-token")
            .expect("the capability-token grant appears with its source");
        // pid 999999 is dead, so the token grant renders not-live.
        assert_eq!(token_view["live"], false, "dead-pid token not-live: {json}");

        let consent_view = arr
            .iter()
            .find(|v| v["source"] == "consent")
            .expect("the consent grant appears with source=consent");
        assert_eq!(consent_view["consent_class"], "contacts.read");
        assert_eq!(consent_view["consent_scope"], "self");
        // A consent grant has no pid; liveness rests on stored_live/revoke/supersede,
        // so it renders live even though no process backs it.
        assert_eq!(
            consent_view["live"], true,
            "pid-free consent grant renders live: {json}"
        );
        assert_eq!(consent_view["revoked"], false);
    }

    #[tokio::test]
    async fn access_grants_renders_a_declared_network_grant_live() {
        // A `declared`-source grant (a profile-projected NetworkAccess reach) has
        // pid 0 and no process; it must render live from the stored flag, not be
        // failed by a process_alive(0) check (the process-independent exemption).
        let tmp = tempfile::TempDir::new().unwrap();
        let graph = crate::graph::spawn(tmp.path().join("graph").to_str().unwrap()).unwrap();

        crate::lcg::emit_declared_network_grant(&graph, "com.app", Some("api.openai.com"))
            .await
            .unwrap();

        let json = handle_access_grants("com.app", &graph).await;
        let views: serde_json::Value = serde_json::from_str(&json).unwrap();
        let arr = views.as_array().unwrap();
        let net = arr
            .iter()
            .find(|v| v["consent_class"] == "NetworkAccess")
            .expect("the declared network grant surfaces");
        assert_eq!(net["source"], "declared");
        assert_eq!(net["consent_scope"], "api.openai.com");
        assert_eq!(net["live"], true, "a pid-free declared grant renders live: {json}");
        assert_eq!(net["revoked"], false);
    }

    /// A time-boxed grant whose window has closed must not read as live, and one
    /// whose window is still open must. Nothing but the clock changes between
    /// them, which is why the reader has to do this rather than trust the stored
    /// flag: no emitter runs at the moment a window closes.
    #[tokio::test]
    async fn a_grant_past_its_expiry_is_not_live() {
        let tmp = tempfile::TempDir::new().unwrap();
        let graph = crate::graph::spawn(tmp.path().join("graph").to_str().unwrap()).unwrap();
        let now = crate::time::now().0;

        for (id, expires) in [("g-open", now + 60_000_000), ("g-closed", now - 1)] {
            graph
                .write(format!(
                    "CREATE (:Grant {{id:'{id}', app_id:'com.app', pid:0, issued_at:{now},                      expires_at:{expires}, declared_ceiling:'', required:false,                      identity_verified:false, live:true, revoked:false, superseded:false,                      last_exercised_at:0, use_count:0, source:'consent',                      consent_class:'NetworkAccess', consent_scope:'api.example.com'}})"
                ))
                .await
                .unwrap();
        }

        let json = handle_access_grants("com.app", &graph).await;
        let views: serde_json::Value = serde_json::from_str(&json).unwrap();
        let arr = views.as_array().unwrap();
        let of = |id: &str| {
            arr.iter()
                .find(|v| v["id"] == id)
                .unwrap_or_else(|| panic!("{id} surfaces: {json}"))
                .clone()
        };
        assert_eq!(of("g-open")["live"], true, "an open window is live: {json}");
        assert_eq!(of("g-closed")["live"], false, "a closed window is not: {json}");
        // Expiry is not revocation: the row still says what it was.
        assert_eq!(of("g-closed")["revoked"], false);
        // And it carries WHEN, so a surface can say the window closed rather
        // than leaving "not live" to mean any of three different things.
        assert_eq!(of("g-closed")["expires_at"], now - 1);
    }

    #[tokio::test]
    async fn code_analysis_is_gated_to_system_anchored_callers() {
        let tmp = tempfile::TempDir::new().unwrap();
        let graph = crate::graph::spawn(tmp.path().join("graph").to_str().unwrap()).unwrap();
        // A small call graph: a.rs has a hub called by p, q, r, calling y in b.rs.
        for id in [
            "a.rs#fn:hub@1",
            "a.rs#fn:p@2",
            "a.rs#fn:q@3",
            "a.rs#fn:r@4",
            "b.rs#fn:y@5",
        ] {
            graph
                .write(format!("CREATE (:CodeSymbol {{id: '{id}', name: 'n'}})"))
                .await
                .unwrap();
        }
        for (from, to) in [
            ("a.rs#fn:p@2", "a.rs#fn:hub@1"),
            ("a.rs#fn:q@3", "a.rs#fn:hub@1"),
            ("a.rs#fn:r@4", "a.rs#fn:hub@1"),
            ("a.rs#fn:hub@1", "b.rs#fn:y@5"),
        ] {
            graph
                .write(format!(
                    "MATCH (a:CodeSymbol {{id:'{from}'}}), (b:CodeSymbol {{id:'{to}'}}) \
                     CREATE (a)-[:CALLS {{confidence:'extracted'}}]->(b)"
                ))
                .await
                .unwrap();
        }

        // A resolved `unknown` and a ThirdParty app are denied the aggregate.
        assert_eq!(handle_code_analysis("unknown", &graph).await, CODE_ANALYSIS_DENIED);
        assert_eq!(
            handle_code_analysis("com.example.thirdparty", &graph).await,
            CODE_ANALYSIS_DENIED
        );

        // A system-anchored caller (the agent is FirstParty in the default quota
        // config) receives the analysis as JSON, with the hub among the
        // god-symbols (threshold default 5 is too high for this toy graph, so the
        // analysis runs but flags no hub here; assert it parses and is the
        // expected shape with the lone cross-module surprise).
        let json = handle_code_analysis("ai-agent", &graph).await;
        assert_ne!(json, CODE_ANALYSIS_DENIED, "the agent is system-anchored: {json}");
        let v: serde_json::Value = serde_json::from_str(&json).expect("valid CodeAnalysis JSON");
        assert!(v.get("god_symbols").is_some() && v.get("surprises").is_some());
        let surprises = v["surprises"].as_array().unwrap();
        assert_eq!(surprises.len(), 1, "the lone a.rs->b.rs bridge is the surprise: {json}");
        assert_eq!(surprises[0]["from"], "a.rs#fn:hub@1");
        assert_eq!(surprises[0]["to"], "b.rs#fn:y@5");
    }

    #[tokio::test]
    async fn permission_changed_for_an_uninstalled_app_removes_its_grants() {
        use crate::token::{CapabilityToken, EntityScope, InstanceScope};
        let auth = Arc::new(Mutex::new(Authenticator::new()));
        let tmp = tempfile::TempDir::new().unwrap();
        let graph = crate::graph::spawn(tmp.path().join("graph").to_str().unwrap()).unwrap();

        // An app id that has no profile on disk (so profile_exists is false =
        // uninstalled).
        let app = "com.test.uninstalled.nonexistent.example";
        let token = CapabilityToken::new(
            app.into(),
            7,
            vec![EntityScope {
                entity_type: "system.File".into(),
                fields: None,
                exclude_fields: vec![],
            }],
            vec![],
            vec![],
            InstanceScope::Own,
        );
        crate::lcg::emit_grant_node(&graph, &token).await.unwrap();

        // Sanity: the grant exists before the uninstall event.
        let before = graph
            .query_rows_json(format!("MATCH (g:Grant {{app_id:'{app}'}}) RETURN g.id"))
            .await
            .unwrap();
        assert!(before.contains("rows"));
        let before_parsed: serde_json::Value = serde_json::from_str(&before).unwrap();
        assert_eq!(before_parsed["rows"].as_array().unwrap().len(), 1);

        // The uninstall event (profile gone) removes the orphaned grant.
        handle_graph_event(
            &auth,
            &graph,
            GraphEvent::PermissionChanged { app_id: app.into() },
        )
        .await;

        let after = graph
            .query_rows_json(format!("MATCH (g:Grant {{app_id:'{app}'}}) RETURN g.id"))
            .await
            .unwrap();
        let after_parsed: serde_json::Value = serde_json::from_str(&after).unwrap();
        assert_eq!(
            after_parsed["rows"].as_array().unwrap().len(),
            0,
            "the uninstalled app's grants are removed: {after}"
        );
    }

    #[tokio::test]
    async fn mark_app_grants_stale_flips_a_live_grant() {
        use crate::token::{CapabilityToken, EntityScope, InstanceScope};
        let tmp = tempfile::TempDir::new().unwrap();
        let graph = crate::graph::spawn(tmp.path().join("graph").to_str().unwrap()).unwrap();

        // Project a live grant for the app.
        let token = CapabilityToken::new(
            "com.x".into(),
            7,
            vec![EntityScope {
                entity_type: "system.File".into(),
                fields: None,
                exclude_fields: vec![],
            }],
            vec![],
            vec![],
            InstanceScope::Own,
        );
        crate::lcg::emit_grant_node(&graph, &token).await.unwrap();

        mark_app_grants_stale(&graph, "com.x").await;

        let state = graph
            .query_rows_json("MATCH (g:Grant {app_id:'com.x'}) RETURN g.live".to_string())
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&state).unwrap();
        assert_eq!(parsed["rows"][0][0], false, "the grant is no longer live: {state}");
    }

    #[tokio::test]
    async fn permission_changed_projects_declared_grants_for_a_never_run_app() {
        // E1: an installed-but-never-run app has a profile on disk but has never
        // connected to the graph, so the connect-time projection never fired. The
        // permission-changed event (installd wrote the profile) must project its
        // DECLARED grants from the profile alone, with no token mint / running pid.
        let (graph, _tmp) = spawn_test_graph().await;
        let perms = tempfile::TempDir::new().unwrap();
        let app = "com.example.enrolled";
        std::fs::write(
            perms.path().join(format!("{app}.toml")),
            "[info]\napp_id = \"com.example.enrolled\"\n[network]\nallow_all = true\n",
        )
        .unwrap();
        std::env::set_var("ARLEN_PERMISSIONS_DIR", perms.path());

        // No Grant nodes before the event: the app has never connected.
        let before = graph
            .query_rows_json(format!("MATCH (g:Grant {{app_id:'{app}'}}) RETURN g.id"))
            .await
            .unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&before).unwrap()["rows"]
                .as_array()
                .unwrap()
                .len(),
            0
        );

        let auth = Arc::new(Mutex::new(Authenticator::new()));
        handle_graph_event(
            &auth,
            &graph,
            GraphEvent::PermissionChanged { app_id: app.into() },
        )
        .await;

        let after = graph
            .query_rows_json(format!("MATCH (g:Grant {{app_id:'{app}'}}) RETURN g.id"))
            .await
            .unwrap();
        std::env::remove_var("ARLEN_PERMISSIONS_DIR");
        assert!(
            !serde_json::from_str::<serde_json::Value>(&after).unwrap()["rows"]
                .as_array()
                .unwrap()
                .is_empty(),
            "the enrolled app's declared grants are projected at permission-change time: {after}"
        );
    }

    #[test]
    fn rate_state_emit_is_throttled() {
        let mut rs = RateState::new();
        assert!(rs.should_emit("com.test"), "first violation emits");
        assert!(!rs.should_emit("com.test"), "a repeat within the window is throttled");
        // A different identity emits independently.
        assert!(rs.should_emit("com.other"));
    }

    /// Spawn a fresh graph in a temp dir and wait for schema init. These tests
    /// touch a real Ladybug instance, so they flake under a parallel `cargo
    /// test` (multi-instance); run the suite with `--test-threads=1`.
    async fn spawn_test_graph() -> (GraphHandle, tempfile::TempDir) {
        let tmp = tempfile::TempDir::new().unwrap();
        let graph = crate::graph::spawn(tmp.path().join("graph").to_str().unwrap()).unwrap();
        tokio::time::sleep(Duration::from_millis(500)).await;
        (graph, tmp)
    }

    /// An empty raw-event store for a write-path test.
    ///
    /// `handle_write_request` needs one because a range delete now takes the raw
    /// events with it. These tests never delete, so an empty schema is enough -
    /// what matters is that the handler is exercised at the same arity production
    /// uses rather than through a stripped-down variant of itself.
    async fn spawn_test_pool() -> (sqlx::SqlitePool, tempfile::TempDir) {
        let tmp = tempfile::TempDir::new().unwrap();
        let pool = crate::db::open(tmp.path().join("events.db").to_str().unwrap())
            .await
            .expect("open a test event store");
        (pool, tmp)
    }

    fn file_part_of(from_id: &str, to_id: &str) -> RelationResult {
        RelationResult {
            from_type: "system.File".into(),
            from_id: from_id.into(),
            to_type: "system.Project".into(),
            to_id: to_id.into(),
            relation_type: "FILE_PART_OF".into(),
        }
    }

    #[tokio::test]
    async fn code_symbol_context_op_gates_and_serializes() {
        let (graph, _tmp) = spawn_test_graph().await;
        // A ThirdParty caller is denied (the fusion exceeds its read scope).
        let denied =
            handle_code_symbol_context("com.third.party", &graph, br#"{"symbol_id":"x"}"#).await;
        assert!(denied.starts_with("ERROR:"), "ThirdParty denied: {denied}");
        // A system-anchored caller with a malformed body is denied, not guessed.
        let bad = handle_code_symbol_context("ai-agent", &graph, b"not json").await;
        assert!(bad.starts_with("ERROR:"), "malformed body denied: {bad}");
        // A system-anchored caller with a valid request for an absent symbol gets
        // a context JSON (file_path null), proving the op runs + serialises.
        let ok =
            handle_code_symbol_context("ai-agent", &graph, br#"{"symbol_id":"/p/x#fn:none@1"}"#)
                .await;
        assert!(!ok.starts_with("ERROR:"), "valid request returns a context: {ok}");
        let v: serde_json::Value = serde_json::from_str(&ok).unwrap();
        assert_eq!(v["symbol_id"], "/p/x#fn:none@1");
        assert!(v["file_path"].is_null(), "an absent symbol has no defining file");
    }

    #[tokio::test]
    async fn retrieve_filter_drops_ids_under_unreadable_labels() {
        let (graph, _tmp) = spawn_test_graph().await;
        graph
            .write("CREATE (f:File {id: 'f1', path: '/x', app_id: 't', last_accessed: 0})".into())
            .await
            .unwrap();
        graph
            .write("CREATE (s:Session {id: 's1'})".into())
            .await
            .unwrap();
        // File readable, Session not: s1 is dropped, the ranking order is preserved.
        // A tally already past its interval, so anything recorded flushes here
        // and is visible below.
        let uses = Arc::new(Mutex::new(crate::lcg::UseTally::new(
            crate::time::now().0 - crate::lcg::USE_FLUSH_INTERVAL_MICROS,
        )));
        let kept = filter_ids_to_readable_labels(
            &graph,
            &["f1".to_string(), "s1".to_string()],
            &["File".to_string()],
            Some((&uses, "com.reader")),
        )
        .await;
        assert_eq!(kept, vec!["f1".to_string()]);
        // No readable label → nothing survives (fail-closed).
        assert!(
            filter_ids_to_readable_labels(&graph, &["f1".to_string()], &[], None)
                .await
                .is_empty()
        );

        // The label that resolved a row counted; the one that resolved nothing
        // did not, and neither did the label with no grant behind it.
        let rows = graph
            .query_rows_json("MATCH (u:CapabilityUse) RETURN u.id ORDER BY u.id".to_string())
            .await
            .unwrap();
        assert!(rows.contains("com.reader:system.File"), "the File read counted: {rows}");
        assert!(!rows.contains("Session"), "a label that matched nothing is not use: {rows}");
    }

    #[tokio::test]
    async fn persist_relation_links_existing_nodes() {
        let (graph, _tmp) = spawn_test_graph().await;
        graph
            .write("CREATE (f:File {id: 'f1', path: '/x', app_id: 'test', last_accessed: 0})".into())
            .await
            .unwrap();
        graph
            .write("CREATE (p:Project {id: 'p1'})".into())
            .await
            .unwrap();

        let resp = persist_relation(&graph, &file_part_of("f1", "p1"), "op-1").await;
        assert_eq!(resp, "OK: created", "the first link creates the edge");

        // The edge is actually present, exactly once.
        let rows = graph
            .query_rows(
                "MATCH (:File {id: 'f1'})-[:FILE_PART_OF]->(:Project {id: 'p1'}) RETURN count(*) AS n"
                    .into(),
            )
            .await
            .unwrap();
        assert_eq!(rows.rows[0][0].as_i64(), 1, "the FILE_PART_OF edge exists");

        // The op_id is persisted, so a reconciliation by op_id finds THIS
        // operation's edge but not a different operation's.
        let mine = graph
            .query_rows(
                "MATCH (:File {id: 'f1'})-[:FILE_PART_OF {op_id: 'op-1'}]->(:Project {id: 'p1'}) RETURN count(*) AS n".into(),
            )
            .await
            .unwrap();
        assert_eq!(mine.rows[0][0].as_i64(), 1, "op-1 reconciles to its own edge");
        let other = graph
            .query_rows(
                "MATCH (:File {id: 'f1'})-[:FILE_PART_OF {op_id: 'op-other'}]->(:Project {id: 'p1'}) RETURN count(*) AS n".into(),
            )
            .await
            .unwrap();
        assert_eq!(other.rows[0][0].as_i64(), 0, "a different op does not match");

        // A second create is an idempotent no-op reported as `exists`, and does
        // not duplicate the edge (the conditional create is strict).
        let again = persist_relation(&graph, &file_part_of("f1", "p1"), "op-2").await;
        assert_eq!(again, "OK: exists", "a repeat link reports exists, not created");
        let rows = graph
            .query_rows(
                "MATCH (:File {id: 'f1'})-[:FILE_PART_OF]->(:Project {id: 'p1'}) RETURN count(*) AS n"
                    .into(),
            )
            .await
            .unwrap();
        assert_eq!(rows.rows[0][0].as_i64(), 1, "no duplicate edge after a repeat");
    }

    #[tokio::test]
    async fn persist_relation_supersedes_a_live_membership_to_another_project() {
        let (graph, _tmp) = spawn_test_graph().await;
        graph
            .write("CREATE (f:File {id: 'f1', path: '/x', app_id: 'test', last_accessed: 0})".into())
            .await
            .unwrap();
        graph.write("CREATE (p:Project {id: 'p1'})".into()).await.unwrap();
        graph.write("CREATE (p:Project {id: 'p2'})".into()).await.unwrap();

        assert_eq!(persist_relation(&graph, &file_part_of("f1", "p1"), "op-1").await, "OK: created");
        // A membership to a DIFFERENT project supersedes the first: it closes the
        // p1 edge and appends the p2 edge in one statement, still reporting created.
        assert_eq!(
            persist_relation(&graph, &file_part_of("f1", "p2"), "op-2").await,
            "OK: created",
            "a supersession still reports created (the close is internal)"
        );

        // Both edges are retained: the closed p1 edge alongside the live p2 edge.
        let total = graph
            .query_rows(
                "MATCH (:File {id: 'f1'})-[:FILE_PART_OF]->(:Project) RETURN count(*) AS n".into(),
            )
            .await
            .unwrap();
        assert_eq!(total.rows[0][0].as_i64(), 2, "the closed edge is retained alongside the new one");

        // Exactly one live membership, to p2.
        let live = graph
            .query_rows(
                "MATCH (:File {id: 'f1'})-[r:FILE_PART_OF]->(p:Project) \
                 WHERE r.invalid_at IS NULL AND r.expired_at IS NULL RETURN p.id AS id"
                    .into(),
            )
            .await
            .unwrap();
        assert_eq!(live.rows.len(), 1, "exactly one live membership after the supersession");
        assert_eq!(live.rows[0][0].as_str(), "p2", "the live membership is the new project");

        // The new edge back-references the superseded edge's op_id (§4.6), so a
        // later one-unit compensation can re-open what it replaced.
        let sup = graph
            .query_rows(
                "MATCH (:File {id: 'f1'})-[r:FILE_PART_OF {op_id: 'op-2'}]->(:Project) RETURN r.superseded AS s".into(),
            )
            .await
            .unwrap();
        assert_eq!(sup.rows[0][0].as_str(), "op-1", "the new edge records what it superseded");
    }

    #[test]
    fn content_merge_key_is_deterministic_content_specific_and_collision_safe() {
        let k = content_merge_key("File", "f1", "FILE_PART_OF", "Project", "p1");
        // Deterministic + fixed 64-char lowercase hex (so it is injection-safe).
        assert_eq!(k, content_merge_key("File", "f1", "FILE_PART_OF", "Project", "p1"));
        assert_eq!(k.len(), 64);
        assert!(k.bytes().all(|b| b.is_ascii_hexdigit()), "merge key is hex");
        // A different fact (different project) is a different key.
        assert_ne!(k, content_merge_key("File", "f1", "FILE_PART_OF", "Project", "p2"));
        // The length prefix means a tuple-boundary shift cannot collide: moving a
        // character across the from/to boundary yields a different key.
        assert_ne!(
            content_merge_key("File", "f1x", "FILE_PART_OF", "Project", "p1"),
            content_merge_key("File", "f1", "FILE_PART_OF", "Project", "xp1"),
        );
    }

    #[tokio::test]
    async fn two_replicas_assert_the_same_membership_to_the_same_merge_key() {
        // GD-R1 convergence property: two independent "devices" each write the
        // same fact (f1 PART_OF p1) under their OWN op_id. The op_ids differ (the
        // per-device idempotency key), but the content-addressed merge_key is
        // identical, so a future cross-device union dedups them to one membership.
        async fn write_on_a_replica(op_id: &str) -> (GraphHandle, tempfile::TempDir, String) {
            let (graph, tmp) = spawn_test_graph().await;
            graph
                .write(
                    "CREATE (f:File {id: 'f1', path: '/x', app_id: 't', last_accessed: 0})".into(),
                )
                .await
                .unwrap();
            graph.write("CREATE (p:Project {id: 'p1'})".into()).await.unwrap();
            assert_eq!(persist_relation(&graph, &file_part_of("f1", "p1"), op_id).await, "OK: created");
            let row = graph
                .query_rows(
                    "MATCH (:File {id: 'f1'})-[r:FILE_PART_OF]->(:Project {id: 'p1'}) \
                     RETURN r.merge_key AS mk"
                        .into(),
                )
                .await
                .unwrap();
            let mk = row.rows[0][0].as_str().to_string();
            (graph, tmp, mk)
        }

        let (_ga, _ta, mk_a) = write_on_a_replica("op-device-a").await;
        let (_gb, _tb, mk_b) = write_on_a_replica("op-device-b").await;

        assert_eq!(mk_a, mk_b, "the same fact converges to one merge_key across replicas");
        assert_eq!(
            mk_a,
            content_merge_key("File", "f1", "FILE_PART_OF", "Project", "p1"),
            "the stamped key is the content digest, independent of the per-device op_id"
        );
        // And it is content-specific: the key for a different membership differs.
        assert_ne!(mk_a, content_merge_key("File", "f1", "FILE_PART_OF", "Project", "p2"));
    }

    #[tokio::test]
    async fn persist_create_node_is_atomic_and_create_only() {
        let (graph, _tmp) = spawn_test_graph().await;
        // A fresh node is created.
        assert_eq!(persist_create_node(&graph, "Summary", "s1").await, "OK: created");
        // Re-creating the same id+label is a guarded no-op (never overwrites).
        assert_eq!(persist_create_node(&graph, "Summary", "s1").await, "OK: exists");
        // A DIFFERENT label at the same id (a foreign-label collision) is refused
        // by the label-agnostic probe, so no second node appears.
        assert_eq!(persist_create_node(&graph, "Project", "s1").await, "OK: exists");

        let summaries = graph
            .query_rows("MATCH (s:Summary {id: 's1'}) RETURN count(*) AS n".into())
            .await
            .unwrap();
        assert_eq!(summaries.rows[0][0].as_i64(), 1, "exactly one Summary at s1");
        let projects = graph
            .query_rows("MATCH (p:Project {id: 's1'}) RETURN count(*) AS n".into())
            .await
            .unwrap();
        assert_eq!(projects.rows[0][0].as_i64(), 0, "the foreign-label create was refused");
    }

    #[tokio::test]
    async fn persist_create_node_reserves_the_canary_namespace() {
        // The ingestion boundary refuses any id mentioning the reserved canary
        // token, whether as a prefix or embedded, so a producer can never seed the
        // namespace the agent's tripwire relies on. No node is created.
        let (graph, _tmp) = spawn_test_graph().await;
        for id in ["__canary:credentials-vault", "x__canary:embedded"] {
            assert_eq!(
                persist_create_node(&graph, "Summary", id).await,
                "ERROR: id is in the reserved canary namespace"
            );
        }
        let n = graph
            .query_rows("MATCH (s:Summary) RETURN count(*) AS n".into())
            .await
            .unwrap();
        assert_eq!(n.rows[0][0].as_i64(), 0, "no canary-id node was created");
    }

    #[tokio::test]
    async fn retracting_a_supersession_reopens_the_edge_it_replaced() {
        // §4.6: a retract is the one-unit inverse of supersession. After f1 moves
        // p1 -> p2, retracting the p2 membership must re-open p1, not leave f1 in
        // neither project.
        let (graph, _tmp) = spawn_test_graph().await;
        graph
            .write("CREATE (f:File {id: 'f1', path: '/x', app_id: 'test', last_accessed: 0})".into())
            .await
            .unwrap();
        graph.write("CREATE (p:Project {id: 'p1'})".into()).await.unwrap();
        graph.write("CREATE (p:Project {id: 'p2'})".into()).await.unwrap();
        assert_eq!(persist_relation(&graph, &file_part_of("f1", "p1"), "op-1").await, "OK: created");
        assert_eq!(persist_relation(&graph, &file_part_of("f1", "p2"), "op-2").await, "OK: created");

        // Retract the superseding p2 membership.
        assert_eq!(
            persist_retract(&graph, &file_part_of("f1", "p2"), "op-2").await,
            "OK: retracted"
        );

        // f1 is back in p1, and only p1, as one unit. The restore is an APPEND:
        // the live edge is a fresh `reopen:op-2` edge, not the original op-1 edge.
        let live = graph
            .query_rows(
                "MATCH (:File {id: 'f1'})-[r:FILE_PART_OF]->(p:Project) \
                 WHERE r.invalid_at IS NULL AND r.expired_at IS NULL RETURN p.id AS id, r.op_id AS op"
                    .into(),
            )
            .await
            .unwrap();
        assert_eq!(live.rows.len(), 1, "exactly one live membership after the undo");
        assert_eq!(live.rows[0][0].as_str(), "p1", "the superseded p1 membership is restored");
        assert_eq!(
            live.rows[0][1].as_str(),
            "reopen:op-2",
            "the restore is an appended reopen edge, not the resurrected original"
        );

        // The original op-1 edge stays CLOSED (close-never-delete): its stamps
        // were not cleared, so its supersession history is retained.
        let old = graph
            .query_rows(
                "MATCH (:File {id: 'f1'})-[r:FILE_PART_OF {op_id: 'op-1'}]->(:Project) \
                 RETURN r.invalid_at AS iv"
                    .into(),
            )
            .await
            .unwrap();
        assert_eq!(old.rows.len(), 1, "the original op-1 edge is retained");
        assert!(
            !matches!(old.rows[0][0], crate::graph::CellValue::Null),
            "the original op-1 edge stays closed, its stamps preserved"
        );

        // The reopen restores the original edge's `valid_at` (no valid-time hole
        // over the superseded period), so the two carry the same valid_at.
        let valids = graph
            .query_rows(
                "MATCH (:File {id: 'f1'})-[r:FILE_PART_OF]->(:Project {id: 'p1'}) \
                 RETURN r.op_id AS op, r.valid_at AS v ORDER BY r.op_id"
                    .into(),
            )
            .await
            .unwrap();
        assert_eq!(valids.rows.len(), 2, "p1 has the closed original and the live reopen");
        // ORDER BY op_id: 'op-1' < 'reopen:op-2'.
        assert_eq!(valids.rows[0][0].as_str(), "op-1");
        assert_eq!(valids.rows[1][0].as_str(), "reopen:op-2");
        assert_eq!(
            valids.rows[0][1].as_i64(),
            valids.rows[1][1].as_i64(),
            "the reopen carries the original membership's valid_at (valid-time continuity)"
        );

        // GD-R1: the reopen also carries the superseded membership's content
        // merge_key, so the restored live edge is content-identical to the rest of
        // the (f1 -> p1) history (a future merge sees one fact, not two).
        let keys = graph
            .query_rows(
                "MATCH (:File {id: 'f1'})-[r:FILE_PART_OF]->(:Project {id: 'p1'}) \
                 RETURN r.merge_key AS mk ORDER BY r.op_id"
                    .into(),
            )
            .await
            .unwrap();
        let want = content_merge_key("File", "f1", "FILE_PART_OF", "Project", "p1");
        assert_eq!(keys.rows[0][0].as_str(), want, "the closed original carries the content key");
        assert_eq!(
            keys.rows[1][0].as_str(),
            want,
            "the reopen carries the same content key (copied from the superseded edge)"
        );

        // A retried retract (crash-recovery / at-least-once) must NOT append a
        // second reopen edge: the live-edge guard skips the CREATE because a live
        // edge to p1 already exists, and the close is an idempotent `absent`.
        assert_eq!(
            persist_retract(&graph, &file_part_of("f1", "p2"), "op-2").await,
            "OK: absent"
        );
        let still = graph
            .query_rows(
                "MATCH (:File {id: 'f1'})-[r:FILE_PART_OF]->(:Project {id: 'p1'}) \
                 WHERE r.invalid_at IS NULL AND r.expired_at IS NULL RETURN count(*) AS n"
                    .into(),
            )
            .await
            .unwrap();
        assert_eq!(
            still.rows[0][0].as_i64(),
            1,
            "the reopen is idempotent: a retried retract appends no duplicate"
        );
    }

    #[tokio::test]
    async fn persist_retract_closes_only_the_op_id_edge() {
        let (graph, _tmp) = spawn_test_graph().await;
        graph
            .write("CREATE (f:File {id: 'f1', path: '/x', app_id: 'test', last_accessed: 0})".into())
            .await
            .unwrap();
        graph
            .write("CREATE (p:Project {id: 'p1'})".into())
            .await
            .unwrap();

        // Create the edge under op-1.
        assert_eq!(persist_relation(&graph, &file_part_of("f1", "p1"), "op-1").await, "OK: created");

        // A retract under a *different* op_id matches nothing: the edge is the
        // caller's own only when the op_id matches, so this is an idempotent
        // no-op and the edge survives.
        let miss = persist_retract(&graph, &file_part_of("f1", "p1"), "op-other").await;
        assert_eq!(miss, "OK: absent", "a non-matching op_id retracts nothing");
        let rows = graph
            .query_rows(
                "MATCH (:File {id: 'f1'})-[:FILE_PART_OF]->(:Project {id: 'p1'}) RETURN count(*) AS n".into(),
            )
            .await
            .unwrap();
        assert_eq!(rows.rows[0][0].as_i64(), 1, "the edge survives a wrong-op retract");

        // The matching op_id closes exactly that edge.
        let hit = persist_retract(&graph, &file_part_of("f1", "p1"), "op-1").await;
        assert_eq!(hit, "OK: retracted", "the owning op_id closes its edge");
        // The edge is RETAINED (closed, not deleted): the row still exists for
        // audit, but it is no longer live.
        let total = graph
            .query_rows(
                "MATCH (:File {id: 'f1'})-[:FILE_PART_OF]->(:Project {id: 'p1'}) RETURN count(*) AS n".into(),
            )
            .await
            .unwrap();
        assert_eq!(total.rows[0][0].as_i64(), 1, "the closed edge is retained for audit");
        let live = graph
            .query_rows(
                "MATCH (:File {id: 'f1'})-[r:FILE_PART_OF]->(:Project {id: 'p1'}) \
                 WHERE r.invalid_at IS NULL AND r.expired_at IS NULL RETURN count(*) AS n"
                    .into(),
            )
            .await
            .unwrap();
        assert_eq!(live.rows[0][0].as_i64(), 0, "no live edge remains after the retract");

        // Retracting again is an idempotent success (no live edge with this op_id).
        let again = persist_retract(&graph, &file_part_of("f1", "p1"), "op-1").await;
        assert_eq!(again, "OK: absent", "a repeat retract is an idempotent no-op");
    }

    #[tokio::test]
    async fn persist_retract_refuses_a_relation_without_op_id_column() {
        let (graph, _tmp) = spawn_test_graph().await;
        // ACCESSED_BY carries no op_id column, so it has no precise per-operation
        // key; a retract of it must be refused rather than risk a bare delete.
        let rel = RelationResult {
            from_type: "system.File".into(),
            from_id: "f1".into(),
            to_type: "system.App".into(),
            to_id: "a1".into(),
            relation_type: "ACCESSED_BY".into(),
        };
        let resp = persist_retract(&graph, &rel, "op-1").await;
        assert_eq!(resp, "ERROR: relation does not support op-id retract");
    }

    const VALID_REL_BODY: &str = r#"{"op":"create_relation","from_type":"system.File","from_id":"f1","to_type":"system.Project","to_id":"p1","relation_type":"FILE_PART_OF"}"#;

    #[tokio::test]
    async fn a_refused_write_records_no_capability_use() {
        let (graph, _tmp) = spawn_test_graph().await;
        let (pool, _pool_tmp) = spawn_test_pool().await;
        let gate = crate::activity_delete::promotion_gate();
        let auth = Arc::new(Mutex::new(Authenticator::new()));
        let registry = SchemaRegistry::new(vec![]);
        // A tally whose interval has ALREADY elapsed, so any recorded use would
        // flush on this very call and leave a node behind. Nothing may.
        let uses = Arc::new(Mutex::new(crate::lcg::UseTally::new(
            crate::time::now().0 - crate::lcg::USE_FLUSH_INTERVAL_MICROS,
        )));
        let peer = WritePeer {
            pid: std::process::id(),
            // SAFETY: getuid never fails.
            uid: unsafe { libc::getuid() },
            start_time: Some(0),
        };

        let resp = handle_write_request(
            VALID_REL_BODY.as_bytes(),
            Some(peer),
            &registry,
            &graph,
            &pool,
            &gate,
            &auth,
            None,
            Some(&uses),
        )
        .await;
        assert!(resp.starts_with("ERROR:"), "the reuse guard refuses this write: {resp}");

        let rows = graph
            .query_rows_json("MATCH (u:CapabilityUse) RETURN count(*)".to_string())
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&rows).unwrap();
        assert_eq!(
            parsed["rows"][0][0], 0,
            "a denial is not use; counting it would hide an unused grant from the revoke prompt"
        );
    }

    #[tokio::test]
    async fn write_rejects_recycled_pid() {
        let (graph, _tmp) = spawn_test_graph().await;
        let (pool, _pool_tmp) = spawn_test_pool().await;
        let gate = crate::activity_delete::promotion_gate();
        let auth = Arc::new(Mutex::new(Authenticator::new()));
        let registry = SchemaRegistry::new(vec![]);
        // A start time that cannot match the live process: the reuse guard must
        // fire before any token issuance or graph write.
        let peer = WritePeer {
            pid: std::process::id(),
            // SAFETY: getuid never fails.
            uid: unsafe { libc::getuid() },
            start_time: Some(0),
        };
        let resp =
            handle_write_request(
                VALID_REL_BODY.as_bytes(),
                Some(peer),
                &registry,
                &graph,
                &pool,
                &gate,
                &auth,
                None,
                None,
            )
            .await;
        assert_eq!(resp, "ERROR: peer process changed since connection");
    }

    #[tokio::test]
    async fn write_rejects_unverifiable_peer() {
        let (graph, _tmp) = spawn_test_graph().await;
        let (pool, _pool_tmp) = spawn_test_pool().await;
        let gate = crate::activity_delete::promotion_gate();
        let auth = Arc::new(Mutex::new(Authenticator::new()));
        let registry = SchemaRegistry::new(vec![]);
        // No captured start time: reuse cannot be guarded, so fail closed.
        let peer = WritePeer {
            pid: std::process::id(),
            // SAFETY: getuid never fails.
            uid: unsafe { libc::getuid() },
            start_time: None,
        };
        let resp =
            handle_write_request(
                VALID_REL_BODY.as_bytes(),
                Some(peer),
                &registry,
                &graph,
                &pool,
                &gate,
                &auth,
                None,
                None,
            )
            .await;
        assert_eq!(resp, "ERROR: write requires a verifiable peer process");
    }

    #[tokio::test]
    async fn write_rejects_absent_peer_and_malformed_body() {
        let (graph, _tmp) = spawn_test_graph().await;
        let (pool, _pool_tmp) = spawn_test_pool().await;
        let gate = crate::activity_delete::promotion_gate();
        let auth = Arc::new(Mutex::new(Authenticator::new()));
        let registry = SchemaRegistry::new(vec![]);

        let no_peer =
            handle_write_request(
                VALID_REL_BODY.as_bytes(), None, &registry, &graph, &pool, &gate, &auth, None, None,
            )
                .await;
        assert_eq!(no_peer, "ERROR: write requires a resolvable peer process");

        // A malformed body is rejected before the peer is even consulted.
        let bad = handle_write_request(b"not json", None, &registry, &graph, &pool, &gate, &auth, None, None)
            .await;
        assert!(bad.starts_with("ERROR: malformed write request"), "got: {bad}");
    }

    #[tokio::test]
    async fn persist_relation_reports_absent_endpoint() {
        let (graph, _tmp) = spawn_test_graph().await;
        graph
            .write("CREATE (f:File {id: 'f1', path: '/x', app_id: 'test', last_accessed: 0})".into())
            .await
            .unwrap();
        // No Project node exists, so the MATCH binds nothing and the checked
        // persistence must report not-found rather than a silent success.
        let resp = persist_relation(&graph, &file_part_of("f1", "missing"), "").await;
        assert_eq!(resp, "ERROR: relation endpoints not found");
    }

    #[test]
    fn arlen_default_throttles_apps_but_not_the_ai_daemon() {
        // A normal (ThirdParty) caller bursting past its 200-token
        // burst is rate-limited; the AI daemon's higher FirstParty
        // limit (2000 burst) is not tripped by the same burst.
        let mut rs = RateState::new();

        let mut app_limited = false;
        for _ in 0..201 {
            if rs.limiter.check_query("com.test").is_err() {
                app_limited = true;
                break;
            }
        }
        assert!(app_limited, "a normal app bursting past 200 must be RateLimited");

        let mut ai_limited = false;
        for _ in 0..201 {
            if rs.limiter.check_query("ai-daemon").is_err() {
                ai_limited = true;
                break;
            }
        }
        assert!(!ai_limited, "the AI daemon's higher tier must not be tripped at 201");
    }
}

#[cfg(test)]
mod socket_exposure_tests {
    use super::{apply_socket_exposure, socket_exposure, SocketExposure};
    use std::os::unix::fs::PermissionsExt;

    /// Naming an owner narrows the door; naming none leaves the peer check alone.
    #[test]
    fn an_owner_narrows_the_socket_and_no_owner_leaves_it_shared() {
        assert_eq!(socket_exposure(Some(1000), 0), SocketExposure::OwnerOnly(1000));
        // Owner and daemon being the same uid still narrows: the point is that
        // nobody ELSE may connect, not that the daemon is not the owner.
        assert_eq!(socket_exposure(Some(1000), 1000), SocketExposure::OwnerOnly(1000));
        assert_eq!(socket_exposure(None, 0), SocketExposure::Shared);
    }

    fn mode_of(path: &std::path::Path) -> u32 {
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    /// The shared case is the old behaviour, unchanged: a cross-uid first-party
    /// client must be able to connect, and `bind` under a 0022 umask would not let
    /// it.
    #[test]
    fn a_shared_socket_stays_connectable_by_anyone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shared.sock");
        let _l = std::os::unix::net::UnixListener::bind(&path).unwrap();
        apply_socket_exposure(path.to_str().unwrap(), SocketExposure::Shared).unwrap();
        assert_eq!(mode_of(&path), 0o666);
    }

    /// Narrowed to its owner, and the owner can still reach it. Chowning to one's
    /// own uid is permitted, so this runs unprivileged and still proves the pair
    /// that matters: the mode is owner-only AND a connect from the owner works.
    #[test]
    fn an_owned_socket_is_owner_only_and_the_owner_can_still_connect() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("owned.sock");
        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        // SAFETY: getuid() has no preconditions and cannot fail.
        let us = unsafe { libc::getuid() };
        apply_socket_exposure(path.to_str().unwrap(), SocketExposure::OwnerOnly(us)).unwrap();
        assert_eq!(mode_of(&path), 0o600, "a named owner must be the only one who may connect");
        std::os::unix::net::UnixStream::connect(&path).expect("the owner must still connect");
        drop(listener);
    }

    /// A chown that cannot be done leaves the socket connectable rather than
    /// stranding the owner behind a door the daemon could not hand over. It says so
    /// in the log; what it must not do is leave 0600 owned by the wrong uid, which
    /// would present as narrowed while refusing the very peer it was narrowed for.
    #[test]
    fn a_failed_handover_widens_back_rather_than_stranding_the_owner() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("unowned.sock");
        let _l = std::os::unix::net::UnixListener::bind(&path).unwrap();
        // Root can chown to anyone, so the failure path is only reachable
        // unprivileged - which is where the test suite runs.
        // SAFETY: getuid() has no preconditions and cannot fail.
        if unsafe { libc::getuid() } == 0 {
            return;
        }
        let someone_else = 65534; // nobody
        apply_socket_exposure(path.to_str().unwrap(), SocketExposure::OwnerOnly(someone_else))
            .unwrap();
        assert_eq!(mode_of(&path), 0o666, "a failed handover must not look like a narrowed socket");
    }
}


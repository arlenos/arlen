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
use crate::utils::escape_cypher;
use crate::write::{create_relation, retract_relation, RelationResult};

/// Producer socket default (overridable via `ARLEN_PRODUCER_SOCKET`).
const DEFAULT_PRODUCER_SOCKET: &str = "/run/arlen/event-bus-producer.sock";

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
        let path = std::env::var("ARLEN_PRODUCER_SOCKET")
            .unwrap_or_else(|_| DEFAULT_PRODUCER_SOCKET.to_string());
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
            session_id: "knowledge-daemon".to_string(),
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
pub async fn listen(socket_path: &str, graph: GraphHandle, pool: sqlx::SqlitePool) -> Result<()> {
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

    // Schema registry for write-mode relation validation. Built with the
    // compiled-in system entity types only; that is sufficient for the agent's
    // built-in system relations (the only write op today), since
    // `create_relation` refuses anything outside the built-in allowlist anyway.
    // Loading app-defined schemas for app-relation writes is a follow-up.
    let registry = Arc::new(SchemaRegistry::new(vec![]));

    tokio::try_join!(
        listen_queries(socket_path, graph.clone(), pool, auth.clone(), rate, emitter, registry),
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
    auth: Arc<Mutex<Authenticator>>,
    rate: Arc<Mutex<RateState>>,
    emitter: Arc<RateLimitEmitter>,
    registry: Arc<SchemaRegistry>,
) -> Result<()> {
    if Path::new(socket_path).exists() {
        std::fs::remove_file(socket_path)?;
    }
    if let Some(parent) = Path::new(socket_path).parent() {
        std::fs::create_dir_all(parent)?;
    }

    let listener = UnixListener::bind(socket_path)?;
    info!(socket = socket_path, "graph daemon listening");

    // SAFETY: getuid() has no preconditions and cannot fail.
    let our_uid = unsafe { libc::getuid() };

    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                let graph = graph.clone();
                let pool = pool.clone();
                let auth = auth.clone();
                let rate = rate.clone();
                let emitter = emitter.clone();
                let registry = registry.clone();
                tokio::spawn(async move {
                    if let Err(e) =
                        handle_client(stream, graph, pool, auth, rate, emitter, registry, our_uid)
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
    let app_esc = escape_cypher(app_id);
    if let Err(e) = graph
        .write(format!(
            "MATCH (g:Grant {{app_id: '{app_esc}'}}) WHERE g.live SET g.live = false"
        ))
        .await
    {
        warn!("marking {app_id} grants stale failed: {e}");
    }
}

/// Process a graph-relevant event.
async fn handle_graph_event(
    auth: &Arc<Mutex<Authenticator>>,
    graph: &GraphHandle,
    event: GraphEvent,
) {
    match event {
        GraphEvent::PermissionChanged { app_id } => {
            info!("permission changed for {app_id}, invalidating token");
            auth.lock().await.invalidate(&app_id);
            // The app's prior live reach no longer verifies; reflect that in the
            // projection alongside the token invalidation (§4.2).
            mark_app_grants_stale(graph, &app_id).await;
        }
        GraphEvent::AiLevelChanged => {
            info!("AI level changed, invalidating ai-daemon token");
            auth.lock().await.invalidate("ai-daemon");
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
}

/// The node types creatable via the `0x02` write socket: the consolidation node
/// types only (§5.3). A narrow allowlist, like `BUILTIN_RELATIONS` for edges, so
/// a token write-scope alone cannot create an arbitrary node label.
const CREATABLE_NODES: &[&str] = &["system.Summary"];

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
fn readable_system_labels(read_scopes: &[crate::token::EntityScope]) -> Vec<String> {
    read_scopes
        .iter()
        .filter_map(|s| s.entity_type.strip_prefix("system."))
        .filter(|l| !l.is_empty() && l.chars().all(|c| c.is_ascii_alphanumeric()))
        .map(str::to_string)
        .collect()
}

/// The uniform denial for the provenance read op. Every failure mode (malformed
/// request, no or changed peer, out-of-scope, absent object) returns this one
/// shape so a spoofed reference cannot become a file-existence oracle by shape;
/// the caller pairs it with `timing_noise()` so timing cannot either.
const PROVENANCE_OUT_OF_SCOPE: &str = "ERROR: OutOfScope";

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
    let token = match auth.lock().await.issue_token_for_pid(peer.pid) {
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
    let id_esc = escape_cypher(&req.object_id);
    let mut found_label: Option<String> = None;
    for label in readable_system_labels(&token.read_scopes) {
        let cypher = format!("MATCH (n:{label} {{id: '{id_esc}'}}) RETURN n.id LIMIT 1");
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
        format!("MATCH (n:{label} {{id: '{id_esc}'}})-[:ACCESSED_BY]->(a) RETURN a.id AS id");
    let actors: Vec<String> = match graph.query_rows(cypher).await {
        Ok(rs) => rs
            .rows
            .iter()
            .filter_map(|r| r.first().map(|c| c.as_str().to_string()))
            .collect(),
        Err(_) => return PROVENANCE_OUT_OF_SCOPE.to_string(),
    };
    let (visible, accessed_by_others) = co_tenant_filter(&actors, &token.app_id);
    serde_json::json!({ "actors": visible, "accessed_by_others": accessed_by_others }).to_string()
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
    reach: Vec<String>,
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
/// whole-system view. The privileged check is `is_privileged_authority_reader`,
/// false for every caller until F3, so today no caller can enumerate another's
/// authority through this op (the §5 leak the dedicated reader exists to prevent).
///
/// `live` is recomputed fresh from `process_alive(g.pid)`: a node stored live but
/// whose process is gone renders not-live, so the flag never lies beyond the read
/// instant (§4.2). The general read path already denies the `Grant` label, so this
/// is the only way these nodes are ever served.
async fn handle_access_grants(app_id: &str, graph: &GraphHandle) -> String {
    let privileged = is_privileged_authority_reader(app_id);
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
         g.live, g.revoked, g.superseded, g.pid, g.issued_at, t.label \
         LIMIT {ACCESS_GRANTS_ROW_CAP}"
    );
    let rows = match graph.query_rows(cypher).await {
        Ok(rs) => rs.rows,
        Err(_) => return ACCESS_GRANTS_ERROR.to_string(),
    };

    let mut views: Vec<GrantView> = Vec::new();
    let mut index: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for row in rows {
        if row.len() < 11 {
            continue;
        }
        let id = row[0].as_str().to_string();
        let slot = *index.entry(id.clone()).or_insert_with(|| {
            let revoked = row[6].as_bool();
            let superseded = row[7].as_bool();
            let stored_live = row[5].as_bool();
            let pid = row[8].as_i64();
            // Fresh liveness: a stored-live grant renders live only if its process
            // is still alive (death caught at read time) AND it is neither revoked
            // nor superseded (defensive: the active-reach flag is correct in the
            // reader, not only by emitter discipline). `try_from` keeps an
            // out-of-range or negative stored pid from wrapping.
            let live = stored_live
                && !revoked
                && !superseded
                && u32::try_from(pid).map(process_alive).unwrap_or(false);
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
                reach: Vec::new(),
            });
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
    }
    serde_json::to_string(&views).unwrap_or_else(|_| "[]".to_string())
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
/// development. Cross-uid peers are already rejected at connection.
async fn handle_write_request(
    body: &[u8],
    peer: Option<WritePeer>,
    registry: &SchemaRegistry,
    graph: &GraphHandle,
    auth: &Arc<Mutex<Authenticator>>,
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
    let token = match auth.lock().await.issue_token_for_pid(peer.pid) {
        Ok(t) => t,
        Err(e) => return format!("ERROR: {e}"),
    };

    // Living Capability Graph (living-capability-graph.md §4.1): project this
    // graph-access peer's minted token into the browse graph. The auth lock is
    // already released; failure is logged and swallowed so a graph hiccup never
    // fails a validly-issued token (the projection degrades, the write does not).
    if let Err(e) = crate::lcg::emit_grant_node(graph, &token).await {
        tracing::warn!("emit_grant_node failed (capability projection degraded): {e}");
    }

    match req {
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
            let label = node_type.strip_prefix("system.").unwrap_or(&node_type);
            persist_create_node(graph, label, &id).await
        }
    }
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
        return persist_file_part_of(graph, from_label, to_label, &from_id, &to_id, op_id).await;
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
) -> String {
    let now = crate::time::now().0;
    let op_prop = if op_id.is_empty() {
        String::new()
    } else {
        format!("op_id: '{}', ", escape_cypher(op_id))
    };
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
/// live edge that carries this `op_id`, re-opening the edge it superseded if any
/// (the one-unit inverse of supersession, §4.6).
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
    // than vanishing. The `MATCH` carries the liveness predicate, so `closed`
    // counts only edges that were live before this call (a `SET` over an already-
    // closed edge would otherwise inflate the count); R0 confirmed
    // `SET ... RETURN count(*)` counts matched-and-mutated rows.
    //
    // A retract is the inverse of supersession as ONE unit (§4.6): if the edge
    // being closed superseded an earlier one (it carries `superseded = old.op_id`),
    // re-open that earlier edge so the file returns to its pre-supersession
    // membership rather than landing in neither project. A first assertion (no
    // `superseded`) closes its edge alone (the `OPTIONAL MATCH` binds nothing).
    //
    // Endpoint labels and the relation type are validated identifiers; only the
    // ids and op_id are caller-supplied and escaped into the literals. `now` is
    // the server clock at the close (a server-computed i64, safe to interpolate).
    let now = crate::time::now().0;
    let cypher = format!(
        "MATCH (a:{from_label} {{id: '{from_id}'}})-[r:{rel_type} {{op_id: '{op}'}}]->(b:{to_label} {{id: '{to_id}'}}) \
         WHERE r.invalid_at IS NULL AND r.expired_at IS NULL \
         SET r.invalid_at = {now}, r.expired_at = {now} \
         WITH a, r \
         OPTIONAL MATCH (a)-[old:FILE_PART_OF]->(:Project) \
           WHERE r.superseded IS NOT NULL AND old.op_id = r.superseded \
         SET old.invalid_at = NULL, old.expired_at = NULL \
         RETURN count(*) AS closed"
    );
    match graph.query_rows(cypher).await {
        Ok(rs) if row_count(&rs) > 0 => "OK: retracted".to_string(),
        Ok(_) => "OK: absent".to_string(),
        Err(e) => format!("ERROR: {e}"),
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

/// Handle a single client connection.
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
    auth: Arc<Mutex<Authenticator>>,
    rate: Arc<Mutex<RateState>>,
    emitter: Arc<RateLimitEmitter>,
    registry: Arc<SchemaRegistry>,
    our_uid: u32,
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
            if uid != our_uid {
                warn!(peer_uid = uid, "graph daemon: rejecting cross-uid client");
                return Ok(());
            }
            if pid > 0 {
                let pid = pid as u32;
                let id = app_id_from_pid(pid).unwrap_or_else(|_| "unknown".to_string());
                let start_time = pid_start_time(pid).ok();
                (id, Some(WritePeer { pid, start_time }))
            } else {
                ("unknown".to_string(), None)
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
            let minted = auth.lock().await.issue_token_for_pid(p.pid).ok();
            if let Some(token) = minted {
                if let Err(e) = crate::lcg::emit_grant_node(&graph, &token).await {
                    warn!(app_id = %app_id, "connect-time grant emit failed: {e}");
                }
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
                    handle_write_request(body, peer, &registry, &graph, &auth).await,
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
                                serde_json::to_string(&ids).unwrap_or_else(|_| "[]".to_string())
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
                    handle_provenance_read(&buf[1..], &peer, &auth, &graph),
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
        } else {
            // Bounded *client wait*: the connection returns QueryTimeout
            // after 500 ms (foundation §8.4) so the caller is never
            // stuck. Because the graph enqueue now yields under
            // backpressure rather than blocking a worker (see graph.rs),
            // this deadline also covers queue admission. NB it unblocks
            // the caller; it does not abort the graph worker's in-flight
            // query, which runs to completion — a true execution deadline
            // needs an interruptible graph API and is a follow-up.
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
    const WRITE_KEYWORDS: [&str; 15] = [
        "CREATE", "MERGE", "DELETE", "SET", "REMOVE", "DROP", "DETACH", "ALTER",
        "ATTACH", "USE", "COPY", "LOAD", "INSTALL", "EXPORT", "IMPORT",
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

#[cfg(test)]
mod tests {
    use super::*;

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

        // f1 is back in p1 (re-opened), and only p1, as one unit.
        let live = graph
            .query_rows(
                "MATCH (:File {id: 'f1'})-[r:FILE_PART_OF]->(p:Project) \
                 WHERE r.invalid_at IS NULL AND r.expired_at IS NULL RETURN p.id AS id"
                    .into(),
            )
            .await
            .unwrap();
        assert_eq!(live.rows.len(), 1, "exactly one live membership after the undo");
        assert_eq!(live.rows[0][0].as_str(), "p1", "the superseded p1 membership is re-opened");
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
    async fn write_rejects_recycled_pid() {
        let (graph, _tmp) = spawn_test_graph().await;
        let auth = Arc::new(Mutex::new(Authenticator::new()));
        let registry = SchemaRegistry::new(vec![]);
        // A start time that cannot match the live process: the reuse guard must
        // fire before any token issuance or graph write.
        let peer = WritePeer {
            pid: std::process::id(),
            start_time: Some(0),
        };
        let resp =
            handle_write_request(VALID_REL_BODY.as_bytes(), Some(peer), &registry, &graph, &auth)
                .await;
        assert_eq!(resp, "ERROR: peer process changed since connection");
    }

    #[tokio::test]
    async fn write_rejects_unverifiable_peer() {
        let (graph, _tmp) = spawn_test_graph().await;
        let auth = Arc::new(Mutex::new(Authenticator::new()));
        let registry = SchemaRegistry::new(vec![]);
        // No captured start time: reuse cannot be guarded, so fail closed.
        let peer = WritePeer {
            pid: std::process::id(),
            start_time: None,
        };
        let resp =
            handle_write_request(VALID_REL_BODY.as_bytes(), Some(peer), &registry, &graph, &auth)
                .await;
        assert_eq!(resp, "ERROR: write requires a verifiable peer process");
    }

    #[tokio::test]
    async fn write_rejects_absent_peer_and_malformed_body() {
        let (graph, _tmp) = spawn_test_graph().await;
        let auth = Arc::new(Mutex::new(Authenticator::new()));
        let registry = SchemaRegistry::new(vec![]);

        let no_peer =
            handle_write_request(VALID_REL_BODY.as_bytes(), None, &registry, &graph, &auth).await;
        assert_eq!(no_peer, "ERROR: write requires a resolvable peer process");

        // A malformed body is rejected before the peer is even consulted.
        let bad = handle_write_request(b"not json", None, &registry, &graph, &auth).await;
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

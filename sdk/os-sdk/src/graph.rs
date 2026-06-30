use std::collections::HashMap;
use std::future::Future;
use std::path::Path;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::sync::Mutex;

/// Error type for graph query failures.
#[derive(Debug)]
pub enum QueryError {
    /// The connection to the Graph Daemon could not be established or was lost.
    ConnectionFailed(String),
    /// The Cypher query was rejected by the daemon (syntax error or write attempt).
    InvalidQuery(String),
    /// The caller does not have permission to access the requested data.
    PermissionDenied,
}

impl std::fmt::Display for QueryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            QueryError::ConnectionFailed(msg) => write!(f, "connection failed: {msg}"),
            QueryError::InvalidQuery(msg) => write!(f, "invalid query: {msg}"),
            QueryError::PermissionDenied => write!(f, "permission denied"),
        }
    }
}

impl std::error::Error for QueryError {}

/// The caller-scoped provenance of one graph object: which apps accessed it,
/// filtered to the caller's own identity (a co-tenant is never named, only
/// summarised by [`accessed_by_others`](ProvenanceView::accessed_by_others)).
/// Returned by [`UnixGraphClient::read_provenance`] when the object is within the
/// caller's read scope.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ProvenanceView {
    /// The actor app ids the caller may see (its own identity), never a co-tenant.
    pub actors: Vec<String>,
    /// Whether a foreign actor also accessed the object, without naming it.
    pub accessed_by_others: bool,
}

/// One capability grant in the Living Capability Graph browse surface, as served
/// by [`UnixGraphClient::access_grants`]. Mirrors the daemon's projection: the
/// declared ceiling (faithful scope JSON), the queryable type `reach`, and the
/// lifecycle flags. `live` is resolved fresh by the daemon at read time.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GrantView {
    /// The grant id (the projected token's UUIDv7).
    pub id: String,
    /// The principal the grant belongs to.
    pub app_id: String,
    /// The declared capability ceiling, as canonical scope JSON.
    pub declared_ceiling: String,
    /// Whether this reach was declared essential at enroll.
    pub required: bool,
    /// Whether the app identity is verified (false while it rests on the
    /// spoofable app-id resolution, the F3 caveat).
    pub identity_verified: bool,
    /// Whether the projected token still verifies (resolved fresh at read time).
    pub live: bool,
    /// Whether the user severed this reach.
    pub revoked: bool,
    /// Whether a fresher mint replaced this node.
    pub superseded: bool,
    /// When the grant was issued (epoch micros).
    pub issued_at: i64,
    /// The entity types this grant can reach (the queryable projection).
    pub reach: Vec<String>,
    /// The grant kind: `capability-token` or `consent` (system-dialog-plan.md
    /// Option A); an older daemon omits it (defaults to empty = capability-token).
    #[serde(default)]
    pub source: String,
    /// The consent class, when `source == "consent"` (else empty).
    #[serde(default)]
    pub consent_class: String,
    /// The concrete consent scope, when `source == "consent"` (else empty).
    #[serde(default)]
    pub consent_scope: String,
}

/// Largest response frame the client will allocate for the legacy text path.
/// Generous, because a display query can legitimately return a large blob;
/// the limit only exists so a buggy or compromised daemon cannot exhaust
/// client memory by advertising a multi-gigabyte frame.
const MAX_RESPONSE_BYTES: usize = 64 * 1024 * 1024;

/// Largest response frame the client will allocate for the typed path. Tighter
/// than [`MAX_RESPONSE_BYTES`] and matched to the daemon's own typed payload
/// cap (a few MiB of cell content plus JSON structure), so a bounded frame
/// cannot be parsed into a much larger tree of values.
const MAX_TYPED_RESPONSE_BYTES: usize = 16 * 1024 * 1024;

/// Largest response frame the client will allocate for a write request. The
/// daemon answers a write with a tiny plaintext status (`OK` / `ERROR: ...`),
/// so this is deliberately small.
const MAX_WRITE_RESPONSE_BYTES: usize = 64 * 1024;

/// Largest column count the client will accept in a typed result, matching the
/// daemon's own cap. Bounds work before any per-row map is built.
const MAX_TYPED_COLUMNS: usize = 256;

/// Largest row count the client will accept in a typed result, matching the
/// daemon's own cap. Without this an under-frame-limit body of many empty rows
/// could still be amplified into one map allocation per row.
const MAX_TYPED_ROWS: usize = 10_000;

/// Largest length, in bytes, of a single column name. Graph column names and
/// aliases are short identifiers, so this is generous for real queries; the
/// cap exists because each cell clones its column name into the row map, so an
/// uncapped name length multiplied by the cell budget would be a key-clone
/// amplification vector. With this and [`MAX_TYPED_CELLS`] the total cloned
/// key bytes are bounded.
const MAX_TYPED_COLUMN_NAME_BYTES: usize = 128;

/// Largest total cell count (`columns x rows`) the client will materialize.
/// The row and column caps alone permit a daemon-legal worst case of
/// `10_000 x 256` cells, which fits well under the frame cap yet would build
/// millions of map entries. This bound matches the daemon's own typed payload
/// budget (a 4 MiB cost cap at a minimum 8 bytes per cell), so a well-behaved
/// daemon is never rejected while a malicious one cannot amplify a bounded
/// frame into a large heap of per-cell allocations.
const MAX_TYPED_CELLS: usize = 4 * 1024 * 1024 / 8;

/// Internal result of a single framed round trip, kept separate from
/// [`QueryError`] so the retry loop can distinguish a fatal framing error (do
/// not retry, the socket is desynchronised) from a transient I/O error.
enum FrameError {
    /// The daemon advertised a response larger than [`MAX_RESPONSE_BYTES`].
    Oversized(usize),
    /// A socket read or write failed.
    Io(std::io::Error),
}

impl From<std::io::Error> for FrameError {
    fn from(e: std::io::Error) -> Self {
        FrameError::Io(e)
    }
}

/// Executes read-only Cypher queries against the Arlen Knowledge Graph.
///
/// Implemented by [`UnixGraphClient`] for production use and by
/// [`crate::mock::MockGraphClient`] for testing.
pub trait GraphClient: Send + Sync {
    /// Execute a read-only Cypher query and return the results.
    ///
    /// Each row in the result is a map of column name to JSON value.
    /// Write queries (CREATE, MERGE, DELETE, SET, REMOVE, DROP) are rejected
    /// by the daemon with [`QueryError::InvalidQuery`].
    ///
    /// # Errors
    /// Returns [`QueryError::ConnectionFailed`] if the Knowledge Daemon is unreachable.
    /// Returns [`QueryError::InvalidQuery`] if the query is malformed or a write query.
    /// Returns [`QueryError::PermissionDenied`] if the capability token is insufficient.
    fn query<'a>(
        &'a self,
        cypher: &'a str,
        params: HashMap<String, serde_json::Value>,
    ) -> impl Future<Output = Result<Vec<HashMap<String, serde_json::Value>>, QueryError>> + Send + 'a;
}

/// Production [`GraphClient`] that queries the Knowledge Graph over a Unix socket.
///
/// Connects lazily on first query and reconnects automatically if the connection
/// is lost. Thread-safe: clone freely across async tasks.
///
/// # Example
/// ```no_run
/// use os_sdk::graph::{GraphClient, UnixGraphClient};
/// use std::collections::HashMap;
///
/// #[tokio::main]
/// async fn main() {
///     let client = UnixGraphClient::new("/run/arlen/knowledge.sock");
///     let rows = client
///         .query("MATCH (f:File) RETURN f.path LIMIT 10", HashMap::new())
///         .await
///         .unwrap();
///     for row in rows {
///         println!("{row:?}");
///     }
/// }
/// ```
#[derive(Clone)]
pub struct UnixGraphClient {
    socket_path: String,
    stream: Arc<Mutex<Option<UnixStream>>>,
}

impl UnixGraphClient {
    /// Create a new client that will connect to the given socket path.
    ///
    /// Does not connect immediately; the connection is established on the
    /// first call to [`query`](GraphClient::query).
    pub fn new(socket_path: impl Into<String>) -> Self {
        Self {
            socket_path: socket_path.into(),
            stream: Arc::new(Mutex::new(None)),
        }
    }

    /// Send a length-framed request body and read the length-framed response
    /// as **raw bytes**, reconnecting once on a dropped connection. Shared by
    /// the text and typed-row query paths.
    ///
    /// The response length the daemon advertises is rejected before any
    /// allocation if it exceeds `max_bytes`, so a buggy or compromised daemon
    /// cannot OOM the caller with an oversized frame. The caller passes the
    /// limit appropriate to its path (tighter for typed rows than for display
    /// text). Decoding (lossy for display text, strict for typed rows) is left
    /// to the caller.
    async fn round_trip(&self, body: &[u8], max_bytes: usize) -> Result<Vec<u8>, QueryError> {
        let len = u32::try_from(body.len())
            .map_err(|e| QueryError::InvalidQuery(e.to_string()))?
            .to_be_bytes();

        let mut guard = self.stream.lock().await;
        for attempt in 0..2u8 {
            // Take the stream OUT of the mutex before any I/O. If this future
            // is cancelled mid-round-trip (e.g. a caller timeout) after the
            // request is written but before the response is read, the
            // half-used stream is dropped rather than left cached with a
            // pending response that the next query would misread as its own.
            let mut stream = match guard.take() {
                Some(s) => s,
                None => match UnixStream::connect(Path::new(&self.socket_path)).await {
                    Ok(s) => s,
                    Err(e) => return Err(QueryError::ConnectionFailed(e.to_string())),
                },
            };
            let result = async {
                stream.write_all(&len).await?;
                stream.write_all(body).await?;
                let mut resp_len_buf = [0u8; 4];
                stream.read_exact(&mut resp_len_buf).await?;
                let resp_len = u32::from_be_bytes(resp_len_buf) as usize;
                if resp_len > max_bytes {
                    // Reject before allocating. This is a framing/trust error,
                    // not a transient I/O error, so do not retry: the socket
                    // is desynchronised (the oversized body is still pending).
                    return Err(FrameError::Oversized(resp_len));
                }
                let mut resp_buf = vec![0u8; resp_len];
                stream
                    .read_exact(&mut resp_buf)
                    .await
                    .map_err(FrameError::Io)?;
                Ok::<_, FrameError>(resp_buf)
            }
            .await;

            match result {
                // Cache the stream again only after a complete round trip.
                Ok(response) => {
                    *guard = Some(stream);
                    return Ok(response);
                }
                // An oversized frame is fatal: the stream is dropped (not
                // cached) and we do not retry it as a connection blip.
                Err(FrameError::Oversized(n)) => {
                    return Err(QueryError::InvalidQuery(format!(
                        "daemon response of {n} bytes exceeds the {max_bytes}-byte limit"
                    )));
                }
                // On an I/O error the stream is dropped (not cached); retry once.
                Err(FrameError::Io(_)) if attempt == 0 => {}
                Err(FrameError::Io(e)) => return Err(QueryError::ConnectionFailed(e.to_string())),
            }
        }
        Err(QueryError::ConnectionFailed("failed after reconnect".to_string()))
    }

    /// Map a daemon `ERROR:` response to a [`QueryError`].
    fn check_error(response: &str) -> Result<(), QueryError> {
        if let Some(rest) = response.strip_prefix("ERROR:") {
            let msg = rest.trim().to_string();
            if msg.contains("permission") {
                return Err(QueryError::PermissionDenied);
            }
            return Err(QueryError::InvalidQuery(msg));
        }
        Ok(())
    }

    /// Execute a read-only Cypher query and return **typed** rows.
    ///
    /// Unlike [`GraphClient::query`], which surfaces the daemon's
    /// pipe-delimited result text as a single `result` value (lossy: a value
    /// containing `|` or a newline corrupts it), this requests the daemon's
    /// structured JSON `RowSet` mode and returns one map per row, keyed by
    /// column name, with each cell a properly-typed JSON value. Use this when
    /// the result drives a decision rather than display.
    pub async fn query_rows(
        &self,
        cypher: &str,
    ) -> Result<Vec<HashMap<String, serde_json::Value>>, QueryError> {
        // A leading 0x01 byte selects the daemon's structured-row mode.
        let mut body = Vec::with_capacity(cypher.len() + 1);
        body.push(0x01);
        body.extend_from_slice(cypher.as_bytes());

        let bytes = self.round_trip(&body, MAX_TYPED_RESPONSE_BYTES).await?;
        // The daemon reports errors as plaintext ("ERROR: ..."), not JSON.
        // Only an error body is decoded as text (errors are short); a valid
        // typed body is never lossily decoded just to look for the prefix.
        if bytes.starts_with(b"ERROR:") {
            Self::check_error(&String::from_utf8_lossy(&bytes))?;
        }
        parse_row_set(&bytes)
    }

    /// Execute a read-only Cypher query and return typed rows preserving COLUMN
    /// ORDER: the column names and each row's cells positionally. Same wire mode
    /// and validation as [`query_rows`], but the rows are not collapsed into
    /// column-keyed maps - use this when access is positional (the timeline FUSE
    /// reader maps these into the knowledge daemon's own `RowSet`).
    #[allow(clippy::type_complexity)]
    pub async fn query_rows_ordered(
        &self,
        cypher: &str,
    ) -> Result<(Vec<String>, Vec<Vec<serde_json::Value>>), QueryError> {
        let mut body = Vec::with_capacity(cypher.len() + 1);
        body.push(0x01);
        body.extend_from_slice(cypher.as_bytes());

        let bytes = self.round_trip(&body, MAX_TYPED_RESPONSE_BYTES).await?;
        if bytes.starts_with(b"ERROR:") {
            Self::check_error(&String::from_utf8_lossy(&bytes))?;
        }
        parse_row_set_ordered(&bytes)
    }

    /// LLM-free retrieval: ask the daemon for the node ids most relevant to a
    /// keyword `query`, best-first, via the read socket's retrieval mode.
    ///
    /// A leading `0x03` byte selects the daemon's retrieval op (beside `0x01`
    /// typed-rows and `0x02` write); the body is the JSON request. The daemon
    /// fuses its keyword index and a bounded graph expansion, drops candidates
    /// with no current graph presence, and returns up to `limit` ranked node ids
    /// (the daemon clamps `limit` to its own ceiling). On success the response is
    /// a JSON array of ids; a daemon `ERROR:` maps to [`QueryError`]. This makes
    /// no LLM call at query time.
    pub async fn retrieve(&self, query: &str, limit: i64) -> Result<Vec<String>, QueryError> {
        let req = serde_json::json!({ "query": query, "limit": limit });
        let json = serde_json::to_vec(&req).map_err(|e| QueryError::InvalidQuery(e.to_string()))?;

        let mut body = Vec::with_capacity(json.len() + 1);
        body.push(0x03);
        body.extend_from_slice(&json);

        let bytes = self.round_trip(&body, MAX_TYPED_RESPONSE_BYTES).await?;
        if bytes.starts_with(b"ERROR:") {
            Self::check_error(&String::from_utf8_lossy(&bytes))?;
        }
        serde_json::from_slice::<Vec<String>>(&bytes)
            .map_err(|e| QueryError::InvalidQuery(format!("malformed retrieve response: {e}")))
    }

    /// Read the caller-scoped provenance of a graph object via the read socket's
    /// provenance op.
    ///
    /// A leading `0x04` byte selects the daemon's caller-scoped provenance read
    /// (beside `0x01` typed-rows, `0x02` write, `0x03` retrieve); the body is the
    /// JSON request `{ "object_id": ... }`. The daemon resolves the caller's read
    /// scope from its kernel-attested identity, returns the object's actors
    /// filtered to the caller (a co-tenant is summarised, never named), and gives a
    /// **uniform** out-of-scope denial for any object the scope does not cover.
    ///
    /// Returns `Ok(Some(view))` when the object is in scope, `Ok(None)` for the
    /// uniform out-of-scope denial. The `None` case is deliberately
    /// indistinguishable from "object absent": the daemon emits one denial shape so
    /// the caller cannot use this op as an existence oracle for objects outside its
    /// read scope. Other daemon errors map to [`QueryError`].
    pub async fn read_provenance(
        &self,
        object_id: &str,
    ) -> Result<Option<ProvenanceView>, QueryError> {
        let req = serde_json::json!({ "object_id": object_id });
        let json = serde_json::to_vec(&req).map_err(|e| QueryError::InvalidQuery(e.to_string()))?;

        let mut body = Vec::with_capacity(json.len() + 1);
        body.push(0x04);
        body.extend_from_slice(&json);

        let bytes = self.round_trip(&body, MAX_TYPED_RESPONSE_BYTES).await?;
        let text = String::from_utf8_lossy(&bytes);
        // The uniform out-of-scope/absent denial (must match the daemon's
        // `PROVENANCE_OUT_OF_SCOPE`). Surfaced as `None`, the no-oracle outcome,
        // not an error: the caller cannot tell out-of-scope from absent.
        if text.trim() == "ERROR: OutOfScope" {
            return Ok(None);
        }
        if bytes.starts_with(b"ERROR:") {
            Self::check_error(&text)?;
        }
        let view = serde_json::from_slice::<ProvenanceView>(&bytes)
            .map_err(|e| QueryError::InvalidQuery(format!("malformed provenance response: {e}")))?;
        Ok(Some(view))
    }

    /// Read the caller's capability grants via the read socket's access-grants op
    /// (living-capability-graph.md §5).
    ///
    /// A leading `0x05` byte selects the daemon's caller-scoped grant browse read.
    /// The daemon scopes the result to the caller's kernel-attested identity (a
    /// normal app receives only its own grants; the request carries no scope
    /// field), recomputes `live` fresh, and returns a JSON array of [`GrantView`].
    /// An app with no grants gets an empty vector, not an error. A daemon `ERROR:`
    /// (rate-limited, internal failure) maps to [`QueryError`].
    pub async fn access_grants(&self) -> Result<Vec<GrantView>, QueryError> {
        // The op takes no request body; the single prefix byte selects it.
        let body = [0x05u8];
        let bytes = self.round_trip(&body, MAX_TYPED_RESPONSE_BYTES).await?;
        if bytes.starts_with(b"ERROR:") {
            Self::check_error(&String::from_utf8_lossy(&bytes))?;
        }
        serde_json::from_slice::<Vec<GrantView>>(&bytes)
            .map_err(|e| QueryError::InvalidQuery(format!("malformed access_grants response: {e}")))
    }

    /// Read the token-free code-graph analysis (CG-R5) via the read socket's
    /// analysis op: god-symbols (degree-centrality hubs) and surprises (sole
    /// cross-module call bridges) over the whole `CodeSymbol`/`CALLS` graph.
    ///
    /// A leading `0x09` byte selects the op and takes no request body. The daemon
    /// gates it to system-anchored callers (the aggregate exceeds a ThirdParty's
    /// per-label read scope) and returns the analysis as JSON; a daemon `ERROR:`
    /// (not permitted, rate-limited) maps to [`QueryError`]. The result is parsed
    /// as a `serde_json::Value` (shape `{god_symbols, surprises}`): the rich type
    /// lives in the knowledge daemon, so the SDK exposes the validated JSON rather
    /// than depending on the daemon crate. The consumer (`knowledge-mcp`, the
    /// agent, the Knowledge app) navigates it; making it a shared typed result is
    /// a contracts-crate follow-on. No LLM call is made.
    pub async fn code_analysis(&self) -> Result<serde_json::Value, QueryError> {
        let body = [0x09u8];
        let bytes = self.round_trip(&body, MAX_TYPED_RESPONSE_BYTES).await?;
        if bytes.starts_with(b"ERROR:") {
            Self::check_error(&String::from_utf8_lossy(&bytes))?;
        }
        serde_json::from_slice::<serde_json::Value>(&bytes)
            .map_err(|e| QueryError::InvalidQuery(format!("malformed code_analysis response: {e}")))
    }

    /// Resolve a code symbol's activity-layer context (CG-R6): its defining
    /// file, the project that file belongs to (bitemporally — `as_of_micros`
    /// `None` is now, `Some(t)` the membership valid at `t` µs), and the apps
    /// that accessed it.
    ///
    /// A leading `0x0A` byte selects the op; the body is a JSON
    /// `{symbol_id, as_of_micros?}`. Gated to system-anchored callers like
    /// [`code_analysis`](Self::code_analysis); a daemon `ERROR:` maps to
    /// [`QueryError`]. Parsed as a `serde_json::Value` (shape `{symbol_id,
    /// file_path, project, accessed_by}`) — the rich type lives in the daemon,
    /// so the SDK exposes the validated JSON.
    pub async fn code_symbol_context(
        &self,
        symbol_id: &str,
        as_of_micros: Option<i64>,
    ) -> Result<serde_json::Value, QueryError> {
        let request = serde_json::json!({
            "symbol_id": symbol_id,
            "as_of_micros": as_of_micros,
        });
        let json = serde_json::to_vec(&request)
            .map_err(|e| QueryError::InvalidQuery(format!("encode code_symbol_context: {e}")))?;
        let mut body = Vec::with_capacity(1 + json.len());
        body.push(0x0Au8);
        body.extend_from_slice(&json);
        let bytes = self.round_trip(&body, MAX_TYPED_RESPONSE_BYTES).await?;
        if bytes.starts_with(b"ERROR:") {
            Self::check_error(&String::from_utf8_lossy(&bytes))?;
        }
        serde_json::from_slice::<serde_json::Value>(&bytes).map_err(|e| {
            QueryError::InvalidQuery(format!("malformed code_symbol_context response: {e}"))
        })
    }

    /// Revoke a capability from an app's permission profile, narrowing-only
    /// (living-capability-graph.md §6), via the read socket's revoke op.
    ///
    /// A leading `0x06` byte selects the daemon's revoke op; the body is the JSON
    /// [`RevokeReach`]. The daemon admits only the canonical `settings` principal,
    /// refuses a system-tier target, and applies the narrowing through the
    /// strict-subset gate, writing the target's profile only if authority strictly
    /// shrank. Returns the [`RevokeOutcome`] (Revoked / NoChange / NotNarrowing /
    /// NotFound) parsed from the wire token; a daemon `ERROR:` (not permitted,
    /// invalid request, system-tier, io) maps to [`QueryError`]. The closed request
    /// enum cannot express a widening.
    pub async fn revoke(
        &self,
        request: &arlen_permissions::revoke::RevokeReach,
    ) -> Result<arlen_permissions::revoke::RevokeOutcome, QueryError> {
        let json = serde_json::to_vec(request).map_err(|e| QueryError::InvalidQuery(e.to_string()))?;
        let mut body = Vec::with_capacity(json.len() + 1);
        body.push(0x06);
        body.extend_from_slice(&json);

        let bytes = self.round_trip(&body, MAX_TYPED_RESPONSE_BYTES).await?;
        let text = String::from_utf8_lossy(&bytes);
        if let Some(outcome) = arlen_permissions::revoke::RevokeOutcome::from_wire_token(&text) {
            return Ok(outcome);
        }
        // Not a recognised outcome token: an `ERROR:` reply or an unknown string.
        if bytes.starts_with(b"ERROR:") {
            Self::check_error(&text)?;
        }
        Err(QueryError::InvalidQuery(format!("malformed revoke response: {text}")))
    }

    /// Materialize a Context Capsule frozen slice for `scope` as of now, via the
    /// daemon's capsule read op (context-capsule.md §4, loader (b)).
    ///
    /// A leading `0x07` byte selects the op; the body is the JSON
    /// [`CapsuleScope`](arlen_capsule::scope::CapsuleScope). The daemon expands the
    /// scope, reads the projected fields and the live membership relations as of
    /// now, and returns the canonical JSON of the
    /// [`FrozenSlice`](arlen_capsule::slice::FrozenSlice); a daemon `ERROR:` (rate
    /// limit, invalid scope, read failure) maps to [`QueryError`]. The caller
    /// (`capsuled`) content-addresses the returned slice; re-canonicalizing it
    /// yields the same bytes the daemon hashed.
    pub async fn materialize_capsule(
        &self,
        scope: &arlen_capsule::scope::CapsuleScope,
    ) -> Result<arlen_capsule::slice::FrozenSlice, QueryError> {
        let json = serde_json::to_vec(scope).map_err(|e| QueryError::InvalidQuery(e.to_string()))?;
        let mut body = Vec::with_capacity(json.len() + 1);
        body.push(0x07);
        body.extend_from_slice(&json);

        let bytes = self.round_trip(&body, MAX_TYPED_RESPONSE_BYTES).await?;
        if bytes.starts_with(b"ERROR:") {
            Self::check_error(&String::from_utf8_lossy(&bytes))?;
        }
        serde_json::from_slice::<arlen_capsule::slice::FrozenSlice>(&bytes)
            .map_err(|e| QueryError::InvalidQuery(format!("malformed capsule slice: {e}")))
    }

    /// Create a built-in graph relation `from -[relation_type]-> to` between two
    /// existing nodes, via the daemon's write socket.
    ///
    /// This is the write counterpart to [`query_rows`]: a leading `0x02` byte
    /// selects the daemon's structured write mode, and the body is the JSON
    /// request the daemon authorises against the caller's permission profile
    /// (the relation must be one the profile grants) and persists with a checked
    /// MATCH/MERGE. Endpoint types are the namespaced graph types (e.g.
    /// `system.File`, `system.Project`) and the ids are the concrete node ids.
    ///
    /// On success returns whether the edge was newly [`Created`] or
    /// [`AlreadyExists`]ed (the daemon's conditional create is atomic and reports
    /// which for a single attempt, and never double-creates). A daemon `ERROR:`
    /// maps to [`QueryError`] (a permission error to
    /// [`QueryError::PermissionDenied`], a missing endpoint to
    /// [`QueryError::InvalidQuery`]). The create never duplicates the edge, so a
    /// retry is safe.
    ///
    /// `op_id` is the caller's durable operation identity, persisted on the edge
    /// (on `FILE_PART_OF` today). The per-attempt `Created`/`AlreadyExists` flag
    /// is not durable across a lost response, but a caller that supplies a stable
    /// `op_id` can reconcile a commit-unknown write by reading whether its own
    /// `op_id` edge exists. Pass an empty `op_id` to skip it (the edge's `op_id`
    /// stays unset).
    ///
    /// [`Created`]: RelationWriteOutcome::Created
    /// [`AlreadyExists`]: RelationWriteOutcome::AlreadyExists
    pub async fn create_relation(
        &self,
        from_type: &str,
        from_id: &str,
        to_type: &str,
        to_id: &str,
        relation_type: &str,
        op_id: &str,
    ) -> Result<RelationWriteOutcome, QueryError> {
        let req = serde_json::json!({
            "op": "create_relation",
            "from_type": from_type,
            "from_id": from_id,
            "to_type": to_type,
            "to_id": to_id,
            "relation_type": relation_type,
            "op_id": op_id,
        });
        let json = serde_json::to_vec(&req).map_err(|e| QueryError::InvalidQuery(e.to_string()))?;

        // A leading 0x02 byte selects the daemon's structured write mode.
        let mut body = Vec::with_capacity(json.len() + 1);
        body.push(0x02);
        body.extend_from_slice(&json);

        let bytes = self.round_trip(&body, MAX_WRITE_RESPONSE_BYTES).await?;
        // The daemon answers with a tiny plaintext status, so a lossy decode is
        // fine here (unlike the typed-row path).
        let response = String::from_utf8_lossy(&bytes);
        Self::check_error(&response)?;
        match response.trim() {
            "OK: created" => Ok(RelationWriteOutcome::Created),
            "OK: exists" => Ok(RelationWriteOutcome::AlreadyExists),
            other => Err(QueryError::InvalidQuery(format!(
                "unexpected daemon write response: {other}"
            ))),
        }
    }

    /// Create a node of a bounded built-in type at a caller-supplied id, via the
    /// daemon's write socket (the node counterpart to [`create_relation`]).
    ///
    /// The daemon guards this so it can only ever create, never overwrite: the
    /// `node_type` must be one of its creatable built-in types (the consolidation
    /// nodes), and the id is checked label-agnostically, so an id that already
    /// names a node of any label is refused rather than overwritten. Endpoint
    /// types are the namespaced graph types (e.g. `system.Summary`) and the id is
    /// the caller's own (e.g. a deterministic UUIDv5).
    ///
    /// On success returns whether the node was newly [`Created`] or
    /// [`AlreadyExists`]ed; the create is atomic and never overwrites, so a retry
    /// is safe. A daemon `ERROR:` maps to [`QueryError`] (a permission error to
    /// [`QueryError::PermissionDenied`]).
    ///
    /// [`Created`]: NodeWriteOutcome::Created
    /// [`AlreadyExists`]: NodeWriteOutcome::AlreadyExists
    pub async fn create_node(
        &self,
        node_type: &str,
        id: &str,
    ) -> Result<NodeWriteOutcome, QueryError> {
        let req = serde_json::json!({
            "op": "create_node",
            "node_type": node_type,
            "id": id,
        });
        let json = serde_json::to_vec(&req).map_err(|e| QueryError::InvalidQuery(e.to_string()))?;

        // A leading 0x02 byte selects the daemon's structured write mode.
        let mut body = Vec::with_capacity(json.len() + 1);
        body.push(0x02);
        body.extend_from_slice(&json);

        let bytes = self.round_trip(&body, MAX_WRITE_RESPONSE_BYTES).await?;
        let response = String::from_utf8_lossy(&bytes);
        Self::check_error(&response)?;
        match response.trim() {
            "OK: created" => Ok(NodeWriteOutcome::Created),
            "OK: exists" => Ok(NodeWriteOutcome::AlreadyExists),
            other => Err(QueryError::InvalidQuery(format!(
                "unexpected daemon node-write response: {other}"
            ))),
        }
    }

    /// Upsert (create-or-update) an instance of the caller's OWN declared entity
    /// type over the daemon write socket, keyed by `external_key` for idempotent
    /// re-sync (foreign-app-bridges piece 1): the general app-tier instance write.
    ///
    /// The daemon enforces, fail-closed, that the type is in the caller's
    /// namespace and registered, that `system.*`/`shared.*` are unwritable, and
    /// that the fields validate against the registered schema; a re-sync of the
    /// same `external_key` updates the existing node in place rather than
    /// duplicating. A daemon `ERROR:` maps to [`QueryError`] (a permission error
    /// to [`QueryError::PermissionDenied`]). Idempotent, so a retry is safe.
    pub async fn upsert_entity(
        &self,
        qualified_type: &str,
        external_key: &str,
        fields: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<(), QueryError> {
        let req = serde_json::json!({
            "op": "upsert_entity",
            "qualified_type": qualified_type,
            "external_key": external_key,
            "fields": fields,
        });
        let json = serde_json::to_vec(&req).map_err(|e| QueryError::InvalidQuery(e.to_string()))?;

        // A leading 0x02 byte selects the daemon's structured write mode.
        let mut body = Vec::with_capacity(json.len() + 1);
        body.push(0x02);
        body.extend_from_slice(&json);

        let bytes = self.round_trip(&body, MAX_WRITE_RESPONSE_BYTES).await?;
        let response = String::from_utf8_lossy(&bytes);
        Self::check_error(&response)?;
        match response.trim() {
            "OK: upserted" => Ok(()),
            other => Err(QueryError::InvalidQuery(format!(
                "unexpected daemon entity-write response: {other}"
            ))),
        }
    }

    /// Link two instances of the caller's OWN declared entity types with an edge
    /// over the daemon write socket, idempotently (foreign-app-bridges piece 2):
    /// the app-tier entity-edge write.
    ///
    /// The daemon enforces, fail-closed, that BOTH endpoint types are in the
    /// caller's namespace and registered, that `system.*`/`shared.*` are
    /// unlinkable, and that the edge type is a safe identifier; the endpoints are
    /// addressed by their stable external keys (the daemon owns the deterministic
    /// id scheme), and the edge MERGE never duplicates on a re-sync. A daemon
    /// `ERROR:` maps to [`QueryError`]; a forward reference to a not-yet-synced
    /// endpoint surfaces as [`QueryError::InvalidQuery`] carrying `link endpoints
    /// not found`, so a caller can re-sync rather than treat it as a hard
    /// failure. Idempotent, so a retry is safe.
    pub async fn link_entities(
        &self,
        edge_type: &str,
        from_type: &str,
        from_key: &str,
        to_type: &str,
        to_key: &str,
    ) -> Result<(), QueryError> {
        let req = serde_json::json!({
            "op": "link_entities",
            "edge_type": edge_type,
            "from_type": from_type,
            "from_key": from_key,
            "to_type": to_type,
            "to_key": to_key,
        });
        let json = serde_json::to_vec(&req).map_err(|e| QueryError::InvalidQuery(e.to_string()))?;

        // A leading 0x02 byte selects the daemon's structured write mode.
        let mut body = Vec::with_capacity(json.len() + 1);
        body.push(0x02);
        body.extend_from_slice(&json);

        let bytes = self.round_trip(&body, MAX_WRITE_RESPONSE_BYTES).await?;
        let response = String::from_utf8_lossy(&bytes);
        Self::check_error(&response)?;
        match response.trim() {
            "OK: linked" => Ok(()),
            other => Err(QueryError::InvalidQuery(format!(
                "unexpected daemon entity-link response: {other}"
            ))),
        }
    }

    /// Persist a consent grant into the shared LCG Grant node over the daemon
    /// write socket (system-dialog-plan.md Option A): the durable half of the
    /// consent grant lifecycle, surfaced by the `access_grants` read in the same
    /// see+revoke place. Keyed by `revocation_handle` so a re-consent strengthens
    /// the same node. Only the consent broker is admitted; a daemon `ERROR:` maps
    /// to [`QueryError`] (a permission error to [`QueryError::PermissionDenied`]).
    /// Idempotent, so a retry is safe.
    pub async fn persist_consent_grant(
        &self,
        recipient: &str,
        consent_class: &str,
        consent_scope: Option<&str>,
        revocation_handle: &str,
    ) -> Result<(), QueryError> {
        let req = serde_json::json!({
            "op": "persist_consent_grant",
            "recipient": recipient,
            "consent_class": consent_class,
            "consent_scope": consent_scope,
            "revocation_handle": revocation_handle,
        });
        let json = serde_json::to_vec(&req).map_err(|e| QueryError::InvalidQuery(e.to_string()))?;

        // A leading 0x02 byte selects the daemon's structured write mode.
        let mut body = Vec::with_capacity(json.len() + 1);
        body.push(0x02);
        body.extend_from_slice(&json);

        let bytes = self.round_trip(&body, MAX_WRITE_RESPONSE_BYTES).await?;
        let response = String::from_utf8_lossy(&bytes);
        Self::check_error(&response)?;
        match response.trim() {
            "OK: persisted" => Ok(()),
            other => Err(QueryError::InvalidQuery(format!(
                "unexpected daemon consent-grant response: {other}"
            ))),
        }
    }

    /// Retract (compensate) a relation this caller previously created, deleting
    /// only the edge that carries `op_id`, via the daemon's write socket.
    ///
    /// This is the inverse of [`create_relation`](Self::create_relation): it
    /// undoes exactly the edge a prior create stamped with the same `op_id`, so
    /// the create grant alone authorises the undo (the daemon never deletes a
    /// bare edge here). `op_id` is therefore **mandatory and must be non-empty**;
    /// an empty id is rejected by the daemon. Only relations that carry the
    /// `op_id` column (`FILE_PART_OF` today) can be retracted.
    ///
    /// Deletion is idempotent: a retract that matches no edge (already gone, or
    /// never created) succeeds as [`Absent`]. A retract that removed the edge
    /// returns [`Retracted`]. A daemon `ERROR:` maps to [`QueryError`] (a
    /// permission error to [`QueryError::PermissionDenied`]). The idempotency
    /// makes a retry safe.
    ///
    /// [`Retracted`]: RelationRetractOutcome::Retracted
    /// [`Absent`]: RelationRetractOutcome::Absent
    pub async fn retract_relation(
        &self,
        from_type: &str,
        from_id: &str,
        to_type: &str,
        to_id: &str,
        relation_type: &str,
        op_id: &str,
    ) -> Result<RelationRetractOutcome, QueryError> {
        let req = serde_json::json!({
            "op": "retract_relation",
            "from_type": from_type,
            "from_id": from_id,
            "to_type": to_type,
            "to_id": to_id,
            "relation_type": relation_type,
            "op_id": op_id,
        });
        let json = serde_json::to_vec(&req).map_err(|e| QueryError::InvalidQuery(e.to_string()))?;

        // A leading 0x02 byte selects the daemon's structured write mode.
        let mut body = Vec::with_capacity(json.len() + 1);
        body.push(0x02);
        body.extend_from_slice(&json);

        let bytes = self.round_trip(&body, MAX_WRITE_RESPONSE_BYTES).await?;
        let response = String::from_utf8_lossy(&bytes);
        Self::check_error(&response)?;
        match response.trim() {
            "OK: retracted" => Ok(RelationRetractOutcome::Retracted),
            "OK: absent" => Ok(RelationRetractOutcome::Absent),
            other => Err(QueryError::InvalidQuery(format!(
                "unexpected daemon write response: {other}"
            ))),
        }
    }
}

/// The outcome of a successful [`create_relation`](UnixGraphClient::create_relation):
/// whether this call created the edge or found it already present.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelationWriteOutcome {
    /// This call created the edge (it was absent before).
    Created,
    /// The edge already existed; the call was an idempotent no-op.
    AlreadyExists,
}

/// The outcome of a successful [`create_node`](UnixGraphClient::create_node):
/// whether this call created the node or found a node with that id already
/// present (of any label, since the daemon's id check is label-agnostic).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeWriteOutcome {
    /// This call created the node (no node had that id before).
    Created,
    /// A node with this id already existed; the call was an idempotent no-op.
    AlreadyExists,
}

/// The outcome of a successful [`retract_relation`](UnixGraphClient::retract_relation):
/// whether this call removed the op-id-keyed edge or found nothing to remove.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelationRetractOutcome {
    /// This call deleted the edge carrying the given `op_id`.
    Retracted,
    /// No edge carried the given `op_id` (already gone, or never created); the
    /// call was an idempotent no-op.
    Absent,
}

/// Parse the daemon's structured `{"columns": [...], "rows": [[..], ..]}`
/// JSON into rows keyed by column name.
///
/// Validation is strict and fails closed: a non-string or duplicate column
/// name, a row that is not an array, or a row whose cell count does not match
/// the column count is an [`QueryError::InvalidQuery`] rather than a coerced
/// partial row. Callers drive decisions from these rows, so a corrupt or
/// version-skewed body must surface as an error, never as plausible-looking
/// data.
/// Parse the daemon's typed `{columns, rows}` result preserving COLUMN ORDER:
/// the column names and each row's cells positionally (not collapsed into a
/// `HashMap`, which loses order). Applies the same column/row/cell-count caps,
/// duplicate-column rejection, and per-row shape check as [`parse_row_set`],
/// which is now a thin wrapper that zips this into column-keyed maps. Used where
/// positional access matters (the timeline FUSE reader's queries).
#[allow(clippy::type_complexity)]
fn parse_row_set_ordered(
    json: &[u8],
) -> Result<(Vec<String>, Vec<Vec<serde_json::Value>>), QueryError> {
    // `from_slice` requires valid UTF-8, so invalid bytes fail closed here
    // instead of being lossily replaced and parsed as plausible typed data.
    let value: serde_json::Value = serde_json::from_slice(json)
        .map_err(|e| QueryError::InvalidQuery(format!("malformed typed result: {e}")))?;

    let columns_json = value
        .get("columns")
        .and_then(|c| c.as_array())
        .ok_or_else(|| QueryError::InvalidQuery("typed result missing 'columns'".to_string()))?;
    if columns_json.len() > MAX_TYPED_COLUMNS {
        return Err(QueryError::InvalidQuery(format!(
            "typed result has {} columns, more than the {MAX_TYPED_COLUMNS} allowed",
            columns_json.len()
        )));
    }
    let mut columns: Vec<String> = Vec::with_capacity(columns_json.len());
    for col in columns_json {
        let name = col.as_str().ok_or_else(|| {
            QueryError::InvalidQuery("typed result has a non-string column name".to_string())
        })?;
        if name.len() > MAX_TYPED_COLUMN_NAME_BYTES {
            return Err(QueryError::InvalidQuery(format!(
                "typed result has a column name of {} bytes, more than the {MAX_TYPED_COLUMN_NAME_BYTES} allowed",
                name.len()
            )));
        }
        if columns.iter().any(|existing| existing == name) {
            return Err(QueryError::InvalidQuery(format!(
                "typed result has a duplicate column name {name:?}"
            )));
        }
        columns.push(name.to_string());
    }

    let rows_json = value
        .get("rows")
        .and_then(|r| r.as_array())
        .ok_or_else(|| QueryError::InvalidQuery("typed result missing 'rows'".to_string()))?;
    if rows_json.len() > MAX_TYPED_ROWS {
        return Err(QueryError::InvalidQuery(format!(
            "typed result has {} rows, more than the {MAX_TYPED_ROWS} allowed",
            rows_json.len()
        )));
    }
    // Bound total work before materializing any row. With the shape check
    // below every row carries exactly `columns.len()` cells, so this product
    // is the cell count the body claims; rejecting it here avoids building
    // millions of map entries from a frame that is itself within the limit.
    let total_cells = columns.len().saturating_mul(rows_json.len());
    if total_cells > MAX_TYPED_CELLS {
        return Err(QueryError::InvalidQuery(format!(
            "typed result claims {total_cells} cells, more than the {MAX_TYPED_CELLS} allowed"
        )));
    }
    let mut rows = Vec::with_capacity(rows_json.len());
    for row in rows_json {
        let cells = row.as_array().ok_or_else(|| {
            QueryError::InvalidQuery("typed result row is not an array".to_string())
        })?;
        if cells.len() != columns.len() {
            return Err(QueryError::InvalidQuery(format!(
                "typed result row has {} cells, expected {}",
                cells.len(),
                columns.len()
            )));
        }
        rows.push(cells.to_vec());
    }
    Ok((columns, rows))
}

/// Parse the daemon's typed result into column-keyed rows. Thin wrapper over
/// [`parse_row_set_ordered`] that zips each positional row against the column
/// names; callers that need column order use the ordered form directly.
fn parse_row_set(json: &[u8]) -> Result<Vec<HashMap<String, serde_json::Value>>, QueryError> {
    let (columns, rows) = parse_row_set_ordered(json)?;
    Ok(rows
        .into_iter()
        .map(|cells| {
            columns
                .iter()
                .cloned()
                .zip(cells)
                .collect::<HashMap<String, serde_json::Value>>()
        })
        .collect())
}

impl GraphClient for UnixGraphClient {
    #[allow(clippy::manual_async_fn)]
    fn query<'a>(
        &'a self,
        cypher: &'a str,
        _params: HashMap<String, serde_json::Value>,
    ) -> impl Future<Output = Result<Vec<HashMap<String, serde_json::Value>>, QueryError>> + Send + 'a
    {
        async move {
            // Params are not sent on the wire; the daemon accepts raw Cypher
            // (typed/parameterised reads use `query_rows`). The daemon returns
            // its result as a raw string, surfaced as a single `result` column
            // — see `query_rows` for typed, column-keyed rows.
            let bytes = self.round_trip(cypher.as_bytes(), MAX_RESPONSE_BYTES).await?;
            // Text results are display-only, so lossy decoding is acceptable.
            let response = String::from_utf8_lossy(&bytes).to_string();
            Self::check_error(&response)?;
            let row = HashMap::from([("result".to_string(), serde_json::Value::String(response))]);
            Ok(vec![row])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_typed_columns_and_rows() {
        let json = r#"{"columns":["id","root_path"],"rows":[["p1","/home/tim/proj"],["p2","/x"]]}"#;
        let rows = parse_row_set(json.as_bytes()).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["id"], serde_json::Value::String("p1".into()));
        assert_eq!(rows[0]["root_path"], serde_json::Value::String("/home/tim/proj".into()));
    }

    #[test]
    fn preserves_cell_types() {
        let json = r#"{"columns":["n","ok","missing"],"rows":[[3,true,null]]}"#;
        let rows = parse_row_set(json.as_bytes()).unwrap();
        assert_eq!(rows[0]["n"], serde_json::json!(3));
        assert_eq!(rows[0]["ok"], serde_json::Value::Bool(true));
        assert_eq!(rows[0]["missing"], serde_json::Value::Null);
    }

    #[test]
    fn a_delimiter_or_newline_in_a_value_round_trips_intact() {
        // The point of the typed path: JSON escaping means a value with `|`
        // or a newline can no longer corrupt parsing (the old pipe-delimited
        // text format would split it into a forged row).
        let json = r#"{"columns":["name"],"rows":[["a|b\nc"]]}"#;
        let rows = parse_row_set(json.as_bytes()).unwrap();
        assert_eq!(rows[0]["name"], serde_json::Value::String("a|b\nc".into()));
    }

    #[test]
    fn rejects_malformed_or_incomplete_results() {
        assert!(parse_row_set(b"not json").is_err());
        assert!(parse_row_set(br#"{"rows":[]}"#).is_err()); // missing columns
        assert!(parse_row_set(br#"{"columns":["id"]}"#).is_err()); // missing rows
    }

    #[test]
    fn rejects_corrupt_row_shapes_rather_than_coercing() {
        // Non-string column name.
        assert!(parse_row_set(br#"{"columns":[1],"rows":[]}"#).is_err());
        // Duplicate column name would silently overwrite in a map.
        assert!(parse_row_set(br#"{"columns":["id","id"],"rows":[]}"#).is_err());
        // A row that is not an array.
        assert!(parse_row_set(br#"{"columns":["id"],"rows":[{"id":"x"}]}"#).is_err());
        // Short row: fewer cells than columns.
        assert!(parse_row_set(br#"{"columns":["a","b"],"rows":[["x"]]}"#).is_err());
        // Long row: more cells than columns.
        assert!(parse_row_set(br#"{"columns":["a"],"rows":[["x","y"]]}"#).is_err());
    }

    #[test]
    fn rejects_amplifying_row_and_column_counts() {
        // Many empty rows fit under the byte cap but would each become a map.
        let mut body = br#"{"columns":[],"rows":["#.to_vec();
        for i in 0..(MAX_TYPED_ROWS + 1) {
            if i > 0 {
                body.push(b',');
            }
            body.extend_from_slice(b"[]");
        }
        body.extend_from_slice(b"]}");
        assert!(parse_row_set(&body).is_err());

        // More columns than the daemon would ever emit.
        let cols: Vec<String> = (0..=MAX_TYPED_COLUMNS).map(|i| format!("\"c{i}\"")).collect();
        let body = format!(r#"{{"columns":[{}],"rows":[]}}"#, cols.join(","));
        assert!(parse_row_set(body.as_bytes()).is_err());

        // A single over-long column name (a per-cell key-clone amplifier).
        let long = "x".repeat(MAX_TYPED_COLUMN_NAME_BYTES + 1);
        let body = format!(r#"{{"columns":["{long}"],"rows":[]}}"#);
        assert!(parse_row_set(body.as_bytes()).is_err());

        // Within the row and column caps, but the cell product exceeds the
        // total budget. Empty row arrays keep the test body tiny; the product
        // check fires before any per-row shape validation.
        let full_cols: Vec<String> = (0..MAX_TYPED_COLUMNS).map(|i| format!("\"c{i}\"")).collect();
        let row_count = MAX_TYPED_CELLS / MAX_TYPED_COLUMNS + 2;
        let empty_rows = vec!["[]"; row_count].join(",");
        let body = format!(
            r#"{{"columns":[{}],"rows":[{}]}}"#,
            full_cols.join(","),
            empty_rows
        );
        assert!(parse_row_set(body.as_bytes()).is_err());
    }

    #[test]
    fn invalid_utf8_fails_closed_rather_than_decoding_lossily() {
        // A 0xFF byte is not valid UTF-8. The typed path must reject it, not
        // replace it with U+FFFD and parse the surrounding bytes as a row.
        let mut body = br#"{"columns":["name"],"rows":[[""#.to_vec();
        body.push(0xFF);
        body.extend_from_slice(br#""]]}"#);
        assert!(parse_row_set(&body).is_err());
    }

    #[tokio::test]
    async fn an_oversized_response_frame_is_rejected_before_allocation() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::UnixListener;

        let path = std::env::temp_dir().join("arlen-os-sdk-oversized-frame-test.sock");
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).unwrap();

        let server = tokio::spawn(async move {
            let (mut conn, _) = listener.accept().await.unwrap();
            // Drain the request frame (4-byte length + body).
            let mut len_buf = [0u8; 4];
            conn.read_exact(&mut len_buf).await.unwrap();
            let req_len = u32::from_be_bytes(len_buf) as usize;
            let mut req = vec![0u8; req_len];
            conn.read_exact(&mut req).await.unwrap();
            // Advertise a response one byte past the client's limit, then send
            // no body: a well-behaved client must reject on the length alone.
            let huge = (MAX_RESPONSE_BYTES as u32 + 1).to_be_bytes();
            conn.write_all(&huge).await.unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        });

        let client = UnixGraphClient::new(path.to_string_lossy().to_string());
        let result = client.query("MATCH (n) RETURN n", HashMap::new()).await;
        let _ = server.await;
        let _ = std::fs::remove_file(&path);

        match result {
            Err(QueryError::InvalidQuery(msg)) => assert!(msg.contains("exceeds")),
            other => panic!("expected oversized-frame rejection, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn create_relation_sends_a_tagged_write_request_and_accepts_ok() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::UnixListener;

        let path = std::env::temp_dir().join("arlen-os-sdk-write-ok-test.sock");
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).unwrap();

        let server = tokio::spawn(async move {
            let (mut conn, _) = listener.accept().await.unwrap();
            let mut len_buf = [0u8; 4];
            conn.read_exact(&mut len_buf).await.unwrap();
            let req_len = u32::from_be_bytes(len_buf) as usize;
            let mut req = vec![0u8; req_len];
            conn.read_exact(&mut req).await.unwrap();

            // The write-mode prefix, then the tagged JSON request.
            assert_eq!(req[0], 0x02, "write requests carry the 0x02 prefix");
            let body: serde_json::Value = serde_json::from_slice(&req[1..]).unwrap();
            assert_eq!(body["op"], "create_relation");
            assert_eq!(body["from_type"], "system.File");
            assert_eq!(body["from_id"], "f1");
            assert_eq!(body["to_type"], "system.Project");
            assert_eq!(body["to_id"], "p1");
            assert_eq!(body["relation_type"], "FILE_PART_OF");

            let ok = b"OK: created";
            conn.write_all(&(ok.len() as u32).to_be_bytes()).await.unwrap();
            conn.write_all(ok).await.unwrap();
        });

        let client = UnixGraphClient::new(path.to_string_lossy().to_string());
        let result = client
            .create_relation("system.File", "f1", "system.Project", "p1", "FILE_PART_OF", "op-test")
            .await;
        let _ = server.await;
        let _ = std::fs::remove_file(&path);

        assert!(
            matches!(result, Ok(RelationWriteOutcome::Created)),
            "an `OK: created` status must map to Created, got {result:?}"
        );
    }

    #[tokio::test]
    async fn retrieve_sends_a_tagged_request_and_parses_ranked_ids() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::UnixListener;

        let path = std::env::temp_dir().join("arlen-os-sdk-retrieve-test.sock");
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).unwrap();

        let server = tokio::spawn(async move {
            let (mut conn, _) = listener.accept().await.unwrap();
            let mut len_buf = [0u8; 4];
            conn.read_exact(&mut len_buf).await.unwrap();
            let req_len = u32::from_be_bytes(len_buf) as usize;
            let mut req = vec![0u8; req_len];
            conn.read_exact(&mut req).await.unwrap();

            // The retrieval-mode prefix, then the JSON request.
            assert_eq!(req[0], 0x03, "retrieve requests carry the 0x03 prefix");
            let body: serde_json::Value = serde_json::from_slice(&req[1..]).unwrap();
            assert_eq!(body["query"], "main.rs");
            assert_eq!(body["limit"], 10);

            let resp = br#"["/a/main.rs","p1"]"#;
            conn.write_all(&(resp.len() as u32).to_be_bytes()).await.unwrap();
            conn.write_all(resp).await.unwrap();
        });

        let client = UnixGraphClient::new(path.to_string_lossy().to_string());
        let result = client.retrieve("main.rs", 10).await;
        let _ = server.await;
        let _ = std::fs::remove_file(&path);

        assert_eq!(
            result.unwrap(),
            vec!["/a/main.rs".to_string(), "p1".to_string()],
            "the ranked ids are parsed from the JSON array response"
        );
    }

    #[tokio::test]
    async fn read_provenance_sends_a_tagged_request_and_parses_the_view() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::UnixListener;

        let path = std::env::temp_dir().join("arlen-os-sdk-provenance-test.sock");
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).unwrap();

        let server = tokio::spawn(async move {
            let (mut conn, _) = listener.accept().await.unwrap();
            let mut len_buf = [0u8; 4];
            conn.read_exact(&mut len_buf).await.unwrap();
            let req_len = u32::from_be_bytes(len_buf) as usize;
            let mut req = vec![0u8; req_len];
            conn.read_exact(&mut req).await.unwrap();

            assert_eq!(req[0], 0x04, "provenance reads carry the 0x04 prefix");
            let body: serde_json::Value = serde_json::from_slice(&req[1..]).unwrap();
            assert_eq!(body["object_id"], "/a/main.rs");

            let resp = br#"{"actors":["ai-agent"],"accessed_by_others":true}"#;
            conn.write_all(&(resp.len() as u32).to_be_bytes()).await.unwrap();
            conn.write_all(resp).await.unwrap();
        });

        let client = UnixGraphClient::new(path.to_string_lossy().to_string());
        let result = client.read_provenance("/a/main.rs").await;
        let _ = server.await;
        let _ = std::fs::remove_file(&path);

        let view = result.unwrap().expect("in-scope object yields a view");
        assert_eq!(view.actors, vec!["ai-agent".to_string()]);
        assert!(view.accessed_by_others, "a foreign actor is summarised, not named");
    }

    #[tokio::test]
    async fn read_provenance_maps_out_of_scope_to_none() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::UnixListener;

        let path = std::env::temp_dir().join("arlen-os-sdk-provenance-oos-test.sock");
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).unwrap();

        let server = tokio::spawn(async move {
            let (mut conn, _) = listener.accept().await.unwrap();
            let mut len_buf = [0u8; 4];
            conn.read_exact(&mut len_buf).await.unwrap();
            let req_len = u32::from_be_bytes(len_buf) as usize;
            let mut req = vec![0u8; req_len];
            conn.read_exact(&mut req).await.unwrap();

            // The uniform out-of-scope/absent denial.
            let resp = b"ERROR: OutOfScope";
            conn.write_all(&(resp.len() as u32).to_be_bytes()).await.unwrap();
            conn.write_all(resp).await.unwrap();
        });

        let client = UnixGraphClient::new(path.to_string_lossy().to_string());
        let result = client.read_provenance("/secret").await;
        let _ = server.await;
        let _ = std::fs::remove_file(&path);

        assert_eq!(result.unwrap(), None, "out-of-scope is the no-oracle None, not an error");
    }

    #[tokio::test]
    async fn access_grants_sends_the_prefix_and_parses_the_views() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::UnixListener;

        let path = std::env::temp_dir().join("arlen-os-sdk-access-grants-test.sock");
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).unwrap();

        let server = tokio::spawn(async move {
            let (mut conn, _) = listener.accept().await.unwrap();
            let mut len_buf = [0u8; 4];
            conn.read_exact(&mut len_buf).await.unwrap();
            let req_len = u32::from_be_bytes(len_buf) as usize;
            let mut req = vec![0u8; req_len];
            conn.read_exact(&mut req).await.unwrap();

            // The op is selected by the single 0x05 prefix byte, no body.
            assert_eq!(req, vec![0x05], "access_grants is a bare 0x05 prefix");

            let resp = br#"[{"id":"g1","app_id":"com.x","declared_ceiling":"{}","required":false,"identity_verified":false,"live":true,"revoked":false,"superseded":false,"issued_at":1,"reach":["File"]}]"#;
            conn.write_all(&(resp.len() as u32).to_be_bytes()).await.unwrap();
            conn.write_all(resp).await.unwrap();
        });

        let client = UnixGraphClient::new(path.to_string_lossy().to_string());
        let grants = client.access_grants().await;
        let _ = server.await;
        let _ = std::fs::remove_file(&path);

        let grants = grants.unwrap();
        assert_eq!(grants.len(), 1);
        assert_eq!(grants[0].app_id, "com.x");
        assert!(grants[0].live);
        assert_eq!(grants[0].reach, vec!["File".to_string()]);
    }

    #[tokio::test]
    async fn code_analysis_sends_the_prefix_and_parses_the_json() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::UnixListener;

        let path = std::env::temp_dir().join("arlen-os-sdk-code-analysis-test.sock");
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).unwrap();

        let server = tokio::spawn(async move {
            let (mut conn, _) = listener.accept().await.unwrap();
            let mut len_buf = [0u8; 4];
            conn.read_exact(&mut len_buf).await.unwrap();
            let req_len = u32::from_be_bytes(len_buf) as usize;
            let mut req = vec![0u8; req_len];
            conn.read_exact(&mut req).await.unwrap();
            // The op is selected by the single 0x09 prefix byte, no body.
            assert_eq!(req, vec![0x09], "code_analysis is a bare 0x09 prefix");

            let resp = br#"{"god_symbols":[{"id":"a.rs#fn:hub@1","in_degree":3,"out_degree":1}],"surprises":[{"from":"a.rs#fn:hub@1","to":"b.rs#fn:y@5","from_module":"a.rs","to_module":"b.rs"}]}"#;
            conn.write_all(&(resp.len() as u32).to_be_bytes()).await.unwrap();
            conn.write_all(resp).await.unwrap();
        });

        let client = UnixGraphClient::new(path.to_string_lossy().to_string());
        let analysis = client.code_analysis().await;
        let _ = server.await;
        let _ = std::fs::remove_file(&path);

        let analysis = analysis.unwrap();
        assert_eq!(analysis["god_symbols"][0]["id"], "a.rs#fn:hub@1");
        assert_eq!(analysis["surprises"][0]["to"], "b.rs#fn:y@5");
    }

    #[tokio::test]
    async fn code_symbol_context_sends_the_prefix_body_and_parses_the_json() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::UnixListener;

        let path = std::env::temp_dir().join("arlen-os-sdk-code-symbol-test.sock");
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).unwrap();

        let server = tokio::spawn(async move {
            let (mut conn, _) = listener.accept().await.unwrap();
            let mut len_buf = [0u8; 4];
            conn.read_exact(&mut len_buf).await.unwrap();
            let req_len = u32::from_be_bytes(len_buf) as usize;
            let mut req = vec![0u8; req_len];
            conn.read_exact(&mut req).await.unwrap();
            // 0x0A prefix + a JSON body carrying the symbol id + as-of.
            assert_eq!(req[0], 0x0A, "code_symbol_context is selected by 0x0A");
            let parsed: serde_json::Value = serde_json::from_slice(&req[1..]).unwrap();
            assert_eq!(parsed["symbol_id"], "/p/lib.rs#fn:helper@1");
            assert_eq!(parsed["as_of_micros"], 150);

            let resp = br#"{"symbol_id":"/p/lib.rs#fn:helper@1","file_path":"/p/lib.rs","project":{"id":"proj-1","name":"MyProj"},"accessed_by":["editor"]}"#;
            conn.write_all(&(resp.len() as u32).to_be_bytes()).await.unwrap();
            conn.write_all(resp).await.unwrap();
        });

        let client = UnixGraphClient::new(path.to_string_lossy().to_string());
        let ctx = client
            .code_symbol_context("/p/lib.rs#fn:helper@1", Some(150))
            .await;
        let _ = server.await;
        let _ = std::fs::remove_file(&path);

        let ctx = ctx.unwrap();
        assert_eq!(ctx["file_path"], "/p/lib.rs");
        assert_eq!(ctx["project"]["id"], "proj-1");
        assert_eq!(ctx["accessed_by"][0], "editor");
    }

    #[tokio::test]
    async fn revoke_sends_a_tagged_request_and_parses_the_outcome() {
        use arlen_permissions::revoke::{RevokeInitiator, RevokeOutcome, RevokeReach, RevokedReach};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::UnixListener;

        let path = std::env::temp_dir().join("arlen-os-sdk-revoke-test.sock");
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).unwrap();

        let server = tokio::spawn(async move {
            let (mut conn, _) = listener.accept().await.unwrap();
            let mut len_buf = [0u8; 4];
            conn.read_exact(&mut len_buf).await.unwrap();
            let req_len = u32::from_be_bytes(len_buf) as usize;
            let mut req = vec![0u8; req_len];
            conn.read_exact(&mut req).await.unwrap();

            assert_eq!(req[0], 0x06, "revoke carries the 0x06 prefix");
            let body: serde_json::Value = serde_json::from_slice(&req[1..]).unwrap();
            assert_eq!(body["target_app_id"], "com.x");

            let resp = b"OK: revoked";
            conn.write_all(&(resp.len() as u32).to_be_bytes()).await.unwrap();
            conn.write_all(resp).await.unwrap();
        });

        let client = UnixGraphClient::new(path.to_string_lossy().to_string());
        let req = RevokeReach {
            target_app_id: "com.x".into(),
            reach: RevokedReach::Read { entity_pattern: "system.File".into() },
            initiator: RevokeInitiator::User,
        };
        let outcome = client.revoke(&req).await;
        let _ = server.await;
        let _ = std::fs::remove_file(&path);

        assert_eq!(outcome.unwrap(), RevokeOutcome::Revoked);
    }

    #[tokio::test]
    async fn materialize_capsule_sends_the_scope_and_parses_the_slice() {
        use arlen_capsule::scope::CapsuleScope;
        use arlen_capsule::slice::SliceValue;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::UnixListener;

        let path = std::env::temp_dir().join("arlen-os-sdk-capsule-test.sock");
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).unwrap();

        let server = tokio::spawn(async move {
            let (mut conn, _) = listener.accept().await.unwrap();
            let mut len_buf = [0u8; 4];
            conn.read_exact(&mut len_buf).await.unwrap();
            let req_len = u32::from_be_bytes(len_buf) as usize;
            let mut req = vec![0u8; req_len];
            conn.read_exact(&mut req).await.unwrap();

            assert_eq!(req[0], 0x07, "capsule materialize carries the 0x07 prefix");
            let scope: CapsuleScope = serde_json::from_slice(&req[1..]).unwrap();
            assert_eq!(scope.roots, vec!["p1".to_string()]);
            assert_eq!(scope.expand_hops, 1);

            let resp = br#"{"nodes":[{"id":"f1","label":"File","fields":{"path":{"text":"/a"}}}],"relations":[{"from":"f1","rel_type":"FILE_PART_OF","to":"p1"}]}"#;
            conn.write_all(&(resp.len() as u32).to_be_bytes()).await.unwrap();
            conn.write_all(resp).await.unwrap();
        });

        let client = UnixGraphClient::new(path.to_string_lossy().to_string());
        let scope = CapsuleScope { roots: vec!["p1".into()], expand_hops: 1 };
        let slice = client.materialize_capsule(&scope).await;
        let _ = server.await;
        let _ = std::fs::remove_file(&path);

        let slice = slice.unwrap();
        assert_eq!(slice.nodes.len(), 1);
        assert_eq!(slice.nodes[0].id, "f1");
        assert_eq!(slice.nodes[0].fields.get("path"), Some(&SliceValue::Text("/a".into())));
        assert_eq!(slice.relations.len(), 1);
        assert_eq!(slice.relations[0].to, "p1");
    }

    #[tokio::test]
    async fn revoke_maps_an_error_reply_to_query_error() {
        use arlen_permissions::revoke::{RevokeInitiator, RevokeReach, RevokedReach};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::UnixListener;

        let path = std::env::temp_dir().join("arlen-os-sdk-revoke-err-test.sock");
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).unwrap();

        let server = tokio::spawn(async move {
            let (mut conn, _) = listener.accept().await.unwrap();
            let mut len_buf = [0u8; 4];
            conn.read_exact(&mut len_buf).await.unwrap();
            let req_len = u32::from_be_bytes(len_buf) as usize;
            let mut req = vec![0u8; req_len];
            conn.read_exact(&mut req).await.unwrap();
            let resp = b"ERROR: revoke not permitted for this caller";
            conn.write_all(&(resp.len() as u32).to_be_bytes()).await.unwrap();
            conn.write_all(resp).await.unwrap();
        });

        let client = UnixGraphClient::new(path.to_string_lossy().to_string());
        let req = RevokeReach {
            target_app_id: "com.x".into(),
            reach: RevokedReach::InstanceAll,
            initiator: RevokeInitiator::User,
        };
        let outcome = client.revoke(&req).await;
        let _ = server.await;
        let _ = std::fs::remove_file(&path);

        assert!(outcome.is_err(), "an ERROR reply maps to a QueryError");
    }

    #[tokio::test]
    async fn create_node_sends_a_tagged_request_and_accepts_created() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::UnixListener;

        let path = std::env::temp_dir().join("arlen-os-sdk-create-node-test.sock");
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).unwrap();

        let server = tokio::spawn(async move {
            let (mut conn, _) = listener.accept().await.unwrap();
            let mut len_buf = [0u8; 4];
            conn.read_exact(&mut len_buf).await.unwrap();
            let req_len = u32::from_be_bytes(len_buf) as usize;
            let mut req = vec![0u8; req_len];
            conn.read_exact(&mut req).await.unwrap();

            assert_eq!(req[0], 0x02, "node writes carry the 0x02 prefix");
            let body: serde_json::Value = serde_json::from_slice(&req[1..]).unwrap();
            assert_eq!(body["op"], "create_node");
            assert_eq!(body["node_type"], "system.Summary");
            assert_eq!(body["id"], "s1");

            let ok = b"OK: created";
            conn.write_all(&(ok.len() as u32).to_be_bytes()).await.unwrap();
            conn.write_all(ok).await.unwrap();
        });

        let client = UnixGraphClient::new(path.to_string_lossy().to_string());
        let result = client.create_node("system.Summary", "s1").await;
        let _ = server.await;
        let _ = std::fs::remove_file(&path);

        assert!(
            matches!(result, Ok(NodeWriteOutcome::Created)),
            "an `OK: created` status must map to Created, got {result:?}"
        );
    }

    #[tokio::test]
    async fn create_relation_maps_a_permission_error() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::UnixListener;

        let path = std::env::temp_dir().join("arlen-os-sdk-write-denied-test.sock");
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).unwrap();

        let server = tokio::spawn(async move {
            let (mut conn, _) = listener.accept().await.unwrap();
            let mut len_buf = [0u8; 4];
            conn.read_exact(&mut len_buf).await.unwrap();
            let req_len = u32::from_be_bytes(len_buf) as usize;
            let mut req = vec![0u8; req_len];
            conn.read_exact(&mut req).await.unwrap();

            let err = b"ERROR: permission denied: cannot create relation";
            conn.write_all(&(err.len() as u32).to_be_bytes()).await.unwrap();
            conn.write_all(err).await.unwrap();
        });

        let client = UnixGraphClient::new(path.to_string_lossy().to_string());
        let result = client
            .create_relation("system.File", "f1", "system.Project", "p1", "FILE_PART_OF", "op-test")
            .await;
        let _ = server.await;
        let _ = std::fs::remove_file(&path);

        assert!(
            matches!(result, Err(QueryError::PermissionDenied)),
            "a permission ERROR must map to PermissionDenied, got {result:?}"
        );
    }

    #[tokio::test]
    async fn retract_relation_sends_a_tagged_request_and_maps_both_outcomes() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::UnixListener;

        let path = std::env::temp_dir().join("arlen-os-sdk-retract-test.sock");
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).unwrap();

        // The fake daemon answers the first request `OK: retracted` and the
        // second `OK: absent`, asserting the request shape on each. Both are
        // served on one connection, matching the client's stream caching.
        let server = tokio::spawn(async move {
            let (mut conn, _) = listener.accept().await.unwrap();
            for reply in [b"OK: retracted".as_slice(), b"OK: absent".as_slice()] {
                let mut len_buf = [0u8; 4];
                conn.read_exact(&mut len_buf).await.unwrap();
                let req_len = u32::from_be_bytes(len_buf) as usize;
                let mut req = vec![0u8; req_len];
                conn.read_exact(&mut req).await.unwrap();

                assert_eq!(req[0], 0x02, "write requests carry the 0x02 prefix");
                let body: serde_json::Value = serde_json::from_slice(&req[1..]).unwrap();
                assert_eq!(body["op"], "retract_relation");
                assert_eq!(body["relation_type"], "FILE_PART_OF");
                assert_eq!(body["op_id"], "op-test");

                conn.write_all(&(reply.len() as u32).to_be_bytes()).await.unwrap();
                conn.write_all(reply).await.unwrap();
            }
        });

        let client = UnixGraphClient::new(path.to_string_lossy().to_string());
        let first = client
            .retract_relation("system.File", "f1", "system.Project", "p1", "FILE_PART_OF", "op-test")
            .await;
        let second = client
            .retract_relation("system.File", "f1", "system.Project", "p1", "FILE_PART_OF", "op-test")
            .await;
        let _ = server.await;
        let _ = std::fs::remove_file(&path);

        assert!(matches!(first, Ok(RelationRetractOutcome::Retracted)), "got {first:?}");
        assert!(matches!(second, Ok(RelationRetractOutcome::Absent)), "got {second:?}");
    }

    #[tokio::test]
    async fn upsert_entity_sends_a_tagged_request_and_parses_ok() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::UnixListener;

        let path = std::env::temp_dir().join("arlen-os-sdk-upsert-entity-test.sock");
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).unwrap();

        let server = tokio::spawn(async move {
            let (mut conn, _) = listener.accept().await.unwrap();
            let mut len_buf = [0u8; 4];
            conn.read_exact(&mut len_buf).await.unwrap();
            let req_len = u32::from_be_bytes(len_buf) as usize;
            let mut req = vec![0u8; req_len];
            conn.read_exact(&mut req).await.unwrap();

            assert_eq!(req[0], 0x02, "entity writes carry the 0x02 write prefix");
            let body: serde_json::Value = serde_json::from_slice(&req[1..]).unwrap();
            assert_eq!(body["op"], "upsert_entity");
            assert_eq!(body["qualified_type"], "md.obsidian.Note");
            assert_eq!(body["external_key"], "note-1");
            assert_eq!(body["fields"]["title"], "Hello");

            let reply = b"OK: upserted";
            conn.write_all(&(reply.len() as u32).to_be_bytes()).await.unwrap();
            conn.write_all(reply).await.unwrap();
        });

        let client = UnixGraphClient::new(path.to_string_lossy().to_string());
        let mut fields = serde_json::Map::new();
        fields.insert("title".to_string(), serde_json::json!("Hello"));
        let result = client.upsert_entity("md.obsidian.Note", "note-1", &fields).await;
        let _ = server.await;
        let _ = std::fs::remove_file(&path);

        assert!(result.is_ok(), "got {result:?}");
    }

    #[tokio::test]
    async fn link_entities_sends_a_tagged_request_and_parses_ok() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::UnixListener;

        let path = std::env::temp_dir().join("arlen-os-sdk-link-entities-test.sock");
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).unwrap();

        let server = tokio::spawn(async move {
            let (mut conn, _) = listener.accept().await.unwrap();
            let mut len_buf = [0u8; 4];
            conn.read_exact(&mut len_buf).await.unwrap();
            let req_len = u32::from_be_bytes(len_buf) as usize;
            let mut req = vec![0u8; req_len];
            conn.read_exact(&mut req).await.unwrap();

            assert_eq!(req[0], 0x02, "entity writes carry the 0x02 write prefix");
            let body: serde_json::Value = serde_json::from_slice(&req[1..]).unwrap();
            assert_eq!(body["op"], "link_entities");
            assert_eq!(body["edge_type"], "LINKS_TO");
            assert_eq!(body["from_type"], "md.obsidian.Note");
            assert_eq!(body["from_key"], "note-1");
            assert_eq!(body["to_type"], "md.obsidian.Note");
            assert_eq!(body["to_key"], "note-2");

            let reply = b"OK: linked";
            conn.write_all(&(reply.len() as u32).to_be_bytes()).await.unwrap();
            conn.write_all(reply).await.unwrap();
        });

        let client = UnixGraphClient::new(path.to_string_lossy().to_string());
        let result = client
            .link_entities("LINKS_TO", "md.obsidian.Note", "note-1", "md.obsidian.Note", "note-2")
            .await;
        let _ = server.await;
        let _ = std::fs::remove_file(&path);

        assert!(result.is_ok(), "got {result:?}");
    }

    #[tokio::test]
    async fn persist_consent_grant_sends_a_tagged_request_and_parses_ok() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::UnixListener;

        let path = std::env::temp_dir().join("arlen-os-sdk-consent-grant-test.sock");
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).unwrap();

        let server = tokio::spawn(async move {
            let (mut conn, _) = listener.accept().await.unwrap();
            let mut len_buf = [0u8; 4];
            conn.read_exact(&mut len_buf).await.unwrap();
            let req_len = u32::from_be_bytes(len_buf) as usize;
            let mut req = vec![0u8; req_len];
            conn.read_exact(&mut req).await.unwrap();

            assert_eq!(req[0], 0x02, "consent-grant writes carry the 0x02 write prefix");
            let body: serde_json::Value = serde_json::from_slice(&req[1..]).unwrap();
            assert_eq!(body["op"], "persist_consent_grant");
            assert_eq!(body["recipient"], "org.arlen.files");
            assert_eq!(body["consent_class"], "Destructive");
            assert_eq!(body["consent_scope"], "/home/x");
            assert_eq!(body["revocation_handle"], "rh-1");

            let reply = b"OK: persisted";
            conn.write_all(&(reply.len() as u32).to_be_bytes()).await.unwrap();
            conn.write_all(reply).await.unwrap();
        });

        let client = UnixGraphClient::new(path.to_string_lossy().to_string());
        let result = client
            .persist_consent_grant("org.arlen.files", "Destructive", Some("/home/x"), "rh-1")
            .await;
        let _ = server.await;
        let _ = std::fs::remove_file(&path);

        assert!(result.is_ok(), "got {result:?}");
    }

    #[tokio::test]
    async fn upsert_entity_maps_a_daemon_error() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::UnixListener;

        let path = std::env::temp_dir().join("arlen-os-sdk-upsert-entity-err-test.sock");
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).unwrap();

        let server = tokio::spawn(async move {
            let (mut conn, _) = listener.accept().await.unwrap();
            let mut len_buf = [0u8; 4];
            conn.read_exact(&mut len_buf).await.unwrap();
            let req_len = u32::from_be_bytes(len_buf) as usize;
            let mut req = vec![0u8; req_len];
            conn.read_exact(&mut req).await.unwrap();

            let reply = b"ERROR: namespace violation: md.obsidian cannot write com.other.Note";
            conn.write_all(&(reply.len() as u32).to_be_bytes()).await.unwrap();
            conn.write_all(reply).await.unwrap();
        });

        let client = UnixGraphClient::new(path.to_string_lossy().to_string());
        let fields = serde_json::Map::new();
        let result = client.upsert_entity("com.other.Note", "k1", &fields).await;
        let _ = server.await;
        let _ = std::fs::remove_file(&path);

        assert!(result.is_err(), "a daemon ERROR must surface as Err");
    }
}

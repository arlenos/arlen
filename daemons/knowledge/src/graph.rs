use anyhow::{anyhow, Result};
use lbug::{Connection, Database, SystemConfig, Value};
use std::thread;
use tokio::sync::mpsc;
use tracing::{debug, info};

/// A cell value extracted from a Ladybug QueryResult, safe to send
/// across threads.
#[derive(Debug, Clone)]
pub enum CellValue {
    Null,
    String(String),
    Int64(i64),
    Bool(bool),
    Float(f64),
}

impl CellValue {
    /// Extract a string reference, returning empty string for non-string values.
    pub fn as_str(&self) -> &str {
        match self {
            CellValue::String(s) => s,
            _ => "",
        }
    }

    /// Extract an i64, returning 0 for non-integer values.
    pub fn as_i64(&self) -> i64 {
        match self {
            CellValue::Int64(i) => *i,
            _ => 0,
        }
    }

    /// Extract a bool, returning false for non-boolean values.
    pub fn as_bool(&self) -> bool {
        match self {
            CellValue::Bool(b) => *b,
            _ => false,
        }
    }
}

/// Structured query result with column names and typed rows.
#[derive(Debug, Clone)]
pub struct RowSet {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<CellValue>>,
}

/// Convert a Ladybug [`Value`] directly to a JSON value for the typed query
/// response, faithfully or not at all. Returns `None` for anything JSON
/// cannot represent without loss — a non-finite float, or a complex/temporal/
/// binary value — so the query fails rather than silently shipping a wrapped
/// integer or a `Display` string the client would mistake for typed data.
/// (The KG's stored fields are string/int64/bool, so the fallible cases do
/// not arise for real node fields; they only guard exotic query expressions.)
fn value_to_json(v: Value) -> Option<serde_json::Value> {
    use serde_json::Value as J;
    Some(match v {
        Value::Null(_) => J::Null,
        Value::String(s) => J::String(s),
        Value::Bool(b) => J::Bool(b),
        Value::Int64(i) => J::Number(i.into()),
        Value::Int32(i) => J::Number(i64::from(i).into()),
        Value::Int16(i) => J::Number(i64::from(i).into()),
        Value::Int8(i) => J::Number(i64::from(i).into()),
        // Unsigned: preserved as-is (JSON numbers cover u64), never `as i64`.
        Value::UInt64(i) => J::Number(i.into()),
        Value::UInt32(i) => J::Number(u64::from(i).into()),
        Value::UInt16(i) => J::Number(u64::from(i).into()),
        Value::UInt8(i) => J::Number(u64::from(i).into()),
        Value::Double(f) => J::Number(serde_json::Number::from_f64(f)?),
        Value::Float(f) => J::Number(serde_json::Number::from_f64(f64::from(f))?),
        // Int128, temporal, blob, list, struct, etc.: not faithfully
        // representable here, so fail closed rather than stringify.
        _ => return None,
    })
}

/// A conservative upper bound on a cell's serialised cost, used to bound a
/// typed response *before* serialising it. Includes fixed structural overhead
/// (quotes, the separating comma, array brackets) so an empty or short cell is
/// never counted as free, and doubles string length to cover worst-case JSON
/// escaping (a control char becomes `\uXXXX`). Overcounting only makes the cap
/// fire sooner, which is the safe direction.
fn cell_cost(v: &serde_json::Value) -> usize {
    const OVERHEAD: usize = 8;
    OVERHEAD
        + match v {
            serde_json::Value::String(s) => s.len().saturating_mul(2),
            _ => 24,
        }
}

/// A message sent to the Ladybug thread.
/// Each variant carries a one-shot channel to send the result back.
///
/// The request channel is a bounded `tokio::sync::mpsc` (1024 slots) because
/// Ladybug's Connection is not Send and must stay on the dedicated thread.
/// Async callers `send().await` the request (which yields cooperatively under
/// backpressure instead of blocking a runtime worker, the difference that lets
/// the query daemon's per-request timeout actually bound the client wait), and
/// then await a tokio oneshot for the response. The Ladybug thread is a plain
/// OS thread with no runtime and drains the receiver via `blocking_recv`.
pub enum GraphRequest {
    /// Execute a Cypher query and return the raw result as a string.
    Query {
        cypher: String,
        reply: tokio::sync::oneshot::Sender<Result<String>>,
    },
    /// Execute a Cypher query and return structured rows.
    QueryRows {
        cypher: String,
        reply: tokio::sync::oneshot::Sender<Result<RowSet>>,
    },
    /// Execute a Cypher query and return structured rows serialised to a JSON
    /// string. The serialisation happens on the Ladybug thread, inside the
    /// work the query daemon's deadline covers, so a large typed result is
    /// bounded by the same client-wait timeout as a text query rather than
    /// serialising unbounded after it.
    QueryRowsJson {
        cypher: String,
        reply: tokio::sync::oneshot::Sender<Result<String>>,
    },
    /// Run several Cypher statements under one transaction on the serial thread,
    /// committing all or none (bitemporal-knowledge-graph.md §4.5). For the
    /// genuinely multi-statement writes that cannot be one Cypher query (a
    /// node-create plus its edges, §5.3); single-statement writes stay a plain
    /// `Query`, which is already atomic on the serial thread.
    Transaction {
        statements: Vec<String>,
        reply: tokio::sync::oneshot::Sender<Result<()>>,
    },
    /// Shut down the Ladybug thread cleanly.
    Shutdown,
}

/// Maximum rows a typed (JSON) query result may carry. A query exceeding it
/// fails rather than serialising an unbounded response; the agent's targeted
/// slices are far below this.
const MAX_TYPED_ROWS: usize = 10_000;

/// Maximum approximate byte size of a typed result's cells, so a query with
/// few rows but huge string cells is also bounded (the row cap alone is not).
const MAX_TYPED_BYTES: usize = 4 * 1024 * 1024;

/// Maximum columns a typed result may have, so a very wide result cannot
/// multiply the cell count past what the row + byte caps assume. The agent's
/// targeted slices return a handful of columns.
const MAX_TYPED_COLUMNS: usize = 256;

/// Handle to the dedicated Ladybug thread.
/// Clone this to get additional senders to the same thread.
#[derive(Clone)]
pub struct GraphHandle {
    sender: mpsc::Sender<GraphRequest>,
}

impl GraphHandle {
    /// Execute a read-only Cypher query and return the result as a string.
    ///
    /// This sends the query to the dedicated Ladybug thread and awaits
    /// the response on a tokio oneshot channel.
    pub async fn query(&self, cypher: String) -> Result<String> {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        self.sender
            .send(GraphRequest::Query {
                cypher,
                reply: reply_tx,
            })
            .await
            .map_err(|_| anyhow!("ladybug thread has stopped"))?;
        reply_rx
            .await
            .map_err(|_| anyhow!("ladybug thread dropped reply sender"))?
    }

    /// Write a node or relationship to Ladybug.
    /// Internally this is just a query that happens to be a write.
    pub async fn write(&self, cypher: String) -> Result<String> {
        self.query(cypher).await
    }

    /// Execute a Cypher query and return structured rows (async).
    pub async fn query_rows(&self, cypher: String) -> Result<RowSet> {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        self.sender
            .send(GraphRequest::QueryRows {
                cypher,
                reply: reply_tx,
            })
            .await
            .map_err(|_| anyhow!("ladybug thread has stopped"))?;
        reply_rx
            .await
            .map_err(|_| anyhow!("ladybug thread dropped reply sender"))?
    }

    /// Run several Cypher statements atomically under one transaction: all of
    /// them commit, or (on any statement error) none do. For a multi-statement
    /// write that cannot be one Cypher query, such as a node-create plus its
    /// edges (§5.3); a single-statement write should use [`write`](Self::write),
    /// which is already atomic on the serial thread. An empty statement list is a
    /// no-op success.
    pub async fn transaction(&self, statements: Vec<String>) -> Result<()> {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        self.sender
            .send(GraphRequest::Transaction {
                statements,
                reply: reply_tx,
            })
            .await
            .map_err(|_| anyhow!("ladybug thread has stopped"))?;
        reply_rx
            .await
            .map_err(|_| anyhow!("ladybug thread dropped reply sender"))?
    }

    /// Execute a Cypher query and return structured rows serialised to a JSON
    /// string (async). Serialisation runs on the Ladybug thread, so the query
    /// daemon's per-request deadline bounds it together with the query.
    pub async fn query_rows_json(&self, cypher: String) -> Result<String> {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        self.sender
            .send(GraphRequest::QueryRowsJson {
                cypher,
                reply: reply_tx,
            })
            .await
            .map_err(|_| anyhow!("ladybug thread has stopped"))?;
        reply_rx
            .await
            .map_err(|_| anyhow!("ladybug thread dropped reply sender"))?
    }

    /// Execute a Cypher query and return structured rows (blocking).
    ///
    /// Intended for use on non-tokio threads (e.g. the FUSE thread).
    /// Must NOT be called from within a tokio async context.
    pub fn query_rows_sync(&self, cypher: String) -> Result<RowSet> {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        self.sender
            .blocking_send(GraphRequest::QueryRows {
                cypher,
                reply: reply_tx,
            })
            .map_err(|_| anyhow!("ladybug thread has stopped"))?;
        reply_rx
            .blocking_recv()
            .map_err(|_| anyhow!("ladybug thread dropped reply sender"))?
    }
}

/// Spawn the dedicated Ladybug thread and return a handle to it.
///
/// The thread opens the database at `path`, creates the schema if needed,
/// and then loops waiting for `GraphRequest` messages. It runs until it
/// receives `GraphRequest::Shutdown` or the channel is closed.
///
/// # Why a dedicated thread?
/// `lbug::Connection` is not `Send`. It cannot be moved between threads or
/// shared across async tasks. Keeping it on one dedicated thread and
/// communicating via channels is the standard pattern for non-Send resources
/// in async Rust. It is similar to how you would marshal calls to a COM object
/// on a single-threaded apartment in Windows.
pub fn spawn(path: &str) -> Result<GraphHandle> {
    let path = path.to_string();

    // Bounded channel with 1024 pending-request slots. If the Ladybug thread
    // falls behind, async senders await (yielding the worker) rather than
    // blocking it. 1024 is generous; normal load is much lower.
    let (tx, rx) = mpsc::channel::<GraphRequest>(1024);

    thread::Builder::new()
        .name("ladybug".to_string())
        .spawn(move || {
            if let Err(e) = ladybug_thread(&path, rx) {
                tracing::error!("ladybug thread exited with error: {e}");
            }
        })?;

    Ok(GraphHandle { sender: tx })
}

/// The body of the dedicated Ladybug thread.
fn ladybug_thread(path: &str, mut rx: mpsc::Receiver<GraphRequest>) -> Result<()> {
    let db = Database::new(path, SystemConfig::default())
        .map_err(|e| anyhow!("failed to open ladybug database: {e}"))?;
    let conn = Connection::new(&db)
        .map_err(|e| anyhow!("failed to create ladybug connection: {e}"))?;

    info!(path, "ladybug database opened");
    create_schema(&conn)?;
    info!("ladybug schema ready");

    while let Some(request) = rx.blocking_recv() {
        match request {
            GraphRequest::Query { cypher, reply } => {
                debug!(cypher = %cypher, "executing cypher");
                let result = conn
                    .query(&cypher)
                    .map(|r| r.to_string())
                    .map_err(|e| anyhow!("{e}"));
                // If the caller dropped the oneshot receiver we just ignore the error.
                reply.send(result).ok();
            }
            GraphRequest::QueryRows { cypher, reply } => {
                debug!(cypher = %cypher, "executing cypher (rows)");
                let result = conn.query(&cypher).map_err(|e| anyhow!("{e}"));
                let row_set = result.map(|mut qr| {
                    let columns = qr.get_column_names();
                    let rows = qr
                        .by_ref()
                        .map(|row| row.into_iter().map(value_to_cell).collect())
                        .collect();
                    RowSet { columns, rows }
                });
                reply.send(row_set).ok();
            }
            GraphRequest::QueryRowsJson { cypher, reply } => {
                debug!(cypher = %cypher, "executing cypher (rows -> json)");
                // Build and serialise on this thread, so the work is bounded
                // by the daemon's per-request deadline (the text path
                // serialises here too). Bound the result by row count AND
                // byte size, and stop early if the caller has already given
                // up (its reply channel closed), so a large result cannot
                // monopolise the thread or grow unbounded after a timeout.
                // (A single Kuzu query that itself runs past the deadline
                // still cannot be aborted mid-execution; that interruptible
                // graph API is the same documented follow-up the text path
                // notes.)
                // The lbug result iterator converts each cell with an internal
                // `try_into().unwrap()`, so an unrepresentable database value
                // (e.g. an out-of-range temporal) panics *during* iteration,
                // before `value_to_json` can reject it. Arbitrary client Cypher
                // reaches this path, so contain any such panic: it becomes an
                // error and the dedicated graph thread keeps serving instead of
                // unwinding and taking the whole graph service down.
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                    || -> Result<String> {
                        // Drop a request whose caller already gave up before we
                        // even execute it (it may have queued behind a slow one).
                        if reply.is_closed() {
                            return Err(anyhow!("client disconnected before execution"));
                        }
                        let mut qr = conn.query(&cypher).map_err(|e| anyhow!("{e}"))?;
                        let columns = qr.get_column_names();
                        if columns.len() > MAX_TYPED_COLUMNS {
                            return Err(anyhow!(
                                "typed result exceeds {MAX_TYPED_COLUMNS} columns"
                            ));
                        }
                        let mut rows: Vec<Vec<serde_json::Value>> = Vec::new();
                        let mut bytes = 0usize;
                        for row in qr.by_ref() {
                            if reply.is_closed() {
                                return Err(anyhow!(
                                    "client disconnected before the result was ready"
                                ));
                            }
                            if rows.len() >= MAX_TYPED_ROWS {
                                return Err(anyhow!("typed result exceeds {MAX_TYPED_ROWS} rows"));
                            }
                            let mut cells = Vec::with_capacity(row.len());
                            for v in row {
                                let cell = value_to_json(v).ok_or_else(|| {
                                    anyhow!(
                                        "query result has a value JSON cannot represent faithfully"
                                    )
                                })?;
                                // Bound per cell, not per row: a single wide
                                // row of large strings must trip the cap before
                                // it is fully materialised.
                                bytes = bytes.saturating_add(cell_cost(&cell));
                                if bytes > MAX_TYPED_BYTES {
                                    return Err(anyhow!(
                                        "typed result exceeds {MAX_TYPED_BYTES} bytes"
                                    ));
                                }
                                cells.push(cell);
                            }
                            rows.push(cells);
                        }
                        Ok(serde_json::json!({ "columns": columns, "rows": rows }).to_string())
                    },
                ))
                .unwrap_or_else(|_| {
                    Err(anyhow!("the graph engine could not convert a value in the result"))
                });
                reply.send(result).ok();
            }
            GraphRequest::Transaction { statements, reply } => {
                debug!(count = statements.len(), "executing transaction");
                reply.send(run_transaction(&conn, &statements)).ok();
            }
            GraphRequest::Shutdown => {
                info!("ladybug thread shutting down");
                break;
            }
        }
    }

    Ok(())
}

/// Run `statements` under one Kuzu transaction on the calling (serial) thread:
/// `BEGIN TRANSACTION`, each statement in order, `COMMIT`; any statement error
/// triggers a `ROLLBACK` and returns the error, so the write is all-or-nothing.
/// An empty list is a no-op success. lbug exposes no transaction handle, so the
/// boundaries are Cypher statements (standard Kuzu `BEGIN TRANSACTION` /
/// `COMMIT` / `ROLLBACK`). Runs on the serial graph thread, so no other request
/// interleaves between BEGIN and COMMIT.
fn run_transaction(conn: &Connection, statements: &[String]) -> Result<()> {
    if statements.is_empty() {
        return Ok(());
    }
    conn.query("BEGIN TRANSACTION")
        .map_err(|e| anyhow!("begin transaction: {e}"))?;
    for stmt in statements {
        if let Err(e) = conn.query(stmt) {
            // Best-effort rollback; surface the original statement error.
            let _ = conn.query("ROLLBACK");
            return Err(anyhow!("transaction statement failed: {e}"));
        }
    }
    conn.query("COMMIT").map_err(|e| {
        let _ = conn.query("ROLLBACK");
        anyhow!("commit failed: {e}")
    })?;
    Ok(())
}

/// Convert a Ladybug Value to a thread-safe CellValue.
fn value_to_cell(v: Value) -> CellValue {
    match v {
        Value::String(s) => CellValue::String(s),
        Value::Int64(i) => CellValue::Int64(i),
        Value::Int32(i) => CellValue::Int64(i64::from(i)),
        Value::Int16(i) => CellValue::Int64(i64::from(i)),
        Value::Int8(i) => CellValue::Int64(i64::from(i)),
        Value::UInt64(i) => CellValue::Int64(i as i64),
        Value::UInt32(i) => CellValue::Int64(i64::from(i)),
        Value::UInt16(i) => CellValue::Int64(i64::from(i)),
        Value::UInt8(i) => CellValue::Int64(i64::from(i)),
        Value::Bool(b) => CellValue::Bool(b),
        Value::Double(f) => CellValue::Float(f),
        Value::Float(f) => CellValue::Float(f64::from(f)),
        Value::Null(_) => CellValue::Null,
        other => CellValue::String(other.to_string()),
    }
}

/// Create the Knowledge Graph node and relationship tables.
///
/// Uses `CREATE ... IF NOT EXISTS` so this is safe to call on every startup.
/// Schema changes require a migration strategy; for Phase 1A we keep the
/// schema minimal and stable.
fn create_schema(conn: &Connection) -> Result<()> {
    // Node tables
    conn.query(
        "CREATE NODE TABLE IF NOT EXISTS File(
            id          STRING,
            path        STRING,
            app_id      STRING,
            last_accessed INT64,
            last_cgroup_id INT64,
            PRIMARY KEY(id)
        )",
    )
    .map_err(|e| anyhow!("create File table: {e}"))?;

    conn.query(
        "CREATE NODE TABLE IF NOT EXISTS App(
            id      STRING,
            name    STRING,
            PRIMARY KEY(id)
        )",
    )
    .map_err(|e| anyhow!("create App table: {e}"))?;

    conn.query(
        "CREATE NODE TABLE IF NOT EXISTS Session(
            id         STRING,
            started_at INT64,
            PRIMARY KEY(id)
        )",
    )
    .map_err(|e| anyhow!("create Session table: {e}"))?;

    conn.query(
        "CREATE NODE TABLE IF NOT EXISTS Event(
            id         STRING,
            type       STRING,
            timestamp  INT64,
            source     STRING,
            title      STRING,
            PRIMARY KEY(id)
        )",
    )
    .map_err(|e| anyhow!("create Event table: {e}"))?;

    conn.query(
        "CREATE NODE TABLE IF NOT EXISTS UserAction(
            id        STRING,
            category  STRING,
            action    STRING,
            subject   STRING,
            timestamp INT64,
            PRIMARY KEY(id)
        )",
    )
    .map_err(|e| anyhow!("create UserAction table: {e}"))?;

    // Reserved git node types. The foundation reserves Commit and Branch from the
    // start so a future git-ingestion tier can add rows without a schema
    // migration (Arlen's no-migration promise). No producer writes them yet;
    // fields beyond these conventional ones are added additively via
    // `ALTER TABLE ... ADD IF NOT EXISTS` when the ingestion lands, the same way
    // the other tables evolve.
    conn.query(
        "CREATE NODE TABLE IF NOT EXISTS Commit(
            id           STRING,
            message      STRING,
            author       STRING,
            author_email STRING,
            committed_at INT64,
            PRIMARY KEY(id)
        )",
    )
    .map_err(|e| anyhow!("create Commit table: {e}"))?;

    conn.query(
        "CREATE NODE TABLE IF NOT EXISTS Branch(
            id   STRING,
            name STRING,
            head STRING,
            PRIMARY KEY(id)
        )",
    )
    .map_err(|e| anyhow!("create Branch table: {e}"))?;

    // Relationship tables
    conn.query(
        "CREATE REL TABLE IF NOT EXISTS ACCESSED_BY(FROM File TO App)",
    )
    .map_err(|e| anyhow!("create ACCESSED_BY rel: {e}"))?;

    conn.query(
        "CREATE REL TABLE IF NOT EXISTS ACTIVE_IN(FROM App TO Session)",
    )
    .map_err(|e| anyhow!("create ACTIVE_IN rel: {e}"))?;

    conn.query(
        "CREATE REL TABLE IF NOT EXISTS EMITTED_BY(FROM Event TO App)",
    )
    .map_err(|e| anyhow!("create EMITTED_BY rel: {e}"))?;

    conn.query(
        "CREATE REL TABLE IF NOT EXISTS DERIVED_FROM(FROM UserAction TO Event)",
    )
    .map_err(|e| anyhow!("create DERIVED_FROM rel: {e}"))?;

    // Project system: project detection and file association.
    conn.query(
        "CREATE NODE TABLE IF NOT EXISTS Project(
            id             STRING,
            name           STRING,
            description    STRING,
            root_path      STRING,
            accent_color   STRING,
            icon           STRING,
            status         STRING,
            created_at     INT64,
            last_accessed  INT64,
            inferred       BOOL,
            confidence     INT64,
            promoted       BOOL,
            archived_at    INT64,
            PRIMARY KEY(id)
        )",
    )
    .map_err(|e| anyhow!("create Project table: {e}"))?;
    // Transaction-time close stamp for the node lifecycle (§4.9): archiving a
    // project is "the system stopped believing it is active", which is
    // `expired_at`. A live project is `expired_at IS NULL`; `status`/`archived_at`
    // stay as denormalised read filters. Convergent ADD IF NOT EXISTS, as for the
    // edge temporal columns.
    conn.query("ALTER TABLE Project ADD IF NOT EXISTS expired_at INT64")
        .map_err(|e| anyhow!("ensure Project.expired_at column: {e}"))?;

    // The window/event title carried on a promoted Event (e.g. window.focused
    // records the focused window's title). Without this column the window.focused
    // promotion's `SET e.title` is a binder error that stalls the whole promotion
    // batch. Convergent ADD IF NOT EXISTS for already-initialized DBs.
    conn.query("ALTER TABLE Event ADD IF NOT EXISTS title STRING")
        .map_err(|e| anyhow!("ensure Event.title column: {e}"))?;

    // The cgroup v2 id of the most recent open (Strand 4 attribution). A File node
    // is path-keyed, so this is the LATEST cgroup, not a history; NULL/0 means no
    // eBPF attribution. Convergent ADD IF NOT EXISTS for already-initialized DBs.
    conn.query("ALTER TABLE File ADD IF NOT EXISTS last_cgroup_id INT64")
        .map_err(|e| anyhow!("ensure File.last_cgroup_id column: {e}"))?;

    conn.query(
        "CREATE NODE TABLE IF NOT EXISTS Directory(
            id         STRING,
            path       STRING,
            name       STRING,
            project_id STRING,
            created_at INT64,
            PRIMARY KEY(id)
        )",
    )
    .map_err(|e| anyhow!("create Directory table: {e}"))?;

    // `op_id` records the agent operation that created an edge (durable
    // operation identity), so a write whose response was lost can be reconciled
    // by asking whether *this* operation's edge exists. Backward compatible: the
    // promotion pipeline's own `FILE_PART_OF` creates omit it and it defaults to
    // NULL, so only the agent's write socket sets it.
    conn.query(
        "CREATE REL TABLE IF NOT EXISTS FILE_PART_OF(FROM File TO Project, op_id STRING)",
    )
    .map_err(|e| anyhow!("create FILE_PART_OF rel: {e}"))?;
    // Schema convergence for a store created before `op_id` existed: the
    // `CREATE ... IF NOT EXISTS` above no-ops on an existing table and would
    // leave it without the column, so the agent's op-id-keyed create/retract
    // would fail at runtime. `ADD IF NOT EXISTS` is idempotent (a no-op when the
    // column is already present, as on a fresh store), so this brings any store
    // up to the declared schema. Not a migration shim: it is the same
    // declarative schema, made convergent regardless of when the store was made.
    conn.query("ALTER TABLE FILE_PART_OF ADD IF NOT EXISTS op_id STRING")
        .map_err(|e| anyhow!("ensure FILE_PART_OF.op_id column: {e}"))?;

    // Bi-temporal stamps + the provenance quad on the agent-curated assertion
    // edge (bitemporal-knowledge-graph.md §4.1). Four INT64-micros stamps: two
    // transaction-axis (created_at = when Arlen learned the fact, expired_at =
    // when it learned the fact was superseded) and two valid-axis (valid_at =
    // when it became true in the world, invalid_at = when it stopped). NULL on
    // either axis means open (eternal / still believed), so every edge that
    // exists today reads as an always-known eternal fact with no backfill (§4.3).
    // The provenance columns (origin, prov_beh) record who asserted it and
    // `superseded` back-references the edge a supersession replaced (§4.6). Same
    // convergent `ADD IF NOT EXISTS` pattern proven for `op_id`.
    //
    // `merge_key` is the content-addressed identity of the membership fact,
    // `digest(from, rel, to)` (graph-drift.md §2 / GD-R1), orthogonal to `op_id`:
    // `op_id` is the per-device write idempotency/crash-replay key (different on
    // each device), `merge_key` is the same on every device that asserts the same
    // fact, so a future cross-device union dedups two writes of "f1 PART_OF p1" to
    // one membership identity. It is the merge-prep column the resolve pass
    // (GD-R2) keys on; on a single device it is hardening-only (set but not yet
    // merged on). Same convergent `ADD IF NOT EXISTS` pattern.
    //
    // GD-R2 obligation: only the agent write path stamps this today; the
    // promotion pipeline's own FILE_PART_OF creates leave it NULL (as they do
    // `op_id`). When GD-R2's resolve pass starts deduping on `merge_key`, a
    // promoted (NULL) and an agent-created (stamped) edge for the SAME membership
    // would read as distinct facts, so that pass must either stamp promotion
    // edges with the same content key or special-case NULL.
    for column in [
        "valid_at INT64",
        "invalid_at INT64",
        "created_at INT64",
        "expired_at INT64",
        "origin STRING",
        "prov_beh STRING",
        "superseded STRING",
        "merge_key STRING",
    ] {
        conn.query(&format!(
            "ALTER TABLE FILE_PART_OF ADD IF NOT EXISTS {column}"
        ))
        .map_err(|e| anyhow!("ensure FILE_PART_OF.{column} column: {e}"))?;
    }

    conn.query(
        "CREATE REL TABLE IF NOT EXISTS DIR_PART_OF(FROM Directory TO Project)",
    )
    .map_err(|e| anyhow!("create DIR_PART_OF rel: {e}"))?;

    // Annotation: structured per-app metadata attached to existing graph
    // nodes. Foundation §395. The composite identity is
    // (target_type, target_id, namespace) — a re-set on the same
    // triple replaces the previous data. We store target as
    // properties rather than edges so the schema stays flat across
    // target types (File, App, Project, Session, ...) and so the
    // common "fetch all annotations targeting X" query is a single
    // property scan.
    conn.query(
        "CREATE NODE TABLE IF NOT EXISTS Annotation(
            id            STRING,
            namespace     STRING,
            target_type   STRING,
            target_id     STRING,
            data          STRING,
            created_at    INT64,
            last_modified INT64,
            PRIMARY KEY(id)
        )",
    )
    .map_err(|e| anyhow!("create Annotation table: {e}"))?;

    // Annotation history (bitemporal-knowledge-graph.md §4.8): the mutable
    // `Annotation.data` is reified onto a temporal `HAS_VERSION` edge to an
    // `AnnotationVersion` content node, so a `set` closes the live version and
    // appends a new one rather than overwriting (prior value lost). The
    // `Annotation` node stays the stable identity (its deterministic id, which
    // the SDK queries by the (target_type, target_id, namespace) triple); the
    // value lives on the versioned edge.
    conn.query(
        "CREATE NODE TABLE IF NOT EXISTS AnnotationVersion(
            id          STRING,
            data        STRING,
            recorded_at INT64,
            PRIMARY KEY(id)
        )",
    )
    .map_err(|e| anyhow!("create AnnotationVersion table: {e}"))?;
    // The versioned edge carries the four bi-temporal stamps (§4.1) plus `op_id`,
    // exactly like FILE_PART_OF, so a `set` is a close-then-append and history is
    // retained. Convergent column adds for a store created before they existed.
    conn.query(
        "CREATE REL TABLE IF NOT EXISTS HAS_VERSION(FROM Annotation TO AnnotationVersion)",
    )
    .map_err(|e| anyhow!("create HAS_VERSION rel: {e}"))?;
    for column in [
        "valid_at INT64",
        "invalid_at INT64",
        "created_at INT64",
        "expired_at INT64",
        "op_id STRING",
    ] {
        conn.query(&format!("ALTER TABLE HAS_VERSION ADD IF NOT EXISTS {column}"))
            .map_err(|e| anyhow!("ensure HAS_VERSION.{column} column: {e}"))?;
    }

    // Retention policy: summary nodes for compacted old data.
    conn.query(
        "CREATE NODE TABLE IF NOT EXISTS Summary(
            id                   STRING,
            type                 STRING,
            app_id               STRING,
            access_count         INT64,
            primary_application  STRING,
            active_period_start  INT64,
            active_period_end    INT64,
            PRIMARY KEY(id)
        )",
    )
    .map_err(|e| anyhow!("create Summary table: {e}"))?;

    conn.query(
        "CREATE REL TABLE IF NOT EXISTS SUMMARIZES(FROM Summary TO App)",
    )
    .map_err(|e| anyhow!("create SUMMARIZES rel: {e}"))?;

    // Pin marker: separate node table to mark nodes as permanent.
    // Using a separate table avoids ALTER TABLE on existing node tables.
    conn.query(
        "CREATE NODE TABLE IF NOT EXISTS PinnedMarker(
            id         STRING,
            node_id    STRING,
            node_type  STRING,
            pinned_at  INT64,
            PRIMARY KEY(id)
        )",
    )
    .map_err(|e| anyhow!("create PinnedMarker table: {e}"))?;

    // Living Capability Graph (living-capability-graph.md §3): the capability
    // projection. One Grant node per (app_id, token.id) recording an app's
    // declared reach and its lifecycle state; CapabilityUse the effective-use
    // tier (one per (app_id, capability), populated only once the usage feed
    // lands); EntityType a marker per grantable type string. These are
    // daemon-internal (written by emit_grant_node, not by clients), and the
    // general read path denies these labels to non-privileged callers, so a
    // co-tenant cannot harvest the whole machine's authority graph.
    conn.query(
        "CREATE NODE TABLE IF NOT EXISTS Grant(
            id                STRING,
            app_id            STRING,
            pid               INT64,
            issued_at         INT64,
            expires_at        INT64,
            declared_ceiling  STRING,
            required          BOOL,
            identity_verified BOOL,
            live              BOOL,
            revoked           BOOL,
            superseded        BOOL,
            last_exercised_at INT64,
            use_count         INT64,
            PRIMARY KEY(id)
        )",
    )
    .map_err(|e| anyhow!("create Grant table: {e}"))?;

    conn.query(
        "CREATE NODE TABLE IF NOT EXISTS CapabilityUse(
            id             STRING,
            app_id         STRING,
            capability     STRING,
            first_observed INT64,
            last_observed  INT64,
            observe_count  INT64,
            PRIMARY KEY(id)
        )",
    )
    .map_err(|e| anyhow!("create CapabilityUse table: {e}"))?;

    conn.query(
        "CREATE NODE TABLE IF NOT EXISTS EntityType(
            id    STRING,
            label STRING,
            PRIMARY KEY(id)
        )",
    )
    .map_err(|e| anyhow!("create EntityType table: {e}"))?;

    // GRANTS carries the queryable type projection (one hop = "which types can
    // this app reach"); deliberately NOT in BUILTIN_RELATIONS (it is never
    // client-writable). USED_BY ties a Grant to its App; LAST_EXERCISED points a
    // CapabilityUse at the audit event that last exercised it (once the feed
    // lands).
    conn.query("CREATE REL TABLE IF NOT EXISTS GRANTS(FROM Grant TO EntityType)")
        .map_err(|e| anyhow!("create GRANTS rel: {e}"))?;
    conn.query("CREATE REL TABLE IF NOT EXISTS USED_BY(FROM Grant TO App)")
        .map_err(|e| anyhow!("create USED_BY rel: {e}"))?;
    conn.query("CREATE REL TABLE IF NOT EXISTS LAST_EXERCISED(FROM CapabilityUse TO Event)")
        .map_err(|e| anyhow!("create LAST_EXERCISED rel: {e}"))?;

    debug!("schema created");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn value_to_json_preserves_types() {
        assert_eq!(value_to_json(Value::Int64(3)), Some(serde_json::json!(3)));
        assert_eq!(value_to_json(Value::Bool(true)), Some(serde_json::json!(true)));
        assert_eq!(
            value_to_json(Value::String("p1".into())),
            Some(serde_json::json!("p1"))
        );
    }

    #[test]
    fn unsigned_64bit_is_preserved_not_wrapped() {
        // The old CellValue path did `UInt64 as i64`, wrapping a large value
        // to a negative; the direct converter keeps it faithful.
        let big = u64::MAX;
        assert_eq!(value_to_json(Value::UInt64(big)), Some(serde_json::json!(big)));
    }

    #[test]
    fn a_string_with_a_delimiter_or_newline_is_json_escaped() {
        // The reason for the typed path: JSON escaping makes a value with `|`
        // or a newline safe, where the pipe-delimited text would corrupt it.
        let v = value_to_json(Value::String("a|b\nc".into())).unwrap();
        assert_eq!(v, serde_json::json!("a|b\nc"));
        // Round-trips through serialisation intact.
        let json = serde_json::json!({ "rows": [[v]] }).to_string();
        let back: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(back["rows"][0][0], serde_json::json!("a|b\nc"));
    }

    #[test]
    fn a_non_finite_float_fails_rather_than_corrupting() {
        assert_eq!(value_to_json(Value::Double(f64::NAN)), None);
        assert_eq!(value_to_json(Value::Double(f64::INFINITY)), None);
    }

    #[test]
    fn cell_cost_charges_overhead_so_empty_cells_are_not_free() {
        // An empty/short cell must still cost something (structural overhead),
        // so a wide result of empty strings cannot slip under the byte cap.
        assert!(cell_cost(&serde_json::json!("")) >= 8);
        assert!(cell_cost(&serde_json::json!("ab")) > cell_cost(&serde_json::json!("")));
        // Strings are charged extra for worst-case escaping.
        let long = "x".repeat(100);
        assert!(cell_cost(&serde_json::json!(long)) >= 200);
        // A wide row of empty strings accrues real cost.
        let row_cost: usize = (0..256).map(|_| cell_cost(&serde_json::json!(""))).sum();
        assert!(row_cost >= 256 * 8);
    }

    #[test]
    fn create_schema_adds_op_id_to_a_pre_op_id_file_part_of() {
        // Simulate a store created before `op_id` existed: FILE_PART_OF without
        // the column. create_schema must converge it (via ALTER ADD IF NOT
        // EXISTS) so the agent's op-id-keyed create/retract work, rather than
        // leaving an old table that fails every op-id query at runtime.
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("graph");
        let db = Database::new(path.to_str().unwrap(), SystemConfig::default()).unwrap();
        let conn = Connection::new(&db).unwrap();
        conn.query("CREATE NODE TABLE File(id STRING, PRIMARY KEY(id))").unwrap();
        conn.query("CREATE NODE TABLE Project(id STRING, PRIMARY KEY(id))").unwrap();
        // The OLD shape: no op_id column.
        conn.query("CREATE REL TABLE FILE_PART_OF(FROM File TO Project)").unwrap();

        // Run the real schema setup over the old store. It must add the column.
        create_schema(&conn).expect("create_schema converges the old store");
        // Idempotent: a second run is a no-op, not an error.
        create_schema(&conn).expect("create_schema is idempotent");

        // The op-id-keyed write the agent relies on now works end to end.
        conn.query("CREATE (:File {id:'f1'})").unwrap();
        conn.query("CREATE (:Project {id:'p1'})").unwrap();
        conn.query(
            "MATCH (f:File {id:'f1'}),(p:Project {id:'p1'}) CREATE (f)-[:FILE_PART_OF {op_id:'op-1'}]->(p)",
        )
        .expect("op-id-keyed create works after convergence");
        let mut qr = conn
            .query("MATCH (:File)-[r:FILE_PART_OF {op_id:'op-1'}]->(:Project) RETURN count(*)")
            .expect("op-id-keyed read works after convergence");
        let rows = qr.by_ref().count();
        assert_eq!(rows, 1, "the op-id edge is queryable after convergence");
    }

    #[test]
    fn create_schema_reserves_the_git_node_types() {
        // The foundation reserves Commit and Branch from the start: they must be
        // createable on a fresh store and accept a row, so a future git-ingestion
        // tier needs no schema migration. Idempotent on an existing store.
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("graph");
        let db = Database::new(path.to_str().unwrap(), SystemConfig::default()).unwrap();
        let conn = Connection::new(&db).unwrap();

        create_schema(&conn).expect("create_schema reserves the git tables");
        create_schema(&conn).expect("create_schema is idempotent");

        conn.query(
            "CREATE (:Commit {id:'abc123', message:'init', author:'A', \
             author_email:'a@x', committed_at:1})",
        )
        .expect("a Commit node writes with its conventional columns");
        conn.query("CREATE (:Branch {id:'b1', name:'main', head:'abc123'})")
            .expect("a Branch node writes");

        let mut qr = conn
            .query("MATCH (b:Branch {id:'b1'}) RETURN b.head")
            .expect("the Branch is queryable");
        let head: Vec<String> = qr
            .by_ref()
            .filter_map(|row| match row.first() {
                Some(Value::String(s)) => Some(s.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(head, vec!["abc123".to_string()], "the reserved Branch round-trips");
    }

    #[test]
    fn create_schema_builds_the_living_capability_graph_tables() {
        // LCG-R1: the Grant / CapabilityUse / EntityType nodes and the
        // GRANTS / USED_BY / LAST_EXERCISED edges must be createable on a fresh
        // store, and a Grant node + its GRANTS projection must round-trip with the
        // lifecycle-state columns. App/Event already exist for the edge endpoints.
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("graph");
        let db = Database::new(path.to_str().unwrap(), SystemConfig::default()).unwrap();
        let conn = Connection::new(&db).unwrap();

        create_schema(&conn).expect("create_schema builds the LCG tables");
        create_schema(&conn).expect("create_schema is idempotent");

        // A Grant projecting one reach onto an EntityType marker.
        conn.query(
            "CREATE (:Grant {id:'g1', app_id:'com.x', pid:42, issued_at:1, expires_at:0, \
             declared_ceiling:'{}', required:true, identity_verified:false, live:true, \
             revoked:false, superseded:false, last_exercised_at:0, use_count:0})",
        )
        .expect("a Grant node writes with its lifecycle-state columns");
        conn.query("CREATE (:EntityType {id:'system.File', label:'File'})").unwrap();
        conn.query(
            "MATCH (g:Grant {id:'g1'}),(t:EntityType {id:'system.File'}) CREATE (g)-[:GRANTS]->(t)",
        )
        .expect("the GRANTS projection edge writes");

        // The one-hop "which types can this live app reach" read.
        let mut qr = conn
            .query(
                "MATCH (g:Grant {app_id:'com.x'})-[:GRANTS]->(t:EntityType) \
                 WHERE g.live AND NOT g.revoked AND NOT g.superseded RETURN t.label",
            )
            .expect("the GRANTS projection is queryable");
        let labels: Vec<String> = qr
            .by_ref()
            .filter_map(|row| match row.first() {
                Some(Value::String(s)) => Some(s.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(labels, vec!["File".to_string()], "the live grant's reach projects one hop");
    }

    #[test]
    fn r0_engine_permits_parallel_same_type_edges_and_set_counts_matched() {
        // KG-R0 (bitemporal-knowledge-graph.md §4.10): the append-don't-overwrite
        // bi-temporal model needs multiple FILE_PART_OF edges between one
        // (File, Project) pair (joined / left / re-joined is three retained
        // rows). The op_id precedent proves edges carry properties; it does NOT
        // prove the engine permits parallel same-type edges. This probes ladybug
        // empirically. It passing SELECTS the on-edge-stamps representation
        // (§4.1-4.9); a failure (the engine collapses or rejects the second edge)
        // would instead select the reified `Membership` fact-node fallback.
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("graph");
        let db = Database::new(path.to_str().unwrap(), SystemConfig::default()).unwrap();
        let conn = Connection::new(&db).unwrap();
        conn.query("CREATE NODE TABLE File(id STRING, PRIMARY KEY(id))").unwrap();
        conn.query("CREATE NODE TABLE Project(id STRING, PRIMARY KEY(id))").unwrap();
        conn.query("CREATE REL TABLE FILE_PART_OF(FROM File TO Project, op_id STRING)").unwrap();
        conn.query("CREATE (:File {id:'f1'})").unwrap();
        conn.query("CREATE (:Project {id:'p1'})").unwrap();

        // Two edges of the same type between the same node pair.
        conn.query("MATCH (f:File{id:'f1'}),(p:Project{id:'p1'}) CREATE (f)-[:FILE_PART_OF {op_id:'a'}]->(p)")
            .unwrap();
        conn.query("MATCH (f:File{id:'f1'}),(p:Project{id:'p1'}) CREATE (f)-[:FILE_PART_OF {op_id:'b'}]->(p)")
            .unwrap();

        // Both persist and both are returned: parallel same-type edges permitted.
        let parallel = conn
            .query("MATCH (:File{id:'f1'})-[r:FILE_PART_OF]->(:Project{id:'p1'}) RETURN r.op_id")
            .unwrap()
            .by_ref()
            .count();
        assert_eq!(
            parallel, 2,
            "ladybug must retain both parallel FILE_PART_OF edges (else the Membership fallback, §4.10)"
        );

        // SET ... RETURN count(*) counts matched-and-mutated rows, so the
        // close-then-append write can report how many edges it closed.
        let mut qr = conn
            .query("MATCH (:File{id:'f1'})-[r:FILE_PART_OF]->(:Project{id:'p1'}) SET r.op_id = 'c' RETURN count(*)")
            .unwrap();
        let row = qr.by_ref().next().expect("count(*) returns one row");
        let count = row.into_iter().next().and_then(value_to_json).and_then(|j| j.as_i64());
        assert_eq!(
            count,
            Some(2),
            "SET ... RETURN count(*) reports the matched-and-mutated row count"
        );
    }

    /// GD-R4 capability constraint: ladybug (Kuzu) does NOT support `FOREACH`, so
    /// the append-a-reopen in `persist_retract` cannot be the clean single
    /// statement that closes the retracted edge AND, only-if-it-superseded-one,
    /// conditionally appends a fresh reopen edge (the parser rejects `FOREACH`
    /// after an `OPTIONAL MATCH` with "expected oC_MultiPartQuery"). This pins
    /// that constraint: it forces the GD-R4 design onto a transaction / a
    /// restructured statement rather than a conditional `FOREACH ... CREATE`, and
    /// canaries a future Kuzu that gains `FOREACH` (then the clean atomic form,
    /// which preserves bi-temporal history without a crash window, is available
    /// and the design should be revisited). Probed empirically rather than
    /// assumed, the same way KG-R0 selected the on-edge-stamps representation.
    #[test]
    fn gd_r4_kuzu_does_not_support_foreach_create() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("graph");
        let db = Database::new(path.to_str().unwrap(), SystemConfig::default()).unwrap();
        let conn = Connection::new(&db).unwrap();
        conn.query("CREATE NODE TABLE File(id STRING, PRIMARY KEY(id))").unwrap();
        conn.query("CREATE NODE TABLE Project(id STRING, PRIMARY KEY(id))").unwrap();
        conn.query("CREATE REL TABLE FILE_PART_OF(FROM File TO Project, op_id STRING)").unwrap();
        conn.query("CREATE (:File {id:'f1'})").unwrap();
        conn.query("CREATE (:Project {id:'p1'})").unwrap();

        // A minimal FOREACH-CREATE. If ladybug supported it this would append one
        // edge; today the parser rejects the clause outright.
        let res = conn.query(
            "MATCH (f:File{id:'f1'}),(p:Project{id:'p1'}) \
             FOREACH (_ IN [1] | CREATE (f)-[:FILE_PART_OF {op_id:'x'}]->(p))",
        );
        assert!(
            res.is_err(),
            "FOREACH is unexpectedly supported now — the clean single-statement \
             append-a-reopen is available; revisit the GD-R4 design that works \
             around its absence: {res:?}"
        );
    }

    #[test]
    fn create_schema_adds_temporal_and_provenance_columns_to_file_part_of() {
        // KG-R2 (§4.1): a store created before the bi-temporal columns existed
        // (here, even before op_id) must converge to the full schema, so a write
        // can stamp the four temporal axes and the provenance quad. Same
        // `ALTER ADD IF NOT EXISTS` convergence proven for op_id.
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("graph");
        let db = Database::new(path.to_str().unwrap(), SystemConfig::default()).unwrap();
        let conn = Connection::new(&db).unwrap();
        conn.query("CREATE NODE TABLE File(id STRING, PRIMARY KEY(id))").unwrap();
        conn.query("CREATE NODE TABLE Project(id STRING, PRIMARY KEY(id))").unwrap();
        // The OLD shape: no temporal/provenance columns.
        conn.query("CREATE REL TABLE FILE_PART_OF(FROM File TO Project)").unwrap();

        create_schema(&conn).expect("create_schema converges the old store");
        create_schema(&conn).expect("create_schema is idempotent");

        conn.query("CREATE (:File {id:'f1'})").unwrap();
        conn.query("CREATE (:Project {id:'p1'})").unwrap();
        // A write stamping every new column now works end to end.
        conn.query(
            "MATCH (f:File {id:'f1'}),(p:Project {id:'p1'}) \
             CREATE (f)-[:FILE_PART_OF {valid_at:1000, created_at:1000, origin:'agent', op_id:'op-1'}]->(p)",
        )
        .expect("a temporally-stamped edge write works after convergence");
        // A live-edge read (NULL invalid_at/expired_at = open interval) returns it.
        let live = conn
            .query(
                "MATCH (:File)-[r:FILE_PART_OF]->(:Project) \
                 WHERE r.invalid_at IS NULL AND r.expired_at IS NULL RETURN r.origin",
            )
            .expect("the live-edge predicate columns exist")
            .by_ref()
            .count();
        assert_eq!(live, 1, "the stamped edge reads as a live, open-interval fact");
    }

    #[test]
    fn run_transaction_commits_all_or_rolls_back_on_error() {
        // KG-R5 (§4.5): the multi-statement transaction variant must be atomic.
        // This also probes that lbug honours `BEGIN TRANSACTION`/`COMMIT`/
        // `ROLLBACK` (it exposes no transaction handle, so the boundaries are
        // Cypher statements); a failure here would mean the node-create-plus-edges
        // path needs a different mechanism.
        let tmp = tempfile::TempDir::new().unwrap();
        let db = Database::new(tmp.path().join("graph").to_str().unwrap(), SystemConfig::default())
            .unwrap();
        let conn = Connection::new(&db).unwrap();
        conn.query("CREATE NODE TABLE N(id STRING, PRIMARY KEY(id))").unwrap();

        let count = |conn: &Connection| -> i64 {
            conn.query("MATCH (n:N) RETURN count(*)")
                .unwrap()
                .by_ref()
                .next()
                .and_then(|row| row.into_iter().next())
                .and_then(value_to_json)
                .and_then(|j| j.as_i64())
                .unwrap()
        };

        // A valid transaction commits every statement.
        run_transaction(
            &conn,
            &["CREATE (:N {id: 'a'})".to_string(), "CREATE (:N {id: 'b'})".to_string()],
        )
        .expect("a valid transaction commits");
        assert_eq!(count(&conn), 2, "both nodes committed");

        // A transaction whose second statement fails (duplicate primary key)
        // rolls back the first, so nothing from it persists.
        let r = run_transaction(
            &conn,
            &["CREATE (:N {id: 'c'})".to_string(), "CREATE (:N {id: 'a'})".to_string()],
        );
        assert!(r.is_err(), "the duplicate-key statement fails the transaction");
        assert_eq!(count(&conn), 2, "the failed transaction rolled back 'c'");

        // An empty transaction is a no-op success.
        run_transaction(&conn, &[]).expect("empty transaction is a no-op");
        assert_eq!(count(&conn), 2);
    }

    #[test]
    fn create_schema_supports_annotation_versioning() {
        // KG-R6 (§4.8): an Annotation links via a temporal HAS_VERSION edge to an
        // AnnotationVersion content node, so annotation history is retained
        // instead of overwritten. This checks the schema supports that shape.
        let tmp = tempfile::TempDir::new().unwrap();
        let db = Database::new(tmp.path().join("graph").to_str().unwrap(), SystemConfig::default())
            .unwrap();
        let conn = Connection::new(&db).unwrap();
        create_schema(&conn).expect("schema");

        conn.query(
            "CREATE (:Annotation {id: 'a1', namespace: 'notes', target_type: 'File', target_id: 'f1'})",
        )
        .unwrap();
        conn.query("CREATE (:AnnotationVersion {id: 'v1', data: 'hello', recorded_at: 1000})")
            .unwrap();
        conn.query(
            "MATCH (a:Annotation {id: 'a1'}), (v:AnnotationVersion {id: 'v1'}) \
             CREATE (a)-[:HAS_VERSION {valid_at: 1000, created_at: 1000, op_id: 'op-1'}]->(v)",
        )
        .expect("a temporally-stamped version edge writes");

        // The live version (open intervals) is reachable via the two-hop read.
        let live = conn
            .query(
                "MATCH (:Annotation {id: 'a1'})-[r:HAS_VERSION]->(v:AnnotationVersion) \
                 WHERE r.invalid_at IS NULL AND r.expired_at IS NULL RETURN v.data",
            )
            .expect("the versioned-annotation read works")
            .by_ref()
            .count();
        assert_eq!(live, 1, "the live annotation version is reachable");
    }
}

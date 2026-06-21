use crate::graph::GraphHandle;
use crate::project::ProjectStore;
use crate::proto::{
    AnnotationClearPayload, AnnotationSetPayload, BadgeSetPayload, BadgeStatus,
    CodeFileIndexPayload, FileOpenedPayload, FileWrittenPayload, NetworkConnectionPayload,
    PresenceClearPayload, PresenceSetPayload, ShortcutActionInvokedPayload, TimelineRecordPayload,
    WindowFocusedPayload,
};
use crate::utils::escape_cypher;
use anyhow::Result;
use prost::Message;
use sqlx::SqlitePool;
use std::time::Duration;
use tokio::time;
use tracing::{debug, error, info};

/// Fixed UUIDv5 namespace for deriving deterministic annotation ids
/// from the `(target_type, target_id, namespace)` triple. The exact
/// bytes are arbitrary but must stay stable forever — they are baked
/// into every Annotation node ever written. Changing this would
/// orphan existing annotations on the next set.
const ANNOTATION_UUID_NAMESPACE: uuid::Uuid = uuid::Uuid::from_bytes([
    0x6e, 0xed, 0x73, 0x05, 0xc4, 0x83, 0x4d, 0x73, 0xa6, 0x86, 0xc1, 0x73, 0x4d, 0xb1, 0x29, 0x7e,
]);

/// Derive the deterministic Annotation node id from the spec's
/// composite identity (target_type, target_id, namespace). UUIDv5 so
/// the same triple always maps to the same id, enabling MERGE-based
/// dedup in promotion without a separate lookup query.
pub(crate) fn annotation_id(target_type: &str, target_id: &str, namespace: &str) -> uuid::Uuid {
    let key = format!("{target_type}\x1f{target_id}\x1f{namespace}");
    uuid::Uuid::new_v5(&ANNOTATION_UUID_NAMESPACE, key.as_bytes())
}

/// Fallback when `graph.toml [projects].auto_promote_threshold` is
/// not set. Mirrors `WatchConfig::default().auto_promote_threshold`.
const PROMOTION_THRESHOLD_DEFAULT: usize = 3;

/// How often the promotion pass runs.
const PROMOTION_INTERVAL: Duration = Duration::from_secs(30);

/// High-water mark key in a metadata table we use to track progress.
/// The promotion pass only processes events newer than the last run.
const HWM_KEY: &str = "promotion_hwm";

/// Run the promotion pass forever, waking every `PROMOTION_INTERVAL`.
///
/// The promotion pass reads events from SQLite that have not yet been
/// promoted to Ladybug and creates the corresponding graph nodes.
/// It tracks progress via a high-water mark (the timestamp of the last
/// promoted event) so each run only processes new events.
pub async fn run(pool: SqlitePool, graph: GraphHandle) -> Result<()> {
    // Ensure the metadata table exists for high-water mark tracking.
    ensure_metadata_table(&pool).await?;
    // The FTS5 keyword index for LLM-free retrieval (§7.1) lives beside `events`
    // and is populated here as nodes are promoted.
    crate::fts::create_fact_text_index(&pool).await?;

    let project_store = ProjectStore::new(graph.clone());

    // Read once at startup. graph.toml hot-reload is a separate
    // sprint item — for now, threshold changes need a daemon
    // restart. Documented in the Settings UI.
    let watch_config = crate::project::watch_config::WatchConfig::load();
    let threshold = watch_config.auto_promote_threshold;
    info!(threshold, "auto-promotion threshold loaded from graph.toml");

    let mut interval = time::interval(PROMOTION_INTERVAL);
    // Skip the first immediate tick so we don't run before the write store
    // has had a chance to accumulate events.
    interval.tick().await;

    loop {
        interval.tick().await;
        if let Err(e) = run_pass(&pool, &graph, &project_store, threshold).await {
            error!("promotion pass failed: {e}");
        }
    }
}

/// Create the metadata table if it does not exist.
async fn ensure_metadata_table(pool: &SqlitePool) -> Result<()> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS metadata (
            key   TEXT PRIMARY KEY,
            value TEXT NOT NULL
        )",
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Read the current high-water mark timestamp from the metadata table.
/// Returns 0 if no HWM has been recorded yet (first run).
pub(crate) async fn read_hwm(pool: &SqlitePool) -> Result<i64> {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT value FROM metadata WHERE key = ?")
            .bind(HWM_KEY)
            .fetch_optional(pool)
            .await?;
    Ok(row
        .and_then(|(v,)| v.parse().ok())
        .unwrap_or(0))
}

/// Write a new high-water mark to the metadata table.
async fn write_hwm(pool: &SqlitePool, hwm: i64) -> Result<()> {
    sqlx::query(
        "INSERT INTO metadata (key, value) VALUES (?, ?)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(HWM_KEY)
    .bind(hwm.to_string())
    .execute(pool)
    .await?;
    Ok(())
}

/// Run a single promotion pass.
///
/// Reads all events from SQLite with timestamp > high-water mark,
/// promotes them to Ladybug, and updates the HWM only if every event
/// in the batch was promoted successfully.
///
/// Promotion criteria for Phase 1A:
/// - All `file.opened` events become `File` and `App` nodes with an `ACCESSED_BY` edge.
/// - All `window.focused` events become `App`, `Session`, and `Event` nodes with `ACTIVE_IN` edge.
/// - Other event types are stored in SQLite but not yet promoted (Phase 2).
async fn run_pass(
    pool: &SqlitePool,
    graph: &GraphHandle,
    project_store: &ProjectStore,
    promote_threshold: usize,
) -> Result<()> {
    let hwm = read_hwm(pool).await?;

    // Fetch unprocessed events ordered by timestamp, including the payload.
    let rows: Vec<(String, String, i64, String, i64, String, Vec<u8>)> = sqlx::query_as(
        "SELECT id, type, timestamp, source, pid, session_id, payload
         FROM events
         WHERE timestamp > ?
         ORDER BY timestamp ASC
         LIMIT 1000",
    )
    .bind(hwm)
    .fetch_all(pool)
    .await?;

    if rows.is_empty() {
        debug!("no new events to promote");
        return Ok(());
    }

    info!(count = rows.len(), "promoting events to ladybug");

    let mut all_ok = true;
    let mut last_timestamp = hwm;

    for (id, event_type, timestamp, source, pid, session_id, payload) in &rows {
        let result = match event_type.as_str() {
            "file.opened" => {
                let res = promote_file_opened(
                    graph, id, timestamp, source, pid, session_id, payload,
                )
                .await;
                // After the File node exists, try linking it to a project.
                if res.is_ok() {
                    if let Ok(fp) = FileOpenedPayload::decode(payload.as_slice()) {
                        if !fp.path.is_empty() {
                            if let Err(e) = link_file_to_project(
                                &fp.path,
                                session_id,
                                project_store,
                                promote_threshold,
                            )
                            .await
                            {
                                debug!("project link skipped for {}: {e}", fp.path);
                            }
                            // Index the File's keyword text for retrieval (§7.1).
                            // Non-fatal: the index is an optimisation, so a
                            // transient error must not stall graph promotion.
                            if let Err(e) = index_file_fact_text(pool, &fp.path).await {
                                debug!("fact_text index skipped for {}: {e}", fp.path);
                            }
                        }
                    }
                }
                res
            }
            "window.focused" => {
                promote_window_focused(graph, id, timestamp, session_id, payload).await
            }
            "file.written" => {
                promote_file_written(graph, id, timestamp, source, pid, payload).await
            }
            // The OS-observed app<->network edge family (KG-richness Thrust 1):
            // a connection becomes an App -> NetworkEndpoint CONNECTED_TO edge.
            "network.connect" | "network.accept" => {
                promote_network_connection(graph, id, timestamp, payload).await
            }
            "app.presence.set" => {
                promote_presence_set(graph, id, timestamp, payload).await
            }
            "app.presence.clear" => {
                promote_presence_clear(graph, id, timestamp, payload).await
            }
            "app.timeline.record" => {
                promote_timeline_record(graph, id, timestamp, payload).await
            }
            "app.annotation.set" => {
                promote_annotation_set(graph, timestamp, payload).await
            }
            "app.annotation.cleared" => promote_annotation_cleared(graph, payload).await,
            "app.badge.set" => promote_badge_set(graph, id, timestamp, payload).await,
            // User interactions (toolbar / shortcut / menu) become UserAction
            // nodes, so the KG carries a native interaction history (the GAP-10
            // follow-up: previously these were queryable RAW events only). One
            // arm, three shape-identical surfaces.
            "app.toolbar.action_invoked"
            | "app.shortcut.action_invoked"
            | "app.menu.action_invoked" => {
                promote_action_invoked(
                    graph, id, event_type, timestamp, session_id, payload,
                )
                .await
            }
            // Coarse power transitions become timeline Event nodes (§3f). The
            // high-frequency `power.state` snapshot is deliberately NOT listed,
            // so battery-% churn is never promoted.
            "power.low" | "power.critical" | "power.recovered" | "power.profile_changed"
            | "power.suspend" | "power.resume" | "power.lid_closed" => {
                promote_power_transition(graph, id, event_type, timestamp, source).await
            }
            // The code-graph layer (CG-R1): replace a file's CodeSymbols with the
            // freshly-parsed set and fuse each to its File via DEFINES.
            "code.indexed" => promote_code_indexed(graph, payload).await,
            _ => {
                // Not yet promoted; will be handled in a later phase.
                debug!(event_type, "skipping promotion for unhandled event type");
                Ok(())
            }
        };

        if let Err(e) = result {
            error!(event_id = %id, event_type, "promotion failed: {e}");
            all_ok = false;
            // Stop advancing HWM: we do not skip failed events.
            break;
        }

        last_timestamp = *timestamp;
    }

    // Only advance the HWM if every event in the batch succeeded.
    // On failure, the next pass will retry from the same position.
    if all_ok && last_timestamp > hwm {
        write_hwm(pool, last_timestamp).await?;
        info!(hwm = last_timestamp, promoted = rows.len(), "promotion pass complete");
    } else if !all_ok {
        // Advance to the last successfully promoted event so we do not
        // re-process events that already succeeded.
        if last_timestamp > hwm {
            write_hwm(pool, last_timestamp).await?;
        }
        info!(hwm = last_timestamp, "promotion pass incomplete, will retry failed events");
    }

    Ok(())
}

/// Promote a `file.opened` event.
///
/// Deserializes the payload to obtain the file path and app ID, then
/// creates or merges a `File` node (keyed by path) and an `App` node,
/// with an `ACCESSED_BY` edge between them.
/// Synthesise and index a File node's keyword text for retrieval (§7.1). The
/// File node id is its path, so the index entry is keyed by the same id the graph
/// uses, which is what lets a keyword hit fuse with graph traversal. Idempotent:
/// re-promotion of the same file re-upserts identical text, never a duplicate.
/// The full §7.1 "same SQLite transaction as the HWM advance" (no drift) is a
/// follow-up; this idempotent per-file upsert converges (a crash before the
/// upsert is fixed on the next pass, which re-promotes from the un-advanced HWM).
async fn index_file_fact_text(pool: &SqlitePool, path: &str) -> Result<()> {
    let mut fields = std::collections::BTreeMap::new();
    fields.insert("path".to_string(), path.to_string());
    let text = crate::retrieval::fact_text("File", &fields);
    crate::fts::upsert_fact_text(pool, path, &text).await
}

async fn promote_file_opened(
    graph: &GraphHandle,
    event_id: &str,
    timestamp: &i64,
    source: &str,
    pid: &i64,
    session_id: &str,
    payload: &[u8],
) -> Result<()> {
    let file_payload = FileOpenedPayload::decode(payload)?;

    // Use the file path as the node ID so repeated opens of the same file
    // merge into a single node rather than creating duplicates.
    let path = if file_payload.path.is_empty() {
        // Fallback: eBPF events from Phase 1A may not have a resolved path yet.
        format!("unknown:{event_id}")
    } else {
        file_payload.path.clone()
    };

    // Prefer the app_id from the payload; fall back to source:pid.
    let app_id = if file_payload.app_id.is_empty() {
        format!("{source}:{pid}")
    } else {
        file_payload.app_id.clone()
    };

    let path_esc = escape_cypher(&path);
    let app_id_esc = escape_cypher(&app_id);
    let source_esc = escape_cypher(source);
    // The cgroup v2 id of the opening task (0 when the open was not eBPF-sourced, so
    // carries no attribution). Kernel-sourced and a bare integer, so it is
    // interpolated unquoted (Kuzu stores INT64). A File node is path-keyed and opens
    // merge, so this records only the LATEST cgroup, not a history; the per-open
    // history with its cgroup lives in the SQLite event log, which the TOUCHED join
    // reads from, not this merged node.
    let cgroup_id = file_payload.cgroup_id;

    graph
        .write(format!(
            "MERGE (a:App {{id: '{app_id_esc}'}}) SET a.name = '{source_esc}'"
        ))
        .await?;

    graph
        .write(format!(
            "MERGE (f:File {{id: '{path_esc}'}})
             SET f.path = '{path_esc}', f.last_accessed = {timestamp}, f.app_id = '{app_id_esc}', f.last_cgroup_id = {cgroup_id}"
        ))
        .await?;

    graph
        .write(format!(
            "MATCH (f:File {{id: '{path_esc}'}}), (a:App {{id: '{app_id_esc}'}})
             MERGE (f)-[:ACCESSED_BY]->(a)"
        ))
        .await?;

    // Session<->activity edge (KG-richness Thrust 1): link the file to the
    // focus/activity session it was accessed in, so the graph can answer "which
    // files did I touch in this session". The Session node is MERGEd (a
    // window.focused promotion enriches it; here it may be a bare id) so the edge
    // never dangles. Skipped when the event carries no session (e.g. a raw eBPF
    // kernel open). The session id is metadata symmetric with the existing
    // File/ACCESSED_BY promotion - no new hard-exclude surface: a private/
    // incognito session is excluded upstream before promotion, like every other
    // event, and Session nodes are already materialised by window.focused.
    if !session_id.is_empty() {
        let session_esc = escape_cypher(session_id);
        graph
            .write(format!("MERGE (s:Session {{id: '{session_esc}'}})"))
            .await?;
        graph
            .write(format!(
                "MATCH (f:File {{id: '{path_esc}'}}), (s:Session {{id: '{session_esc}'}})
                 MERGE (f)-[:ACCESSED_IN]->(s)"
            ))
            .await?;

        // File<->file co-access (KG-richness Thrust 1): link this file to the
        // most-recently-accessed other files in the same session, the strongest
        // project-inference signal. Bounded to CO_ACCESS_FANOUT peers per
        // promotion so a long session stays linear, not quadratic.
        link_co_accessed(graph, &path, &session_esc, timestamp).await?;
    }

    debug!(event_id, path = %file_payload.path, "promoted file.opened");
    Ok(())
}

/// The maximum number of peer files a single file.opened links to via
/// `CO_ACCESSED`. Bounds the fan-out so a session with many files stays linear
/// in edge count rather than quadratic.
const CO_ACCESS_FANOUT: usize = 8;

/// Link a freshly-accessed file to the most recent other files in the same
/// session via `CO_ACCESSED` (KG-richness Thrust 1, file<->file co-access).
///
/// The edge is semantically undirected, so it is stored canonically (the
/// lexicographically smaller path is FROM) to keep exactly one edge per pair
/// regardless of which file was accessed first. `last_seen` is refreshed on each
/// co-access. The peer set is bounded to [`CO_ACCESS_FANOUT`] so the fan-out per
/// promotion is constant.
async fn link_co_accessed(
    graph: &GraphHandle,
    path: &str,
    session_esc: &str,
    timestamp: &i64,
) -> Result<()> {
    let path_esc = escape_cypher(path);
    // The most-recently-accessed other files in this session (recency proxy for
    // "used close together"). Self is excluded by id.
    let peers = graph
        .query_rows(format!(
            "MATCH (other:File)-[:ACCESSED_IN]->(s:Session {{id: '{session_esc}'}})
             WHERE other.id <> '{path_esc}'
             RETURN other.id AS id
             ORDER BY other.last_accessed DESC
             LIMIT {CO_ACCESS_FANOUT}"
        ))
        .await?;

    for row in &peers.rows {
        let Some(peer) = row.first().map(|v| v.as_str()) else {
            continue;
        };
        if peer.is_empty() {
            continue;
        }
        // Canonical direction: the lexicographically smaller id is FROM, so the
        // pair maps to one edge no matter the access order.
        let (from, to) = if path < peer { (path, peer) } else { (peer, path) };
        let from_esc = escape_cypher(from);
        let to_esc = escape_cypher(to);
        graph
            .write(format!(
                "MATCH (a:File {{id: '{from_esc}'}}), (b:File {{id: '{to_esc}'}})
                 MERGE (a)-[c:CO_ACCESSED]->(b)
                 SET c.last_seen = {timestamp}"
            ))
            .await?;
    }
    Ok(())
}

/// Promote a `file.written` event into a write-provenance edge.
///
/// Mirrors [`promote_file_opened`]'s File + App merge but creates a
/// `MODIFIED_BY` edge (the app WROTE the file) rather than the read/open
/// `ACCESSED_BY`. The File node is path-keyed and shared with the read path, so
/// a write before any read still creates the node; the read's `last_accessed`
/// is not clobbered here (a write is not an access). The per-write history (the
/// byte count, the timestamp) stays in the SQLite event log; this records only
/// that the relationship exists.
async fn promote_file_written(
    graph: &GraphHandle,
    event_id: &str,
    _timestamp: &i64,
    source: &str,
    pid: &i64,
    payload: &[u8],
) -> Result<()> {
    let file_payload = FileWrittenPayload::decode(payload)?;

    let path = if file_payload.path.is_empty() {
        format!("unknown:{event_id}")
    } else {
        file_payload.path.clone()
    };
    let app_id = if file_payload.app_id.is_empty() {
        format!("{source}:{pid}")
    } else {
        file_payload.app_id.clone()
    };

    let path_esc = escape_cypher(&path);
    let app_id_esc = escape_cypher(&app_id);
    let source_esc = escape_cypher(source);

    graph
        .write(format!(
            "MERGE (a:App {{id: '{app_id_esc}'}}) SET a.name = '{source_esc}'"
        ))
        .await?;
    graph
        .write(format!(
            "MERGE (f:File {{id: '{path_esc}'}}) SET f.path = '{path_esc}'"
        ))
        .await?;
    graph
        .write(format!(
            "MATCH (f:File {{id: '{path_esc}'}}), (a:App {{id: '{app_id_esc}'}})
             MERGE (f)-[:MODIFIED_BY]->(a)"
        ))
        .await?;

    debug!(event_id, path = %file_payload.path, "promoted file.written");
    Ok(())
}

/// Promote a `network.connect` / `network.accept` event into the OS-observed
/// app-to-network edge (KG-richness Thrust 1).
///
/// Creates or merges an `App` node and a `NetworkEndpoint` node (keyed by the
/// remote IP:port), with a `CONNECTED_TO` edge carrying the direction and the
/// most recent observation time. Only the remote address is recorded, never
/// connection payload, so no secret/credential content enters the graph; a
/// private/incognito session is excluded upstream before promotion. An event
/// missing the app id or the remote address is skipped (no dangling node).
async fn promote_network_connection(
    graph: &GraphHandle,
    event_id: &str,
    timestamp: &i64,
    payload: &[u8],
) -> Result<()> {
    let p = NetworkConnectionPayload::decode(payload)?;
    if p.app_id.is_empty() || p.remote_addr.is_empty() {
        debug!(event_id, "network connection not promoted (missing app or remote)");
        return Ok(());
    }

    let app_esc = escape_cypher(&p.app_id);
    let addr_esc = escape_cypher(&p.remote_addr);
    let proto_esc = escape_cypher(&p.protocol);
    let dir_esc = escape_cypher(&p.direction);

    graph
        .write(format!("MERGE (a:App {{id: '{app_esc}'}})"))
        .await?;
    graph
        .write(format!(
            "MERGE (e:NetworkEndpoint {{id: '{addr_esc}'}}) SET e.protocol = '{proto_esc}'"
        ))
        .await?;
    graph
        .write(format!(
            "MATCH (a:App {{id: '{app_esc}'}}), (e:NetworkEndpoint {{id: '{addr_esc}'}})
             MERGE (a)-[c:CONNECTED_TO]->(e)
             SET c.direction = '{dir_esc}', c.last_seen = {timestamp}"
        ))
        .await?;

    debug!(event_id, app_id = %p.app_id, remote = %p.remote_addr, "promoted network connection");
    Ok(())
}

/// Promote a `window.focused` event.
///
/// Deserializes the payload to obtain the app ID and window title, then
/// creates or merges `App`, `Session`, and `Event` nodes with an
/// `ACTIVE_IN` edge from App to Session.
async fn promote_window_focused(
    graph: &GraphHandle,
    event_id: &str,
    timestamp: &i64,
    session_id: &str,
    payload: &[u8],
) -> Result<()> {
    let win_payload = WindowFocusedPayload::decode(payload)?;

    let app_id = if win_payload.app_id.is_empty() {
        "unknown".to_string()
    } else {
        win_payload.app_id.clone()
    };

    let app_id_esc = escape_cypher(&app_id);
    let session_id_esc = escape_cypher(session_id);
    let event_id_esc = escape_cypher(event_id);
    let title_esc = escape_cypher(&win_payload.window_title);

    graph
        .write(format!(
            "MERGE (a:App {{id: '{app_id_esc}'}}) SET a.name = '{app_id_esc}'"
        ))
        .await?;

    graph
        .write(format!(
            "MERGE (s:Session {{id: '{session_id_esc}'}})
             SET s.started_at = {timestamp}"
        ))
        .await?;

    graph
        .write(format!(
            "MERGE (e:Event {{id: '{event_id_esc}'}})
             SET e.type = 'window.focused', e.timestamp = {timestamp},
                 e.source = 'wayland', e.title = '{title_esc}'"
        ))
        .await?;

    // Create the ACTIVE_IN edge: the focused app is active in this session.
    graph
        .write(format!(
            "MATCH (a:App {{id: '{app_id_esc}'}}), (s:Session {{id: '{session_id_esc}'}})
             MERGE (a)-[:ACTIVE_IN]->(s)"
        ))
        .await?;

    debug!(event_id, app_id = %app_id, "promoted window.focused");
    Ok(())
}

/// Promote a coarse power transition into a generic `Event` node on the timeline
/// (system-services-plan.md §3f/§194: promote the COARSE power transitions
/// `power.suspend`/`resume`/`lid_*`/`profile_changed`/`critical` as local
/// provenance, so the AI layer can reason about session boundaries - "what was I
/// doing before the machine slept here"). The Event node is the same generic
/// timeline node `window.focused` uses; the transition `type` carries the coarse
/// fact, correlated to sessions by timestamp (a power transition is system-wide,
/// not session-scoped, so it carries no session edge). Only schema columns are
/// set. The high-frequency `power.state` snapshot and battery-% churn are NEVER
/// promoted - the caller's match list excludes `power.state`, and the power
/// daemon already emits a transition only once per crossing.
async fn promote_power_transition(
    graph: &GraphHandle,
    event_id: &str,
    event_type: &str,
    timestamp: &i64,
    source: &str,
) -> Result<()> {
    let event_id_esc = escape_cypher(event_id);
    let type_esc = escape_cypher(event_type);
    let source_esc = escape_cypher(source);
    graph
        .write(format!(
            "MERGE (e:Event {{id: '{event_id_esc}'}})
             SET e.type = '{type_esc}', e.timestamp = {timestamp}, e.source = '{source_esc}'"
        ))
        .await?;
    debug!(event_id, event_type, "promoted power transition");
    Ok(())
}

/// Promote a `code.indexed` event into the code-graph layer (code-graph-layer.md
/// CG-R1): replace the file's prior `CodeSymbol`s with the freshly-parsed set and
/// fuse each to its `File` via a `DEFINES` edge (confidence `extracted` - the
/// definition is syntactically explicit). The whole file is replaced atomically
/// in one transaction (per-file isolation: a re-parse wholly supersedes the
/// file's previous symbols, so a crash mid-replace never leaves a partial set).
/// Cross-file call/import edges are resolved at query time (CG-R2); this promotes
/// only the definitions and their file fusion. All interpolated values are
/// escaped, so file content cannot inject Cypher.
async fn promote_code_indexed(graph: &GraphHandle, payload: &[u8]) -> Result<()> {
    let p = CodeFileIndexPayload::decode(payload)?;
    if p.source_file.is_empty() {
        return Ok(());
    }
    let file_esc = escape_cypher(&p.source_file);
    let lang_esc = escape_cypher(&p.language);

    let mut stmts = Vec::new();
    // Per-file replace: detach-delete the file's prior CodeSymbols (and their
    // DEFINES/edges). A re-parse fully supersedes them.
    stmts.push(format!(
        "MATCH (s:CodeSymbol {{source_file: '{file_esc}'}}) DETACH DELETE s"
    ));
    // The fusion anchor: the File node the symbols hang off (the activity graph
    // already carries its provenance/project/timeline).
    stmts.push(format!("MERGE (f:File {{id: '{file_esc}'}})"));
    // Defence in depth at the write boundary: a duplicate id in the payload would
    // make two `CREATE`s collide on the primary key, failing the whole transaction
    // and stalling ALL promotion. The daemon already dedups, but a crafted bus
    // message must not be able to wedge the pipeline - skip a repeated id here too.
    let mut seen_ids = std::collections::HashSet::new();
    for sym in &p.symbols {
        if sym.id.is_empty() || sym.name.is_empty() || !seen_ids.insert(sym.id.as_str()) {
            continue;
        }
        let id_esc = escape_cypher(&sym.id);
        let name_esc = escape_cypher(&sym.name);
        let loc_esc = escape_cypher(&sym.source_location);
        let kind_esc = escape_cypher(&sym.kind);
        stmts.push(format!(
            "CREATE (:CodeSymbol {{id: '{id_esc}', name: '{name_esc}', \
             source_file: '{file_esc}', source_location: '{loc_esc}', \
             language: '{lang_esc}', kind: '{kind_esc}'}})"
        ));
        stmts.push(format!(
            "MATCH (f:File {{id: '{file_esc}'}}), (s:CodeSymbol {{id: '{id_esc}'}}) \
             CREATE (f)-[:DEFINES {{confidence: 'extracted'}}]->(s)"
        ));
    }

    let count = p.symbols.len();
    graph.transaction(stmts).await?;
    debug!(source_file = %p.source_file, symbols = count, "promoted code.indexed");
    Ok(())
}

/// Check if a file belongs to a known project and create a FILE_PART_OF
/// edge. Also updates the project's `last_accessed` timestamp and checks
/// the auto-promotion threshold.
async fn link_file_to_project(
    file_path: &str,
    session_id: &str,
    store: &ProjectStore,
    promote_threshold: usize,
) -> Result<()> {
    let Some(project) = store.find_by_path_prefix(file_path).await? else {
        return Ok(()); // file not inside any project
    };

    // Create FILE_PART_OF edge (MERGE is idempotent).
    if !store.is_file_linked(file_path, project.id).await? {
        store.link_file(file_path, project.id).await?;
        debug!(file_path, project = %project.name, "linked file to project");
    }

    store.touch(project.id).await?;

    // Auto-promote inferred projects after enough session activity.
    if !project.promoted {
        let count = store.count_session_files(session_id, project.id).await?;
        if count >= promote_threshold {
            store.promote(project.id).await?;
            info!(
                project = %project.name,
                files = count,
                threshold = promote_threshold,
                "auto-promoted project (session threshold)",
            );
        }
    }

    Ok(())
}

/// Promote an `app.presence.set` event into a UserAction node with
/// `category = "presence"`. The metadata map and auto_clear hint stay in
/// the SQLite event row — the graph node is intentionally lightweight so
/// presence queries (e.g. "what was I editing yesterday at 14:00") stay
/// fast and the per-app metadata schemas don't pollute the graph schema.
async fn promote_presence_set(
    graph: &GraphHandle,
    event_id: &str,
    timestamp: &i64,
    payload: &[u8],
) -> Result<()> {
    let p = PresenceSetPayload::decode(payload)?;
    let id_esc = escape_cypher(event_id);
    let activity_esc = escape_cypher(&p.activity);
    let subject_esc = escape_cypher(&p.subject);

    graph
        .write(format!(
            "MERGE (u:UserAction {{id: '{id_esc}'}})
             SET u.category = 'presence',
                 u.action   = '{activity_esc}',
                 u.subject  = '{subject_esc}',
                 u.timestamp = {timestamp}"
        ))
        .await?;

    debug!(event_id, app_id = %p.app_id, activity = %p.activity, "promoted app.presence.set");
    Ok(())
}

/// Promote an `app.presence.clear` event. Apps emit this when their
/// previous presence state is no longer accurate — explicit clear, or
/// auto-clear from the SDK's window-blur listener. We record the clear
/// as its own UserAction so a query can reconstruct presence intervals
/// (set timestamp .. clear timestamp).
async fn promote_presence_clear(
    graph: &GraphHandle,
    event_id: &str,
    timestamp: &i64,
    payload: &[u8],
) -> Result<()> {
    let p = PresenceClearPayload::decode(payload)?;
    let id_esc = escape_cypher(event_id);
    let app_esc = escape_cypher(&p.app_id);

    graph
        .write(format!(
            "MERGE (u:UserAction {{id: '{id_esc}'}})
             SET u.category = 'presence',
                 u.action   = 'clear',
                 u.subject  = '{app_esc}',
                 u.timestamp = {timestamp}"
        ))
        .await?;

    debug!(event_id, app_id = %p.app_id, "promoted app.presence.clear");
    Ok(())
}

/// Promote an `app.timeline.record` event into a UserAction node with
/// `category = "timeline"`. Persistent semantic record — distinct from
/// presence which is ephemeral. Started/ended timestamps and metadata
/// remain in the SQLite event row; the graph node carries the type as
/// `action` and the user-facing label as `subject`.
async fn promote_timeline_record(
    graph: &GraphHandle,
    event_id: &str,
    _timestamp: &i64,
    payload: &[u8],
) -> Result<()> {
    let p = TimelineRecordPayload::decode(payload)?;
    let id_esc = escape_cypher(event_id);
    let type_esc = escape_cypher(&p.r#type);
    let label_esc = escape_cypher(&p.label);
    // Use ended_at when present (duration event), otherwise started_at,
    // and finally fall back to the wall-clock timestamp from the
    // Event envelope. This keeps timeline queries time-ordered by the
    // *user-meaningful* moment rather than when the event arrived.
    let ts = if p.ended_at != 0 {
        p.ended_at
    } else if p.started_at != 0 {
        p.started_at
    } else {
        *_timestamp
    };

    graph
        .write(format!(
            "MERGE (u:UserAction {{id: '{id_esc}'}})
             SET u.category = 'timeline',
                 u.action   = '{type_esc}',
                 u.subject  = '{label_esc}',
                 u.timestamp = {ts}"
        ))
        .await?;

    debug!(event_id, app_id = %p.app_id, label = %p.label, "promoted app.timeline.record");
    Ok(())
}

/// Promote an `app.annotation.set` event into an Annotation node
/// keyed by the deterministic UUIDv5 of (target_type, target_id,
/// namespace). MERGE-style upsert: re-setting on the same triple
/// updates `data` and `last_modified` while preserving `created_at`.
///
/// Foundation §395 — apps write only to their own namespace, the
/// daemon does not enforce that here yet (write-token-authentication
/// is Phase 3.2-full); for now the SDK declares its own namespace
/// honestly and the trust boundary is the SO_PEERCRED-derived uid on
/// the producer socket.
async fn promote_annotation_set(
    graph: &GraphHandle,
    timestamp: &i64,
    payload: &[u8],
) -> Result<()> {
    let p = AnnotationSetPayload::decode(payload)?;
    let id = annotation_id(&p.target_type, &p.target_id, &p.namespace);
    let version_id = uuid::Uuid::now_v7();

    let id_esc = escape_cypher(&id.to_string());
    let ns_esc = escape_cypher(&p.namespace);
    let tt_esc = escape_cypher(&p.target_type);
    let ti_esc = escape_cypher(&p.target_id);
    let data_esc = escape_cypher(&p.data_json);
    let vid_esc = escape_cypher(&version_id.to_string());

    // §4.8: the Annotation is the stable identity (created_at set once); its
    // value lives on a temporal HAS_VERSION edge to an AnnotationVersion content
    // node. A set closes the live version and appends a new one in ONE statement,
    // so the prior value is retained as history rather than overwritten. The
    // Annotation node is namespace-specific by id, so closing *this* node's live
    // version is namespace-scoped by construction (other-namespace annotations
    // are different nodes). Promotion collapses valid==created to the event
    // instant (§7.5, no ingest LLM to extract a content valid-time).
    graph
        .write(format!(
            "MERGE (a:Annotation {{id: '{id_esc}'}}) \
               ON CREATE SET a.namespace = '{ns_esc}', a.target_type = '{tt_esc}', \
                             a.target_id = '{ti_esc}', a.created_at = {timestamp} \
             WITH a \
             OPTIONAL MATCH (a)-[old:HAS_VERSION]->(:AnnotationVersion) \
               WHERE old.invalid_at IS NULL AND old.expired_at IS NULL \
             SET old.invalid_at = {timestamp}, old.expired_at = {timestamp} \
             WITH a \
             CREATE (a)-[:HAS_VERSION {{valid_at: {timestamp}, invalid_at: NULL, \
               created_at: {timestamp}, expired_at: NULL}}]->\
               (:AnnotationVersion {{id: '{vid_esc}', data: '{data_esc}', recorded_at: {timestamp}}})"
        ))
        .await?;

    debug!(
        target_type = %p.target_type,
        target_id = %p.target_id,
        namespace = %p.namespace,
        annotation_id = %id,
        "promoted app.annotation.set"
    );
    Ok(())
}

/// Promote an `app.annotation.cleared` event by temporally closing the live
/// version of the Annotation keyed on the same deterministic id (§4.8). The
/// Annotation node and its closed versions are RETAINED for history (no
/// `DETACH DELETE`), so a cleared annotation's prior values can still be read at
/// a past `T_asof`. Idempotent: clearing a non-existent or already-cleared
/// annotation closes nothing (the liveness predicate matches no live version).
async fn promote_annotation_cleared(graph: &GraphHandle, payload: &[u8]) -> Result<()> {
    let p = AnnotationClearPayload::decode(payload)?;
    let id = annotation_id(&p.target_type, &p.target_id, &p.namespace);
    let id_esc = escape_cypher(&id.to_string());
    let now = crate::time::now().0;

    graph
        .write(format!(
            "MATCH (a:Annotation {{id: '{id_esc}'}})-[r:HAS_VERSION]->(:AnnotationVersion) \
             WHERE r.invalid_at IS NULL AND r.expired_at IS NULL \
             SET r.invalid_at = {now}, r.expired_at = {now}"
        ))
        .await?;

    debug!(
        target_type = %p.target_type,
        target_id = %p.target_id,
        namespace = %p.namespace,
        annotation_id = %id,
        "promoted app.annotation.cleared"
    );
    Ok(())
}

/// Promote an `app.badge.set` event into a UserAction node —
/// but only for `error` / `warning` status. Foundation §6.4
/// Listing 14: "Error and warning badges are recorded in the
/// Knowledge Graph; count-only badges are not."
///
/// `category = "badge"`, `action = "error" | "warning"`,
/// `subject = app_id`. Reuses the existing UserAction schema;
/// no new node type. AI queries can correlate badge spikes
/// with build / test failures over time.
async fn promote_badge_set(
    graph: &GraphHandle,
    event_id: &str,
    timestamp: &i64,
    payload: &[u8],
) -> Result<()> {
    let p = BadgeSetPayload::decode(payload)?;
    let status_kind = match BadgeStatus::try_from(p.status).unwrap_or(BadgeStatus::Unspecified) {
        BadgeStatus::Error => "error",
        BadgeStatus::Warning => "warning",
        // Success / Progress / Unspecified / count-only — not promoted.
        _ => {
            debug!(
                event_id,
                app_id = %p.app_id,
                status = p.status,
                "badge not promoted (status not error/warning)"
            );
            return Ok(());
        }
    };

    let id_esc = escape_cypher(event_id);
    let app_esc = escape_cypher(&p.app_id);

    graph
        .write(format!(
            "MERGE (u:UserAction {{id: '{id_esc}'}})
             SET u.category = 'badge',
                 u.action   = '{status_kind}',
                 u.subject  = '{app_esc}',
                 u.timestamp = {timestamp}"
        ))
        .await?;

    debug!(
        event_id,
        app_id = %p.app_id,
        status = status_kind,
        "promoted app.badge.set"
    );
    Ok(())
}

/// Promote a `*.action_invoked` event (toolbar / shortcut / menu) into a
/// UserAction node, so a user's interactions become KG-native history (the
/// "what did I export last Tuesday" query). `category` is the surface, `action`
/// the app-defined dispatch id, `subject` the app id. Content-free per S13: only
/// the structural action label, never a payload body. The three surfaces share
/// this arm because their payloads are byte-identical (`app_id`, `action`,
/// `window_id`); `window_id` is irrelevant to the interaction record.
async fn promote_action_invoked(
    graph: &GraphHandle,
    event_id: &str,
    event_type: &str,
    timestamp: &i64,
    session_id: &str,
    payload: &[u8],
) -> Result<()> {
    let p = ShortcutActionInvokedPayload::decode(payload)?;
    let category = match event_type {
        "app.toolbar.action_invoked" => "toolbar",
        "app.shortcut.action_invoked" => "shortcut",
        "app.menu.action_invoked" => "menu",
        _ => "action",
    };
    // A malformed emit with no action would write a blank node; skip it.
    if p.action.is_empty() {
        debug!(event_id, category, "action_invoked not promoted (empty action)");
        return Ok(());
    }

    let id_esc = escape_cypher(event_id);
    let action_esc = escape_cypher(&p.action);
    let app_esc = escape_cypher(&p.app_id);

    graph
        .write(format!(
            "MERGE (u:UserAction {{id: '{id_esc}'}})
             SET u.category = '{category}',
                 u.action   = '{action_esc}',
                 u.subject  = '{app_esc}',
                 u.timestamp = {timestamp}"
        ))
        .await?;

    // Session<->activity edge (KG-richness Thrust 1): link the interaction to
    // the session it happened in, so the graph answers "what did I do in this
    // session". The Session is MERGEd (window.focused enriches it) so the edge
    // never dangles. Skipped when the event carries no session. A private/
    // incognito session is excluded upstream before promotion, like every other
    // event, so no new hard-exclude surface.
    if !session_id.is_empty() {
        let session_esc = escape_cypher(session_id);
        graph
            .write(format!("MERGE (s:Session {{id: '{session_esc}'}})"))
            .await?;
        graph
            .write(format!(
                "MATCH (u:UserAction {{id: '{id_esc}'}}), (s:Session {{id: '{session_esc}'}})
                 MERGE (u)-[:PERFORMED_IN]->(s)"
            ))
            .await?;
    }

    debug!(
        event_id,
        category,
        app_id = %p.app_id,
        action = %p.action,
        "promoted action_invoked"
    );
    Ok(())
}

#[cfg(test)]
mod shell_event_tests {
    use super::*;
    use std::collections::HashMap;
    use tempfile::TempDir;

    async fn setup() -> (GraphHandle, TempDir) {
        let tmp = TempDir::new().unwrap();
        let graph =
            crate::graph::spawn(tmp.path().join("graph").to_str().unwrap()).unwrap();
        tokio::time::sleep(Duration::from_millis(500)).await;
        (graph, tmp)
    }

    fn encode_presence_set(p: &PresenceSetPayload) -> Vec<u8> {
        let mut buf = Vec::new();
        p.encode(&mut buf).unwrap();
        buf
    }

    fn encode_presence_clear(p: &PresenceClearPayload) -> Vec<u8> {
        let mut buf = Vec::new();
        p.encode(&mut buf).unwrap();
        buf
    }

    fn encode_timeline(p: &TimelineRecordPayload) -> Vec<u8> {
        let mut buf = Vec::new();
        p.encode(&mut buf).unwrap();
        buf
    }

    async fn count_user_actions_by_category(graph: &GraphHandle, category: &str) -> i64 {
        let rs = graph
            .query_rows(format!(
                "MATCH (u:UserAction) WHERE u.category = '{category}' RETURN count(*) AS cnt"
            ))
            .await
            .unwrap();
        rs.rows
            .first()
            .and_then(|r| r.first())
            .map(|v| v.as_i64())
            .unwrap_or(0)
    }

    #[tokio::test]
    async fn promote_file_opened_records_cgroup_id() {
        // A file.opened payload carrying an eBPF cgroup id lands as last_cgroup_id
        // on the File node (Strand 4 attribution).
        let (graph, _tmp) = setup().await;
        let payload = FileOpenedPayload {
            path: "/proj/main.rs".into(),
            app_id: "ebpf:42".into(),
            flags: 0,
            cgroup_id: 987_654,
        };
        let mut buf = Vec::new();
        payload.encode(&mut buf).unwrap();
        promote_file_opened(&graph, "ev1", &100, "ebpf", &42, "sess", &buf)
            .await
            .unwrap();
        let rs = graph
            .query_rows("MATCH (f:File {id: '/proj/main.rs'}) RETURN f.last_cgroup_id AS cg".into())
            .await
            .unwrap();
        let cg = rs
            .rows
            .first()
            .and_then(|r| r.first())
            .map(|v| v.as_i64())
            .unwrap();
        assert_eq!(cg, 987_654, "the cgroup id round-trips onto the File node");
    }

    #[tokio::test]
    async fn promote_file_opened_links_the_file_to_its_session() {
        // KG-richness Thrust 1: a file.opened with a session links the File to a
        // MERGEd Session node (no dangling edge); a session-less event adds none.
        let (graph, _tmp) = setup().await;
        let payload = FileOpenedPayload {
            path: "/proj/a.rs".into(),
            app_id: "editor".into(),
            flags: 0,
            cgroup_id: 0,
        };
        let mut buf = Vec::new();
        payload.encode(&mut buf).unwrap();
        promote_file_opened(&graph, "ev1", &100, "ebpf", &42, "sess-9", &buf)
            .await
            .unwrap();
        let rs = graph
            .query_rows(
                "MATCH (:File {id:'/proj/a.rs'})-[:ACCESSED_IN]->(:Session {id:'sess-9'}) \
                 RETURN count(*) AS c"
                    .into(),
            )
            .await
            .unwrap();
        let c = rs.rows.first().and_then(|r| r.first()).map(|v| v.as_i64()).unwrap();
        assert_eq!(c, 1, "the file is linked to its session");

        // A session-less event (a raw eBPF open) adds no Session edge and errors not.
        let mut buf2 = Vec::new();
        FileOpenedPayload {
            path: "/proj/b.rs".into(),
            app_id: "editor".into(),
            flags: 0,
            cgroup_id: 0,
        }
        .encode(&mut buf2)
        .unwrap();
        promote_file_opened(&graph, "ev2", &100, "ebpf", &42, "", &buf2)
            .await
            .unwrap();
        let rs2 = graph
            .query_rows(
                "MATCH (:File {id:'/proj/b.rs'})-[:ACCESSED_IN]->() RETURN count(*) AS c".into(),
            )
            .await
            .unwrap();
        let c2 = rs2.rows.first().and_then(|r| r.first()).map(|v| v.as_i64()).unwrap();
        assert_eq!(c2, 0, "a session-less event creates no session edge");
    }

    #[tokio::test]
    async fn co_access_links_files_in_the_same_session_canonically() {
        // KG-richness Thrust 1: files opened in one session get a CO_ACCESSED edge,
        // stored canonically (one edge per pair regardless of access order) and
        // refreshed on re-access, never duplicated.
        let (graph, _tmp) = setup().await;
        let open = |path: &str, ts: i64, ev: &str| {
            let mut buf = Vec::new();
            FileOpenedPayload {
                path: path.into(),
                app_id: "editor".into(),
                flags: 0,
                cgroup_id: 0,
            }
            .encode(&mut buf)
            .unwrap();
            (buf, ts, ev.to_string())
        };

        // Open /z then /a in one session: /a (smaller id) is FROM, /z is TO.
        let (b1, t1, e1) = open("/proj/z.rs", 100, "co1");
        promote_file_opened(&graph, &e1, &t1, "ebpf", &1, "sess-co", &b1).await.unwrap();
        let (b2, t2, e2) = open("/proj/a.rs", 200, "co2");
        promote_file_opened(&graph, &e2, &t2, "ebpf", &1, "sess-co", &b2).await.unwrap();

        // Exactly one CO_ACCESSED edge between the pair, in canonical direction.
        let dir = graph
            .query_rows(
                "MATCH (:File {id:'/proj/a.rs'})-[c:CO_ACCESSED]->(:File {id:'/proj/z.rs'}) \
                 RETURN c.last_seen AS seen"
                    .into(),
            )
            .await
            .unwrap();
        assert_eq!(dir.rows.len(), 1, "one canonical edge a->z exists");
        assert_eq!(dir.rows[0][0].as_i64(), 200, "last_seen is the co-access time");

        // The reverse direction was never created (canonicalisation holds).
        let rev = graph
            .query_rows(
                "MATCH (:File {id:'/proj/z.rs'})-[c:CO_ACCESSED]->(:File {id:'/proj/a.rs'}) \
                 RETURN count(*) AS c"
                    .into(),
            )
            .await
            .unwrap();
        assert_eq!(rev.rows[0][0].as_i64(), 0, "no reverse-direction edge");

        // Re-open /z later: same pair, refreshed last_seen, still one edge.
        let (b3, t3, e3) = open("/proj/z.rs", 300, "co3");
        promote_file_opened(&graph, &e3, &t3, "ebpf", &1, "sess-co", &b3).await.unwrap();
        let again = graph
            .query_rows(
                "MATCH (:File {id:'/proj/a.rs'})-[c:CO_ACCESSED]->(:File {id:'/proj/z.rs'}) \
                 RETURN count(*) AS n, max(c.last_seen) AS seen"
                    .into(),
            )
            .await
            .unwrap();
        assert_eq!(again.rows[0][0].as_i64(), 1, "re-access does not duplicate the edge");
        assert_eq!(again.rows[0][1].as_i64(), 300, "last_seen refreshed on re-access");

        // A file in a different session shares no co-access edge.
        let (b4, t4, e4) = open("/other/x.rs", 400, "co4");
        promote_file_opened(&graph, &e4, &t4, "ebpf", &1, "sess-other", &b4).await.unwrap();
        let cross = graph
            .query_rows(
                "MATCH (:File {id:'/other/x.rs'})-[:CO_ACCESSED]-(:File) RETURN count(*) AS c".into(),
            )
            .await
            .unwrap();
        assert_eq!(cross.rows[0][0].as_i64(), 0, "no co-access across sessions");
    }

    #[tokio::test]
    async fn promote_file_written_records_a_modified_by_edge() {
        // A file.written event creates a MODIFIED_BY edge (the app WROTE the
        // file), distinct from the read-side ACCESSED_BY, and does not touch
        // the read's last_accessed (a write is not an access).
        let (graph, _tmp) = setup().await;
        let payload = FileWrittenPayload {
            path: "/proj/out.bin".into(),
            app_id: "build:7".into(),
            bytes: 4096,
        };
        let mut buf = Vec::new();
        payload.encode(&mut buf).unwrap();
        promote_file_written(&graph, "w1", &200, "build", &7, &buf)
            .await
            .unwrap();
        let rs = graph
            .query_rows(
                "MATCH (f:File {id: '/proj/out.bin'})-[:MODIFIED_BY]->(a:App {id: 'build:7'}) \
                 RETURN count(*) AS cnt"
                    .into(),
            )
            .await
            .unwrap();
        let cnt = rs.rows.first().and_then(|r| r.first()).map(|v| v.as_i64()).unwrap();
        assert_eq!(cnt, 1, "the write creates exactly one MODIFIED_BY edge");
        // Re-promoting the same write is idempotent (MERGE), not a second edge.
        promote_file_written(&graph, "w1", &200, "build", &7, &buf)
            .await
            .unwrap();
        let rs2 = graph
            .query_rows(
                "MATCH (:File {id: '/proj/out.bin'})-[:MODIFIED_BY]->(:App {id: 'build:7'}) \
                 RETURN count(*) AS cnt"
                    .into(),
            )
            .await
            .unwrap();
        let cnt2 = rs2.rows.first().and_then(|r| r.first()).map(|v| v.as_i64()).unwrap();
        assert_eq!(cnt2, 1, "a repeated write does not duplicate the edge");
    }

    #[tokio::test]
    async fn promote_network_connection_records_a_connected_to_edge() {
        // KG-richness Thrust 1: a network.connect becomes an App -> NetworkEndpoint
        // CONNECTED_TO edge keyed by the remote IP:port, with direction + last_seen.
        let (graph, _tmp) = setup().await;
        let payload = NetworkConnectionPayload {
            app_id: "dev.arlen.browser".into(),
            remote_addr: "93.184.216.34:443".into(),
            protocol: "tcp".into(),
            direction: "outbound".into(),
        };
        let mut buf = Vec::new();
        payload.encode(&mut buf).unwrap();
        promote_network_connection(&graph, "n1", &500, &buf)
            .await
            .unwrap();
        let rs = graph
            .query_rows(
                "MATCH (:App {id: 'dev.arlen.browser'})-[c:CONNECTED_TO]->(e:NetworkEndpoint) \
                 RETURN e.id AS addr, e.protocol AS proto, c.direction AS dir, c.last_seen AS seen"
                    .into(),
            )
            .await
            .unwrap();
        let row = rs.rows.first().expect("the CONNECTED_TO edge exists");
        assert_eq!(row[0].as_str(), "93.184.216.34:443");
        assert_eq!(row[1].as_str(), "tcp");
        assert_eq!(row[2].as_str(), "outbound");
        assert_eq!(row[3].as_i64(), 500);

        // Re-observing the same connection is idempotent (one edge) and refreshes
        // last_seen rather than appending a duplicate.
        let payload2 = NetworkConnectionPayload {
            direction: "outbound".into(),
            ..payload.clone()
        };
        let mut buf2 = Vec::new();
        payload2.encode(&mut buf2).unwrap();
        promote_network_connection(&graph, "n2", &900, &buf2)
            .await
            .unwrap();
        let rs2 = graph
            .query_rows(
                "MATCH (:App {id: 'dev.arlen.browser'})-[c:CONNECTED_TO]->() \
                 RETURN count(*) AS cnt, max(c.last_seen) AS seen"
                    .into(),
            )
            .await
            .unwrap();
        assert_eq!(rs2.rows[0][0].as_i64(), 1, "a repeat connection does not duplicate the edge");
        assert_eq!(rs2.rows[0][1].as_i64(), 900, "last_seen is refreshed");

        // An event missing the app id or remote address is skipped, no node.
        let mut buf3 = Vec::new();
        NetworkConnectionPayload {
            app_id: String::new(),
            remote_addr: "1.2.3.4:80".into(),
            protocol: "tcp".into(),
            direction: "outbound".into(),
        }
        .encode(&mut buf3)
        .unwrap();
        promote_network_connection(&graph, "n3", &1000, &buf3)
            .await
            .unwrap();
        let rs3 = graph
            .query_rows("MATCH (e:NetworkEndpoint {id: '1.2.3.4:80'}) RETURN count(*) AS c".into())
            .await
            .unwrap();
        assert_eq!(rs3.rows[0][0].as_i64(), 0, "a connection with no app id is not promoted");
    }

    #[tokio::test]
    async fn index_file_fact_text_makes_a_file_searchable_by_basename() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        crate::fts::create_fact_text_index(&pool).await.unwrap();

        index_file_fact_text(&pool, "/home/tim/proj/main.rs").await.unwrap();

        // The synthesised text includes the basename, so a keyword search finds
        // the File by its node id (the path).
        let hits = crate::fts::search_fact_text(&pool, "main.rs", 10).await.unwrap();
        assert_eq!(hits, vec!["/home/tim/proj/main.rs".to_string()]);
        // Re-indexing the same file does not duplicate it.
        index_file_fact_text(&pool, "/home/tim/proj/main.rs").await.unwrap();
        assert_eq!(crate::fts::search_fact_text(&pool, "main.rs", 10).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn promote_power_transition_creates_a_timeline_event() {
        // A coarse power transition lands as a generic Event node carrying its
        // type, timestamp and source - the timeline marker the AI correlates to
        // session boundaries (§3f). No battery-% churn: only the transition.
        let (graph, _tmp) = setup().await;
        promote_power_transition(&graph, "pwr1", "power.critical", &4242, "app:arlen-powerd")
            .await
            .unwrap();
        let rs = graph
            .query_rows(
                "MATCH (e:Event {id: 'pwr1'}) RETURN e.type AS t, e.timestamp AS ts, e.source AS s"
                    .into(),
            )
            .await
            .unwrap();
        let row = rs.rows.first().expect("the power transition Event exists");
        assert_eq!(row[0].as_str(), "power.critical");
        assert_eq!(row[1].as_i64(), 4242);
        assert_eq!(row[2].as_str(), "app:arlen-powerd");
    }

    #[tokio::test]
    async fn promote_window_focused_creates_app_session_and_event() {
        // Coverage for the foundational window.focused path: it must promote
        // without error (it SETs e.title on the Event node, so this also confirms
        // the graph accepts that write) and wire the App -> Session ACTIVE_IN edge.
        let (graph, _tmp) = setup().await;
        let payload = WindowFocusedPayload {
            app_id: "com.example.editor".into(),
            window_title: "notes.md".into(),
            prev_app_id: String::new(),
        };
        let mut buf = Vec::new();
        payload.encode(&mut buf).unwrap();
        promote_window_focused(&graph, "win1", &500, "sess-1", &buf)
            .await
            .expect("window.focused promotes without error");

        let rs = graph
            .query_rows("MATCH (e:Event {id: 'win1'}) RETURN e.type AS t, e.timestamp AS ts".into())
            .await
            .unwrap();
        let row = rs.rows.first().expect("the window.focused Event exists");
        assert_eq!(row[0].as_str(), "window.focused");
        assert_eq!(row[1].as_i64(), 500);

        let edge = graph
            .query_rows(
                "MATCH (:App {id: 'com.example.editor'})-[:ACTIVE_IN]->(:Session {id: 'sess-1'}) \
                 RETURN count(*) AS c"
                    .into(),
            )
            .await
            .unwrap();
        assert_eq!(edge.rows[0][0].as_i64(), 1, "the app is active in the session");
    }

    #[tokio::test]
    async fn promote_code_indexed_defines_symbols_and_replaces_per_file() {
        use crate::proto::CodeSymbolPayload;
        let (graph, _tmp) = setup().await;

        let sym = |id: &str, name: &str, line: &str, kind: &str| CodeSymbolPayload {
            id: id.to_string(),
            name: name.to_string(),
            source_location: line.to_string(),
            kind: kind.to_string(),
        };
        let first = CodeFileIndexPayload {
            source_file: "/p/lib.rs".to_string(),
            language: "rust".to_string(),
            symbols: vec![
                sym("/p/lib.rs#function:helper@1", "helper", "1", "function"),
                sym("/p/lib.rs#function:main@2", "main", "2", "function"),
            ],
        };
        let mut buf = Vec::new();
        first.encode(&mut buf).unwrap();
        promote_code_indexed(&graph, &buf)
            .await
            .expect("the first index promotes");

        // Two CodeSymbols, each DEFINES'd by the File with confidence extracted.
        let rs = graph
            .query_rows(
                "MATCH (:File {id:'/p/lib.rs'})-[d:DEFINES]->(s:CodeSymbol) \
                 RETURN s.name AS n, d.confidence AS c"
                    .into(),
            )
            .await
            .unwrap();
        assert_eq!(rs.rows.len(), 2, "both symbols defined");
        assert!(rs.rows.iter().all(|r| r[1].as_str() == "extracted"), "all DEFINES are extracted");

        // Re-parse the same file with a DIFFERENT symbol set: the prior symbols
        // are fully replaced (per-file isolation), not accumulated.
        let second = CodeFileIndexPayload {
            source_file: "/p/lib.rs".to_string(),
            language: "rust".to_string(),
            symbols: vec![sym("/p/lib.rs#struct:Widget@1", "Widget", "1", "struct")],
        };
        let mut buf2 = Vec::new();
        second.encode(&mut buf2).unwrap();
        promote_code_indexed(&graph, &buf2)
            .await
            .expect("the re-parse promotes");

        let after = graph
            .query_rows(
                "MATCH (s:CodeSymbol {source_file:'/p/lib.rs'}) RETURN s.name AS n".into(),
            )
            .await
            .unwrap();
        let names: Vec<&str> = after.rows.iter().map(|r| r[0].as_str()).collect();
        assert_eq!(names, vec!["Widget"], "the file's symbols are replaced, not accumulated");
    }

    #[tokio::test]
    async fn promote_code_indexed_dedups_duplicate_ids_without_stalling() {
        // A crafted/duplicate-id payload must NOT fail the transaction (a duplicate
        // primary key would stall ALL promotion). The promoter dedups, so it
        // promotes one symbol and succeeds.
        use crate::proto::CodeSymbolPayload;
        let (graph, _tmp) = setup().await;
        let dup = CodeSymbolPayload {
            id: "/p/lib.rs#function:f@1:1".to_string(),
            name: "f".to_string(),
            source_location: "1:1".to_string(),
            kind: "function".to_string(),
        };
        let payload = CodeFileIndexPayload {
            source_file: "/p/lib.rs".to_string(),
            language: "rust".to_string(),
            symbols: vec![dup.clone(), dup],
        };
        let mut buf = Vec::new();
        payload.encode(&mut buf).unwrap();
        promote_code_indexed(&graph, &buf)
            .await
            .expect("a duplicate-id payload promotes without a transaction failure");
        let rs = graph
            .query_rows("MATCH (s:CodeSymbol {source_file:'/p/lib.rs'}) RETURN s.id AS i".into())
            .await
            .unwrap();
        assert_eq!(rs.rows.len(), 1, "the duplicate id is collapsed to one node");
    }

    #[tokio::test]
    async fn promote_badge_set_records_only_error_and_warning_badges() {
        // Coverage for the last promoter: an error/warning badge becomes a
        // UserAction (writing only schema columns); a success badge is not
        // promoted. This guards the whole promotion pipeline against a future
        // schema/SET drift (the class of bug that stalled window.focused).
        let (graph, _tmp) = setup().await;
        let err_badge = BadgeSetPayload {
            app_id: "com.example.mail".into(),
            variant: 2, // status
            count: 0,
            status: BadgeStatus::Error as i32,
            progress_value: None,
        };
        let mut buf = Vec::new();
        err_badge.encode(&mut buf).unwrap();
        promote_badge_set(&graph, "badge1", &900, &buf)
            .await
            .expect("an error badge promotes without error");
        let rs = graph
            .query_rows(
                "MATCH (u:UserAction {id: 'badge1'}) RETURN u.category AS c, u.action AS a, u.subject AS s"
                    .into(),
            )
            .await
            .unwrap();
        let row = rs.rows.first().expect("the error badge promoted to a UserAction");
        assert_eq!(row[0].as_str(), "badge");
        assert_eq!(row[1].as_str(), "error");
        assert_eq!(row[2].as_str(), "com.example.mail");

        // A success badge is not promoted (only error/warning are).
        let ok_badge = BadgeSetPayload {
            app_id: "com.example.mail".into(),
            variant: 2,
            count: 0,
            status: BadgeStatus::Success as i32,
            progress_value: None,
        };
        let mut buf2 = Vec::new();
        ok_badge.encode(&mut buf2).unwrap();
        promote_badge_set(&graph, "badge2", &901, &buf2).await.unwrap();
        let none = graph
            .query_rows("MATCH (u:UserAction {id: 'badge2'}) RETURN count(*) AS c".into())
            .await
            .unwrap();
        assert_eq!(none.rows[0][0].as_i64(), 0, "a success badge is not promoted");
    }

    #[tokio::test]
    async fn promote_action_invoked_records_user_actions_per_surface() {
        // A user interaction on any surface becomes a UserAction with the
        // surface as category; the three surfaces share one arm + payload shape.
        let (graph, _tmp) = setup().await;

        let menu = ShortcutActionInvokedPayload {
            app_id: "dev.arlen.terminal".into(),
            action: "new-session".into(),
            window_id: String::new(),
        };
        let mut buf = Vec::new();
        menu.encode(&mut buf).unwrap();
        promote_action_invoked(
            &graph,
            "menu1",
            "app.menu.action_invoked",
            &900,
            "sess-A",
            &buf,
        )
        .await
        .expect("a menu action promotes");
        let rs = graph
            .query_rows(
                "MATCH (u:UserAction {id: 'menu1'}) RETURN u.category AS c, u.action AS a, u.subject AS s"
                    .into(),
            )
            .await
            .unwrap();
        let row = rs.rows.first().expect("the menu action promoted to a UserAction");
        assert_eq!(row[0].as_str(), "menu");
        assert_eq!(row[1].as_str(), "new-session");
        assert_eq!(row[2].as_str(), "dev.arlen.terminal");

        // KG-richness Thrust 1: the action links to its session (PERFORMED_IN).
        let edge = graph
            .query_rows(
                "MATCH (:UserAction {id: 'menu1'})-[:PERFORMED_IN]->(s:Session) RETURN s.id AS sid"
                    .into(),
            )
            .await
            .unwrap();
        assert_eq!(
            edge.rows.first().expect("the PERFORMED_IN edge exists")[0].as_str(),
            "sess-A",
            "the action is linked to the session it happened in"
        );

        // A toolbar action goes through the same arm with category=toolbar.
        let toolbar = ShortcutActionInvokedPayload {
            app_id: "dev.arlen.files".into(),
            action: "compress".into(),
            window_id: "main".into(),
        };
        let mut buf2 = Vec::new();
        toolbar.encode(&mut buf2).unwrap();
        promote_action_invoked(
            &graph,
            "tb1",
            "app.toolbar.action_invoked",
            &901,
            "", // no session: the edge is skipped, not dangling
            &buf2,
        )
        .await
        .unwrap();
        let rs2 = graph
            .query_rows("MATCH (u:UserAction {id: 'tb1'}) RETURN u.category AS c".into())
            .await
            .unwrap();
        assert_eq!(rs2.rows.first().expect("toolbar promoted")[0].as_str(), "toolbar");
        let tb_edge = graph
            .query_rows(
                "MATCH (:UserAction {id: 'tb1'})-[:PERFORMED_IN]->() RETURN count(*) AS c".into(),
            )
            .await
            .unwrap();
        assert_eq!(
            tb_edge.rows[0][0].as_i64(),
            0,
            "a session-less action has no PERFORMED_IN edge"
        );

        // An empty action (malformed emit) writes no node.
        let empty = ShortcutActionInvokedPayload {
            app_id: "dev.arlen.x".into(),
            action: String::new(),
            window_id: String::new(),
        };
        let mut buf3 = Vec::new();
        empty.encode(&mut buf3).unwrap();
        promote_action_invoked(
            &graph,
            "empty1",
            "app.menu.action_invoked",
            &902,
            "sess-A",
            &buf3,
        )
        .await
        .unwrap();
        let none = graph
            .query_rows("MATCH (u:UserAction {id: 'empty1'}) RETURN count(*) AS c".into())
            .await
            .unwrap();
        assert_eq!(none.rows[0][0].as_i64(), 0, "an empty action is not promoted");
    }

    #[tokio::test]
    async fn presence_set_creates_user_action() {
        let (graph, _tmp) = setup().await;
        let payload = PresenceSetPayload {
            app_id: "com.example.editor".into(),
            activity: "editing".into(),
            subject: "/home/tim/notes.md".into(),
            project: String::new(),
            auto_clear: "on-blur".into(),
            metadata: HashMap::new(),
        };
        let bytes = encode_presence_set(&payload);

        promote_presence_set(&graph, "evt-presence-1", &1_000_000, &bytes)
            .await
            .unwrap();

        assert_eq!(count_user_actions_by_category(&graph, "presence").await, 1);
    }

    #[tokio::test]
    async fn presence_clear_creates_user_action() {
        let (graph, _tmp) = setup().await;
        let payload = PresenceClearPayload {
            app_id: "com.example.editor".into(),
        };
        let bytes = encode_presence_clear(&payload);

        promote_presence_clear(&graph, "evt-presence-clear-1", &2_000_000, &bytes)
            .await
            .unwrap();

        // Clear is also a presence-category record.
        let rs = graph
            .query_rows(
                "MATCH (u:UserAction) WHERE u.category = 'presence' AND u.action = 'clear' \
                 RETURN u.subject"
                    .to_string(),
            )
            .await
            .unwrap();
        assert_eq!(rs.rows.len(), 1);
    }

    #[tokio::test]
    async fn timeline_record_uses_ended_at_when_set() {
        let (graph, _tmp) = setup().await;
        let payload = TimelineRecordPayload {
            app_id: "com.example.builder".into(),
            label: "Build succeeded".into(),
            subject: "coffeeshop".into(),
            r#type: "build".into(),
            started_at: 5_000_000,
            ended_at: 9_500_000,
            metadata: HashMap::new(),
        };
        let bytes = encode_timeline(&payload);

        promote_timeline_record(&graph, "evt-timeline-1", &10_000_000, &bytes)
            .await
            .unwrap();

        let rs = graph
            .query_rows(
                "MATCH (u:UserAction) WHERE u.category = 'timeline' \
                 RETURN u.timestamp, u.action, u.subject"
                    .to_string(),
            )
            .await
            .unwrap();
        let row = rs.rows.first().expect("user action created");
        assert_eq!(row[0].as_i64(), 9_500_000); // ended_at wins
        assert_eq!(row[1].as_str(), "build");
        assert_eq!(row[2].as_str(), "Build succeeded");
    }

    #[tokio::test]
    async fn timeline_record_falls_back_to_envelope_timestamp() {
        let (graph, _tmp) = setup().await;
        // Point-in-time event: both started_at and ended_at are 0 in the
        // payload; promotion must fall back to the Event envelope's
        // wall-clock timestamp.
        let payload = TimelineRecordPayload {
            app_id: "com.example.editor".into(),
            label: "Exported PDF".into(),
            subject: "/home/tim/report.pdf".into(),
            r#type: "export".into(),
            started_at: 0,
            ended_at: 0,
            metadata: HashMap::new(),
        };
        let bytes = encode_timeline(&payload);

        promote_timeline_record(&graph, "evt-timeline-2", &7_777_777, &bytes)
            .await
            .unwrap();

        let rs = graph
            .query_rows(
                "MATCH (u:UserAction) WHERE u.category = 'timeline' \
                 RETURN u.timestamp"
                    .to_string(),
            )
            .await
            .unwrap();
        assert_eq!(rs.rows[0][0].as_i64(), 7_777_777);
    }

    fn encode_annotation_set(p: &AnnotationSetPayload) -> Vec<u8> {
        let mut buf = Vec::new();
        p.encode(&mut buf).unwrap();
        buf
    }

    fn encode_annotation_clear(p: &AnnotationClearPayload) -> Vec<u8> {
        let mut buf = Vec::new();
        p.encode(&mut buf).unwrap();
        buf
    }

    async fn fetch_annotation(
        graph: &GraphHandle,
        target_type: &str,
        target_id: &str,
        namespace: &str,
    ) -> Option<(String, i64, i64)> {
        // The LIVE version's data, the Annotation's first-seen created_at, and
        // the live version's recorded_at (the §4.8 two-hop read).
        let rs = graph
            .query_rows(format!(
                "MATCH (a:Annotation)-[r:HAS_VERSION]->(v:AnnotationVersion) \
                 WHERE a.target_type = '{target_type}' AND a.target_id = '{target_id}' \
                 AND a.namespace = '{namespace}' \
                 AND r.invalid_at IS NULL AND r.expired_at IS NULL \
                 RETURN v.data, a.created_at, v.recorded_at"
            ))
            .await
            .unwrap();
        rs.rows.first().map(|r| {
            (
                r[0].as_str().to_string(),
                r[1].as_i64(),
                r[2].as_i64(),
            )
        })
    }

    #[tokio::test]
    async fn annotation_set_creates_node() {
        let (graph, _tmp) = setup().await;
        let payload = AnnotationSetPayload {
            app_id: "com.example.editor".into(),
            namespace: "com.example.editor".into(),
            target_type: "File".into(),
            target_id: "/home/tim/report.md".into(),
            data_json: r#"{"word_count":1240}"#.into(),
        };

        promote_annotation_set(&graph, &1_000_000, &encode_annotation_set(&payload))
            .await
            .unwrap();

        let got = fetch_annotation(&graph, "File", "/home/tim/report.md", "com.example.editor")
            .await
            .expect("annotation should exist");
        assert_eq!(got.0, r#"{"word_count":1240}"#);
        assert_eq!(got.1, 1_000_000); // created_at
        assert_eq!(got.2, 1_000_000); // last_modified
    }

    #[tokio::test]
    async fn annotation_re_set_replaces_data_and_keeps_created_at() {
        let (graph, _tmp) = setup().await;
        let p1 = AnnotationSetPayload {
            app_id: "com.example.editor".into(),
            namespace: "com.example.editor".into(),
            target_type: "File".into(),
            target_id: "/home/tim/notes.md".into(),
            data_json: r#"{"word_count":100}"#.into(),
        };
        let p2 = AnnotationSetPayload {
            app_id: "com.example.editor".into(),
            namespace: "com.example.editor".into(),
            target_type: "File".into(),
            target_id: "/home/tim/notes.md".into(),
            data_json: r#"{"word_count":250}"#.into(),
        };

        promote_annotation_set(&graph, &1_000, &encode_annotation_set(&p1))
            .await
            .unwrap();
        promote_annotation_set(&graph, &5_000, &encode_annotation_set(&p2))
            .await
            .unwrap();

        let got = fetch_annotation(&graph, "File", "/home/tim/notes.md", "com.example.editor")
            .await
            .unwrap();
        assert_eq!(got.0, r#"{"word_count":250}"#); // the live version's data
        assert_eq!(got.1, 1_000); // original created_at preserved
        assert_eq!(got.2, 5_000); // the live version's recorded_at advanced

        // History is retained (§4.8): both versions exist, exactly one live.
        let total = graph
            .query_rows(
                "MATCH (:Annotation)-[:HAS_VERSION]->(:AnnotationVersion) RETURN count(*) AS n".into(),
            )
            .await
            .unwrap();
        assert_eq!(total.rows[0][0].as_i64(), 2, "the prior version is retained, not overwritten");
        let live = graph
            .query_rows(
                "MATCH (:Annotation)-[r:HAS_VERSION]->(:AnnotationVersion) \
                 WHERE r.invalid_at IS NULL AND r.expired_at IS NULL RETURN count(*) AS n"
                    .into(),
            )
            .await
            .unwrap();
        assert_eq!(live.rows[0][0].as_i64(), 1, "exactly one live version");
    }

    #[tokio::test]
    async fn annotation_clear_closes_the_live_version() {
        let (graph, _tmp) = setup().await;
        let set_payload = AnnotationSetPayload {
            app_id: "com.example.editor".into(),
            namespace: "com.example.editor".into(),
            target_type: "File".into(),
            target_id: "/x".into(),
            data_json: "{}".into(),
        };
        let clear_payload = AnnotationClearPayload {
            app_id: "com.example.editor".into(),
            namespace: "com.example.editor".into(),
            target_type: "File".into(),
            target_id: "/x".into(),
        };

        promote_annotation_set(&graph, &100, &encode_annotation_set(&set_payload))
            .await
            .unwrap();
        assert!(fetch_annotation(&graph, "File", "/x", "com.example.editor")
            .await
            .is_some());

        promote_annotation_cleared(&graph, &encode_annotation_clear(&clear_payload))
            .await
            .unwrap();
        assert!(fetch_annotation(&graph, "File", "/x", "com.example.editor")
            .await
            .is_none());
    }

    #[tokio::test]
    async fn annotation_clear_on_missing_is_noop() {
        let (graph, _tmp) = setup().await;
        let clear_payload = AnnotationClearPayload {
            app_id: "com.example.editor".into(),
            namespace: "com.example.editor".into(),
            target_type: "File".into(),
            target_id: "/never-set".into(),
        };
        // Must not panic / error.
        promote_annotation_cleared(&graph, &encode_annotation_clear(&clear_payload))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn annotation_id_is_deterministic_across_triple() {
        // The UUIDv5 derivation is part of the wire contract: SDK
        // queries by (target_type, target_id, namespace) but the
        // graph node uses the derived id. If this drifts, queries
        // return empty when annotations exist.
        let id1 = annotation_id("File", "/x", "com.app");
        let id2 = annotation_id("File", "/x", "com.app");
        assert_eq!(id1, id2);

        let id3 = annotation_id("File", "/y", "com.app");
        assert_ne!(id1, id3);

        let id4 = annotation_id("File", "/x", "com.other");
        assert_ne!(id1, id4);
    }

    #[tokio::test]
    async fn annotation_namespaces_are_independent_for_same_target() {
        // Two apps annotate the same File — must produce two
        // independent Annotation nodes, each addressable by its own
        // namespace.
        let (graph, _tmp) = setup().await;
        let editor = AnnotationSetPayload {
            app_id: "com.example.editor".into(),
            namespace: "com.example.editor".into(),
            target_type: "File".into(),
            target_id: "/shared.md".into(),
            data_json: r#"{"word_count":500}"#.into(),
        };
        let git = AnnotationSetPayload {
            app_id: "com.example.git".into(),
            namespace: "com.example.git".into(),
            target_type: "File".into(),
            target_id: "/shared.md".into(),
            data_json: r#"{"branch":"main"}"#.into(),
        };

        promote_annotation_set(&graph, &10, &encode_annotation_set(&editor))
            .await
            .unwrap();
        promote_annotation_set(&graph, &20, &encode_annotation_set(&git))
            .await
            .unwrap();

        let editor_got = fetch_annotation(&graph, "File", "/shared.md", "com.example.editor")
            .await
            .unwrap();
        let git_got = fetch_annotation(&graph, "File", "/shared.md", "com.example.git")
            .await
            .unwrap();
        assert_eq!(editor_got.0, r#"{"word_count":500}"#);
        assert_eq!(git_got.0, r#"{"branch":"main"}"#);
    }
}

#[cfg(test)]
mod project_tests {
    use super::*;
    use crate::project::{Project, ProjectStore};
    use tempfile::TempDir;

    async fn setup() -> (GraphHandle, ProjectStore, TempDir) {
        let tmp = TempDir::new().unwrap();
        let graph =
            crate::graph::spawn(tmp.path().join("graph").to_str().unwrap()).unwrap();
        tokio::time::sleep(Duration::from_millis(500)).await;
        let store = ProjectStore::new(graph.clone());
        (graph, store, tmp)
    }

    /// Create a File node (simulates what promote_file_opened does).
    async fn create_file_node(graph: &GraphHandle, path: &str) {
        let p = escape_cypher(path);
        graph
            .write(format!(
                "CREATE (f:File {{id: '{p}', path: '{p}', app_id: 'test', last_accessed: 0}})"
            ))
            .await
            .unwrap();
    }

    /// Create File + App + Session + edges (for count_session_files).
    async fn create_file_with_session(
        graph: &GraphHandle,
        store: &ProjectStore,
        path: &str,
        app_id: &str,
        session_id: &str,
        project_id: uuid::Uuid,
    ) {
        let p = escape_cypher(path);
        let a = escape_cypher(app_id);
        let s = escape_cypher(session_id);

        graph
            .write(format!(
                "MERGE (f:File {{id: '{p}'}}) SET f.path = '{p}', f.app_id = '{a}', f.last_accessed = 1"
            ))
            .await
            .unwrap();
        graph
            .write(format!("MERGE (a:App {{id: '{a}'}}) SET a.name = '{a}'"))
            .await
            .unwrap();
        graph
            .write(format!("MERGE (s:Session {{id: '{s}'}}) SET s.started_at = 1"))
            .await
            .unwrap();
        graph
            .write(format!(
                "MATCH (f:File {{id: '{p}'}}), (a:App {{id: '{a}'}}) MERGE (f)-[:ACCESSED_BY]->(a)"
            ))
            .await
            .unwrap();
        graph
            .write(format!(
                "MATCH (a:App {{id: '{a}'}}), (s:Session {{id: '{s}'}}) MERGE (a)-[:ACTIVE_IN]->(s)"
            ))
            .await
            .unwrap();

        store.link_file(path, project_id).await.unwrap();
    }

    #[tokio::test]
    async fn file_linked_to_project() {
        let (graph, store, _tmp) = setup().await;

        let project = Project::new_inferred("test".into(), "/home/user/proj".into(), 90);
        store.create(&project).await.unwrap();

        create_file_node(&graph, "/home/user/proj/src/main.rs").await;

        link_file_to_project("/home/user/proj/src/main.rs", "sess", &store, PROMOTION_THRESHOLD_DEFAULT)
            .await
            .unwrap();

        assert!(store
            .is_file_linked("/home/user/proj/src/main.rs", project.id)
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn file_outside_project_not_linked() {
        let (_graph, store, _tmp) = setup().await;

        let project = Project::new_inferred("proj".into(), "/home/user/proj".into(), 90);
        store.create(&project).await.unwrap();

        // File is outside the project root.
        link_file_to_project("/home/user/other/file.txt", "sess", &store, PROMOTION_THRESHOLD_DEFAULT)
            .await
            .unwrap();

        // No edge should exist (file not even in graph, but that's fine).
        let files = store.get_project_files(project.id).await.unwrap();
        assert!(files.is_empty());
    }

    #[tokio::test]
    async fn nested_project_wins() {
        let (graph, store, _tmp) = setup().await;

        let parent = Project::new_inferred("mono".into(), "/home/user/mono".into(), 90);
        store.create(&parent).await.unwrap();

        let nested =
            Project::new_inferred("app-a".into(), "/home/user/mono/pkg/app-a".into(), 100);
        store.create(&nested).await.unwrap();

        create_file_node(&graph, "/home/user/mono/pkg/app-a/src/lib.rs").await;

        link_file_to_project("/home/user/mono/pkg/app-a/src/lib.rs", "sess", &store, PROMOTION_THRESHOLD_DEFAULT)
            .await
            .unwrap();

        assert!(store
            .is_file_linked("/home/user/mono/pkg/app-a/src/lib.rs", nested.id)
            .await
            .unwrap());
        assert!(!store
            .is_file_linked("/home/user/mono/pkg/app-a/src/lib.rs", parent.id)
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn idempotent_linking() {
        let (graph, store, _tmp) = setup().await;

        let project = Project::new_inferred("proj".into(), "/a".into(), 90);
        store.create(&project).await.unwrap();

        create_file_node(&graph, "/a/f.rs").await;

        for _ in 0..3 {
            link_file_to_project("/a/f.rs", "sess", &store, PROMOTION_THRESHOLD_DEFAULT)
                .await
                .unwrap();
        }

        assert!(store.is_file_linked("/a/f.rs", project.id).await.unwrap());
        assert_eq!(store.get_project_files(project.id).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn last_accessed_updated() {
        let (_graph, store, _tmp) = setup().await;

        let project = Project::new_inferred("proj".into(), "/a".into(), 90);
        assert!(project.last_accessed.is_none());
        store.create(&project).await.unwrap();

        // link_file_to_project with a file outside the project still calls
        // find_by_path_prefix which returns None, so last_accessed stays None.
        // We need a file INSIDE the project, but the File node must exist too.
        // Just call touch directly to verify it works.
        store.touch(project.id).await.unwrap();

        let p = store.get_by_id(project.id).await.unwrap().unwrap();
        assert!(p.last_accessed.is_some());
    }

    #[tokio::test]
    async fn promotion_threshold() {
        let (graph, store, _tmp) = setup().await;

        let project = Project::new_inferred("proj".into(), "/home/user/proj".into(), 90);
        assert!(!project.promoted);
        store.create(&project).await.unwrap();

        let session = "test-session";

        // Open files 0..2 -> should NOT promote yet.
        for i in 0..2 {
            let path = format!("/home/user/proj/f{i}.rs");
            create_file_with_session(
                &graph, &store, &path, "editor", session, project.id,
            )
            .await;
        }
        let p = store.get_by_id(project.id).await.unwrap().unwrap();
        assert!(!p.promoted, "should not promote with 2 files");

        // File 3 -> should promote.
        let path = "/home/user/proj/f2.rs";
        create_file_with_session(&graph, &store, path, "editor", session, project.id)
            .await;

        // Now call link_file_to_project to trigger the threshold check.
        link_file_to_project(path, session, &store, PROMOTION_THRESHOLD_DEFAULT)
            .await
            .unwrap();

        let p = store.get_by_id(project.id).await.unwrap().unwrap();
        assert!(p.promoted, "should promote with 3 files");
    }
}

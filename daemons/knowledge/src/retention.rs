use crate::graph::GraphHandle;
use crate::promotion;
use crate::utils::escape_cypher;
use anyhow::Result;
use sqlx::SqlitePool;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::time;
use tracing::{debug, error, info, warn};

/// Raw SQLite events older than this are deleted (if already promoted).
const RAW_EVENT_TTL: Duration = Duration::from_secs(30 * 24 * 3600);

/// Semantic Ladybug nodes older than this are compacted into summaries.
const SEMANTIC_NODE_TTL: Duration = Duration::from_secs(365 * 24 * 3600);

/// Return the current time as microseconds since Unix epoch.
fn now_micros() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as i64
}

/// Run the retention daemon. Wakes once per day at approximately 03:00
/// and performs cleanup of old data across both stores.
pub async fn run(pool: SqlitePool, graph: GraphHandle) -> Result<()> {
    // Wait until the first 03:00 boundary, then run every 24 hours.
    // For simplicity we use a fixed 24h interval starting after a short
    // initial delay. A precise wall-clock scheduler is Phase 4 work.
    let initial_delay = Duration::from_secs(60);
    time::sleep(initial_delay).await;

    let mut interval = time::interval(Duration::from_secs(24 * 3600));
    loop {
        interval.tick().await;
        info!("retention pass starting");

        if let Err(e) = purge_raw_events(&pool).await {
            error!("retention: purge_raw_events failed: {e}");
        }

        if let Err(e) = compact_semantic_nodes(&graph).await {
            error!("retention: compact_semantic_nodes failed: {e}");
        }

        if let Err(e) = collapse_grant_history(&graph).await {
            error!("retention: collapse_grant_history failed: {e}");
        }

        info!("retention pass complete");
    }
}

/// Which superseded grants in one app's history are droppable: everything that is
/// neither the first nor the last of a run of consecutive identical ceilings.
///
/// `rows` is one app's superseded Grant nodes as `(id, declared_ceiling)`, oldest
/// first. Keeping the ends of each run preserves "this app held this ceiling from
/// T1 to T2", which is the question anyone actually asks of the record; the
/// copies between the ends say nothing the ends do not.
///
/// This bounds the projection by SEMANTIC CHANGE rather than by time or count. A
/// time rule loses old boundaries, a count cap discards the oldest ones first, and
/// those boundaries are the part worth keeping. Per-exercise frequency is not lost
/// with them: that lives on `CapabilityUse`.
fn collapsible_grants(rows: &[(String, String)]) -> Vec<String> {
    let mut drop = Vec::new();
    let mut i = 0;
    while i < rows.len() {
        let mut j = i;
        while j + 1 < rows.len() && rows[j + 1].1 == rows[i].1 {
            j += 1;
        }
        // A run of three or more has a middle; one or two are already ends.
        if j > i + 1 {
            drop.extend(rows[i + 1..j].iter().map(|(id, _)| id.clone()));
        }
        i = j + 1;
    }
    drop
}

/// Tier 3: collapse each run of consecutive same-ceiling superseded grants to its
/// first and last.
///
/// Only `superseded` nodes are touched. A live grant is current state and a
/// `revoked` one is the record of something the user did, so neither is a
/// candidate however many of them there are.
async fn collapse_grant_history(graph: &GraphHandle) -> Result<()> {
    let rows = graph
        .query_rows(
            "MATCH (g:Grant) WHERE g.superseded = true AND g.revoked = false \
             RETURN g.app_id, g.id, g.declared_ceiling, g.issued_at \
             ORDER BY g.app_id, g.issued_at"
                .to_string(),
        )
        .await?;

    let mut per_app: std::collections::BTreeMap<String, Vec<(String, String)>> =
        std::collections::BTreeMap::new();
    for r in &rows.rows {
        if r.len() < 3 {
            continue;
        }
        let (app, id, ceiling) = (r[0].as_str(), r[1].as_str(), r[2].as_str());
        // `as_str` yields "" for a non-string cell, so an id or ceiling that did
        // not come back as text would otherwise join the wrong run - and a wrong
        // run means deleting the wrong node. Skip instead of guessing.
        if app.is_empty() || id.is_empty() || ceiling.is_empty() {
            continue;
        }
        per_app
            .entry(app.to_string())
            .or_default()
            .push((id.to_string(), ceiling.to_string()));
    }

    let mut dropped = 0usize;
    for (app, grants) in &per_app {
        for id in collapsible_grants(grants) {
            let q = format!(
                "MATCH (g:Grant {{id: '{}'}}) WHERE g.superseded = true AND g.revoked = false \
                 DETACH DELETE g",
                escape_cypher(&id)
            );
            // The WHERE is repeated at the delete rather than trusted from the read:
            // the two are separate statements on a live graph, and a grant that was
            // revoked in between must not be removed by a plan made before that.
            if let Err(e) = graph.write(q).await {
                warn!(app = %app, "retention: collapsing a superseded grant failed: {e}");
                continue;
            }
            dropped += 1;
        }
    }
    if dropped > 0 {
        info!("retention: collapsed {dropped} superseded grant(s)");
    }
    Ok(())
}

/// Tier 1: Delete raw SQLite events older than 30 days that have already
/// been promoted (timestamp < HWM).
async fn purge_raw_events(pool: &SqlitePool) -> Result<()> {
    let hwm = promotion::read_hwm(pool).await?;
    let age_cutoff = now_micros() - RAW_EVENT_TTL.as_micros() as i64;
    // Only delete events that are both old enough AND already promoted.
    let safe_cutoff = age_cutoff.min(hwm);

    if safe_cutoff <= 0 {
        debug!("retention: no raw events eligible for purge");
        return Ok(());
    }

    let result = sqlx::query("DELETE FROM events WHERE timestamp < ?")
        .bind(safe_cutoff)
        .execute(pool)
        .await?;

    let deleted = result.rows_affected();
    if deleted > 0 {
        info!(deleted, cutoff = safe_cutoff, "retention: purged raw events");
    } else {
        debug!("retention: no raw events to purge");
    }

    Ok(())
}

/// Tier 2: Compact Ladybug File nodes older than 12 months into Summary
/// nodes grouped by app. Pinned nodes are skipped.
async fn compact_semantic_nodes(graph: &GraphHandle) -> Result<()> {
    let cutoff = now_micros() - SEMANTIC_NODE_TTL.as_micros() as i64;

    // Find distinct apps that have old, non-pinned File nodes.
    let apps_result = graph
        .query_rows(format!(
            "MATCH (f:File)-[:ACCESSED_BY]->(a:App)
             WHERE f.last_accessed < {cutoff}
             AND NOT EXISTS {{
                 MATCH (p:PinnedMarker) WHERE p.node_id = f.id AND p.node_type = 'File'
             }}
             RETURN DISTINCT a.id AS id"
        ))
        .await?;

    let app_ids: Vec<String> = apps_result
        .rows
        .iter()
        .filter_map(|r| r.first().map(|c| c.as_str().to_string()))
        .filter(|s| !s.is_empty())
        .collect();
    if app_ids.is_empty() {
        debug!("retention: no apps with old nodes to compact");
        return Ok(());
    }

    for app_id in &app_ids {
        if let Err(e) = compact_app_files(graph, app_id, cutoff).await {
            warn!(app_id, "retention: compaction failed for app: {e}");
            // Continue with other apps.
        }
    }

    Ok(())
}

/// Compact all old File nodes for a single app into a Summary node.
///
/// Safety: the Summary node is created before deleting the originals.
/// If deletion fails, the next pass will find the Summary already exists
/// and skip re-creation; the originals will be retried.
async fn compact_app_files(graph: &GraphHandle, app_id: &str, cutoff: i64) -> Result<()> {
    let app_esc = escape_cypher(app_id);

    // Aggregate old non-pinned File nodes for this app.
    let agg_result = graph
        .query_rows(format!(
            "MATCH (f:File)-[:ACCESSED_BY]->(a:App {{id: '{app_esc}'}})
             WHERE f.last_accessed < {cutoff}
             AND NOT EXISTS {{
                 MATCH (p:PinnedMarker) WHERE p.node_id = f.id AND p.node_type = 'File'
             }}
             RETURN count(f) AS cnt, min(f.last_accessed) AS lo, max(f.last_accessed) AS hi"
        ))
        .await?;

    let (count, period_start, period_end) = agg_result
        .rows
        .first()
        .filter(|r| r.len() >= 3)
        .map(|r| (r[0].as_i64(), r[1].as_i64(), r[2].as_i64()))
        .unwrap_or((0, 0, 0));
    if count == 0 {
        return Ok(());
    }

    let summary_id = format!("summary:{app_id}:{cutoff}");
    let summary_id_esc = escape_cypher(&summary_id);

    // Step 1: Create the Summary node (idempotent via MERGE).
    graph
        .write(format!(
            "MERGE (s:Summary {{id: '{summary_id_esc}'}})
             SET s.type = 'file_access',
                 s.app_id = '{app_esc}',
                 s.access_count = {count},
                 s.primary_application = '{app_esc}',
                 s.active_period_start = {period_start},
                 s.active_period_end = {period_end}"
        ))
        .await?;

    // Step 2: Create the SUMMARIZES edge.
    graph
        .write(format!(
            "MATCH (s:Summary {{id: '{summary_id_esc}'}}), (a:App {{id: '{app_esc}'}})
             MERGE (s)-[:SUMMARIZES]->(a)"
        ))
        .await?;

    info!(
        app_id,
        count, period_start, period_end, "retention: created summary node"
    );

    // Step 3: Delete the original File nodes (and their edges).
    graph
        .write(format!(
            "MATCH (f:File)-[:ACCESSED_BY]->(a:App {{id: '{app_esc}'}})
             WHERE f.last_accessed < {cutoff}
             AND NOT EXISTS {{
                 MATCH (p:PinnedMarker) WHERE p.node_id = f.id AND p.node_type = 'File'
             }}
             DETACH DELETE f"
        ))
        .await?;

    info!(app_id, count, "retention: deleted compacted file nodes");
    Ok(())
}


#[cfg(test)]
mod tests {
    use super::*;

    fn g(id: &str, ceiling: &str) -> (String, String) {
        (id.to_string(), ceiling.to_string())
    }

    #[test]
    fn a_run_keeps_its_first_and_last_and_drops_the_middle() {
        // Five mints of one unchanged ceiling: the record worth keeping is that the
        // app held it from the first to the fifth, which the two ends carry.
        let rows = [g("1", "A"), g("2", "A"), g("3", "A"), g("4", "A"), g("5", "A")];
        assert_eq!(collapsible_grants(&rows), vec!["2", "3", "4"]);
    }

    #[test]
    fn a_boundary_between_ceilings_is_never_dropped() {
        // The change from A to B is the whole point of the record. Both grants
        // either side of it are ends of their runs, so nothing goes.
        let rows = [g("1", "A"), g("2", "A"), g("3", "B"), g("4", "B")];
        assert!(collapsible_grants(&rows).is_empty());
    }

    #[test]
    fn alternating_ceilings_are_all_boundaries() {
        let rows = [g("1", "A"), g("2", "B"), g("3", "A"), g("4", "B")];
        assert!(collapsible_grants(&rows).is_empty());
    }

    #[test]
    fn short_runs_and_empty_input_leave_everything_alone() {
        assert!(collapsible_grants(&[]).is_empty());
        assert!(collapsible_grants(&[g("1", "A")]).is_empty());
        assert!(collapsible_grants(&[g("1", "A"), g("2", "A")]).is_empty());
    }

    #[test]
    fn several_runs_each_keep_their_own_ends() {
        let rows = [
            g("1", "A"), g("2", "A"), g("3", "A"),
            g("4", "B"), g("5", "B"), g("6", "B"), g("7", "B"),
            g("8", "A"),
        ];
        // 2 from the first run, 5 and 6 from the second; the lone trailing A stays.
        assert_eq!(collapsible_grants(&rows), vec!["2", "5", "6"]);
    }
    use tempfile::TempDir;

    async fn setup() -> (GraphHandle, TempDir) {
        let tmp = TempDir::new().unwrap();
        let graph =
            crate::graph::spawn(tmp.path().join("graph").to_str().unwrap()).unwrap();
        time::sleep(Duration::from_millis(500)).await;
        (graph, tmp)
    }

    #[tokio::test]
    async fn collapse_grant_history_drops_only_the_middle_of_a_run() {
        // The pure grouping is unit-tested above; this is the half that can be
        // wrong in a way no unit test sees. The pass reads with
        // `g.revoked = false`, so a Grant whose `revoked` came back NULL rather
        // than false would match nothing and the whole tier would be silently
        // inert - a retention rule that quietly retains everything.
        let (graph, _tmp) = setup().await;

        for (i, id) in ["a1", "a2", "a3"].iter().enumerate() {
            graph
                .write(format!(
                    "CREATE (:Grant {{id: '{id}', app_id: 'com.a', pid: 1, \
                     issued_at: {}, declared_ceiling: 'A', required: false, \
                     identity_verified: false, live: false, revoked: false, \
                     superseded: true, use_count: 0}})",
                    100 + i as i64
                ))
                .await
                .unwrap();
        }
        // A live grant of the same ceiling, which is current state and must stay.
        graph
            .write(
                "CREATE (:Grant {id: 'live', app_id: 'com.a', pid: 1, issued_at: 400, \
                 declared_ceiling: 'A', required: false, identity_verified: false, \
                 live: true, revoked: false, superseded: false, use_count: 0})"
                    .to_string(),
            )
            .await
            .unwrap();

        collapse_grant_history(&graph).await.unwrap();

        let left = graph
            .query_rows("MATCH (g:Grant) RETURN g.id ORDER BY g.id".to_string())
            .await
            .unwrap();
        let ids: Vec<&str> = left.rows.iter().map(|r| r[0].as_str()).collect();
        assert_eq!(
            ids,
            vec!["a1", "a3", "live"],
            "the ends of the run and the live grant survive, the middle goes"
        );
    }

    #[tokio::test]
    async fn compact_app_files_summarises_old_files_and_deletes_them() {
        // Coverage for the Tier-2 compaction (a core graph-write path): an old
        // File accessed by an app is folded into a Summary node carrying its
        // schema columns, then the original File is deleted. This guards the
        // Summary write against a future schema/SET drift (the undefined-column
        // class of bug that stalled window.focused).
        let (graph, _tmp) = setup().await;
        graph
            .write("CREATE (a:App {id: 'com.example.editor', name: 'Editor'})".into())
            .await
            .unwrap();
        graph
            .write(
                "CREATE (f:File {id: '/old/notes.md', path: '/old/notes.md', last_accessed: 1000})"
                    .into(),
            )
            .await
            .unwrap();
        graph
            .write(
                "MATCH (f:File {id: '/old/notes.md'}), (a:App {id: 'com.example.editor'}) \
                 MERGE (f)-[:ACCESSED_BY]->(a)"
                    .into(),
            )
            .await
            .unwrap();

        // cutoff well after the file's last_accessed, so it is "old".
        compact_app_files(&graph, "com.example.editor", 10_000)
            .await
            .expect("compaction writes the Summary without error");

        let summ = graph
            .query_rows(
                "MATCH (s:Summary) RETURN s.type AS t, s.app_id AS a, s.access_count AS c".into(),
            )
            .await
            .unwrap();
        let row = summ.rows.first().expect("a Summary node was created");
        assert_eq!(row[0].as_str(), "file_access");
        assert_eq!(row[1].as_str(), "com.example.editor");
        assert_eq!(row[2].as_i64(), 1);

        // The SUMMARIZES edge to the app exists.
        let edge = graph
            .query_rows(
                "MATCH (:Summary)-[:SUMMARIZES]->(:App {id: 'com.example.editor'}) \
                 RETURN count(*) AS c"
                    .into(),
            )
            .await
            .unwrap();
        assert_eq!(edge.rows[0][0].as_i64(), 1, "the summary points at the app");

        // The compacted File node is deleted.
        let files = graph
            .query_rows("MATCH (f:File {id: '/old/notes.md'}) RETURN count(*) AS c".into())
            .await
            .unwrap();
        assert_eq!(files.rows[0][0].as_i64(), 0, "the old file is compacted away");
    }
}

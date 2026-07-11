//! Live working set (agent-work-surfaces-plan.md surface 2, the briefing engine's
//! coder half).
//!
//! The briefing surface answers "what's live right now, what needs you". Its
//! coder foundation is a bounded, graph-wide aggregation of the user's most
//! recently-active entities, scored by [`crate::entity_precision`] so the digest
//! leads with what is live-and-important and drops the stale noise (the same
//! ranking the entity-scoped prep surface uses, applied globally instead of around
//! one subject). The "what needs you" curation on top (open action-items, the
//! since-you-were-away delta) is the collaborative surface design; this is the
//! ranked live set it sits on.
//!
//! Bounded by construction: it reads only the top-N most recently-touched File and
//! Project nodes (an indexed `ORDER BY ... DESC LIMIT`), never a full graph scan,
//! then scores and ranks them in memory.

use anyhow::Result;

use crate::entity_precision::LivenessSignals;
use crate::graph::{CellValue, GraphHandle};
use crate::prep::{rank_prep, PrepCandidate, PrepItem};

/// How many recent nodes to pull per type before scoring. The final view is capped
/// separately; this bounds the graph read.
const GATHER_PER_TYPE: usize = 200;

/// A non-zero int cell as `Some(v)`, else `None` (a `0` / absent stamp is unset).
fn opt_time(cell: Option<&CellValue>) -> Option<i64> {
    cell.map(CellValue::as_i64).filter(|v| *v != 0)
}

/// The most recently-accessed files as prep candidates (id, basename, signals). A
/// bounded, ordered read: `ORDER BY last_accessed DESC LIMIT N`.
async fn recent_files(graph: &GraphHandle) -> Result<Vec<PrepCandidate>> {
    let cypher = format!(
        "MATCH (f:File) WHERE f.last_accessed IS NOT NULL \
         RETURN f.id AS id, f.path AS path, f.last_accessed AS la \
         ORDER BY f.last_accessed DESC LIMIT {GATHER_PER_TYPE}"
    );
    let rs = graph.query_rows(cypher).await?;
    Ok(rs
        .rows
        .iter()
        .filter_map(|row| {
            let id = row.first()?.as_str().to_string();
            if id.is_empty() {
                return None;
            }
            let path = row.get(1).map(|c| c.as_str().to_string()).unwrap_or_default();
            let la = opt_time(row.get(2));
            let label = path
                .rsplit('/')
                .next()
                .filter(|s| !s.is_empty())
                .unwrap_or(&id)
                .to_string();
            Some(PrepCandidate {
                id,
                label,
                kind: "File".to_string(),
                relation: "recent".to_string(),
                signals: LivenessSignals {
                    last_activity_micros: la,
                    created_at_micros: la,
                    activity_count: 1,
                    expired_at_micros: None,
                },
            })
        })
        .collect())
}

/// The most recently-active LIVE projects as prep candidates. Archived projects
/// (`expired_at` set) are excluded at the query.
async fn recent_projects(graph: &GraphHandle) -> Result<Vec<PrepCandidate>> {
    let cypher = format!(
        "MATCH (p:Project) WHERE p.expired_at IS NULL \
         RETURN p.id AS id, p.name AS name, p.last_accessed AS la, p.created_at AS ca \
         ORDER BY coalesce(p.last_accessed, p.created_at) DESC LIMIT {GATHER_PER_TYPE}"
    );
    let rs = graph.query_rows(cypher).await?;
    Ok(rs
        .rows
        .iter()
        .filter_map(|row| {
            let id = row.first()?.as_str().to_string();
            if id.is_empty() {
                return None;
            }
            let name = row.get(1).map(|c| c.as_str().to_string()).unwrap_or_default();
            let la = opt_time(row.get(2));
            let ca = opt_time(row.get(3));
            let label = if name.is_empty() { id.clone() } else { name };
            Some(PrepCandidate {
                id,
                label,
                kind: "Project".to_string(),
                relation: "recent".to_string(),
                signals: LivenessSignals {
                    last_activity_micros: la.or(ca),
                    created_at_micros: ca,
                    activity_count: 1,
                    expired_at_micros: None,
                },
            })
        })
        .collect())
}

/// Gather the live working set: the recent files and projects, scored and ranked by
/// liveness (live-and-important first, stale noise dropped), capped at `max`. This
/// is the briefing digest's ranked foundation. `now_micros` is passed in (the caller
/// supplies the clock), so it is deterministic and testable.
pub(crate) async fn gather_working_set(
    graph: &GraphHandle,
    now_micros: i64,
    max: usize,
) -> Result<Vec<PrepItem>> {
    let mut candidates = recent_files(graph).await?;
    candidates.extend(recent_projects(graph).await?);
    Ok(rank_prep(candidates, now_micros, max))
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: i64 = 86_400 * 1_000_000;
    const NOW: i64 = 1_000_000 * 1_700_000_000;

    #[tokio::test]
    async fn the_working_set_leads_with_the_most_live_entities() {
        let tmp = tempfile::TempDir::new().unwrap();
        let graph = crate::graph::spawn(tmp.path().join("graph").to_str().unwrap()).unwrap();
        let fresh = NOW - 1 * DAY;
        let cold = NOW - 200 * DAY;
        let recent_proj = NOW - 2 * DAY;
        graph
            .write(format!(
                "CREATE (f:File {{id: 'fresh', path: '/p/fresh.rs', app_id: 't', last_accessed: {fresh}}})"
            ))
            .await
            .unwrap();
        graph
            .write(format!(
                "CREATE (f:File {{id: 'cold', path: '/p/cold.rs', app_id: 't', last_accessed: {cold}}})"
            ))
            .await
            .unwrap();
        graph
            .write(format!(
                "CREATE (p:Project {{id: 'proj', name: 'Arlen', last_accessed: {recent_proj}}})"
            ))
            .await
            .unwrap();

        let view = gather_working_set(&graph, NOW, 10).await.unwrap();
        // The fresh file + recent project appear; the long-cold file is noise, dropped.
        let ids: Vec<&str> = view.iter().map(|i| i.id.as_str()).collect();
        assert!(ids.contains(&"fresh"), "the fresh file leads the set");
        assert!(ids.contains(&"proj"), "the recent project is in the set");
        assert!(!ids.contains(&"cold"), "the cold file is dropped as noise");
        // Highest score first.
        assert!(view.windows(2).all(|w| w[0].score >= w[1].score), "ranked by score desc");
    }

    #[tokio::test]
    async fn an_archived_project_is_excluded() {
        let tmp = tempfile::TempDir::new().unwrap();
        let graph = crate::graph::spawn(tmp.path().join("graph").to_str().unwrap()).unwrap();
        let recent = NOW - 1 * DAY;
        graph
            .write(format!(
                "CREATE (p:Project {{id: 'live', name: 'Live', last_accessed: {recent}}})"
            ))
            .await
            .unwrap();
        graph
            .write(format!(
                "CREATE (p:Project {{id: 'archived', name: 'Old', last_accessed: {recent}, expired_at: {recent}}})"
            ))
            .await
            .unwrap();

        let view = gather_working_set(&graph, NOW, 10).await.unwrap();
        let ids: Vec<&str> = view.iter().map(|i| i.id.as_str()).collect();
        assert!(ids.contains(&"live"));
        assert!(!ids.contains(&"archived"), "an archived project is not in the working set");
    }

    #[tokio::test]
    async fn an_empty_graph_yields_an_empty_set() {
        let tmp = tempfile::TempDir::new().unwrap();
        let graph = crate::graph::spawn(tmp.path().join("graph").to_str().unwrap()).unwrap();
        assert!(gather_working_set(&graph, NOW, 10).await.unwrap().is_empty());
    }
}

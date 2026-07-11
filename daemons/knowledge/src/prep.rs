//! Prep-for-this ranking (agent-work-surfaces-plan.md, surface 3).
//!
//! Given an entity, the prep surface pulls every related thread / file / meeting-
//! note / commitment into one prepped view ("prep me for my 2pm with X"). The
//! value is not the gather, it is the RANKING: [`crate::entity_precision`] scores
//! each candidate so the view leads with what is live-and-important and drops the
//! stale-and-noise (the Rowboat "20 unknown entities" graveyard the plan names as
//! the failure mode). This is the pure ranking core over already-gathered
//! candidates; the graph gather and the socket op wire it. Pure + deterministic,
//! so it is fully unit-tested.

use anyhow::Result;
use serde::Serialize;

use crate::entity_precision::{classify, liveness_score, LivenessSignals};
use crate::graph::{CellValue, GraphHandle};
use crate::utils::escape_cypher;

/// A candidate related entity gathered from the graph, carrying the temporal
/// signals its liveness is scored from and how it relates to the prep subject.
#[derive(Debug, Clone)]
pub struct PrepCandidate {
    /// The related entity's node id.
    pub id: String,
    /// A human label (path basename / project name / meeting title).
    pub label: String,
    /// The KG node type (File / Project / Meeting / `ActionItem` / ...).
    pub kind: String,
    /// How it relates to the subject (the edge type or a rendered phrase).
    pub relation: String,
    /// The temporal signals driving its liveness.
    pub signals: LivenessSignals,
}

/// A ranked prep item: the candidate plus its liveness verdict and score. Serialized
/// to the prep surface.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct PrepItem {
    /// The related entity's node id.
    pub id: String,
    /// The human label.
    pub label: String,
    /// The KG node type.
    pub kind: String,
    /// How it relates to the subject.
    pub relation: String,
    /// The liveness bucket (`live` / `dormant` / `stale`), for the surface to group on.
    pub liveness: &'static str,
    /// The liveness score in `[0, 1]`, the sort key.
    pub score: f64,
}

/// The minimum liveness score for a candidate to make the prep view: below this it
/// is noise and is dropped. Retired candidates (score 0) are always below it.
pub const PREP_NOISE_FLOOR: f64 = 0.05;

/// Rank and filter gathered candidates into the prep view: drop anything below the
/// noise floor (retired / stale-and-cold), score each, sort by score descending
/// with a stable id tiebreak, and cap at `max`. Deterministic for a given
/// `(candidates, now, max)`.
pub fn rank_prep(candidates: Vec<PrepCandidate>, now_micros: i64, max: usize) -> Vec<PrepItem> {
    let mut items: Vec<PrepItem> = candidates
        .into_iter()
        .filter_map(|c| {
            let score = liveness_score(&c.signals, now_micros);
            if score < PREP_NOISE_FLOOR {
                return None; // noise: below the floor, kept out of the prep view
            }
            Some(PrepItem {
                id: c.id,
                label: c.label,
                kind: c.kind,
                relation: c.relation,
                liveness: classify(&c.signals, now_micros).as_str(),
                score,
            })
        })
        .collect();
    // Highest score first; a stable id tiebreak keeps the order deterministic when
    // two candidates score equally.
    items.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.id.cmp(&b.id))
    });
    items.truncate(max);
    items
}

/// A non-zero int cell as `Some(v)`, else `None` (a `0` / absent stamp is "unset",
/// never a real epoch-micros time).
fn opt_time(cell: Option<&CellValue>) -> Option<i64> {
    cell.map(CellValue::as_i64).filter(|v| *v != 0)
}

/// Load a neighbour's kind, human label and liveness signals. The `FILE_PART_OF`
/// membership edge only connects `File` and `Project`, so this tries those two;
/// `None` if the id is neither (unexpected, since the gather already filtered to
/// live membership neighbours). Broader entity types (Meeting, `ActionItem`) extend
/// this as their relation families are gathered.
async fn load_prep_node(
    graph: &GraphHandle,
    id: &str,
) -> Result<Option<(String, String, LivenessSignals)>> {
    let id_esc = escape_cypher(id);

    // File: label = the path basename; recency = last_accessed.
    let cypher = format!(
        "MATCH (n:File {{id: '{id_esc}'}}) RETURN n.path AS path, n.last_accessed AS la LIMIT 1"
    );
    let rs = graph.query_rows(cypher).await?;
    if let Some(row) = rs.rows.first() {
        let path = row.first().map(|c| c.as_str().to_string()).unwrap_or_default();
        let la = opt_time(row.get(1));
        let label = path.rsplit('/').next().filter(|s| !s.is_empty()).unwrap_or(id);
        return Ok(Some((
            "File".to_string(),
            label.to_string(),
            LivenessSignals {
                last_activity_micros: la,
                created_at_micros: la,
                activity_count: 1,
                expired_at_micros: None,
            },
        )));
    }

    // Project: label = the name; recency = last_accessed or created_at; expired_at
    // carries the archive state so a retired project is dropped by the ranking.
    let cypher = format!(
        "MATCH (n:Project {{id: '{id_esc}'}}) \
         RETURN n.name AS name, n.last_accessed AS la, n.created_at AS ca, n.expired_at AS ea LIMIT 1"
    );
    let rs = graph.query_rows(cypher).await?;
    if let Some(row) = rs.rows.first() {
        let name = row.first().map(|c| c.as_str().to_string()).unwrap_or_default();
        let la = opt_time(row.get(1));
        let ca = opt_time(row.get(2));
        let ea = opt_time(row.get(3));
        let label = if name.is_empty() { id.to_string() } else { name };
        return Ok(Some((
            "Project".to_string(),
            label,
            LivenessSignals {
                last_activity_micros: la.or(ca),
                created_at_micros: ca,
                activity_count: 1,
                expired_at_micros: ea,
            },
        )));
    }

    Ok(None)
}

/// The subject entity's live `FILE_PART_OF` neighbour ids, both directions (a
/// project's files / a file's project). The same live-edge predicate the capsule
/// materializer uses (`invalid_at`/`expired_at` NULL on the edge and the far node),
/// so no new graph behaviour is introduced.
async fn prep_neighbour_ids(graph: &GraphHandle, subject_id: &str) -> Result<Vec<String>> {
    let id = escape_cypher(subject_id);
    let cypher = format!(
        "MATCH (n {{id: '{id}'}})-[r:FILE_PART_OF]-(m) \
         WHERE r.invalid_at IS NULL AND r.expired_at IS NULL AND m.expired_at IS NULL \
         RETURN m.id AS id"
    );
    let rs = graph.query_rows(cypher).await?;
    let mut ids: Vec<String> = rs
        .rows
        .iter()
        .filter_map(|row| row.first())
        .map(|c| c.as_str().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    ids.sort();
    ids.dedup();
    Ok(ids)
}

/// Gather the subject entity's live `FILE_PART_OF` neighbours as prep candidates,
/// each with its liveness signals loaded. `FILE_PART_OF` is the built membership
/// edge (a project's files / a file's project). Broader relation families
/// (`ACCESSED_BY`, meetings, action-items) extend this as they land.
pub(crate) async fn gather_prep_candidates(
    graph: &GraphHandle,
    subject_id: &str,
) -> Result<Vec<PrepCandidate>> {
    let neighbour_ids = prep_neighbour_ids(graph, subject_id).await?;
    let mut candidates = Vec::new();
    for id in neighbour_ids {
        if let Some((kind, label, signals)) = load_prep_node(graph, &id).await? {
            candidates.push(PrepCandidate {
                id,
                label,
                kind,
                relation: "FILE_PART_OF".to_string(),
                signals,
            });
        }
    }
    Ok(candidates)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: i64 = 86_400 * 1_000_000;
    const NOW: i64 = 1_000_000 * 1_700_000_000;

    fn candidate(id: &str, kind: &str, days_ago: i64, count: u64, expired: bool) -> PrepCandidate {
        PrepCandidate {
            id: id.to_string(),
            label: format!("label-{id}"),
            kind: kind.to_string(),
            relation: "RELATED_TO".to_string(),
            signals: LivenessSignals {
                last_activity_micros: Some(NOW - days_ago * DAY),
                created_at_micros: Some(NOW - days_ago * DAY),
                activity_count: count,
                expired_at_micros: if expired { Some(NOW - DAY) } else { None },
            },
        }
    }

    #[test]
    fn live_candidates_lead_and_noise_is_dropped() {
        let cands = vec![
            candidate("a-old", "File", 200, 1, false),  // stale/cold -> below floor
            candidate("b-fresh", "File", 1, 10, false), // live -> top
            candidate("c-warm", "Project", 15, 5, false), // dormant -> middle
        ];
        let out = rank_prep(cands, NOW, 10);
        // The fresh one leads; the cold one is dropped as noise.
        assert_eq!(out[0].id, "b-fresh");
        assert!(out.iter().all(|i| i.id != "a-old"), "cold noise must be dropped");
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn a_retired_candidate_never_appears() {
        let cands = vec![
            candidate("live", "File", 0, 20, false),
            candidate("archived", "Project", 0, 99, true), // recent but retired
        ];
        let out = rank_prep(cands, NOW, 10);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].id, "live");
    }

    #[test]
    fn the_view_is_capped() {
        let cands: Vec<PrepCandidate> = (0..20)
            .map(|i| candidate(&format!("f{i:02}"), "File", 1, 10, false))
            .collect();
        let out = rank_prep(cands, NOW, 5);
        assert_eq!(out.len(), 5, "the prep view is capped");
    }

    #[test]
    fn ties_break_deterministically_by_id() {
        // Two identical-recency candidates must come back in a stable id order.
        let cands = vec![
            candidate("zzz", "File", 1, 10, false),
            candidate("aaa", "File", 1, 10, false),
        ];
        let out = rank_prep(cands, NOW, 10);
        assert_eq!(out[0].id, "aaa");
        assert_eq!(out[1].id, "zzz");
    }

    #[test]
    fn liveness_bucket_is_carried_for_grouping() {
        let out = rank_prep(vec![candidate("x", "File", 1, 10, false)], NOW, 10);
        assert_eq!(out[0].liveness, "live");
    }

    #[test]
    fn an_empty_gather_yields_an_empty_view() {
        assert!(rank_prep(vec![], NOW, 10).is_empty());
    }

    #[tokio::test]
    async fn gather_ranks_a_projects_live_files_over_its_cold_ones() {
        // Prep for a project: its live files lead, a long-cold file drops as noise.
        let tmp = tempfile::TempDir::new().unwrap();
        let graph = crate::graph::spawn(tmp.path().join("graph").to_str().unwrap()).unwrap();
        graph.write("CREATE (p:Project {id: 'p1', name: 'Arlen'})".into()).await.unwrap();
        let fresh = NOW - 1 * DAY;
        let cold = NOW - 200 * DAY;
        graph
            .write(format!(
                "CREATE (f:File {{id: 'fresh', path: '/proj/fresh.rs', app_id: 't', last_accessed: {fresh}}})"
            ))
            .await
            .unwrap();
        graph
            .write(format!(
                "CREATE (f:File {{id: 'cold', path: '/proj/cold.rs', app_id: 't', last_accessed: {cold}}})"
            ))
            .await
            .unwrap();
        for f in ["fresh", "cold"] {
            graph
                .write(format!(
                    "MATCH (f:File {{id:'{f}'}}), (p:Project {{id:'p1'}}) CREATE (f)-[:FILE_PART_OF]->(p)"
                ))
                .await
                .unwrap();
        }

        let candidates = gather_prep_candidates(&graph, "p1").await.unwrap();
        assert_eq!(candidates.len(), 2, "both files are gathered as candidates");
        let view = rank_prep(candidates, NOW, 10);
        // The fresh file leads and reads as a File with its basename; the cold one
        // is dropped as noise (200 days -> below the floor).
        assert_eq!(view[0].id, "fresh");
        assert_eq!(view[0].kind, "File");
        assert_eq!(view[0].label, "fresh.rs");
        assert_eq!(view[0].liveness, "live");
        assert!(view.iter().all(|i| i.id != "cold"), "the cold file is noise, dropped");
    }

    #[tokio::test]
    async fn gather_from_a_file_returns_its_project() {
        // Prep for a file surfaces the project it belongs to (the reverse direction).
        let tmp = tempfile::TempDir::new().unwrap();
        let graph = crate::graph::spawn(tmp.path().join("graph").to_str().unwrap()).unwrap();
        let recent = NOW - 2 * DAY;
        graph
            .write(format!("CREATE (p:Project {{id: 'p1', name: 'Arlen', last_accessed: {recent}}})"))
            .await
            .unwrap();
        graph
            .write("CREATE (f:File {id: 'f1', path: '/x', app_id: 't', last_accessed: 0})".into())
            .await
            .unwrap();
        graph
            .write("MATCH (f:File {id:'f1'}), (p:Project {id:'p1'}) CREATE (f)-[:FILE_PART_OF]->(p)".into())
            .await
            .unwrap();

        let view = rank_prep(gather_prep_candidates(&graph, "f1").await.unwrap(), NOW, 10);
        assert_eq!(view.len(), 1);
        assert_eq!(view[0].id, "p1");
        assert_eq!(view[0].kind, "Project");
        assert_eq!(view[0].label, "Arlen");
    }
}

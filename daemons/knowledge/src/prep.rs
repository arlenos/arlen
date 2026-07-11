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

use serde::Serialize;

use crate::entity_precision::{classify, liveness_score, LivenessSignals};

/// A candidate related entity gathered from the graph, carrying the temporal
/// signals its liveness is scored from and how it relates to the prep subject.
#[derive(Debug, Clone)]
pub struct PrepCandidate {
    /// The related entity's node id.
    pub id: String,
    /// A human label (path basename / project name / meeting title).
    pub label: String,
    /// The KG node type (File / Project / Meeting / ActionItem / ...).
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
}

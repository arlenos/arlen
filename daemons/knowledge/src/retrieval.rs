//! LLM-free retrieval fusion (bitemporal-knowledge-graph.md §7).
//!
//! The agent finds the relevant slice without an LLM call at query time by
//! fusing ranked id-lists from up to three primitives: BM25 keyword (SQLite
//! FTS5), graph traversal (Kuzu via the typed DSL), and an optional semantic
//! vector search. The three are combined by **Reciprocal Rank Fusion** (§7.3),
//! which is positional, so the primitives' incomparable raw scores never need to
//! be normalised. The default path is the two-primitive subset (keyword + graph)
//! since embeddings are config-gated off; retrieval fails open to LLM-free-anyway
//! (never a generative call) when a model is absent.
//!
//! This module is the pure fusion core. The FTS5 `fact_text` index, the
//! deterministic per-label text synthesis, the optional `fact_vec` table, and
//! the post-fusion `VALID_AS_OF` validity confirm against the bi-temporal graph
//! are separate increments built on this.

use std::collections::{HashMap, HashSet};

/// The standard RRF damping constant (§7.3): a larger `k` flattens the
/// contribution of rank, so top ranks dominate less.
pub const K_RRF: u32 = 60;

/// Fuse several ranked id-lists into one ranking by Reciprocal Rank Fusion.
///
/// Each input list is in rank order (best first). An id's contribution from a
/// list is `1 / (k + rank)` with `rank` 1-based, summed across every list that
/// contains it, so an id surfaced by more primitives, or ranked higher, scores
/// higher. A repeated id within one list counts only its best (first) rank, so a
/// primitive that emits a duplicate cannot inflate a candidate. Returns
/// `(id, score)` sorted by score descending, ties broken by id ascending for a
/// deterministic order.
pub fn rrf_fuse(lists: &[Vec<String>], k: u32) -> Vec<(String, f64)> {
    let mut scores: HashMap<String, f64> = HashMap::new();
    for list in lists {
        let mut seen: HashSet<&str> = HashSet::new();
        for (rank0, id) in list.iter().enumerate() {
            // Only the best rank of an id within a single list contributes.
            if !seen.insert(id.as_str()) {
                continue;
            }
            let rank = rank0 as u32 + 1;
            *scores.entry(id.clone()).or_insert(0.0) += 1.0 / f64::from(k + rank);
        }
    }
    let mut fused: Vec<(String, f64)> = scores.into_iter().collect();
    fused.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });
    fused
}

/// Fuse and return only the ids, best first (the common case where the caller
/// wants the ranking, not the scores).
pub fn rrf_rank(lists: &[Vec<String>], k: u32) -> Vec<String> {
    rrf_fuse(lists, k).into_iter().map(|(id, _)| id).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_single_list_keeps_its_order() {
        let fused = rrf_rank(&[ids(&["a", "b", "c"])], K_RRF);
        assert_eq!(fused, ids(&["a", "b", "c"]));
    }

    #[test]
    fn an_id_in_more_lists_outranks_one_in_fewer() {
        // `b` is mid-rank in both lists; `a` is top of only the first. Appearing
        // in both should let `b` overtake a single-list top.
        let l1 = ids(&["a", "b"]);
        let l2 = ids(&["c", "b"]);
        let fused = rrf_rank(&[l1, l2], K_RRF);
        assert_eq!(fused[0], "b", "the id surfaced by both primitives wins");
    }

    #[test]
    fn higher_rank_in_both_lists_wins() {
        let l1 = ids(&["x", "y"]);
        let l2 = ids(&["x", "y"]);
        let fused = rrf_fuse(&[l1, l2], K_RRF);
        assert_eq!(fused[0].0, "x");
        assert!(fused[0].1 > fused[1].1, "x scores strictly higher than y");
    }

    #[test]
    fn a_duplicate_within_a_list_uses_only_its_best_rank() {
        // `a` repeated in one list must not be counted twice.
        let with_dup = rrf_fuse(&[ids(&["a", "a", "b"])], K_RRF);
        let no_dup = rrf_fuse(&[ids(&["a", "b"])], K_RRF);
        let a_dup = with_dup.iter().find(|(id, _)| id == "a").unwrap().1;
        let a_no = no_dup.iter().find(|(id, _)| id == "a").unwrap().1;
        assert!((a_dup - a_no).abs() < 1e-12, "a duplicate id does not inflate the score");
    }

    #[test]
    fn ties_break_by_id_for_determinism() {
        // Same rank in symmetric lists -> equal scores -> deterministic id order.
        let fused = rrf_rank(&[ids(&["b"]), ids(&["a"])], K_RRF);
        assert_eq!(fused, ids(&["a", "b"]), "equal scores order by id ascending");
    }

    #[test]
    fn empty_input_is_empty() {
        assert!(rrf_rank(&[], K_RRF).is_empty());
        assert!(rrf_rank(&[vec![]], K_RRF).is_empty());
    }
}

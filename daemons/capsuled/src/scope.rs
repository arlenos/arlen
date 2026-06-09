//! Instance-set scope selection (context-capsule.md §4).
//!
//! The canonical capsule ("share this project", "share my travel-2026") is a
//! hand-picked node set, which has no representation in the existing scope model
//! (`InstanceScope` is only `Own | All`). So the capsule scope is net-new: a set
//! of `roots` plus a hop-bounded `expand_hops`, materialized into a concrete id
//! manifest by a bounded breadth-first walk over the graph.
//!
//! This module is the **pure** selection core: it computes the id manifest from a
//! scope and a `neighbors` lookup, with no graph or I/O dependency, so it is fully
//! testable in isolation. The materializer (a later piece) supplies a `neighbors`
//! closure backed by the real graph's edges as of `T_mint`, applies the
//! relation-type / field over-share controls when constructing that closure, and
//! turns the resulting id manifest into the frozen, content-addressed slice.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

/// A capsule's scope: the hand-picked roots and how far to expand from them.
///
/// `expand_hops` is the number of edge hops the walk follows out from the roots
/// (`0` = exactly the roots, no expansion). The expansion is bounded both by this
/// hop count and by what the `neighbors` lookup chooses to return, so the
/// relation-type over-share controls (§4) are applied when the materializer builds
/// the lookup, not here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapsuleScope {
    /// The hand-picked node ids the capsule is built around.
    pub roots: Vec<String>,
    /// How many edge hops to expand out from the roots.
    pub expand_hops: u32,
}

/// Materialize a [`CapsuleScope`] into its concrete id manifest: the set of node
/// ids the capsule includes, computed by a bounded breadth-first walk from the
/// roots following `neighbors` for `expand_hops` hops.
///
/// `neighbors(id)` returns the ids adjacent to `id` (the direction and the
/// relation-type filtering are the caller's: the materializer encodes the
/// over-share controls in the closure it passes). The walk is **cycle-safe** (each
/// id is visited once) and **deterministic** (the manifest is a sorted set), so
/// the same `(scope, graph-as-of-T_mint)` always yields the same manifest — the
/// property the capsule's content-addressed identity rests on. Roots are always
/// included; an empty root set yields an empty manifest with no lookups.
pub fn expand_scope<F>(scope: &CapsuleScope, neighbors: F) -> BTreeSet<String>
where
    F: Fn(&str) -> Vec<String>,
{
    let mut included: BTreeSet<String> = scope.roots.iter().cloned().collect();
    // The frontier is the set newly reached at the previous hop; expanding it
    // and keeping only ids not already included bounds the work to each node once.
    let mut frontier: Vec<String> = included.iter().cloned().collect();

    for _ in 0..scope.expand_hops {
        if frontier.is_empty() {
            break;
        }
        let mut next = Vec::new();
        for node in &frontier {
            for neighbour in neighbors(node) {
                if included.insert(neighbour.clone()) {
                    next.push(neighbour);
                }
            }
        }
        frontier = next;
    }
    included
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    /// A fixed adjacency for the tests: id -> its neighbours.
    fn graph(edges: &[(&str, &[&str])]) -> impl Fn(&str) -> Vec<String> {
        let map: BTreeMap<String, Vec<String>> = edges
            .iter()
            .map(|(k, vs)| (k.to_string(), vs.iter().map(|s| s.to_string()).collect()))
            .collect();
        move |id: &str| map.get(id).cloned().unwrap_or_default()
    }

    fn scope(roots: &[&str], hops: u32) -> CapsuleScope {
        CapsuleScope {
            roots: roots.iter().map(|s| s.to_string()).collect(),
            expand_hops: hops,
        }
    }

    fn set(ids: &[&str]) -> BTreeSet<String> {
        ids.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn zero_hops_is_exactly_the_roots() {
        let g = graph(&[("p", &["f1", "f2"])]);
        assert_eq!(expand_scope(&scope(&["p"], 0), &g), set(&["p"]));
    }

    #[test]
    fn one_hop_includes_direct_neighbours() {
        let g = graph(&[("p", &["f1", "f2"]), ("f1", &["x"])]);
        // x is two hops away, so a one-hop walk must not reach it.
        assert_eq!(expand_scope(&scope(&["p"], 1), &g), set(&["p", "f1", "f2"]));
    }

    #[test]
    fn two_hops_reaches_the_second_ring() {
        let g = graph(&[("p", &["f1"]), ("f1", &["x"]), ("x", &["y"])]);
        assert_eq!(expand_scope(&scope(&["p"], 2), &g), set(&["p", "f1", "x"]));
    }

    #[test]
    fn expansion_is_cycle_safe() {
        // a <-> b: a naive walk would loop forever; the visited set stops it.
        let g = graph(&[("a", &["b"]), ("b", &["a"])]);
        assert_eq!(expand_scope(&scope(&["a"], 10), &g), set(&["a", "b"]));
    }

    #[test]
    fn a_shared_neighbour_appears_once() {
        let g = graph(&[("p", &["f1", "f2"]), ("f1", &["shared"]), ("f2", &["shared"])]);
        assert_eq!(
            expand_scope(&scope(&["p"], 2), &g),
            set(&["p", "f1", "f2", "shared"])
        );
    }

    #[test]
    fn empty_roots_yield_an_empty_manifest() {
        let g = graph(&[("p", &["f1"])]);
        assert!(expand_scope(&scope(&[], 5), &g).is_empty());
    }

    #[test]
    fn scope_round_trips_through_json() {
        let s = scope(&["p1", "p2"], 2);
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(serde_json::from_str::<CapsuleScope>(&json).unwrap(), s);
    }
}

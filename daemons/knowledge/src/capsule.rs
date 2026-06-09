//! Context Capsule materialization, daemon side (context-capsule.md §4, loader
//! option (b)).
//!
//! The knowledge daemon owns the graph and the bitemporal as-of read, so it
//! materializes a capsule's frozen slice server-side: given a
//! [`arlen_capsule::scope::CapsuleScope`] and a mint time, it expands the scope
//! over the graph, reads each included node's projected fields and the edges among
//! them as of `T_mint`, and returns the [`arlen_capsule::slice::FrozenSlice`] the
//! caller (`capsuled`) content-addresses and signs. This keeps the as-of read and
//! the projection where the graph is, rather than duplicating the bitemporal
//! `valid_as_of` logic in an outward reader.
//!
//! This module is being built bottom-up. The first piece is the pure cell
//! conversion; the graph-backed scope expansion, the projected as-of node load and
//! the read op follow.

// API ahead of its consumer: the cell conversion is used by the capsule read op
// (the next piece) and by tests; until the op wires it into the bin tree it reads
// unused in a plain build, like the other not-yet-consumed daemon read APIs.
#![allow(dead_code)]

use anyhow::Result;
use arlen_capsule::slice::SliceValue;

use crate::graph::{CellValue, GraphHandle};
use crate::utils::escape_cypher;

/// Map a graph [`CellValue`] to a frozen-slice [`SliceValue`], or `None` if the
/// value cannot be carried in the canonical frozen form.
///
/// `String`/`Int64`/`Bool`/`Null` map directly (a genuine null field stays a
/// present-but-null `SliceValue::Null`). A `Float` maps to `None`: the canonical
/// slice form deliberately carries no floating point (its byte form must be
/// deterministic for content addressing, and cross-platform float formatting is
/// not), so a float-typed field is **omitted** from the slice rather than encoded
/// lossily. Timestamps are stored as `Int64` epoch microseconds in the graph, so
/// they map faithfully to `SliceValue::Int`; only genuine floats are dropped, a
/// documented frozen-form limit (§4).
pub(crate) fn cell_to_slice_value(cell: &CellValue) -> Option<SliceValue> {
    match cell {
        CellValue::String(s) => Some(SliceValue::Text(s.clone())),
        CellValue::Int64(i) => Some(SliceValue::Int(*i)),
        CellValue::Bool(b) => Some(SliceValue::Bool(*b)),
        CellValue::Null => Some(SliceValue::Null),
        CellValue::Float(_) => None,
    }
}

/// The live `FILE_PART_OF` neighbours of `node_id`, both directions — the
/// adjacency the capsule scope BFS follows.
///
/// "Live" means the bitemporally-current membership (open intervals: `invalid_at`
/// and `expired_at` both null) with a live endpoint (`m.expired_at` null — a File
/// has no such column, which lbug reads as null, so files always pass; an archived
/// Project is excluded). That is exactly the as-of read for a capsule minted now
/// (`T_mint = now`): valid-as-of-now is the open-interval set. Minting a capsule of
/// the graph as it was *earlier* (an as-of-past `T_mint`) is a later refinement
/// over the `valid_as_of` fragment; a capsule is minted now, so live = as-of-now.
///
/// Only `FILE_PART_OF` is followed today: it is the bitemporally-stamped membership
/// edge the "share this project" capsule is built from, so its as-of read is
/// honest. Following other (unstamped) relation types is a documented follow-up
/// (their as-of read would be current-state, not frozen). The result is sorted and
/// deduped so the BFS is deterministic; a read error propagates so the caller can
/// fail the mint closed rather than freeze a partial slice.
pub(crate) async fn capsule_neighbors(graph: &GraphHandle, node_id: &str) -> Result<Vec<String>> {
    let id = escape_cypher(node_id);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_the_carriable_cell_types() {
        assert_eq!(
            cell_to_slice_value(&CellValue::String("x".into())),
            Some(SliceValue::Text("x".into()))
        );
        assert_eq!(
            cell_to_slice_value(&CellValue::Int64(-7)),
            Some(SliceValue::Int(-7))
        );
        assert_eq!(
            cell_to_slice_value(&CellValue::Bool(true)),
            Some(SliceValue::Bool(true))
        );
        // A genuine null is a present-but-null field, distinct from an omitted one.
        assert_eq!(cell_to_slice_value(&CellValue::Null), Some(SliceValue::Null));
    }

    #[test]
    fn drops_a_float_field_to_keep_the_form_deterministic() {
        assert_eq!(cell_to_slice_value(&CellValue::Float(1.5)), None);
    }

    #[tokio::test]
    async fn neighbours_follow_live_memberships_both_directions() {
        let tmp = tempfile::TempDir::new().unwrap();
        let graph = crate::graph::spawn(tmp.path().join("graph").to_str().unwrap()).unwrap();
        graph
            .write("CREATE (f:File {id: 'f1', path: '/x', app_id: 't', last_accessed: 0})".into())
            .await
            .unwrap();
        graph.write("CREATE (p:Project {id: 'p1'})".into()).await.unwrap();
        graph
            .write(
                "MATCH (f:File {id:'f1'}), (p:Project {id:'p1'}) CREATE (f)-[:FILE_PART_OF]->(p)"
                    .into(),
            )
            .await
            .unwrap();

        // The membership is followed in both directions.
        assert_eq!(capsule_neighbors(&graph, "p1").await.unwrap(), vec!["f1".to_string()]);
        assert_eq!(capsule_neighbors(&graph, "f1").await.unwrap(), vec!["p1".to_string()]);
        // A node with no live membership has no neighbours.
        assert!(capsule_neighbors(&graph, "absent").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_closed_membership_is_not_a_neighbour() {
        let tmp = tempfile::TempDir::new().unwrap();
        let graph = crate::graph::spawn(tmp.path().join("graph").to_str().unwrap()).unwrap();
        graph
            .write("CREATE (f:File {id: 'f1', path: '/x', app_id: 't', last_accessed: 0})".into())
            .await
            .unwrap();
        graph.write("CREATE (p:Project {id: 'p1'})".into()).await.unwrap();
        // A closed (retracted) membership: invalid_at/expired_at set.
        graph
            .write(
                "MATCH (f:File {id:'f1'}), (p:Project {id:'p1'}) \
                 CREATE (f)-[:FILE_PART_OF {invalid_at: 1, expired_at: 1}]->(p)"
                    .into(),
            )
            .await
            .unwrap();
        assert!(capsule_neighbors(&graph, "p1").await.unwrap().is_empty());
    }
}

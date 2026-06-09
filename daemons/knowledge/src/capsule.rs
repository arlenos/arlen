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

use std::collections::{BTreeMap, BTreeSet};

use anyhow::Result;
use arlen_capsule::scope::CapsuleScope;
use arlen_capsule::slice::{SliceNode, SliceValue};

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

/// Expand a [`CapsuleScope`] into its id manifest by a bounded breadth-first walk
/// over the live graph, following [`capsule_neighbors`] for `expand_hops` hops.
///
/// This is the live, async counterpart of the pure
/// [`arlen_capsule::scope::expand_scope`] (which stays the tested specification of
/// the algorithm): a graph read is async and fallible, so the walk cannot use the
/// pure version's sync `Fn` neighbour source. The walk is cycle-safe (each id is
/// visited once via the included set) and deterministic (the manifest is the sorted
/// included set), so a fresh mint of the same scope yields the same manifest. A
/// neighbour read error propagates, so the caller fails the mint closed rather than
/// freezing a partial slice. Roots are always included; empty roots yield an empty
/// manifest with no reads.
pub(crate) async fn capsule_expand(
    graph: &GraphHandle,
    scope: &CapsuleScope,
) -> Result<Vec<String>> {
    let mut included: BTreeSet<String> = scope.roots.iter().cloned().collect();
    let mut frontier: Vec<String> = included.iter().cloned().collect();
    for _ in 0..scope.expand_hops {
        if frontier.is_empty() {
            break;
        }
        let mut next = Vec::new();
        for node in &frontier {
            for neighbour in capsule_neighbors(graph, node).await? {
                if included.insert(neighbour.clone()) {
                    next.push(neighbour);
                }
            }
        }
        frontier = next;
    }
    // A BTreeSet iterates in sorted order, so the manifest is deterministic.
    Ok(included.into_iter().collect())
}

/// Load one node's projected fields as a [`SliceNode`], or `None` if no node with
/// that `(label, id)` exists.
///
/// `label` and `fields` are **trusted schema identifiers** (the projection layer
/// resolves them from the entity schema, never from caller input), so they are
/// interpolated into the query; only `id` is escaped. The fields are read
/// explicitly (`RETURN n.id, n.field, ...`) because the typed row protocol has no
/// map/struct cell, so a whole-node read is not representable: the projection is
/// thus applied at read time, not after. Each cell is mapped through
/// [`cell_to_slice_value`]; a field that maps to `None` (a float) or is absent is
/// omitted, so the slice carries only the canonical, projected values. The leading
/// `n.id` column is the existence probe.
pub(crate) async fn load_node_fields(
    graph: &GraphHandle,
    label: &str,
    id: &str,
    fields: &[&str],
) -> Result<Option<SliceNode>> {
    let id_esc = escape_cypher(id);
    let mut cols = vec!["n.id".to_string()];
    cols.extend(fields.iter().map(|f| format!("n.{f}")));
    let cypher = format!(
        "MATCH (n:{label} {{id: '{id_esc}'}}) RETURN {}",
        cols.join(", ")
    );
    let rs = graph.query_rows(cypher).await?;
    let Some(row) = rs.rows.first() else {
        return Ok(None);
    };
    let mut field_map = BTreeMap::new();
    for (i, field) in fields.iter().enumerate() {
        // Column 0 is the existence probe (n.id); projected fields start at 1.
        if let Some(cell) = row.get(i + 1) {
            if let Some(value) = cell_to_slice_value(cell) {
                field_map.insert((*field).to_string(), value);
            }
        }
    }
    Ok(Some(SliceNode {
        id: id.to_string(),
        label: label.to_string(),
        fields: field_map,
    }))
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
    async fn expand_walks_a_project_to_its_members() {
        let tmp = tempfile::TempDir::new().unwrap();
        let graph = crate::graph::spawn(tmp.path().join("graph").to_str().unwrap()).unwrap();
        for f in ["f1", "f2"] {
            graph
                .write(format!(
                    "CREATE (f:File {{id: '{f}', path: '/{f}', app_id: 't', last_accessed: 0}})"
                ))
                .await
                .unwrap();
        }
        graph.write("CREATE (p:Project {id: 'p1'})".into()).await.unwrap();
        for f in ["f1", "f2"] {
            graph
                .write(format!(
                    "MATCH (f:File {{id:'{f}'}}), (p:Project {{id:'p1'}}) CREATE (f)-[:FILE_PART_OF]->(p)"
                ))
                .await
                .unwrap();
        }

        // One hop from the project reaches its two member files (sorted manifest).
        let one_hop = capsule_expand(&graph, &CapsuleScope { roots: vec!["p1".into()], expand_hops: 1 })
            .await
            .unwrap();
        assert_eq!(one_hop, vec!["f1".to_string(), "f2".to_string(), "p1".to_string()]);

        // Zero hops is exactly the roots.
        let zero = capsule_expand(&graph, &CapsuleScope { roots: vec!["p1".into()], expand_hops: 0 })
            .await
            .unwrap();
        assert_eq!(zero, vec!["p1".to_string()]);
    }

    #[tokio::test]
    async fn loads_a_node_with_its_projected_fields() {
        let tmp = tempfile::TempDir::new().unwrap();
        let graph = crate::graph::spawn(tmp.path().join("graph").to_str().unwrap()).unwrap();
        graph
            .write(
                "CREATE (f:File {id: 'f1', path: '/x/y.rs', app_id: 'editor', last_accessed: 42})"
                    .into(),
            )
            .await
            .unwrap();

        let node = load_node_fields(&graph, "File", "f1", &["path", "last_accessed"])
            .await
            .unwrap()
            .expect("the node exists");
        assert_eq!(node.id, "f1");
        assert_eq!(node.label, "File");
        // Only the projected fields, each mapped to its canonical SliceValue type.
        assert_eq!(node.fields.len(), 2);
        assert_eq!(node.fields.get("path"), Some(&SliceValue::Text("/x/y.rs".into())));
        assert_eq!(node.fields.get("last_accessed"), Some(&SliceValue::Int(42)));
        assert!(node.fields.get("app_id").is_none(), "an unprojected field is omitted");

        // A missing node is None.
        assert!(load_node_fields(&graph, "File", "absent", &["path"]).await.unwrap().is_none());
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

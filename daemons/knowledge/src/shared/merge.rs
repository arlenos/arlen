//! Shared-entity merge execution: fold one node's edges onto another, then
//! delete the merged node. This is the act a merge suggestion (`suggestion.rs`)
//! resolves to when the user accepts a `Merge`.
//!
//! Entity edges live in DYNAMIC per-triple relationship tables
//! (`entity_rel_table_name`, one `r_<edge>_<hash>` table per `(edge, from, to)`)
//! whose names hash their endpoints, so the tables touching an entity are found
//! by asking the catalog ([`GraphHandle::rel_tables_involving`]) which tables
//! have the entity's node table as an endpoint. Those tables are declared
//! property-less (`CREATE REL TABLE ... (FROM x TO y)`, no columns), so an edge
//! is a bare `(from, to)` pair: re-pointing it is lossless, and `MERGE`
//! collapses a duplicate onto the kept node.
//!
//! The whole fold runs in one transaction ([`GraphHandle::transaction`]): every
//! edge moves and the node is deleted, or nothing changes. There is no live
//! caller yet (the shared-entity write op is gated), so this is the tested
//! mechanism the accept-a-merge trigger will drive.

use crate::graph::{GraphHandle, RelTable};
use crate::utils::escape_cypher;
use anyhow::{anyhow, Result};

/// Outcome of a merge: how many edge tables were re-pointed and that the merged
/// node was deleted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeReport {
    /// The number of relationship tables whose edges were folded onto the kept
    /// node (a table touching neither node contributes no statements).
    pub tables_repointed: usize,
}

/// Build the ordered Cypher statements that fold `delete_id`'s edges onto
/// `keep_id` (both nodes in `node_table`) across `rel_tables`, then delete the
/// merged node. Pure: it emits the statements, it does not run them. The caller
/// runs them in one transaction so the fold is all-or-nothing.
///
/// Per relationship table, the direction `node_table` sits on decides the
/// re-point:
/// - source only: `del -> t` becomes `keep -> t`;
/// - destination only: `s -> del` becomes `s -> keep`;
/// - both (a self-referential table, e.g. Person KNOWS Person): `del`'s edges to
///   genuine OTHER nodes are re-pointed onto `keep`. Edges purely between `del`
///   and `keep` (`del -> del`, `del -> keep`, `keep -> del`) are merge artifacts
///   (an entity does not meaningfully relate to itself because two of its
///   duplicate records had an edge), so they are dropped rather than folded into
///   a `keep -> keep` self-loop; the final `DETACH DELETE` of the merged node
///   removes them.
///
/// `MERGE` collapses a re-pointed edge that already exists on the kept node, so
/// no duplicate is created. Table names come from the catalog (trusted
/// identifiers); the two ids are escaped.
pub fn merge_statements(
    node_table: &str,
    rel_tables: &[RelTable],
    delete_id: &str,
    keep_id: &str,
) -> Vec<String> {
    let nt = node_table;
    let del = escape_cypher(delete_id);
    let keep = escape_cypher(keep_id);
    let mut stmts = Vec::new();

    for rel in rel_tables {
        let r = &rel.name;
        let is_source = rel.source == nt;
        let is_dest = rel.dest == nt;
        match (is_source, is_dest) {
            (true, true) => {
                // Self-referential table: re-point del's edges to genuine OTHER
                // nodes (target/source neither del nor keep). Edges purely
                // between del and keep are merge artifacts, left for the final
                // DETACH DELETE to remove, not folded into a keep self-loop.
                stmts.push(format!(
                    "MATCH (k:{nt} {{id:'{keep}'}}),(d:{nt} {{id:'{del}'}})-[r:{r}]->(t) \
                     WHERE t.id <> '{del}' AND t.id <> '{keep}' MERGE (k)-[:{r}]->(t) DELETE r"
                ));
                stmts.push(format!(
                    "MATCH (k:{nt} {{id:'{keep}'}}),(s)-[r:{r}]->(d:{nt} {{id:'{del}'}}) \
                     WHERE s.id <> '{del}' AND s.id <> '{keep}' MERGE (s)-[:{r}]->(k) DELETE r"
                ));
            }
            (true, false) => stmts.push(format!(
                "MATCH (k:{nt} {{id:'{keep}'}}),(d:{nt} {{id:'{del}'}})-[r:{r}]->(t) \
                 MERGE (k)-[:{r}]->(t) DELETE r"
            )),
            (false, true) => stmts.push(format!(
                "MATCH (k:{nt} {{id:'{keep}'}}),(s)-[r:{r}]->(d:{nt} {{id:'{del}'}}) \
                 MERGE (s)-[:{r}]->(k) DELETE r"
            )),
            (false, false) => {}
        }
    }

    // The merged node itself. All its edges were re-pointed above, so DETACH is
    // belt-and-suspenders (and guards any table the enumeration could not name).
    stmts.push(format!("MATCH (d:{nt} {{id:'{del}'}}) DETACH DELETE d"));
    stmts
}

/// Execute the merge: fold `delete_id`'s edges onto `keep_id` (both in
/// `node_table`) and delete the merged node, atomically.
///
/// Refuses if the kept node does not exist (a fold onto a missing node would
/// re-point nothing yet still delete the merged node, orphaning its edges).
/// `delete_id == keep_id` is a no-op success. The re-point + delete run in one
/// transaction, so a mid-fold error rolls the whole thing back.
pub async fn merge_node(
    graph: &GraphHandle,
    node_table: &str,
    delete_id: &str,
    keep_id: &str,
) -> Result<MergeReport> {
    if delete_id == keep_id {
        return Ok(MergeReport { tables_repointed: 0 });
    }
    // The kept node must exist before we delete the merged one.
    let keep_present = graph
        .query_rows(format!(
            "MATCH (k:{node_table} {{id:'{}'}}) RETURN k.id LIMIT 1",
            escape_cypher(keep_id)
        ))
        .await?;
    if keep_present.rows.is_empty() {
        return Err(anyhow!(
            "cannot merge onto a missing node: {keep_id} not in {node_table}"
        ));
    }

    let rel_tables = graph.rel_tables_involving(node_table).await?;
    let repointed = rel_tables
        .iter()
        .filter(|r| r.source == node_table || r.dest == node_table)
        .count();
    let stmts = merge_statements(node_table, &rel_tables, delete_id, keep_id);
    graph.transaction(stmts).await?;
    Ok(MergeReport {
        tables_repointed: repointed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::spawn;

    fn rt(name: &str, source: &str, dest: &str) -> RelTable {
        RelTable {
            name: name.to_string(),
            source: source.to_string(),
            dest: dest.to_string(),
        }
    }

    #[test]
    fn statements_cover_each_direction_and_escape_ids() {
        let tables = vec![
            rt("R_out", "P", "O"),  // outgoing
            rt("R_in", "O", "P"),   // incoming
            rt("R_self", "P", "P"), // self-referential
            rt("R_other", "O", "O"), // touches neither -> no statement
        ];
        let s = merge_statements("P", &tables, "a'b", "keep");
        // outgoing (1) + incoming (1) + self (2) + final delete (1) = 5.
        assert_eq!(s.len(), 5);
        assert!(s[0].contains("(d:P {id:'a\\'b'})-[r:R_out]->(t)"));
        assert!(s[0].contains("MERGE (k)-[:R_out]->(t)"));
        assert!(s[1].contains("(s)-[r:R_in]->(d:P {id:'a\\'b'})"));
        assert!(s.last().unwrap().contains("DETACH DELETE d"));
        assert!(!s.iter().any(|q| q.contains("R_other")), "unrelated table skipped");
    }

    #[tokio::test]
    async fn merge_folds_outgoing_incoming_and_self_edges_then_deletes() {
        let tmp = tempfile::TempDir::new().unwrap();
        let graph = spawn(tmp.path().join("g").to_str().unwrap()).unwrap();
        graph.write("CREATE NODE TABLE P(id STRING, PRIMARY KEY(id))".into()).await.unwrap();
        graph.write("CREATE NODE TABLE O(id STRING, PRIMARY KEY(id))".into()).await.unwrap();
        graph.write("CREATE REL TABLE WORKS(FROM P TO O)".into()).await.unwrap();
        graph.write("CREATE REL TABLE OWNS(FROM O TO P)".into()).await.unwrap();
        graph.write("CREATE REL TABLE KNOWS(FROM P TO P)".into()).await.unwrap();
        graph
            .write(
                "CREATE (:P {id:'del'}),(:P {id:'keep'}),(:P {id:'p3'}),\
                 (:O {id:'o1'}),(:O {id:'o2'})"
                    .into(),
            )
            .await
            .unwrap();
        // Outgoing: del->o1, del->o2; keep already ->o2 (must dedup to keep->o1,o2).
        graph.write("MATCH (d:P {id:'del'}),(o:O {id:'o1'}) CREATE (d)-[:WORKS]->(o)".into()).await.unwrap();
        graph.write("MATCH (d:P {id:'del'}),(o:O {id:'o2'}) CREATE (d)-[:WORKS]->(o)".into()).await.unwrap();
        graph.write("MATCH (k:P {id:'keep'}),(o:O {id:'o2'}) CREATE (k)-[:WORKS]->(o)".into()).await.unwrap();
        // Incoming: o1->del (becomes o1->keep).
        graph.write("MATCH (o:O {id:'o1'}),(d:P {id:'del'}) CREATE (o)-[:OWNS]->(d)".into()).await.unwrap();
        // Self-ref: del->p3 (becomes keep->p3), p3->del (becomes p3->keep), del->del (becomes keep->keep).
        graph.write("MATCH (d:P {id:'del'}),(p:P {id:'p3'}) CREATE (d)-[:KNOWS]->(p)".into()).await.unwrap();
        graph.write("MATCH (p:P {id:'p3'}),(d:P {id:'del'}) CREATE (p)-[:KNOWS]->(d)".into()).await.unwrap();
        graph.write("MATCH (d:P {id:'del'}) CREATE (d)-[:KNOWS]->(d)".into()).await.unwrap();

        let report = merge_node(&graph, "P", "del", "keep").await.unwrap();
        assert_eq!(report.tables_repointed, 3);

        // del is gone with no residual edges.
        let del = graph.query_rows("MATCH (d:P {id:'del'}) RETURN d.id".into()).await.unwrap();
        assert!(del.rows.is_empty(), "the merged node is deleted");

        // keep now carries del's outgoing WORKS, deduped: o1 + o2 (not o2 twice).
        let works = graph
            .query_rows("MATCH (:P {id:'keep'})-[:WORKS]->(o) RETURN o.id".into())
            .await
            .unwrap();
        assert_eq!(works.rows.len(), 2, "outgoing folded and deduped");

        // keep now receives o1->keep.
        let owns = graph
            .query_rows("MATCH (o)-[:OWNS]->(:P {id:'keep'}) RETURN o.id".into())
            .await
            .unwrap();
        assert_eq!(owns.rows.len(), 1, "incoming re-pointed onto keep");

        // Self-ref: keep->p3 and p3->keep folded; the del->del self-loop is a
        // merge artifact and is dropped (no keep->keep).
        let knows_out = graph
            .query_rows("MATCH (:P {id:'keep'})-[:KNOWS]->(p) RETURN p.id".into())
            .await
            .unwrap();
        let out_ids: Vec<String> =
            knows_out.rows.iter().map(|r| r[0].as_str().to_string()).collect();
        assert!(out_ids.contains(&"p3".to_string()), "keep->p3 folded");
        assert!(!out_ids.contains(&"keep".to_string()), "self-loop artifact dropped");
        let knows_in = graph
            .query_rows("MATCH (p)-[:KNOWS]->(:P {id:'keep'}) RETURN p.id".into())
            .await
            .unwrap();
        let in_ids: Vec<String> =
            knows_in.rows.iter().map(|r| r[0].as_str().to_string()).collect();
        assert!(in_ids.contains(&"p3".to_string()), "p3->keep folded");
        assert!(!in_ids.contains(&"keep".to_string()), "no keep self-loop created");
    }

    #[tokio::test]
    async fn merge_onto_a_missing_node_is_refused() {
        let tmp = tempfile::TempDir::new().unwrap();
        let graph = spawn(tmp.path().join("g").to_str().unwrap()).unwrap();
        graph.write("CREATE NODE TABLE P(id STRING, PRIMARY KEY(id))".into()).await.unwrap();
        graph.write("CREATE (:P {id:'del'})".into()).await.unwrap();
        let err = merge_node(&graph, "P", "del", "absent").await.unwrap_err();
        assert!(err.to_string().contains("missing node"));
        // del survived (nothing was deleted).
        let del = graph.query_rows("MATCH (d:P {id:'del'}) RETURN d.id".into()).await.unwrap();
        assert_eq!(del.rows.len(), 1, "a refused merge deletes nothing");
    }

    #[tokio::test]
    async fn merging_a_node_into_itself_is_a_noop() {
        let tmp = tempfile::TempDir::new().unwrap();
        let graph = spawn(tmp.path().join("g").to_str().unwrap()).unwrap();
        graph.write("CREATE NODE TABLE P(id STRING, PRIMARY KEY(id))".into()).await.unwrap();
        graph.write("CREATE (:P {id:'x'})".into()).await.unwrap();
        let report = merge_node(&graph, "P", "x", "x").await.unwrap();
        assert_eq!(report.tables_repointed, 0);
        let x = graph.query_rows("MATCH (n:P {id:'x'}) RETURN n.id".into()).await.unwrap();
        assert_eq!(x.rows.len(), 1, "the node still exists");
    }
}


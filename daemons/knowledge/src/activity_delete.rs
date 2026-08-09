//! Deleting your own recorded activity, for good.
//!
//! The timeline's Delete control promises "Removes {range} from the timeline.
//! This cannot be undone", and `bitemporal-knowledge-graph.md` settles what that
//! means against a graph that is otherwise close-never-delete: a HARD delete.
//! The audit guarantee exists so the user can hold the SYSTEM to account, not the
//! other way round, and it lives in the audit ledger, which this never touches.
//! A "delete my history" that quietly meant "hide my history" would be the dark
//! pattern this system is a rebuttal to.
//!
//! ## What "the matching nodes" turns out to mean
//!
//! The timeline is not one shape. Window focus is an `Event` node, which is a
//! pure activity record and goes whole. A file access is a FIELD - `last_accessed`
//! on the `File` entity - and that entity may also be a member of a project. So
//! "delete the node" is the wrong granularity for half the timeline: it would
//! take the user's project membership with it, which they did not ask to delete
//! and would not expect. What the row records is the ACCESS, so the access is
//! what is destroyed: the timestamp and the access edges, hard, not hidden.
//!
//! A `File` left holding nothing but the observation - no project membership - is
//! an artifact of the recording rather than anything the user owns, so it goes
//! whole too. The rule reads: destroy the activity everywhere, and destroy the
//! node when the activity was all it was.
//!
//! Nothing here reads or writes the audit ledger. The act is audited by the
//! caller (the daemon op), which knows the caller identity; recording the act and
//! never the content is the point, so this returns counts rather than paths.

use anyhow::Result;

use crate::graph::GraphHandle;

/// What a delete removed. Counts only: enough to audit that it happened and to
/// tell the user their range is gone, with nothing about what was in it.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct DeletedActivity {
    /// `Event` nodes removed whole.
    pub events: u64,
    /// File access records destroyed (timestamp plus access edges).
    pub file_accesses: u64,
    /// `File` nodes removed because the access was all they held.
    pub orphan_files: u64,
}

impl DeletedActivity {
    /// Did this delete anything at all? A delete over an empty range is a
    /// success, not an error, but the caller may want to say "nothing to remove".
    pub fn is_empty(&self) -> bool {
        self.events == 0 && self.file_accesses == 0 && self.orphan_files == 0
    }
}

/// The statements that destroy activity at or after `from`, in the order they
/// must run.
///
/// Order is load-bearing: every file statement selects on `f.last_accessed`, so
/// clearing it has to come last or the ones after it would match nothing. The
/// orphan sweep runs before the clear for the same reason.
///
/// Kept separate from execution so the shape is readable and unit-testable
/// without a graph; `delete_activity_since` is what actually runs it, inside one
/// transaction, so a failure part-way cannot leave half a range deleted.
fn deletion_statements(from: i64) -> Vec<String> {
    vec![
        // Pure activity records, removed whole with their edges.
        format!("MATCH (e:Event) WHERE e.timestamp >= {from} DETACH DELETE e"),
        // The access edges: who opened it, who wrote it, which session it was in,
        // and what it was opened alongside. All of these say "this happened",
        // which is exactly what is being deleted.
        format!("MATCH (f:File)-[r:ACCESSED_BY]->() WHERE f.last_accessed >= {from} DELETE r"),
        format!("MATCH (f:File)-[r:MODIFIED_BY]->() WHERE f.last_accessed >= {from} DELETE r"),
        format!("MATCH (f:File)-[r:ACCESSED_IN]->() WHERE f.last_accessed >= {from} DELETE r"),
        format!("MATCH (f:File)-[r:CO_ACCESSED]->() WHERE f.last_accessed >= {from} DELETE r"),
        format!("MATCH ()-[r:CO_ACCESSED]->(f:File) WHERE f.last_accessed >= {from} DELETE r"),
        // A file that held nothing but the observation goes whole. Membership is
        // the test: a file in a project is something the user organised, and
        // deleting it here would take a fact they never asked to delete.
        format!(
            "MATCH (f:File) WHERE f.last_accessed >= {from} \
             AND NOT EXISTS {{ MATCH (f)-[:FILE_PART_OF]->() }} DETACH DELETE f"
        ),
        // Last, for the reason in the doc comment above.
        format!("MATCH (f:File) WHERE f.last_accessed >= {from} SET f.last_accessed = NULL"),
    ]
}

/// Count what is about to go, so the act can be audited and reported.
///
/// Read before the write, on the same serial graph thread, so the numbers
/// describe the range the transaction then removes.
async fn count_activity_since(graph: &GraphHandle, from: i64) -> Result<DeletedActivity> {
    let one = |rows: crate::graph::RowSet| -> u64 {
        rows.rows
            .first()
            .and_then(|r| r.first())
            .map(|c| c.as_i64())
            .unwrap_or(0)
            .max(0) as u64
    };

    let events = one(
        graph
            .query_rows(format!(
                "MATCH (e:Event) WHERE e.timestamp >= {from} RETURN count(e) AS n"
            ))
            .await?,
    );
    let file_accesses = one(
        graph
            .query_rows(format!(
                "MATCH (f:File) WHERE f.last_accessed >= {from} RETURN count(f) AS n"
            ))
            .await?,
    );
    let orphan_files = one(
        graph
            .query_rows(format!(
                "MATCH (f:File) WHERE f.last_accessed >= {from} \
                 AND NOT EXISTS {{ MATCH (f)-[:FILE_PART_OF]->() }} RETURN count(f) AS n"
            ))
            .await?,
    );

    Ok(DeletedActivity {
        events,
        file_accesses,
        orphan_files,
    })
}

/// Destroy every recorded activity at or after `from`, and report what went.
///
/// Atomic: one transaction, so a failure leaves the range intact rather than
/// half-deleted. A caller that gets an error must tell the user their history is
/// still there, which is what the app's Delete already does.
pub async fn delete_activity_since(graph: &GraphHandle, from: i64) -> Result<DeletedActivity> {
    let counted = count_activity_since(graph, from).await?;
    graph.transaction(deletion_statements(from)).await?;
    Ok(counted)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_clear_runs_after_everything_that_selects_on_it() {
        // If SET last_accessed = NULL moved earlier, every statement after it
        // would match nothing and the delete would silently do almost nothing -
        // the failure would look like an empty range rather than a bug.
        let stmts = deletion_statements(100);
        let clear = stmts
            .iter()
            .position(|s| s.contains("SET f.last_accessed = NULL"))
            .expect("the clear is in the list");
        assert_eq!(clear, stmts.len() - 1, "the clear must be last: {stmts:#?}");
    }

    /// Against a real graph, because the dialect is the thing I cannot reason my
    /// way to: whether the engine takes an `EXISTS` subquery at all, and whether
    /// a delete over one label leaves the neighbours standing.
    #[tokio::test]
    async fn the_range_goes_and_what_the_user_organised_stays() {
        let tmp = tempfile::tempdir().unwrap();
        let graph = crate::graph::spawn(tmp.path().join("graph").to_str().unwrap()).unwrap();

        graph
            .transaction(vec![
                "CREATE (:Project {id:'p1', name:'Work'})".into(),
                // In a project, accessed inside the range: the access goes, the
                // file and its membership stay.
                "CREATE (:File {id:'/w/kept.rs', path:'/w/kept.rs', last_accessed: 500})".into(),
                // No project, accessed inside the range: nothing but the
                // observation, so it goes whole.
                "CREATE (:File {id:'/tmp/seen.rs', path:'/tmp/seen.rs', last_accessed: 600})"
                    .into(),
                // Before the range: untouched, the proof the cut-off is real.
                "CREATE (:File {id:'/w/old.rs', path:'/w/old.rs', last_accessed: 100})".into(),
                "CREATE (:App {id:'editor'})".into(),
                "CREATE (:Event {id:'e-new', type:'window.focused', timestamp: 700})".into(),
                "CREATE (:Event {id:'e-old', type:'window.focused', timestamp: 100})".into(),
            ])
            .await
            .expect("fixture");
        graph
            .transaction(vec![
                "MATCH (f:File {id:'/w/kept.rs'}), (p:Project {id:'p1'}) \
                 CREATE (f)-[:FILE_PART_OF]->(p)"
                    .into(),
                "MATCH (f:File {id:'/w/kept.rs'}), (a:App {id:'editor'}) \
                 CREATE (f)-[:ACCESSED_BY]->(a)"
                    .into(),
            ])
            .await
            .expect("edges");

        let removed = delete_activity_since(&graph, 400).await.expect("delete");
        assert_eq!(removed.events, 1, "only the in-range event");
        assert_eq!(removed.file_accesses, 2);
        assert_eq!(removed.orphan_files, 1);

        let count = |cypher: &str| {
            let g = graph.clone();
            let c = cypher.to_string();
            async move { g.query_rows(c).await.unwrap().rows[0][0].as_i64() }
        };

        // The range is gone.
        assert_eq!(count("MATCH (e:Event) WHERE e.timestamp >= 400 RETURN count(e)").await, 0);
        assert_eq!(count("MATCH (f:File {id:'/tmp/seen.rs'}) RETURN count(f)").await, 0);
        assert_eq!(
            count("MATCH (:File {id:'/w/kept.rs'})-[r:ACCESSED_BY]->() RETURN count(r)").await,
            0,
            "the access edge is destroyed, not hidden"
        );
        assert_eq!(
            count("MATCH (f:File {id:'/w/kept.rs'}) WHERE f.last_accessed IS NOT NULL RETURN count(f)")
                .await,
            0,
            "the timestamp is destroyed, so the row cannot come back"
        );

        // What the user organised, and what predates the range, both stand.
        assert_eq!(
            count("MATCH (:File {id:'/w/kept.rs'})-[r:FILE_PART_OF]->() RETURN count(r)").await,
            1,
            "deleting an access must not take the project membership with it"
        );
        assert_eq!(count("MATCH (f:File {id:'/w/old.rs'}) RETURN count(f)").await, 1);
        assert_eq!(count("MATCH (e:Event {id:'e-old'}) RETURN count(e)").await, 1);
    }

    #[test]
    fn nothing_touches_a_project_membership_edge() {
        // FILE_PART_OF is the one edge that must survive: it is what the user
        // organised, not what the system observed. It may only be READ, as the
        // orphan test.
        for s in deletion_statements(1) {
            let deletes_membership = s.contains("FILE_PART_OF") && s.contains("DELETE r");
            assert!(!deletes_membership, "membership must survive: {s}");
        }
    }
}

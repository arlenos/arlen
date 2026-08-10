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

use std::sync::Arc;

use anyhow::Result;

use crate::graph::GraphHandle;

/// Held by a promotion pass for its whole run, and by a delete for its whole run,
/// so the two are never in flight together.
///
/// Deleting the raw events is not enough on its own, which a booted run showed
/// two times in three: a pass reads its batch out of SQLite BEFORE writing the
/// nodes, so a delete landing in that gap destroys rows the pass is already
/// holding in memory, and it writes them anyway. The surviving stamp sat 28
/// seconds past the cut-off - written after the delete, from a batch read before
/// it.
///
/// This keeps nothing, which is why it does not reopen the tombstone question: it
/// only orders two operations that must not interleave. A delete waits at most one
/// pass, and it is a rare user-initiated act, so the cost lands where it is
/// affordable and the guarantee stops being probabilistic.
pub type PromotionGate = Arc<tokio::sync::Mutex<()>>;

/// A fresh gate. One per daemon, shared by the pass and the delete op.
pub fn promotion_gate() -> PromotionGate {
    Arc::new(tokio::sync::Mutex::new(()))
}

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

    /// One number for the audit record: how much went. The split matters to the
    /// caller, not to the ledger, which only needs the size of the act.
    pub fn total(&self) -> u64 {
        self.events + self.file_accesses + self.orphan_files
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
///
/// Takes MICROSECONDS. Callers hold seconds and convert through `cutoff_micros`.
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

/// The caller's cut-off in the unit the graph actually stores.
///
/// Everything crossing the socket is Unix SECONDS: the app documents it, and the
/// surface's "today" is a midnight timestamp. Everything IN the graph is epoch
/// MICROSECONDS, which the timeline reader states as it divides by a million.
///
/// Compared raw the two never disagree in the interesting direction - every
/// microsecond stamp since 1970 is larger than every second stamp - so
/// `last_accessed >= from` was true for the entire store, and "delete today's
/// activity" destroyed everything ever recorded. Silent, total, and reported as a
/// success with a count that looked plausible.
///
/// Saturating, so an absurd boundary clamps instead of wrapping into the past and
/// deleting the very thing it overflowed past.
fn cutoff_micros(from_secs: i64) -> i64 {
    from_secs.saturating_mul(1_000_000)
}


/// Destroy the RAW events for the range, in the SQLite store the promotion pass
/// reads from.
///
/// Without this the delete is a delayed failure rather than a smaller version of
/// itself. A user clears a range while raw events inside it are still waiting to
/// be promoted; the pass runs half a minute later and puts the nodes back. "This
/// cannot be undone" would then be false because the system undoes it itself,
/// which is the worst available way for that sentence to be wrong.
///
/// Not a tombstone the pass agrees to skip: that keeps the data and adds a rule to
/// remember, and a store holding what it was told to delete is the thing this
/// feature exists to refuse. Deletion is already routine here - retention drops
/// promoted events past thirty days through the same table.
///
/// Same microseconds as the graph, from the same one conversion, which is how the
/// two halves stay in step.
pub async fn delete_raw_events_since(pool: &sqlx::SqlitePool, from_secs: i64) -> Result<u64> {
    let from = cutoff_micros(from_secs);
    let result = sqlx::query("DELETE FROM events WHERE timestamp >= ?")
        .bind(from)
        .execute(pool)
        .await?;
    Ok(result.rows_affected())
}

/// Count what is about to go, so the act can be audited and reported.
///
/// Read before the write, on the same serial graph thread, so the numbers
/// describe the range the transaction then removes.
pub async fn count_activity_since(graph: &GraphHandle, from_secs: i64) -> Result<DeletedActivity> {
    let from = cutoff_micros(from_secs);
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
    run_deletion(graph, from).await?;
    Ok(counted)
}

/// The deletion on its own, for a caller that has already counted.
///
/// The socket op audits the act BEFORE carrying it out and needs the size in that
/// record, so it counts first; re-counting inside would either repeat the work or
/// report a different number than the one audited.
pub async fn run_deletion(graph: &GraphHandle, from_secs: i64) -> Result<()> {
    graph.transaction(deletion_statements(cutoff_micros(from_secs))).await
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

    /// The units the two sides actually use, which the toy-number test below
    /// cannot see.
    ///
    /// `from` arrives in Unix SECONDS - the app documents it, the surface offers
    /// "today" as a midnight timestamp - and the graph stores epoch MICROSECONDS,
    /// which the timeline reader states in its own doc as it divides by a million.
    /// Compared raw, every microsecond stamp since 1970 is larger than every
    /// second stamp, so `last_accessed >= from` is true for the whole store and
    /// "delete today" takes everything ever recorded.
    ///
    /// The existing test passes because 500 and 400 are in the same made-up unit.
    /// That is the shape worth naming: a test can only compare what it is handed,
    /// so a unit error is invisible to it unless the fixture uses real values.
    #[tokio::test]
    async fn a_range_delete_leaves_what_is_older_than_the_range() {
        let tmp = tempfile::tempdir().unwrap();
        let graph = crate::graph::spawn(tmp.path().join("graph").to_str().unwrap()).unwrap();

        // A year ago, in the microseconds the graph really stores.
        let a_year_ago_micros: i64 = 1_723_000_000_000_000;
        // Today's midnight, in the seconds the app really sends.
        let today_secs: i64 = 1_754_784_000;

        graph
            .transaction(vec![format!(
                "CREATE (:File {{id:'/w/old.rs', path:'/w/old.rs', last_accessed: {a_year_ago_micros}}})"
            )])
            .await
            .expect("fixture");

        delete_activity_since(&graph, today_secs).await.expect("delete");

        let left = graph
            .query_rows(
                "MATCH (f:File {id:'/w/old.rs'}) WHERE f.last_accessed IS NOT NULL \
                 RETURN count(f)"
                    .to_string(),
            )
            .await
            .unwrap()
            .rows[0][0]
            .as_i64();
        assert_eq!(
            left, 1,
            "a file accessed a year before the cut-off was destroyed by a delete \
             of today's activity"
        );
    }

    /// The raw events for the range go, and the ones outside it stay.
    ///
    /// The half that makes "this cannot be undone" true: a raw event left behind
    /// is promoted back into a node half a minute later, so leaving the source is
    /// not a smaller delete, it is a delete the system reverses on its own. The
    /// out-of-range row is here for the same reason the graph test grew one -
    /// deleting everything would also pass a test that only checks the range went.
    #[tokio::test]
    async fn the_raw_events_for_the_range_go_and_the_older_ones_stay() {
        let tmp = tempfile::tempdir().unwrap();
        let pool = crate::db::open(tmp.path().join("events.db").to_str().unwrap())
            .await
            .expect("event store");

        let cutoff_secs: i64 = 1_754_784_000;
        let inside = cutoff_secs * 1_000_000 + 5_000_000;
        let outside = 1_723_000_000_000_000;
        for (id, ts) in [("inside", inside), ("outside", outside)] {
            sqlx::query(
                "INSERT INTO events (id, type, timestamp, source, pid, origin, payload) \
                 VALUES (?, 'file.opened', ?, 'test', 1, 'system:test', X'')",
            )
            .bind(id)
            .bind(ts)
            .execute(&pool)
            .await
            .expect("seed");
        }

        let deleted = delete_raw_events_since(&pool, cutoff_secs).await.expect("delete");
        assert_eq!(deleted, 1, "only the event inside the range");

        let left: Vec<String> = sqlx::query_scalar("SELECT id FROM events ORDER BY id")
            .fetch_all(&pool)
            .await
            .expect("read back");
        assert_eq!(left, vec!["outside".to_string()], "the older event stands");
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
                "CREATE (:File {id:'/w/kept.rs', path:'/w/kept.rs', last_accessed: 500000000})".into(),
                // No project, accessed inside the range: nothing but the
                // observation, so it goes whole.
                "CREATE (:File {id:'/tmp/seen.rs', path:'/tmp/seen.rs', last_accessed: 600000000})"
                    .into(),
                // Before the range: untouched, the proof the cut-off is real.
                "CREATE (:File {id:'/w/old.rs', path:'/w/old.rs', last_accessed: 100000000})".into(),
                "CREATE (:App {id:'editor'})".into(),
                "CREATE (:Event {id:'e-new', type:'window.focused', timestamp: 700000000})".into(),
                "CREATE (:Event {id:'e-old', type:'window.focused', timestamp: 100000000})".into(),
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
        assert_eq!(count("MATCH (e:Event) WHERE e.timestamp >= 400000000 RETURN count(e)").await, 0);
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

    /// The audit obligation, pinned where it can be broken.
    ///
    /// The ruling asks that an audit entry referring to a deleted node still
    /// reads. It does today, and the reason is narrow enough to be worth a test:
    /// the only node reference an audit entry carries is `project_id`
    /// (`StructuralView`; `node_types` holds TYPES, and the forensic tier holds
    /// stored strings), and this delete never removes a `Project`. Widen it to
    /// projects and the obligation stops holding for free - a reader that resolves
    /// `project_id` would then have to say the subject was deleted rather than
    /// break or drop the row.
    #[test]
    fn no_project_node_is_ever_deleted_here() {
        for s in deletion_statements(1) {
            let deletes = s.contains("DELETE");
            assert!(
                !(deletes && s.contains(":Project")),
                "audit entries reference projects by id; deleting one needs the \
                 dangling-reference half of the ruling first: {s}"
            );
        }
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

//! Deterministic, dated KG seed corpus for arlen-ui's KG-surface verification.
//!
//! arlen-ui cannot verify the file manager's KG surfaces (provenance-nav,
//! relationships as-of) because a fresh machine's graph store is empty and the
//! `files_*` commands are native-Tauri-only (vite-dev can't drive them). This
//! writes a fixed corpus - Projects + Files + **bitemporal** `FILE_PART_OF`
//! edges - into a graph store so a real daemon can answer those surfaces and
//! the integration tests have ground truth.
//!
//! The corpus is dated from FIXED absolute timestamps (never `now`), so it is
//! fully reproducible and an `as-of` read returns DIFFERENT membership at
//! different times: `moved.md` belonged to project Alpha early, then moved to
//! Beta. The base instant is safely in the past, so an "as of now" read sees
//! the live (open-interval) memberships.
//!
//! Idempotent: every write is a MERGE, so re-seeding converges. The caller
//! points `seed_corpus` at the ISOLATED dev runtime store, never the real
//! `~/.local/share/arlen/graph`.

use anyhow::Result;

use crate::graph::GraphHandle;

/// The base instant (epoch micros) the corpus is dated from. Fixed +
/// reproducible (~2023-11-14T22:13:20Z) and safely in the past, so an "as of
/// now" read sees the live edges.
pub const BASE_MICROS: i64 = 1_700_000_000_000_000;
const DAY_US: i64 = 86_400_000_000;

/// An as-of instant at which the moved file still belongs to Alpha (3 days in,
/// before the move at +7 days).
pub const ASOF_EARLY: i64 = BASE_MICROS + 3 * DAY_US;
/// An as-of instant at which the moved file belongs to Beta (10 days in, after
/// the move).
pub const ASOF_LATE: i64 = BASE_MICROS + 10 * DAY_US;

/// Project ids in the corpus.
pub const PROJECT_ALPHA: &str = "seed.project.alpha";
/// Project ids in the corpus.
pub const PROJECT_BETA: &str = "seed.project.beta";

/// The file that moves Alpha -> Beta (membership differs by as-of time).
pub const FILE_MOVED: &str = "/work/seed/alpha/moved.md";
/// A file always in Alpha.
pub const FILE_ALPHA_ONLY: &str = "/work/seed/alpha/stable.md";
/// A file always in Beta.
pub const FILE_BETA_ONLY: &str = "/work/seed/beta/notes.md";

/// The instant the move happens (Alpha membership closes, Beta opens).
const MOVE_MICROS: i64 = BASE_MICROS + 7 * DAY_US;

/// Write the deterministic dated corpus into `graph`. Idempotent (MERGE), so
/// re-running converges. All ids are fixed literals with no quote/backslash, so
/// no Cypher escaping is required.
pub async fn seed_corpus(graph: &GraphHandle) -> Result<()> {
    // Projects, both live (expired_at stays NULL).
    for (id, name, root) in [
        (PROJECT_ALPHA, "Alpha", "/work/seed/alpha"),
        (PROJECT_BETA, "Beta", "/work/seed/beta"),
    ] {
        graph
            .write(format!(
                "MERGE (p:Project {{id: '{id}'}})
                 SET p.name = '{name}', p.root_path = '{root}', p.status = 'active',
                     p.created_at = {BASE_MICROS}, p.promoted = true, p.inferred = false"
            ))
            .await?;
    }

    // Files (path-keyed: the File node id is its absolute path).
    for path in [FILE_MOVED, FILE_ALPHA_ONLY, FILE_BETA_ONLY] {
        graph
            .write(format!(
                "MERGE (f:File {{id: '{path}'}})
                 SET f.path = '{path}', f.last_accessed = {BASE_MICROS}"
            ))
            .await?;
    }

    // Bitemporal memberships. `moved.md` is in Alpha for the first week, then
    // Beta from the move onward (open).
    part_of(graph, FILE_MOVED, PROJECT_ALPHA, BASE_MICROS, Some(MOVE_MICROS)).await?;
    part_of(graph, FILE_MOVED, PROJECT_BETA, MOVE_MICROS, None).await?;
    // The stable files never move.
    part_of(graph, FILE_ALPHA_ONLY, PROJECT_ALPHA, BASE_MICROS, None).await?;
    part_of(graph, FILE_BETA_ONLY, PROJECT_BETA, BASE_MICROS, None).await?;

    Ok(())
}

/// MERGE one bitemporal `FILE_PART_OF` edge with explicit stamps. `invalid_at`
/// `None` leaves the membership open (live); `Some(t)` closes it at `t`.
async fn part_of(
    graph: &GraphHandle,
    file: &str,
    project: &str,
    valid_at: i64,
    invalid_at: Option<i64>,
) -> Result<()> {
    let invalid = match invalid_at {
        Some(t) => t.to_string(),
        None => "NULL".to_string(),
    };
    graph
        .write(format!(
            "MATCH (f:File {{id: '{file}'}}), (p:Project {{id: '{project}'}})
             MERGE (f)-[r:FILE_PART_OF]->(p)
             SET r.valid_at = {valid_at}, r.invalid_at = {invalid},
                 r.created_at = {valid_at}, r.origin = 'seed'"
        ))
        .await?;
    Ok(())
}

/// Seed a single untagged fixture: a `Project` rooted at `project_root` and a
/// `File` at `file_path` (its node id, by the path-keying convention) that lies
/// under that root, with NO `FILE_PART_OF` edge. Both paths must be REAL,
/// existing on-disk paths: the agent's predict-before-act step canonicalizes a
/// `PathUnderField` operand through the filesystem, so a fictional path resolves
/// to nothing and the proposal never proves. Kept out of the fixed corpus so
/// exactly one untagged file exists when an integration scenario asks for it
/// (the manual `tag-untagged-files` workflow discovers the first untagged file,
/// so two would race). Idempotent (MERGE).
pub async fn seed_untagged_host(
    graph: &GraphHandle,
    project_id: &str,
    project_root: &str,
    file_path: &str,
) -> Result<()> {
    // These come from a caller (a test's temp paths), not fixed literals, so
    // refuse a quote/backslash rather than risk breaking out of the literal.
    for s in [project_id, project_root, file_path] {
        if s.contains('\'') || s.contains('\\') {
            anyhow::bail!("untagged-host seed inputs must not contain quotes or backslashes");
        }
    }
    graph
        .write(format!(
            "MERGE (p:Project {{id: '{project_id}'}})
             SET p.name = 'Untagged Host', p.root_path = '{project_root}', p.status = 'active',
                 p.created_at = {BASE_MICROS}, p.promoted = true, p.inferred = false"
        ))
        .await?;
    graph
        .write(format!(
            "MERGE (f:File {{id: '{file_path}'}})
             SET f.path = '{file_path}', f.last_accessed = {BASE_MICROS}"
        ))
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The as-of membership of the moved file differs by time: Alpha early,
    /// Beta late - the property arlen-ui's relationships-as-of surface needs.
    /// Mirrors the file manager's `file_part_of_as_of` predicate.
    async fn project_at(graph: &GraphHandle, file: &str, t: i64) -> Vec<String> {
        let cypher = format!(
            "MATCH (f:File {{id: '{file}'}})-[r:FILE_PART_OF]->(p:Project)
             WHERE r.valid_at <= {t} AND (r.invalid_at IS NULL OR r.invalid_at > {t})
               AND r.created_at <= {t} AND (p.expired_at IS NULL OR p.expired_at > {t})
             RETURN p.id AS id ORDER BY p.id"
        );
        let rows = graph.query_rows(cypher).await.expect("as-of query");
        rows.rows
            .iter()
            .filter_map(|r| r.first().map(|c| c.as_str().to_string()))
            .collect()
    }

    #[tokio::test]
    async fn moved_file_membership_differs_by_as_of_time() {
        let dir = tempfile::tempdir().unwrap();
        let graph = crate::graph::spawn(dir.path().join("graph").to_str().unwrap()).unwrap();
        seed_corpus(&graph).await.unwrap();

        // Early: still in Alpha. Late: in Beta. The whole point of the corpus.
        assert_eq!(project_at(&graph, FILE_MOVED, ASOF_EARLY).await, vec![PROJECT_ALPHA]);
        assert_eq!(project_at(&graph, FILE_MOVED, ASOF_LATE).await, vec![PROJECT_BETA]);

        // The stable files never move.
        assert_eq!(project_at(&graph, FILE_ALPHA_ONLY, ASOF_LATE).await, vec![PROJECT_ALPHA]);
        assert_eq!(project_at(&graph, FILE_BETA_ONLY, ASOF_EARLY).await, vec![PROJECT_BETA]);
    }

    #[tokio::test]
    async fn seed_untagged_host_writes_an_unlinked_file_under_a_real_root() {
        let dir = tempfile::tempdir().unwrap();
        let graph = crate::graph::spawn(dir.path().join("graph").to_str().unwrap()).unwrap();
        let root = dir.path().join("proj");
        let file = root.join("new.rs");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(&file, b"// new\n").unwrap();

        seed_untagged_host(
            &graph,
            "seed.project.untagged-host",
            root.to_str().unwrap(),
            file.to_str().unwrap(),
        )
        .await
        .unwrap();

        // The File node exists, and it has no FILE_PART_OF membership at any time.
        let exists = graph
            .query_rows(format!(
                "MATCH (f:File {{id: '{}'}}) RETURN f.id AS id",
                file.to_str().unwrap()
            ))
            .await
            .unwrap();
        assert_eq!(exists.rows.len(), 1, "the untagged File node exists");
        assert!(
            project_at(&graph, file.to_str().unwrap(), ASOF_LATE).await.is_empty(),
            "the untagged file has no project membership"
        );
    }

    #[tokio::test]
    async fn seed_untagged_host_rejects_quoted_inputs() {
        let dir = tempfile::tempdir().unwrap();
        let graph = crate::graph::spawn(dir.path().join("graph").to_str().unwrap()).unwrap();
        assert!(seed_untagged_host(&graph, "p", "/r", "/r/a'b.rs").await.is_err());
    }

    #[tokio::test]
    async fn seed_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let graph = crate::graph::spawn(dir.path().join("graph").to_str().unwrap()).unwrap();
        seed_corpus(&graph).await.unwrap();
        seed_corpus(&graph).await.unwrap();
        // Still exactly one membership per (file, project) - the re-seed merged.
        assert_eq!(project_at(&graph, FILE_MOVED, ASOF_LATE).await, vec![PROJECT_BETA]);
    }
}

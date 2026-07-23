//! Git commit ingestion into the reserved `Commit` node type (foreign-app-bridges.md
//! §13, the curated git bridge). Unlike a third-party app bridge, git populates a
//! RESERVED system-level node table (`graph.rs` reserves `Commit`/`Branch` from the
//! start), which a delegated namespace grant cannot write - so this is a FIRST-PARTY
//! producer inside the daemon, writing through the internal [`GraphHandle`] the same
//! way the promotion pipeline does, not through the third-party write socket.
//!
//! This slice ingests a repository's commits. The commit SHA is the node id, so the
//! MERGE is idempotent: re-ingesting a repo strengthens existing nodes rather than
//! duplicating them, and picks up new commits. Repository discovery (which repos to
//! ingest, from the project watch dirs) and the commit-parent DAG edges + `Branch`
//! heads are follow-ups; the commit nodes are the foundation they attach to.

// No live caller yet: the daemon's repo-discovery + scheduling wire `ingest_repo`
// in a follow-up. Built as the mechanism first (the canary / typed-read precedent).
#![allow(dead_code)]

use std::path::Path;

use anyhow::Result;

use crate::graph::GraphHandle;
use crate::utils::escape_cypher;

/// A commit as its reserved `Commit` node stores it: the SHA id plus the four
/// conventional columns (`message`, `author`, `author_email`, `committed_at`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitRow {
    /// The full commit SHA - the node id, so MERGE dedups a re-ingested commit.
    pub id: String,
    /// The commit subject (first line of the message).
    pub message: String,
    /// The author's name.
    pub author: String,
    /// The author's email.
    pub author_email: String,
    /// The commit time, Unix seconds.
    pub committed_at: i64,
}

/// The unit separator git writes between `--format` fields. It never appears in a
/// commit's own text, so splitting on it is unambiguous even when a subject
/// contains commas, quotes or tabs.
const FIELD_SEP: char = '\u{1f}';

/// The `git log --format` spec matching [`CommitRow`]: SHA, subject, author name,
/// author email, committer Unix time, unit-separated.
pub const LOG_FORMAT: &str = "%H%x1f%s%x1f%an%x1f%ae%x1f%ct";

/// Parse `git log --format` output (one commit per line, [`FIELD_SEP`]-separated
/// fields) into commit rows. A line with too few fields, an empty SHA, or a
/// non-numeric timestamp is skipped rather than guessed at, so a malformed line
/// never fabricates a commit. Pure, so the parse is tested without invoking git.
pub fn parse_git_log(output: &str) -> Vec<CommitRow> {
    output.lines().filter_map(parse_line).collect()
}

fn parse_line(line: &str) -> Option<CommitRow> {
    let mut f = line.split(FIELD_SEP);
    let id = f.next()?.trim().to_string();
    let message = f.next()?.to_string();
    let author = f.next()?.to_string();
    let author_email = f.next()?.to_string();
    let committed_at = f.next()?.trim().parse::<i64>().ok()?;
    if id.is_empty() {
        return None;
    }
    Some(CommitRow {
        id,
        message,
        author,
        author_email,
        committed_at,
    })
}

/// Read the most recent `max` commits of the repository at `repo`. Shells out to
/// `git log`; a path that is not a repo, an empty repo, or a missing `git` binary
/// yields no commits rather than an error, since a broken repo must not fail the
/// whole ingestion pass.
pub fn read_commits(repo: &Path, max: usize) -> Vec<CommitRow> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .arg("log")
        .arg(format!("-{max}"))
        .arg(format!("--format={LOG_FORMAT}"))
        .output();
    match output {
        Ok(o) if o.status.success() => parse_git_log(&String::from_utf8_lossy(&o.stdout)),
        _ => Vec::new(),
    }
}

/// MERGE each commit as a `Commit` node. Idempotent on the SHA id: a commit
/// already present is updated in place, so re-ingestion never duplicates. Each
/// string field is escaped for single-quoted Cypher interpolation, the same as
/// the promotion pipeline's writes; the timestamp is a bare integer.
pub async fn ingest_commits(graph: &GraphHandle, commits: &[CommitRow]) -> Result<usize> {
    for c in commits {
        let id = escape_cypher(&c.id);
        let message = escape_cypher(&c.message);
        let author = escape_cypher(&c.author);
        let email = escape_cypher(&c.author_email);
        graph
            .write(format!(
                "MERGE (c:Commit {{id: '{id}'}}) \
                 SET c.message = '{message}', c.author = '{author}', \
                 c.author_email = '{email}', c.committed_at = {}",
                c.committed_at
            ))
            .await?;
    }
    Ok(commits.len())
}

/// Ingest a repository's commits into `Commit` nodes: read, then MERGE. Returns
/// the number of commits ingested.
pub async fn ingest_repo(graph: &GraphHandle, repo: &Path, max: usize) -> Result<usize> {
    let commits = read_commits(repo, max);
    ingest_commits(graph, &commits).await?;
    Ok(commits.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_git_log_format_field_by_field() {
        // A real `git log --format=%H%x1f%s%x1f%an%x1f%ae%x1f%ct` line, unit-
        // separated. A subject with a comma and a quote must survive intact,
        // which the separator (never in commit text) guarantees.
        let out = "abc123\u{1f}fix: the bug, \"finally\"\u{1f}Tim\u{1f}tim@x.org\u{1f}1700000000\n\
                   def456\u{1f}initial commit\u{1f}Ada\u{1f}ada@x.org\u{1f}1699999999\n";
        let rows = parse_git_log(out);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].id, "abc123");
        assert_eq!(rows[0].message, "fix: the bug, \"finally\"");
        assert_eq!(rows[0].author, "Tim");
        assert_eq!(rows[0].author_email, "tim@x.org");
        assert_eq!(rows[0].committed_at, 1700000000);
        assert_eq!(rows[1].id, "def456");
    }

    #[test]
    fn a_malformed_line_is_skipped_not_guessed() {
        // Too few fields, an empty SHA, and a non-numeric time each drop the line
        // rather than fabricate a partial commit.
        let out = "just-a-hash\u{1f}only two\n\
                   \u{1f}empty sha\u{1f}A\u{1f}a@x\u{1f}1\n\
                   ok123\u{1f}good\u{1f}A\u{1f}a@x\u{1f}notanumber\n\
                   real\u{1f}good\u{1f}A\u{1f}a@x\u{1f}5\n";
        let rows = parse_git_log(out);
        assert_eq!(rows.len(), 1, "only the well-formed line survives");
        assert_eq!(rows[0].id, "real");
    }

    #[test]
    fn empty_output_is_no_commits() {
        assert!(parse_git_log("").is_empty());
        assert!(parse_git_log("\n\n").is_empty());
    }

    #[tokio::test]
    async fn ingest_writes_a_deduplicated_commit_node() {
        // Real graph: ingesting the same commit twice leaves ONE node (MERGE on
        // the SHA), and the fields round-trip. This proves the reserved Commit
        // table accepts a first-party write through the internal handle.
        let tmp = tempfile::TempDir::new().unwrap();
        let graph = crate::graph::spawn(tmp.path().join("g").to_str().unwrap()).unwrap();
        let commits = vec![CommitRow {
            id: "sha-1".into(),
            message: "hello".into(),
            author: "Tim".into(),
            author_email: "tim@x.org".into(),
            committed_at: 42,
        }];
        ingest_commits(&graph, &commits).await.unwrap();
        // Re-ingest: MERGE must not duplicate.
        ingest_commits(&graph, &commits).await.unwrap();

        let rows = graph
            .query_rows("MATCH (c:Commit {id: 'sha-1'}) RETURN c.message AS m, c.author AS a".into())
            .await
            .unwrap();
        assert_eq!(rows.rows.len(), 1, "one commit node, not two");
        assert_eq!(rows.rows[0][0].as_str(), "hello", "the message round-trips");
    }
}

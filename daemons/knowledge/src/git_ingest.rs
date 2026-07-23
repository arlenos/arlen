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

use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use anyhow::Result;
use tokio::time;
use tracing::{info, warn};

use crate::graph::GraphHandle;
use crate::utils::escape_cypher;

/// How often the git-ingestion pass runs. Git history changes far slower than the
/// file/window event stream, so a coarse interval keeps the `git` shell-outs off
/// the hot path while still picking up new commits within minutes.
const GIT_INGEST_INTERVAL: Duration = Duration::from_secs(300);

/// How many recent commits to ingest per repository each pass. Bounds the `git log`
/// cost on a large history; deep-history backfill (a wider window) is a follow-up.
const MAX_COMMITS_PER_REPO: usize = 500;

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
    /// The parent commit SHAs (empty for the root, two for a merge). Written as
    /// `PARENT_OF` edges when both endpoints are ingested nodes.
    pub parents: Vec<String>,
}

/// The unit separator git writes between `--format` fields. It never appears in a
/// commit's own text, so splitting on it is unambiguous even when a subject
/// contains commas, quotes or tabs.
const FIELD_SEP: char = '\u{1f}';

/// The `git log --format` spec matching [`CommitRow`]: SHA, subject, author name,
/// author email, committer Unix time, unit-separated.
pub const LOG_FORMAT: &str = "%H%x1f%s%x1f%an%x1f%ae%x1f%ct%x1f%P";

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
    // `%P` is a space-separated parent list, empty for the root commit. A missing
    // field (an older format) is treated as no parents rather than a parse error.
    let parents = f
        .next()
        .unwrap_or("")
        .split_whitespace()
        .map(str::to_string)
        .collect();
    if id.is_empty() {
        return None;
    }
    Some(CommitRow {
        id,
        message,
        author,
        author_email,
        committed_at,
        parents,
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
        // The DAG edge to each parent. MATCH both commits so an edge is created
        // only when the parent is also an ingested node (no dangling edge); it
        // fills in as more history ingests. MERGE keeps re-ingestion idempotent.
        for parent in &c.parents {
            let parent = escape_cypher(parent);
            graph
                .write(format!(
                    "MATCH (c:Commit {{id: '{id}'}}), (p:Commit {{id: '{parent}'}}) \
                     MERGE (c)-[:PARENT_OF]->(p)"
                ))
                .await?;
        }
    }
    Ok(commits.len())
}

/// A local branch as its reserved `Branch` node stores it: the short name and the
/// head commit SHA. The node id is repo-scoped (`<repo>::<name>`) so a `main` in
/// two repos are distinct nodes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchRow {
    /// The short branch name, e.g. `main` or `feature/x`.
    pub name: String,
    /// The head commit SHA the branch points at.
    pub head: String,
}

/// The `git for-each-ref --format` spec matching [`BranchRow`]: short name and the
/// head object SHA, unit-separated. Restricted to `refs/heads/` (local branches),
/// not remotes or tags.
pub const BRANCH_FORMAT: &str = "%(refname:short)%x1f%(objectname)";

/// Parse `git for-each-ref --format` output (one branch per line,
/// [`FIELD_SEP`]-separated) into branch rows. A line missing a field or with an
/// empty name/head is skipped rather than guessed at. Pure, tested without git.
pub fn parse_branches(output: &str) -> Vec<BranchRow> {
    output
        .lines()
        .filter_map(|line| {
            let mut f = line.split(FIELD_SEP);
            let name = f.next()?.trim().to_string();
            let head = f.next()?.trim().to_string();
            (!name.is_empty() && !head.is_empty()).then_some(BranchRow { name, head })
        })
        .collect()
}

/// Read the repository's local branches. Shells out to `git for-each-ref
/// refs/heads/`; a non-repo or missing `git` yields no branches rather than an
/// error, matching [`read_commits`].
pub fn read_branches(repo: &Path) -> Vec<BranchRow> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .arg("for-each-ref")
        .arg(format!("--format={BRANCH_FORMAT}"))
        .arg("refs/heads/")
        .output();
    match output {
        Ok(o) if o.status.success() => parse_branches(&String::from_utf8_lossy(&o.stdout)),
        _ => Vec::new(),
    }
}

/// The repo-scoped `Branch` node id: `<repo_key>::<name>`, so a branch name is
/// unique across repositories.
fn branch_id(repo_key: &str, name: &str) -> String {
    format!("{repo_key}::{name}")
}

/// MERGE each branch as a `Branch` node (repo-scoped id) and link it to its head
/// commit via `HEAD_AT`. The head edge is `MATCH`-guarded on the commit, so it is
/// created only when the head is an ingested node (no dangling edge); MERGE keeps
/// re-ingestion idempotent. Returns the number of branches written.
pub async fn ingest_branches(
    graph: &GraphHandle,
    repo_key: &str,
    branches: &[BranchRow],
) -> Result<usize> {
    for b in branches {
        let id = escape_cypher(&branch_id(repo_key, &b.name));
        let name = escape_cypher(&b.name);
        let head = escape_cypher(&b.head);
        graph
            .write(format!(
                "MERGE (b:Branch {{id: '{id}'}}) \
                 SET b.name = '{name}', b.head = '{head}'"
            ))
            .await?;
        graph
            .write(format!(
                "MATCH (b:Branch {{id: '{id}'}}), (c:Commit {{id: '{head}'}}) \
                 MERGE (b)-[:HEAD_AT]->(c)"
            ))
            .await?;
    }
    Ok(branches.len())
}

/// The id of the `Project` whose `root_path` is this repository, if the daemon's
/// project detection has one. A git repo is a project (the `.git` signal), so the
/// grouping reuses the Project node the daemon already maintains rather than
/// minting a git-specific one. Matches the stored `root_path` against the repo's
/// canonical path so a trailing slash or a symlink does not miss it; an
/// uncanonicalizable path falls back to the literal form.
pub async fn project_id_for_repo(graph: &GraphHandle, repo: &Path) -> Result<Option<String>> {
    let canonical = std::fs::canonicalize(repo).unwrap_or_else(|_| repo.to_path_buf());
    let path = escape_cypher(&canonical.to_string_lossy());
    let rows = graph
        .query_rows(format!(
            "MATCH (p:Project {{root_path: '{path}'}}) RETURN p.id AS id LIMIT 1"
        ))
        .await?;
    Ok(rows
        .rows
        .first()
        .and_then(|r| r.first())
        .map(|c| c.as_str().to_string())
        .filter(|s| !s.is_empty()))
}

/// Link each commit to its repository's `Project` via `COMMITTED_IN`. MATCH both
/// endpoints so the edge is created only when the Project node exists (no dangling
/// edge); MERGE keeps re-linking idempotent.
pub async fn link_commits_to_project(
    graph: &GraphHandle,
    commit_ids: &[String],
    project_id: &str,
) -> Result<()> {
    let project = escape_cypher(project_id);
    for id in commit_ids {
        let id = escape_cypher(id);
        graph
            .write(format!(
                "MATCH (c:Commit {{id: '{id}'}}), (p:Project {{id: '{project}'}}) \
                 MERGE (c)-[:COMMITTED_IN]->(p)"
            ))
            .await?;
    }
    Ok(())
}

/// Ingest a repository's git state: MERGE the recent commits and their DAG, group
/// them under the repository's `Project` (when the daemon has detected one), and
/// MERGE the local branch heads. The repo key for branch ids is the canonical repo
/// path so branch nodes are stable and unique across repositories. Returns the
/// number of commits ingested.
pub async fn ingest_repo(graph: &GraphHandle, repo: &Path, max: usize) -> Result<usize> {
    let commits = read_commits(repo, max);
    ingest_commits(graph, &commits).await?;
    if let Some(project_id) = project_id_for_repo(graph, repo).await? {
        let ids: Vec<String> = commits.iter().map(|c| c.id.clone()).collect();
        link_commits_to_project(graph, &ids, &project_id).await?;
    }
    let repo_key = std::fs::canonicalize(repo)
        .unwrap_or_else(|_| repo.to_path_buf())
        .to_string_lossy()
        .into_owned();
    ingest_branches(graph, &repo_key, &read_branches(repo)).await?;
    Ok(commits.len())
}

/// The `root_path` of every live (non-expired) project. A git repo is a project
/// (the `.git` detection signal), so the daemon's own project set is the repo
/// discovery source; no separate git-repo scan is needed.
async fn live_project_roots(graph: &GraphHandle) -> Result<Vec<String>> {
    let rows = graph
        .query_rows("MATCH (p:Project) WHERE p.expired_at IS NULL RETURN p.root_path AS r".into())
        .await?;
    Ok(rows
        .rows
        .iter()
        .filter_map(|r| r.first())
        .map(|c| c.as_str().to_string())
        .filter(|s| !s.is_empty())
        .collect())
}

/// The current HEAD commit SHA of a repository, or `None` for a non-repo. Cheap
/// (`git rev-parse`), so a pass reads it for every project to decide whether the
/// repo has moved since last time. Doubles as the git-repo gate: a non-repo (no
/// HEAD) is skipped entirely, sparing it the ingest queries and shell-outs.
fn repo_head(repo: &Path) -> Option<String> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .arg("rev-parse")
        .arg("HEAD")
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let head = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!head.is_empty()).then_some(head)
}

/// Ingest every live project's git state in one pass. Returns `(repos, commits)`:
/// the number of git repos ingested this pass and the total commits. `seen` caches
/// each repo's last-ingested HEAD, so a repo whose HEAD has not moved is skipped
/// (no graph writes) - the steady-state cost is one `git rev-parse` per project. A
/// non-git project is skipped; a failure on one repo is logged, not fatal, and its
/// HEAD is not cached so the next pass retries.
async fn ingest_all_projects(
    graph: &GraphHandle,
    seen: &mut HashMap<String, String>,
) -> Result<(usize, usize)> {
    let roots = live_project_roots(graph).await?;
    let mut repos = 0;
    let mut commits = 0;
    for root in roots {
        let Some(head) = repo_head(Path::new(&root)) else {
            continue; // not a git repo
        };
        if seen.get(&root).is_some_and(|h| h == &head) {
            continue; // HEAD unchanged since the last pass
        }
        match ingest_repo(graph, Path::new(&root), MAX_COMMITS_PER_REPO).await {
            Ok(n) => {
                seen.insert(root.clone(), head);
                if n > 0 {
                    repos += 1;
                    commits += n;
                }
            }
            Err(e) => warn!(root = %root, error = %e, "git ingestion for a project failed"),
        }
    }
    Ok((repos, commits))
}

/// Run the git-ingestion pass forever, waking every [`GIT_INGEST_INTERVAL`]. Each
/// pass discovers the live projects and ingests each one's recent commits, DAG,
/// branch heads, and project grouping. This is the live caller for the ingestion
/// mechanism; it never returns under normal operation. The per-repo HEAD cache is
/// in-memory, so a restart triggers one full pass and then settles to skip-if-
/// unchanged.
pub async fn run(graph: GraphHandle) -> Result<()> {
    let mut seen: HashMap<String, String> = HashMap::new();
    let mut interval = time::interval(GIT_INGEST_INTERVAL);
    // Skip the first immediate tick so the project watcher has a chance to detect
    // projects before the first pass runs.
    interval.tick().await;
    loop {
        interval.tick().await;
        match ingest_all_projects(&graph, &mut seen).await {
            Ok((repos, commits)) if repos > 0 => {
                info!(repos, commits, "git ingestion pass complete");
            }
            Ok(_) => {}
            Err(e) => warn!(error = %e, "git ingestion pass failed"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_git_log_format_field_by_field() {
        // A real `git log --format=%H%x1f%s%x1f%an%x1f%ae%x1f%ct` line, unit-
        // separated. A subject with a comma and a quote must survive intact,
        // which the separator (never in commit text) guarantees.
        let out = "abc123\u{1f}fix: the bug, \"finally\"\u{1f}Tim\u{1f}tim@x.org\u{1f}1700000000\u{1f}def456 aaa111\n\
                   def456\u{1f}initial commit\u{1f}Ada\u{1f}ada@x.org\u{1f}1699999999\u{1f}\n";
        let rows = parse_git_log(out);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].id, "abc123");
        assert_eq!(rows[0].message, "fix: the bug, \"finally\"");
        assert_eq!(rows[0].author, "Tim");
        assert_eq!(rows[0].author_email, "tim@x.org");
        assert_eq!(rows[0].committed_at, 1700000000);
        // A merge commit has two parents; the root has none.
        assert_eq!(rows[0].parents, vec!["def456".to_string(), "aaa111".to_string()]);
        assert_eq!(rows[1].id, "def456");
        assert!(rows[1].parents.is_empty(), "the root commit has no parent");
    }

    #[test]
    fn a_malformed_line_is_skipped_not_guessed() {
        // Too few fields, an empty SHA, and a non-numeric time each drop the line
        // rather than fabricate a partial commit.
        let out = "just-a-hash\u{1f}only two\n\
                   \u{1f}empty sha\u{1f}A\u{1f}a@x\u{1f}1\u{1f}\n\
                   ok123\u{1f}good\u{1f}A\u{1f}a@x\u{1f}notanumber\u{1f}\n\
                   real\u{1f}good\u{1f}A\u{1f}a@x\u{1f}5\u{1f}\n";
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
            parents: vec![],
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

    #[tokio::test]
    async fn a_parent_edge_links_two_ingested_commits() {
        // The DAG: a child PARENT_OF its parent, but only once BOTH are nodes.
        let tmp = tempfile::TempDir::new().unwrap();
        let graph = crate::graph::spawn(tmp.path().join("g").to_str().unwrap()).unwrap();
        let row = |id: &str, parents: Vec<String>| CommitRow {
            id: id.into(),
            message: "m".into(),
            author: "A".into(),
            author_email: "a@x".into(),
            committed_at: 1,
            parents,
        };
        // Ingest the child FIRST, whose parent is not yet a node: no edge is made.
        ingest_commits(&graph, &[row("child", vec!["parent".into()])]).await.unwrap();
        let before = graph
            .query_rows("MATCH (:Commit)-[:PARENT_OF]->(:Commit) RETURN 1 AS x".into())
            .await
            .unwrap();
        assert_eq!(before.rows.len(), 0, "no dangling edge to a missing parent");
        // Now ingest the parent AND re-ingest the child: the edge fills in, once.
        ingest_commits(&graph, &[row("parent", vec![])]).await.unwrap();
        ingest_commits(&graph, &[row("child", vec!["parent".into()])]).await.unwrap();
        ingest_commits(&graph, &[row("child", vec!["parent".into()])]).await.unwrap();
        let after = graph
            .query_rows("MATCH (c:Commit)-[:PARENT_OF]->(p:Commit) RETURN c.id AS c, p.id AS p".into())
            .await
            .unwrap();
        assert_eq!(after.rows.len(), 1, "exactly one edge, child -> parent, not duplicated");
        assert_eq!(after.rows[0][0].as_str(), "child");
        assert_eq!(after.rows[0][1].as_str(), "parent");
    }

    #[tokio::test]
    async fn commits_group_under_the_repos_project() {
        // The grouping: a commit COMMITTED_IN the Project whose root_path is the
        // repo, but only when that Project node exists (no dangling edge to a
        // repo the daemon has not detected).
        let tmp = tempfile::TempDir::new().unwrap();
        let graph = crate::graph::spawn(tmp.path().join("g").to_str().unwrap()).unwrap();
        ingest_commits(&graph, &[CommitRow {
            id: "sha-9".into(),
            message: "m".into(),
            author: "A".into(),
            author_email: "a@x".into(),
            committed_at: 1,
            parents: vec![],
        }])
        .await
        .unwrap();

        // No Project yet: linking to a missing project makes no edge.
        link_commits_to_project(&graph, &["sha-9".into()], "proj-1").await.unwrap();
        let none = graph
            .query_rows("MATCH (:Commit)-[:COMMITTED_IN]->(:Project) RETURN 1 AS x".into())
            .await
            .unwrap();
        assert_eq!(none.rows.len(), 0, "no edge to a project that does not exist");

        // Create the Project, then re-link twice: exactly one edge, idempotent.
        graph
            .write("CREATE (p:Project {id: 'proj-1', root_path: '/repo'})".into())
            .await
            .unwrap();
        link_commits_to_project(&graph, &["sha-9".into()], "proj-1").await.unwrap();
        link_commits_to_project(&graph, &["sha-9".into()], "proj-1").await.unwrap();
        let linked = graph
            .query_rows(
                "MATCH (c:Commit)-[:COMMITTED_IN]->(p:Project) RETURN c.id AS c, p.id AS p".into(),
            )
            .await
            .unwrap();
        assert_eq!(linked.rows.len(), 1, "exactly one grouping edge, not duplicated");
        assert_eq!(linked.rows[0][0].as_str(), "sha-9");
        assert_eq!(linked.rows[0][1].as_str(), "proj-1");
    }

    #[test]
    fn parses_branch_refs_field_by_field() {
        let out = "main\u{1f}abc123\n\
                   feature/x\u{1f}def456\n\
                   \u{1f}headless\n\
                   noname\u{1f}\n";
        let rows = parse_branches(out);
        // Only the two well-formed lines survive; an empty name or head drops.
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0], BranchRow { name: "main".into(), head: "abc123".into() });
        assert_eq!(rows[1], BranchRow { name: "feature/x".into(), head: "def456".into() });
    }

    #[tokio::test]
    async fn a_branch_links_to_its_head_commit_when_ingested() {
        // The Branch node is repo-scoped and its HEAD_AT edge fills in only once
        // the head commit is an ingested node.
        let tmp = tempfile::TempDir::new().unwrap();
        let graph = crate::graph::spawn(tmp.path().join("g").to_str().unwrap()).unwrap();
        let branches = vec![BranchRow { name: "main".into(), head: "sha-h".into() }];

        // Head commit not yet ingested: the Branch node exists but no HEAD_AT.
        ingest_branches(&graph, "/repo", &branches).await.unwrap();
        let node = graph
            .query_rows("MATCH (b:Branch {id: '/repo::main'}) RETURN b.name AS n, b.head AS h".into())
            .await
            .unwrap();
        assert_eq!(node.rows.len(), 1, "the repo-scoped branch node exists");
        assert_eq!(node.rows[0][0].as_str(), "main");
        assert_eq!(node.rows[0][1].as_str(), "sha-h");
        let dangling = graph
            .query_rows("MATCH (:Branch)-[:HEAD_AT]->(:Commit) RETURN 1 AS x".into())
            .await
            .unwrap();
        assert_eq!(dangling.rows.len(), 0, "no HEAD_AT to a missing head commit");

        // Ingest the head commit, re-ingest the branch twice: one HEAD_AT edge.
        ingest_commits(&graph, &[CommitRow {
            id: "sha-h".into(),
            message: "m".into(),
            author: "A".into(),
            author_email: "a@x".into(),
            committed_at: 1,
            parents: vec![],
        }])
        .await
        .unwrap();
        ingest_branches(&graph, "/repo", &branches).await.unwrap();
        ingest_branches(&graph, "/repo", &branches).await.unwrap();
        let edge = graph
            .query_rows(
                "MATCH (b:Branch)-[:HEAD_AT]->(c:Commit) RETURN b.id AS b, c.id AS c".into(),
            )
            .await
            .unwrap();
        assert_eq!(edge.rows.len(), 1, "exactly one head edge, not duplicated");
        assert_eq!(edge.rows[0][0].as_str(), "/repo::main");
        assert_eq!(edge.rows[0][1].as_str(), "sha-h");
    }

    /// Run git in `repo`, isolated from the developer's global config so the test
    /// is hermetic. Returns stdout trimmed.
    fn git(repo: &Path, args: &[&str]) -> String {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .env("GIT_AUTHOR_NAME", "T")
            .env("GIT_AUTHOR_EMAIL", "t@x")
            .env("GIT_COMMITTER_NAME", "T")
            .env("GIT_COMMITTER_EMAIL", "t@x")
            .output()
            .expect("git runs");
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    #[tokio::test]
    async fn a_pass_ingests_then_skips_an_unchanged_head_then_reingests() {
        // End-to-end: a real repo, discovered as a Project, ingested by the pass;
        // a second pass with an unmoved HEAD does no work; a new commit re-ingests.
        let repo = tempfile::TempDir::new().unwrap();
        git(repo.path(), &["init", "-q", "-b", "main"]);
        std::fs::write(repo.path().join("a.txt"), "1").unwrap();
        git(repo.path(), &["add", "."]);
        git(repo.path(), &["commit", "-q", "-m", "first"]);
        let root = std::fs::canonicalize(repo.path())
            .unwrap()
            .to_string_lossy()
            .into_owned();

        let gtmp = tempfile::TempDir::new().unwrap();
        let graph = crate::graph::spawn(gtmp.path().join("g").to_str().unwrap()).unwrap();
        // The daemon detected this repo as a project.
        graph
            .write(format!(
                "CREATE (p:Project {{id: 'proj', root_path: '{}'}})",
                escape_cypher(&root)
            ))
            .await
            .unwrap();

        let mut seen = HashMap::new();
        // First pass: one repo, one commit, grouped under the project.
        let (repos, commits) = ingest_all_projects(&graph, &mut seen).await.unwrap();
        assert_eq!((repos, commits), (1, 1), "the repo is ingested on the first pass");
        let grouped = graph
            .query_rows("MATCH (:Commit)-[:COMMITTED_IN]->(:Project) RETURN 1 AS x".into())
            .await
            .unwrap();
        assert_eq!(grouped.rows.len(), 1, "the commit is grouped under its project");

        // Second pass, HEAD unmoved: skipped, no work reported.
        let again = ingest_all_projects(&graph, &mut seen).await.unwrap();
        assert_eq!(again, (0, 0), "an unchanged HEAD is skipped");

        // A new commit moves HEAD: the next pass re-ingests.
        std::fs::write(repo.path().join("b.txt"), "2").unwrap();
        git(repo.path(), &["add", "."]);
        git(repo.path(), &["commit", "-q", "-m", "second"]);
        let (repos3, commits3) = ingest_all_projects(&graph, &mut seen).await.unwrap();
        assert_eq!(repos3, 1, "the moved HEAD is re-ingested");
        assert_eq!(commits3, 2, "both commits are read in the re-ingest window");
        let n = graph
            .query_rows("MATCH (c:Commit) RETURN c.id AS id".into())
            .await
            .unwrap();
        assert_eq!(n.rows.len(), 2, "two commit nodes total, deduplicated");
    }
}

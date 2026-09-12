//! Project inference by co-occurrence clustering (kg-richness-plan.md Thrust 1,
//! foundation §4.2: "a background inference pass groups co-occurrence clusters
//! into candidate Project nodes"). The pure algorithm core: given file accesses
//! tagged with the session they occurred in, group files that repeatedly appear
//! together across sessions into candidate clusters a background pass can
//! promote to inferred Project nodes. This densifies the graph far beyond the
//! single-session `auto_promote_threshold` heuristic, which only ever sees one
//! session at a time.
//!
//! The deterministic clustering is unit-tested here; [`run`] drives it as a
//! periodic background pass over the live graph.

use crate::fuse::longest_common_dir;
use crate::graph::GraphHandle;
use crate::project::signals::SignalDetector;
use crate::project::store::{Project, ProjectStore};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use tracing::{info, warn};

/// One observed file access, tagged with the session it happened in. The
/// session is the co-occurrence unit (files touched in the same session are
/// candidates for belonging together); the timestamp window is the caller's to
/// pre-filter, so this core stays a pure set operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileAccess {
    /// The file node id (its path id in the graph).
    pub file_id: String,
    /// The session the access occurred in.
    pub session_id: String,
}

/// Tunables for the clustering pass.
#[derive(Debug, Clone, Copy)]
pub struct ClusterParams {
    /// How many distinct sessions two files must SHARE before they are linked.
    /// 1 = "ever seen together"; higher demands a repeated pattern (less noise).
    pub min_cooccurrence: u32,
    /// The smallest cluster worth a candidate Project node. A lone file is not a
    /// project; mirrors the spirit of the existing 3-file auto-promote.
    pub min_cluster_size: usize,
}

impl Default for ClusterParams {
    fn default() -> Self {
        // Repeated togetherness (>=2 shared sessions) and >=3 files: deliberately
        // conservative so a one-off coincidental co-open does not mint a project.
        ClusterParams {
            min_cooccurrence: 2,
            min_cluster_size: 3,
        }
    }
}

/// A candidate Project the pass inferred: the set of files that cluster
/// together. Files are sorted so the output is deterministic (the content-
/// addressed identity + stable tests rest on it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateCluster {
    /// The clustered file ids, sorted.
    pub files: Vec<String>,
}

/// Disjoint-set (union-find) over a dense index space, with path-halving + union
/// by size. Small + self-contained so the crate pulls no extra dependency.
struct UnionFind {
    parent: Vec<usize>,
    size: Vec<usize>,
}

impl UnionFind {
    fn new(n: usize) -> Self {
        UnionFind {
            parent: (0..n).collect(),
            size: vec![1; n],
        }
    }

    fn find(&mut self, mut x: usize) -> usize {
        while self.parent[x] != x {
            self.parent[x] = self.parent[self.parent[x]]; // path halving
            x = self.parent[x];
        }
        x
    }

    fn union(&mut self, a: usize, b: usize) {
        let (mut ra, mut rb) = (self.find(a), self.find(b));
        if ra == rb {
            return;
        }
        if self.size[ra] < self.size[rb] {
            std::mem::swap(&mut ra, &mut rb);
        }
        self.parent[rb] = ra;
        self.size[ra] += self.size[rb];
    }
}

/// Cluster files by cross-session co-occurrence.
///
/// Files that share at least `min_cooccurrence` sessions are linked; the
/// transitive closure of those links forms clusters (A-B and B-C cluster A, B
/// and C even if A and C never shared a session directly). Clusters smaller than
/// `min_cluster_size` are dropped. Output clusters are each sorted, and the list
/// is ordered by the cluster's first file, so the result is deterministic.
pub fn cluster_cooccurrence(accesses: &[FileAccess], params: ClusterParams) -> Vec<CandidateCluster> {
    // Index the distinct files into a dense id space for the union-find.
    let mut index: HashMap<&str, usize> = HashMap::new();
    let mut files: Vec<&str> = Vec::new();
    for a in accesses {
        if !index.contains_key(a.file_id.as_str()) {
            index.insert(a.file_id.as_str(), files.len());
            files.push(a.file_id.as_str());
        }
    }
    if files.is_empty() {
        return Vec::new();
    }

    // The set of distinct files touched in each session (a file accessed twice
    // in a session co-occurs with the rest once, not twice).
    let mut by_session: BTreeMap<&str, BTreeSet<usize>> = BTreeMap::new();
    for a in accesses {
        let fi = index[a.file_id.as_str()];
        by_session.entry(a.session_id.as_str()).or_default().insert(fi);
    }

    // Count, per unordered file pair, how many sessions they shared.
    let mut pair_sessions: HashMap<(usize, usize), u32> = HashMap::new();
    for members in by_session.values() {
        let m: Vec<usize> = members.iter().copied().collect();
        for i in 0..m.len() {
            for j in (i + 1)..m.len() {
                *pair_sessions.entry((m[i], m[j])).or_insert(0) += 1;
            }
        }
    }

    // Link the pairs that share enough sessions.
    let mut uf = UnionFind::new(files.len());
    for (&(a, b), &count) in &pair_sessions {
        if count >= params.min_cooccurrence {
            uf.union(a, b);
        }
    }

    // Collect components, keep the ones big enough, sort for determinism.
    let mut components: BTreeMap<usize, Vec<String>> = BTreeMap::new();
    for (fi, &name) in files.iter().enumerate() {
        let root = uf.find(fi);
        components.entry(root).or_default().push(name.to_string());
    }
    let mut clusters: Vec<CandidateCluster> = components
        .into_values()
        .filter(|c| c.len() >= params.min_cluster_size)
        .map(|mut files| {
            files.sort();
            CandidateCluster { files }
        })
        .collect();
    clusters.sort_by(|a, b| a.files.cmp(&b.files));
    clusters
}

/// Read every (file, session) access pair from the graph's `ACCESSED_IN` edges.
///
/// This is the co-occurrence input the pure [`cluster_cooccurrence`] consumes: a
/// file linked to each session it was accessed in. Reads only ids (no content),
/// so it carries no hard-exclude surface; private/incognito sessions never
/// produced an `ACCESSED_IN` edge in the first place (excluded upstream at
/// promotion). Rows missing either id are skipped.
pub async fn collect_file_accesses(graph: &GraphHandle) -> anyhow::Result<Vec<FileAccess>> {
    let rs = graph
        .query_rows(
            "MATCH (f:File)-[:ACCESSED_IN]->(s:Session) RETURN f.id AS file, s.id AS session"
                .into(),
        )
        .await?;
    let mut out = Vec::with_capacity(rs.rows.len());
    for row in &rs.rows {
        let file = row.first().map(|v| v.as_str()).unwrap_or_default();
        let session = row.get(1).map(|v| v.as_str()).unwrap_or_default();
        if !file.is_empty() && !session.is_empty() {
            out.push(FileAccess {
                file_id: file.to_string(),
                session_id: session.to_string(),
            });
        }
    }
    Ok(out)
}

/// Infer candidate project clusters from the graph's access history (foundation
/// §4.2). Reads the cross-session access pattern from `ACCESSED_IN` and runs the
/// deterministic [`cluster_cooccurrence`]. [`materialize_clusters`] turns these
/// candidates into inferred `Project` nodes (deriving a root from the cluster's
/// common path prefix and deduping against files already in a project, so it does
/// not collide with the signal-detected projects the watcher mints); [`run`]
/// drives both as the periodic background pass.
pub async fn infer_clusters(
    graph: &GraphHandle,
    params: ClusterParams,
) -> anyhow::Result<Vec<CandidateCluster>> {
    let accesses = collect_file_accesses(graph).await?;
    Ok(cluster_cooccurrence(&accesses, params))
}

/// Inference confidence for a cooccurrence-derived project. Lower than a
/// filesystem-signal project (a `.git`/`.project` root is ground truth); this is
/// a behavioural guess from access patterns, surfaced as a low-confidence
/// inferred project the user (or a later signal) can confirm.
const COOCCURRENCE_CONFIDENCE: u8 = 50;

/// The fewest projects inside a directory that make it a CONTAINER of projects
/// rather than a project.
///
/// Two, and the rule is not a tuning knob (`project-system.md`, "A container of
/// projects is not a project"). The floor this replaced was a path-depth number,
/// three components, which minted `~/Repositories` as a project because the
/// number happened to be one too small on this machine - and raising it to four
/// is a guess the next machine disproves.
const CONTAINER_PROJECT_COUNT: usize = 2;

/// What [`materialize_clusters`] did, for the caller to log.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MaterializeStats {
    /// Inferred Project nodes minted this run.
    pub created: usize,
    /// Clusters skipped (no usable root, or a project already exists there).
    pub skipped: usize,
    /// FILE_PART_OF edges created (idempotent, so a re-run adds none).
    pub linked: usize,
}

/// The basename of a root path, the inferred project's display name.
fn project_name_from_root(root: &str) -> String {
    root.rsplit('/')
        .find(|s| !s.is_empty())
        .unwrap_or(root)
        .to_string()
}

/// Is `root` strictly inside the user's home?
///
/// The other half of the floor. A cluster whose files share only `/home/tim`,
/// `/home` or `/` has found the user, not a project, and the home directory is
/// the one boundary that does not move between machines the way a component
/// count does. `home` itself is refused, not only its ancestors: a project AT
/// your home directory is every file you own.
fn is_under_home(root: &str, home: &str) -> bool {
    let home = home.trim_end_matches('/');
    if home.is_empty() {
        return false;
    }
    root.strip_prefix(home).is_some_and(|rest| rest.starts_with('/') && rest.len() > 1)
}

/// Materialise inferred clusters as inferred `Project` nodes (foundation §4.2).
///
/// For each cluster the root is the files' longest common directory, and it must
/// clear two floors before anything is minted (`project-system.md`, ruled 12
/// September). It must be strictly INSIDE `home` - a cluster rooted at the home
/// directory or above has found the user, not a project - and it must not
/// already CONTAIN [`CONTAINER_PROJECT_COUNT`] known projects, because a
/// directory holding two projects is a container of projects and a container is
/// not a project. That pair is what refuses `~/Repositories`, and it stays true
/// whatever the clustering underneath becomes; the path-depth number it replaced
/// was a measurement of one machine.
///
/// If a project already exists at that root - a signal-detected one the watcher
/// minted, or one a prior pass created - the cluster is skipped, so this never
/// clobbers an existing project and a re-run is idempotent. Otherwise it mints a
/// low-confidence inferred project and links each cluster file via FILE_PART_OF
/// (an idempotent MERGE, so a file already in the project is not re-linked). A
/// file may co-belong to other projects; the dedup is on the project root, never
/// the file.
///
/// `home` is the user's home directory. An empty one mints NOTHING: without
/// knowing where home is, the first floor cannot be applied, and the honest
/// answer is to infer no projects rather than to infer them unbounded.
pub async fn materialize_clusters(
    store: &ProjectStore,
    clusters: &[CandidateCluster],
    home: &str,
) -> anyhow::Result<MaterializeStats> {
    let mut stats = MaterializeStats::default();
    for cluster in clusters {
        let refs: Vec<&str> = cluster.files.iter().map(|s| s.as_str()).collect();
        let root = longest_common_dir(&refs);
        if root.is_empty() || root == "/" || !is_under_home(&root, home) {
            stats.skipped += 1;
            continue;
        }
        if store.count_projects_under(&root).await? >= CONTAINER_PROJECT_COUNT {
            stats.skipped += 1;
            continue;
        }
        if store.get_by_root_path(&root).await?.is_some() {
            stats.skipped += 1;
            continue;
        }
        let project =
            Project::new_inferred(project_name_from_root(&root), root.clone(), COOCCURRENCE_CONFIDENCE);
        store.create(&project).await?;
        stats.created += 1;
        for file in &cluster.files {
            store.link_file(file, project.id).await?;
            stats.linked += 1;
        }
    }
    Ok(stats)
}

/// What [`retract_refused_inferences`] did, for the caller to log.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RetractStats {
    /// Inferred projects withdrawn this run.
    pub retracted: usize,
    /// Inferred projects looked at and left alone.
    pub kept: usize,
}

/// Does this project still look exactly the way the pass mints one?
///
/// The name is the root's basename because that is all `materialize_clusters`
/// has to name it with, and the three presentation fields are empty because it
/// fills none of them. A person who renamed it, described it, gave it a colour
/// or an icon has made it theirs, and what we think of how it was born stops
/// mattering at that point.
///
/// NOT a confidence test, and that is worth writing down because it was the
/// obvious one. The pass mints at confidence 50 and so does Makefile detection
/// (`signals.rs`), so an equality there would sweep up signal-detected projects
/// and would not identify a co-occurrence one anyway. What separates them is
/// whether a signal actually sits at the root, which [`retract_refused_inferences`]
/// asks the disk.
fn is_untouched_inference(p: &Project) -> bool {
    p.inferred
        && !p.root_path.is_empty()
        && p.name == project_name_from_root(&p.root_path)
        && p.description.is_empty()
        && p.accent_color.is_empty()
        && p.icon.is_empty()
}

/// Withdraw inferred projects no part of the system would mint today, provided
/// nobody has touched them (`project-system.md`, ruled 12 September: a floor
/// change stops new umbrellas, the ones already minted stay).
///
/// A project is withdrawn only when every one of these holds:
///
/// 1. **Nothing on disk says "project here".** `SignalDetector::detect` finds no
///    marker at the root, so the watcher would not mint it either. This is what
///    keeps a legitimate repository that happens to contain two sub-projects -
///    a top-level `Makefile` over two crates - out of the sweep entirely.
/// 2. **The clustering floor refuses the root**: outside the user's home, or a
///    container of [`CONTAINER_PROJECT_COUNT`] known projects.
/// 3. **It is still exactly as minted** ([`is_untouched_inference`]).
/// 4. **Nobody put a file in it.** Every live membership carries origin `graph`,
///    the stamp the observation pipeline writes. One `user` or `agent` edge and
///    somebody has curated this; it is theirs now.
///
/// Withdrawing means DELETING the node and its edges, which is the tree's
/// existing answer for an inference that turned out wrong (`prune_or_archive`
/// deletes an inferred project whose root is gone, and archives an explicit
/// one). Nothing observed is lost: the File nodes, their access history and the
/// event ledger underneath are untouched, and the projection can be rebuilt by
/// the pass that made it. Archiving instead would leave a graveyard in the
/// project list, and would fight the person the moment they un-archived one.
///
/// Idempotent and self-limiting by construction, which is why it needs no
/// run-once marker and no migration machinery: it only ever touches roots the
/// current floor already refuses, so a re-run finds nothing, and a root the
/// floor accepts is never looked at twice.
///
/// An empty `home` withdraws NOTHING, the same refusal `materialize_clusters`
/// makes: without knowing where home is, the first floor cannot be applied and
/// every root would read as refused.
pub async fn retract_refused_inferences(
    store: &ProjectStore,
    home: &str,
) -> anyhow::Result<RetractStats> {
    let mut stats = RetractStats::default();
    if home.trim_end_matches('/').is_empty() {
        return Ok(stats);
    }
    // Decide over the whole set BEFORE deleting anything. A retraction changes
    // what `count_projects_under` answers for an enclosing root, so a delete
    // mid-scan would make one project's verdict depend on where it happened to
    // sit in the list.
    let mut doomed = Vec::new();
    for project in store.list_active().await? {
        if !is_untouched_inference(&project) {
            stats.kept += 1;
            continue;
        }
        if SignalDetector::detect(std::path::Path::new(&project.root_path)).is_some() {
            stats.kept += 1;
            continue;
        }
        let refused = !is_under_home(&project.root_path, home)
            || store.count_projects_under(&project.root_path).await? >= CONTAINER_PROJECT_COUNT;
        if !refused {
            stats.kept += 1;
            continue;
        }
        if store
            .member_origins(project.id)
            .await?
            .iter()
            .any(|o| o != crate::provenance::Provenance::Graph.as_key())
        {
            stats.kept += 1;
            continue;
        }
        doomed.push(project.id);
    }
    for id in doomed {
        store.delete(id).await?;
        stats.retracted += 1;
    }
    Ok(stats)
}

/// How often the inference pass runs. Co-occurrence is a slow-moving signal (a
/// project emerges over hours of work, not seconds), and the pass is a whole-
/// graph scan, so an hourly cadence keeps it cheap while still surfacing a new
/// project within the same work session.
const INFERENCE_INTERVAL_SECS: u64 = 3600;

/// Run the retraction and say what it did. Best-effort like the rest of the
/// pass: a failed sweep leaves the projects alone and the daemon running.
async fn retract_and_log(store: &ProjectStore, home: &str) {
    match retract_refused_inferences(store, home).await {
        Ok(stats) if stats.retracted > 0 => info!(
            retracted = stats.retracted,
            kept = stats.kept,
            "withdrew inferred projects the floor now refuses"
        ),
        Ok(_) => {}
        Err(e) => warn!(error = %e, "inferred-project retraction failed"),
    }
}

/// Run the project-inference pass periodically (foundation §4.2): scan the
/// graph's co-access history, cluster it, and materialise stable clusters as
/// inferred `Project` nodes. Best-effort densification - a scan or materialise
/// failure is logged, never fatal (the graph is fully usable without inferred
/// projects), so this task never brings the daemon down.
pub async fn run(graph: GraphHandle) -> anyhow::Result<()> {
    let store = ProjectStore::new(graph.clone());
    // Read once, not per pass: the home directory does not move under a running
    // daemon, and a pass that had to ask the environment every hour would be one
    // more thing that can change behind the floor. Empty when `HOME` is unset,
    // which mints nothing - see `materialize_clusters`.
    let home = std::env::var("HOME").unwrap_or_default();
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(INFERENCE_INTERVAL_SECS));
    // The retraction runs at startup rather than waiting an hour with it, because
    // the umbrella it withdraws is already in the person's project list and has
    // been since before this daemon started.
    retract_and_log(&store, &home).await;
    // The first tick fires immediately; consume it so the first real scan waits a
    // full interval, giving promotion time to lay down ACCESSED_IN edges first.
    tick.tick().await;
    loop {
        tick.tick().await;
        // Before minting, withdraw: a root that was a project last hour can have
        // become a container of projects since, and the pass owns its own output.
        retract_and_log(&store, &home).await;
        match infer_clusters(&graph, ClusterParams::default()).await {
            Ok(clusters) => match materialize_clusters(&store, &clusters, &home).await {
                Ok(stats) => info!(
                    created = stats.created,
                    linked = stats.linked,
                    skipped = stats.skipped,
                    "project inference pass complete"
                ),
                Err(e) => warn!(error = %e, "project inference materialise failed"),
            },
            Err(e) => warn!(error = %e, "project inference scan failed"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn access(file: &str, session: &str) -> FileAccess {
        FileAccess {
            file_id: file.to_string(),
            session_id: session.to_string(),
        }
    }

    /// Three files repeatedly opened together across two sessions cluster; a
    /// file seen alone in a third session does not join them.
    #[test]
    fn repeated_co_occurrence_forms_a_cluster() {
        let accesses = vec![
            access("a", "s1"), access("b", "s1"), access("c", "s1"),
            access("a", "s2"), access("b", "s2"), access("c", "s2"),
            access("z", "s3"), // alone
        ];
        let clusters = cluster_cooccurrence(&accesses, ClusterParams::default());
        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0].files, vec!["a", "b", "c"]);
    }

    /// A single shared session is below the default `min_cooccurrence` (2), so a
    /// one-off co-open does not mint a cluster.
    #[test]
    fn a_single_shared_session_is_below_threshold() {
        let accesses = vec![
            access("a", "s1"), access("b", "s1"), access("c", "s1"),
        ];
        let clusters = cluster_cooccurrence(&accesses, ClusterParams::default());
        assert!(clusters.is_empty(), "one shared session must not cluster at min_cooccurrence=2");
        // With min_cooccurrence=1 the same data clusters.
        let relaxed = cluster_cooccurrence(
            &accesses,
            ClusterParams { min_cooccurrence: 1, min_cluster_size: 3 },
        );
        assert_eq!(relaxed.len(), 1);
        assert_eq!(relaxed[0].files, vec!["a", "b", "c"]);
    }

    /// Transitive linking: a-b repeated and b-c repeated cluster {a,b,c} even
    /// though a and c never shared a session.
    #[test]
    fn co_occurrence_is_transitive() {
        let accesses = vec![
            access("a", "s1"), access("b", "s1"),
            access("a", "s2"), access("b", "s2"),
            access("b", "s3"), access("c", "s3"),
            access("b", "s4"), access("c", "s4"),
        ];
        let clusters = cluster_cooccurrence(
            &accesses,
            ClusterParams { min_cooccurrence: 2, min_cluster_size: 3 },
        );
        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0].files, vec!["a", "b", "c"]);
    }

    /// A pair that meets `min_cooccurrence` but not `min_cluster_size` is dropped.
    #[test]
    fn a_too_small_cluster_is_dropped() {
        let accesses = vec![
            access("a", "s1"), access("b", "s1"),
            access("a", "s2"), access("b", "s2"),
        ];
        let clusters = cluster_cooccurrence(&accesses, ClusterParams::default());
        assert!(clusters.is_empty(), "a 2-file cluster is below min_cluster_size=3");
        // Lowering the size floor admits the pair.
        let pairs_ok = cluster_cooccurrence(
            &accesses,
            ClusterParams { min_cooccurrence: 2, min_cluster_size: 2 },
        );
        assert_eq!(pairs_ok.len(), 1);
        assert_eq!(pairs_ok[0].files, vec!["a", "b"]);
    }

    /// Two independent groups produce two clusters, deterministically ordered.
    #[test]
    fn independent_groups_are_separate_clusters() {
        let accesses = vec![
            access("a", "s1"), access("b", "s1"), access("c", "s1"),
            access("a", "s2"), access("b", "s2"), access("c", "s2"),
            access("x", "s3"), access("y", "s3"), access("zz", "s3"),
            access("x", "s4"), access("y", "s4"), access("zz", "s4"),
        ];
        let clusters = cluster_cooccurrence(&accesses, ClusterParams::default());
        assert_eq!(clusters.len(), 2);
        assert_eq!(clusters[0].files, vec!["a", "b", "c"]);
        assert_eq!(clusters[1].files, vec!["x", "y", "zz"]);
    }

    #[test]
    fn empty_input_is_empty() {
        assert!(cluster_cooccurrence(&[], ClusterParams::default()).is_empty());
    }

    /// A file opened twice in one session co-occurs with the others once, not
    /// twice (the session is the unit, deduped).
    #[test]
    fn repeated_access_within_a_session_counts_once() {
        let accesses = vec![
            access("a", "s1"), access("b", "s1"), access("a", "s1"), // a twice
            // Only one shared session, so at min_cooccurrence=2 they must NOT cluster
            // (proving the duplicate did not inflate the shared-session count to 2).
        ];
        let clusters = cluster_cooccurrence(&accesses, ClusterParams { min_cooccurrence: 2, min_cluster_size: 2 });
        assert!(clusters.is_empty());
    }

    /// The graph bridge: ACCESSED_IN edges drive the clustering end to end. Three
    /// files linked to two shared sessions cluster; an unrelated file does not.
    #[tokio::test]
    async fn infer_clusters_reads_accessed_in_from_the_graph() {
        let tmp = tempfile::TempDir::new().unwrap();
        let graph = crate::graph::spawn(tmp.path().join("g").to_str().unwrap()).unwrap();

        // a, b, c each accessed in s1 and s2; z only in s3.
        for (file, sess) in [
            ("a", "s1"), ("b", "s1"), ("c", "s1"),
            ("a", "s2"), ("b", "s2"), ("c", "s2"),
            ("z", "s3"),
        ] {
            graph.write(format!("MERGE (f:File {{id: '{file}'}})")).await.unwrap();
            graph.write(format!("MERGE (s:Session {{id: '{sess}'}})")).await.unwrap();
            graph
                .write(format!(
                    "MATCH (f:File {{id: '{file}'}}), (s:Session {{id: '{sess}'}}) \
                     MERGE (f)-[:ACCESSED_IN]->(s)"
                ))
                .await
                .unwrap();
        }

        let accesses = collect_file_accesses(&graph).await.unwrap();
        assert_eq!(accesses.len(), 7, "every ACCESSED_IN edge is read back");

        let clusters = infer_clusters(&graph, ClusterParams::default()).await.unwrap();
        assert_eq!(clusters.len(), 1, "the repeated trio clusters, z does not");
        assert_eq!(clusters[0].files, vec!["a", "b", "c"]);
    }

    #[test]
    fn a_root_names_its_project_after_its_last_component() {
        assert_eq!(project_name_from_root("/home/tim/proj"), "proj");
        assert_eq!(project_name_from_root("/home/tim/proj/"), "proj");
    }

    /// Materialise mints an inferred Project at a deep common root with its files
    /// linked, skips a cluster whose common root is too shallow to be a project,
    /// and is idempotent on a re-run.
    #[tokio::test]
    async fn materialize_mints_a_deep_root_project_and_skips_shallow_ones() {
        let tmp = tempfile::TempDir::new().unwrap();
        let graph = crate::graph::spawn(tmp.path().join("g").to_str().unwrap()).unwrap();
        let store = ProjectStore::new(graph.clone());

        // A deep-root trio (common root /home/tim/proj, depth 3) and a shallow
        // trio (common root /home, depth 1), each repeated across two sessions.
        let seed = [
            ("/home/tim/proj/a.rs", "s1"), ("/home/tim/proj/b.rs", "s1"), ("/home/tim/proj/c.rs", "s1"),
            ("/home/tim/proj/a.rs", "s2"), ("/home/tim/proj/b.rs", "s2"), ("/home/tim/proj/c.rs", "s2"),
            ("/home/x.rs", "s3"), ("/home/y.rs", "s3"), ("/home/z.rs", "s3"),
            ("/home/x.rs", "s4"), ("/home/y.rs", "s4"), ("/home/z.rs", "s4"),
        ];
        for (file, sess) in seed {
            graph.write(format!("MERGE (f:File {{id: '{file}'}})")).await.unwrap();
            graph.write(format!("MERGE (s:Session {{id: '{sess}'}})")).await.unwrap();
            graph
                .write(format!(
                    "MATCH (f:File {{id: '{file}'}}), (s:Session {{id: '{sess}'}}) \
                     MERGE (f)-[:ACCESSED_IN]->(s)"
                ))
                .await
                .unwrap();
        }

        let clusters = infer_clusters(&graph, ClusterParams::default()).await.unwrap();
        assert_eq!(clusters.len(), 2, "both trios cluster");

        let stats = materialize_clusters(&store, &clusters, "/home/tim").await.unwrap();
        assert_eq!(stats.created, 1, "only the deep-root cluster mints a project");
        assert_eq!(stats.skipped, 1, "the shallow-root cluster is skipped");
        assert_eq!(stats.linked, 3, "the three deep-root files are linked");

        let proj = store.get_by_root_path("/home/tim/proj").await.unwrap();
        let proj = proj.expect("an inferred project exists at the deep root");
        assert!(proj.inferred, "the cooccurrence project is inferred");
        assert!(store.is_file_linked("/home/tim/proj/a.rs", proj.id).await.unwrap());

        // No project was minted over /home (the shallow root).
        assert!(store.get_by_root_path("/home").await.unwrap().is_none());

        // Re-running is idempotent: the project already exists, so nothing new.
        let again = materialize_clusters(&store, &clusters, "/home/tim").await.unwrap();
        assert_eq!(again.created, 0, "a re-run mints no duplicate project");
        assert_eq!(again.skipped, 2, "both clusters skip on the second run");
    }

    /// The retraction fixture: a real home on disk with an umbrella directory
    /// holding two projects, all three registered in the graph and none of them
    /// carrying a signal file. Returns (store, home, umbrella root, umbrella id).
    async fn umbrella_fixture(
        tmp: &tempfile::TempDir,
    ) -> (ProjectStore, GraphHandle, String, String, uuid::Uuid) {
        let graph = crate::graph::spawn(tmp.path().join("graph").to_str().unwrap()).unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        let store = ProjectStore::new(graph.clone());
        let home = tmp.path().join("home");
        let umbrella = home.join("Repos");
        for child in ["a", "b"] {
            std::fs::create_dir_all(umbrella.join(child)).unwrap();
            let root = umbrella.join(child).to_string_lossy().to_string();
            let p = Project::new_inferred(child.to_string(), root, 90);
            store.create(&p).await.unwrap();
        }
        let root = umbrella.to_string_lossy().to_string();
        let p = Project::new_inferred(
            project_name_from_root(&root),
            root.clone(),
            COOCCURRENCE_CONFIDENCE,
        );
        store.create(&p).await.unwrap();
        (store, graph, home.to_string_lossy().to_string(), root, p.id)
    }

    /// The third piece of the floor ruling: the umbrella already minted goes.
    ///
    /// The floor stops the next one; this is the one sitting in the project list
    /// on every machine that has it. Nothing on disk says "project" at that root
    /// and it holds two known projects, so no part of the system would mint it
    /// today - and nobody has touched it since it was minted.
    #[tokio::test]
    async fn an_untouched_umbrella_over_two_projects_is_withdrawn() {
        let tmp = tempfile::TempDir::new().unwrap();
        let (store, _graph, home, root, id) = umbrella_fixture(&tmp).await;

        let stats = retract_refused_inferences(&store, &home).await.unwrap();
        assert_eq!(stats.retracted, 1, "the umbrella is withdrawn");
        assert_eq!(stats.kept, 2, "its two real children are left alone");
        assert!(store.get_by_id(id).await.unwrap().is_none());
        assert!(store.get_by_root_path(&root).await.unwrap().is_none());

        // Self-limiting: the second run finds nothing left to refuse, so it needs
        // no marker saying it has already happened.
        let again = retract_refused_inferences(&store, &home).await.unwrap();
        assert_eq!(again.retracted, 0);
    }

    /// A marker on disk keeps the project, whatever is nested under it.
    ///
    /// This is the case the sweep exists to not break: a repository with a
    /// top-level `.git` over two sub-projects is a container by the clustering
    /// floor's count AND a project by every other measure. The disk decides.
    #[tokio::test]
    async fn a_root_carrying_a_signal_is_kept() {
        let tmp = tempfile::TempDir::new().unwrap();
        let (store, _graph, home, root, id) = umbrella_fixture(&tmp).await;
        std::fs::create_dir_all(std::path::Path::new(&root).join(".git")).unwrap();

        let stats = retract_refused_inferences(&store, &home).await.unwrap();
        assert_eq!(stats.retracted, 0, "a signalled root is not an umbrella");
        assert!(store.get_by_id(id).await.unwrap().is_some());
    }

    /// One file somebody put there and the project is theirs.
    #[tokio::test]
    async fn a_member_nobody_observed_keeps_the_project() {
        let tmp = tempfile::TempDir::new().unwrap();
        let (store, graph, home, root, id) = umbrella_fixture(&tmp).await;
        let file = format!("{root}/asserted.rs");
        graph
            .write(format!(
                "CREATE (f:File {{id: '{file}', path: '{file}', app_id: 't', last_accessed: 0}})"
            ))
            .await
            .unwrap();
        graph
            .write(format!(
                "MATCH (f:File {{id: '{file}'}}), (p:Project {{id: '{id}'}}) \
                 CREATE (f)-[:FILE_PART_OF {{origin: 'user'}}]->(p)"
            ))
            .await
            .unwrap();

        let stats = retract_refused_inferences(&store, &home).await.unwrap();
        assert_eq!(stats.retracted, 0, "a user-asserted membership is a touch");
        assert!(store.get_by_id(id).await.unwrap().is_some());
    }

    /// An observed membership is not a touch - it is what the pass itself lays
    /// down, so a linked umbrella still goes.
    #[tokio::test]
    async fn an_observed_membership_does_not_save_the_umbrella() {
        let tmp = tempfile::TempDir::new().unwrap();
        let (store, graph, home, root, id) = umbrella_fixture(&tmp).await;
        let file = format!("{root}/observed.rs");
        graph
            .write(format!(
                "CREATE (f:File {{id: '{file}', path: '{file}', app_id: 't', last_accessed: 0}})"
            ))
            .await
            .unwrap();
        store.link_file(&file, id).await.unwrap();

        let stats = retract_refused_inferences(&store, &home).await.unwrap();
        assert_eq!(stats.retracted, 1);
        assert!(store.get_by_id(id).await.unwrap().is_none());
    }

    /// A renamed project is not as minted, so it stays whatever the floor says.
    #[tokio::test]
    async fn a_renamed_umbrella_is_kept() {
        let tmp = tempfile::TempDir::new().unwrap();
        let (store, _graph, home, _root, id) = umbrella_fixture(&tmp).await;
        let mut p = store.get_by_id(id).await.unwrap().unwrap();
        p.name = "My code".into();
        store.update(&p).await.unwrap();

        let stats = retract_refused_inferences(&store, &home).await.unwrap();
        assert_eq!(stats.retracted, 0, "a name somebody chose is a touch");
        assert!(store.get_by_id(id).await.unwrap().is_some());
    }

    /// Without a home the first floor cannot be applied, and a sweep that cannot
    /// apply it would read every root as refused. It withdraws nothing.
    #[tokio::test]
    async fn no_home_withdraws_nothing() {
        let tmp = tempfile::TempDir::new().unwrap();
        let (store, _graph, _home, _root, id) = umbrella_fixture(&tmp).await;

        for home in ["", "/"] {
            let stats = retract_refused_inferences(&store, home).await.unwrap();
            assert_eq!(stats.retracted, 0, "no home, no retraction");
        }
        assert!(store.get_by_id(id).await.unwrap().is_some());
    }

    /// The umbrella, and the rule that refuses it.
    ///
    /// `~/Repositories` holds two projects on this machine and was minted as a
    /// third one over the top of them, because the floor it had to clear was a
    /// path-depth number and three components was one too few here. The rule is
    /// structural now: a directory containing two known projects is a container.
    #[tokio::test]
    async fn a_directory_holding_two_projects_is_not_minted_as_a_third() {
        let tmp = tempfile::TempDir::new().unwrap();
        let graph = crate::graph::spawn(tmp.path().join("graph").to_str().unwrap()).unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        let store = ProjectStore::new(graph.clone());

        for (name, root) in [("arlen", "/home/tim/Repositories/arlen"), ("other", "/home/tim/Repositories/other")] {
            let p = Project::new_inferred(name.to_string(), root.to_string(), 90);
            store.create(&p).await.unwrap();
        }

        let cluster = CandidateCluster {
            files: vec![
                "/home/tim/Repositories/arlen/a.rs".into(),
                "/home/tim/Repositories/other/b.rs".into(),
                "/home/tim/Repositories/other/c.rs".into(),
            ],
        };
        let stats = materialize_clusters(&store, &[cluster], "/home/tim").await.unwrap();
        assert_eq!(stats.created, 0, "the container must not become a project");
        assert_eq!(stats.skipped, 1);
        assert!(store.get_by_root_path("/home/tim/Repositories").await.unwrap().is_none());
    }

    /// The other floor, which the depth number used to approximate: a cluster
    /// rooted at the home directory itself has found the user, not a project.
    #[test]
    fn only_a_root_strictly_inside_home_can_mint() {
        assert!(is_under_home("/home/tim/proj", "/home/tim"));
        assert!(is_under_home("/home/tim/a/b/c", "/home/tim/"));
        assert!(!is_under_home("/home/tim", "/home/tim"), "home itself is every file you own");
        assert!(!is_under_home("/home", "/home/tim"));
        assert!(!is_under_home("/", "/home/tim"));
        // A sibling that merely shares the prefix as TEXT is not inside it.
        assert!(!is_under_home("/home/tim2/proj", "/home/tim"));
        // No home, nothing mints.
        assert!(!is_under_home("/home/tim/proj", ""));
    }
}

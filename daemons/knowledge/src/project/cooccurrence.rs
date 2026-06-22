//! Project inference by co-occurrence clustering (kg-richness-plan.md Thrust 1,
//! foundation §4.2: "a background inference pass groups co-occurrence clusters
//! into candidate Project nodes"). The pure algorithm core: given file accesses
//! tagged with the session they occurred in, group files that repeatedly appear
//! together across sessions into candidate clusters a background pass can
//! promote to inferred Project nodes. This densifies the graph far beyond the
//! single-session `auto_promote_threshold` heuristic, which only ever sees one
//! session at a time.
//!
//! No I/O and no graph dependency: the background pass reads the accesses from
//! the event store and writes the clusters as candidate nodes; the clustering
//! itself is deterministic and unit-tested here. Lives behind `allow(dead_code)`
//! until that pass wires it (mechanism before trigger).
#![allow(dead_code)]

use crate::graph::GraphHandle;
use std::collections::{BTreeMap, BTreeSet, HashMap};

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
/// deterministic [`cluster_cooccurrence`]. The materialisation of these
/// candidates into inferred `Project` nodes is the next step (it must derive a
/// root from the cluster's common path prefix and dedup against files already in
/// a project, so it does not collide with the signal-detected projects the
/// watcher mints).
pub async fn infer_clusters(
    graph: &GraphHandle,
    params: ClusterParams,
) -> anyhow::Result<Vec<CandidateCluster>> {
    let accesses = collect_file_accesses(graph).await?;
    Ok(cluster_cooccurrence(&accesses, params))
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
}

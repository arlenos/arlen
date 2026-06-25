//! KG-richness Thrust 1: co-occurrence clustering. Files accessed together build
//! up the `CO_ACCESSED` graph; grouping the connected files into clusters is the
//! densest project-inference signal (foundation §4.2) - a cluster is a candidate
//! `Project`. Pure: the caller supplies the (recency-filtered) co-access edges,
//! this returns the connected file groups; the graph-backed producer that reads
//! `CO_ACCESSED` and proposes candidate `Project` nodes consumes it. Filtering to
//! recent / strong edges happens in the caller's query so the whole graph does not
//! collapse into one giant component.

use std::collections::BTreeMap;

use anyhow::Result;

use crate::graph::GraphHandle;

/// Find the representative of `x`'s set, with path compression. `parent` maps each
/// seen node to its parent (a node not yet inserted is its own root once added).
fn find(parent: &mut BTreeMap<String, String>, x: &str) -> String {
    let mut root = x.to_string();
    loop {
        match parent.get(&root) {
            Some(p) if p != &root => root = p.clone(),
            _ => break,
        }
    }
    // Path-compress: point every node on the walk straight at the root.
    let mut cur = x.to_string();
    loop {
        let next = match parent.get(&cur) {
            Some(p) if p != &cur => p.clone(),
            _ => break,
        };
        parent.insert(cur.clone(), root.clone());
        cur = next;
    }
    root
}

/// Group the files connected (transitively) through `edges` into clusters, keeping
/// only those with at least `min_size` distinct files (a lone file is not a
/// project). Each cluster is sorted and the clusters are sorted, so the output is
/// deterministic for a fixed input. A self-edge (a file co-accessed with itself) is
/// ignored. `min_size` of 0 or 1 is treated as 2 - a single-file "cluster" is never
/// a useful project candidate.
pub fn cluster_co_access(edges: &[(String, String)], min_size: usize) -> Vec<Vec<String>> {
    let min_size = min_size.max(2);
    let mut parent: BTreeMap<String, String> = BTreeMap::new();
    for (a, b) in edges {
        if a == b {
            continue;
        }
        parent.entry(a.clone()).or_insert_with(|| a.clone());
        parent.entry(b.clone()).or_insert_with(|| b.clone());
        let ra = find(&mut parent, a);
        let rb = find(&mut parent, b);
        if ra != rb {
            parent.insert(ra, rb);
        }
    }

    let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let nodes: Vec<String> = parent.keys().cloned().collect();
    for n in nodes {
        let r = find(&mut parent, &n);
        groups.entry(r).or_default().push(n);
    }

    let mut out: Vec<Vec<String>> = groups
        .into_values()
        .filter(|g| g.len() >= min_size)
        .map(|mut g| {
            g.sort();
            g
        })
        .collect();
    out.sort();
    out
}

/// A project candidate derived from a co-access cluster: a name + root directory
/// (the cluster's common path) and a confidence scaled by the cluster size. Fed to
/// `Project::new_inferred` by the materialisation step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateProject {
    /// The candidate project name (the basename of the common root).
    pub name: String,
    /// The common directory the clustered files live under.
    pub root_path: String,
    /// Inference confidence (`0..=90`); never 100, which is reserved for a
    /// user-confirmed project.
    pub confidence: u8,
}

/// The minimum path depth (components below `/`) a co-access cluster's common
/// directory must reach to be a project candidate. A shallower common root (`/`,
/// `/home`, `/home/user`) is the home/system tree shared by unrelated files, not a
/// project, so such a cluster is treated as co-access noise.
const MIN_ROOT_DEPTH: usize = 3;

/// Derive a project candidate from a co-access cluster: the longest common path
/// prefix of the files is the root directory (its basename the name), and the
/// confidence scales with the cluster size (more co-accessed files = a stronger
/// signal), capped at 90. Returns `None` when the files share no sufficiently deep
/// common directory (spread across unrelated trees, or rooted only at the home
/// dir): that cluster is co-access noise, not a project. Pure - the materialisation
/// step turns a `Some` into an inferred `Project`.
pub fn candidate_project_from_cluster(files: &[String]) -> Option<CandidateProject> {
    if files.len() < 2 {
        return None;
    }
    let split = |p: &str| {
        p.trim_start_matches('/')
            .split('/')
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect::<Vec<String>>()
    };
    let mut common = split(&files[0]);
    for f in &files[1..] {
        let parts = split(f);
        let shared = common.iter().zip(parts.iter()).take_while(|(a, b)| a == b).count();
        common.truncate(shared);
        if common.len() < MIN_ROOT_DEPTH {
            return None;
        }
    }
    if common.len() < MIN_ROOT_DEPTH {
        return None;
    }
    Some(CandidateProject {
        name: common.last()?.clone(),
        root_path: format!("/{}", common.join("/")),
        confidence: (40 + files.len() * 10).min(90) as u8,
    })
}

/// Read the recent `CO_ACCESSED` file graph and cluster it into candidate-project
/// file groups. Edges whose `last_seen >= cutoff_micros` are read once on the
/// serial graph thread (the `analyze_code_graph` read pattern), then the pure
/// [`cluster_co_access`] produces the groups - so the graph layer supplies only
/// the edges, never the clustering. A quiet graph yields no candidates.
///
/// Read-only: this SURFACES candidates (for the inference task / a read op to act
/// on); it does NOT create `Project` nodes - that materialisation, with its
/// candidate-vs-confirmed noise policy, is the next slice. `cutoff_micros` is an
/// integer, so it is injection-safe interpolated.
pub async fn candidate_clusters(
    graph: &GraphHandle,
    cutoff_micros: i64,
    min_size: usize,
) -> Result<Vec<Vec<String>>> {
    let rows = graph
        .query_rows(format!(
            "MATCH (a:File)-[c:CO_ACCESSED]->(b:File) WHERE c.last_seen >= {cutoff_micros} \
             RETURN a.id, b.id"
        ))
        .await?;
    let edges: Vec<(String, String)> = rows
        .rows
        .iter()
        .filter_map(|row| {
            let a = row.first()?.as_str().to_string();
            let b = row.get(1)?.as_str().to_string();
            (!a.is_empty() && !b.is_empty()).then_some((a, b))
        })
        .collect();
    Ok(cluster_co_access(&edges, min_size))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(a: &str, b: &str) -> (String, String) {
        (a.to_string(), b.to_string())
    }

    #[test]
    fn groups_transitively_co_accessed_files() {
        let edges = [e("a", "b"), e("b", "c"), e("c", "d")];
        assert_eq!(cluster_co_access(&edges, 2), vec![vec!["a", "b", "c", "d"]]);
    }

    #[test]
    fn keeps_disjoint_clusters_separate_and_sorted() {
        let edges = [e("x", "y"), e("a", "b"), e("b", "c")];
        assert_eq!(cluster_co_access(&edges, 2), vec![vec!["a", "b", "c"], vec!["x", "y"]]);
    }

    #[test]
    fn drops_clusters_below_min_size() {
        let edges = [e("a", "b"), e("c", "d"), e("d", "e"), e("e", "f")];
        // Only the {c,d,e,f} cluster meets min_size 3; the pair {a,b} is dropped.
        assert_eq!(cluster_co_access(&edges, 3), vec![vec!["c", "d", "e", "f"]]);
    }

    #[test]
    fn ignores_self_edges_and_an_empty_graph() {
        assert!(cluster_co_access(&[e("a", "a")], 2).is_empty());
        assert!(cluster_co_access(&[], 2).is_empty());
    }

    #[test]
    fn min_size_is_floored_at_two() {
        // A lone file is never a project candidate even if min_size is 0/1.
        let edges = [e("a", "b")];
        assert_eq!(cluster_co_access(&edges, 0), vec![vec!["a", "b"]]);
        assert_eq!(cluster_co_access(&edges, 1), vec![vec!["a", "b"]]);
    }

    #[test]
    fn candidate_project_uses_the_common_directory() {
        let files = [
            "/home/u/proj/src/a.rs".to_string(),
            "/home/u/proj/src/b.rs".to_string(),
            "/home/u/proj/README.md".to_string(),
        ];
        let c = candidate_project_from_cluster(&files).expect("a project candidate");
        assert_eq!(c.root_path, "/home/u/proj");
        assert_eq!(c.name, "proj");
        assert_eq!(c.confidence, 70); // 40 + 3*10
    }

    #[test]
    fn confidence_caps_at_ninety() {
        let files: Vec<String> = (0..8).map(|i| format!("/home/u/proj/f{i}.rs")).collect();
        assert_eq!(candidate_project_from_cluster(&files).unwrap().confidence, 90);
    }

    #[test]
    fn rejects_shallow_or_unrelated_clusters() {
        // Common root is only /home/u (the home dir, depth 2) -> not a project.
        let shallow = ["/home/u/a/x.rs".to_string(), "/home/u/b/y.rs".to_string()];
        assert!(candidate_project_from_cluster(&shallow).is_none());
        // No common directory at all.
        let unrelated = ["/tmp/x".to_string(), "/var/y".to_string()];
        assert!(candidate_project_from_cluster(&unrelated).is_none());
        // A lone file is never a project.
        assert!(candidate_project_from_cluster(&["/home/u/proj/a.rs".to_string()]).is_none());
    }

    #[test]
    fn merges_a_shared_file_into_one_cluster() {
        // b bridges the two pairs, so all four are one project candidate.
        let edges = [e("a", "b"), e("b", "c"), e("c", "d")];
        let clusters = cluster_co_access(&edges, 2);
        assert_eq!(clusters.len(), 1, "{clusters:?}");
        assert_eq!(clusters[0].len(), 4);
    }
}

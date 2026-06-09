//! LLM-free retrieval fusion (bitemporal-knowledge-graph.md §7).
//!
//! The agent finds the relevant slice without an LLM call at query time by
//! fusing ranked id-lists from up to three primitives: BM25 keyword (SQLite
//! FTS5), graph traversal (Kuzu via the typed DSL), and an optional semantic
//! vector search. The three are combined by **Reciprocal Rank Fusion** (§7.3),
//! which is positional, so the primitives' incomparable raw scores never need to
//! be normalised. The default path is the two-primitive subset (keyword + graph)
//! since embeddings are config-gated off; retrieval fails open to LLM-free-anyway
//! (never a generative call) when a model is absent.
//!
//! This module is the pure fusion core. The FTS5 `fact_text` index, the
//! deterministic per-label text synthesis, the optional `fact_vec` table, and
//! the post-fusion `VALID_AS_OF` validity confirm against the bi-temporal graph
//! are separate increments built on this.

use std::collections::{HashMap, HashSet};

use anyhow::Result;
use sqlx::SqlitePool;

use crate::graph::GraphHandle;
use crate::utils::escape_cypher;

/// LLM-free retrieval (§7): combine the keyword and graph primitives into one
/// temporally-honest ranking of node ids, with no LLM call at query time.
///
/// Runs the BM25 keyword search (`φ_bm25`) and a bounded graph expansion of its
/// hits (`φ_bfs`, the hits' one-hop neighbours), fuses the two ranked id-lists by
/// RRF (positional, so the primitives' incomparable raw scores need no
/// normalisation), then drops any candidate with no current graph presence (the
/// validity confirm, since the keyword index is atemporal). Returns node ids
/// best-first, ready to seed the agent's slice. The optional semantic primitive
/// (`φ_cos`, sqlite-vec, config-gated off) is not included in this default path;
/// when absent, retrieval fails open to LLM-free-anyway, never a generative call.
#[allow(dead_code)]
pub async fn retrieve(
    pool: &SqlitePool,
    graph: &GraphHandle,
    query: &str,
    limit: i64,
) -> Result<Vec<String>> {
    let keyword = crate::fts::search_fact_text(pool, query, limit).await?;
    let graph_hits = if keyword.is_empty() {
        Vec::new()
    } else {
        neighbours(graph, &keyword).await?
    };
    let fused = rrf_rank(&[keyword, graph_hits], K_RRF);
    confirm_present(graph, &fused).await
}

/// The one-hop neighbours of `seeds`, the `φ_bfs` graph primitive's ranked
/// id-list (in traversal order). Distinct, label-agnostic, bounded to one hop so
/// the expansion cannot explode. The seed ids are escaped into the id-list
/// literal.
async fn neighbours(graph: &GraphHandle, seeds: &[String]) -> Result<Vec<String>> {
    let list = seeds
        .iter()
        .map(|id| format!("'{}'", escape_cypher(id)))
        .collect::<Vec<_>>()
        .join(", ");
    let cypher =
        format!("MATCH (s) WHERE s.id IN [{list}] MATCH (s)-[]-(n) RETURN DISTINCT n.id AS id");
    let rs = graph.query_rows(cypher).await?;
    Ok(rs
        .rows
        .iter()
        .filter_map(|row| row.first().map(|cell| cell.as_str().to_string()))
        .collect())
}

/// The standard RRF damping constant (§7.3): a larger `k` flattens the
/// contribution of rank, so top ranks dominate less.
pub const K_RRF: u32 = 60;

/// Fuse several ranked id-lists into one ranking by Reciprocal Rank Fusion.
///
/// Each input list is in rank order (best first). An id's contribution from a
/// list is `1 / (k + rank)` with `rank` 1-based, summed across every list that
/// contains it, so an id surfaced by more primitives, or ranked higher, scores
/// higher. A repeated id within one list counts only its best (first) rank, so a
/// primitive that emits a duplicate cannot inflate a candidate. Returns
/// `(id, score)` sorted by score descending, ties broken by id ascending for a
/// deterministic order.
pub fn rrf_fuse(lists: &[Vec<String>], k: u32) -> Vec<(String, f64)> {
    let mut scores: HashMap<String, f64> = HashMap::new();
    for list in lists {
        let mut seen: HashSet<&str> = HashSet::new();
        for (rank0, id) in list.iter().enumerate() {
            // Only the best rank of an id within a single list contributes.
            if !seen.insert(id.as_str()) {
                continue;
            }
            let rank = rank0 as u32 + 1;
            *scores.entry(id.clone()).or_insert(0.0) += 1.0 / f64::from(k + rank);
        }
    }
    let mut fused: Vec<(String, f64)> = scores.into_iter().collect();
    fused.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });
    fused
}

/// Fuse and return only the ids, best first (the common case where the caller
/// wants the ranking, not the scores).
pub fn rrf_rank(lists: &[Vec<String>], k: u32) -> Vec<String> {
    rrf_fuse(lists, k).into_iter().map(|(id, _)| id).collect()
}

/// Deterministically synthesise the indexed `fact_text` for a node from its own
/// fields (§7.2), the text the FTS5 keyword index holds.
///
/// No LLM at ingest: a tiny pure function per label, regenerated idempotently on
/// re-promotion (same fields → same text), so the index never drifts and costs
/// no tokens on the silent path. The accepted cost is weaker paraphrase recall
/// than an LLM-written fact, mitigated by the semantic primitive when present.
/// `fields` are the node's own fields as strings (the promotion side stringifies
/// them); `label` selects the synthesis. An unknown label flattens its
/// non-internal field values in key order.
pub fn fact_text(label: &str, fields: &std::collections::BTreeMap<String, String>) -> String {
    match label {
        "File" => file_fact_text(fields),
        "Project" => project_fact_text(fields),
        _ => generic_fact_text(fields),
    }
}

/// `File` → path, its basename, and its parent dir. The basename and parent
/// repeat substrings of the path on purpose: a keyword search for a filename or
/// a directory then matches more strongly than against the full path alone.
fn file_fact_text(fields: &std::collections::BTreeMap<String, String>) -> String {
    let path = fields.get("path").map(String::as_str).unwrap_or("");
    let basename = path.rsplit('/').next().unwrap_or("");
    let parent = path.rsplit_once('/').map(|(p, _)| p).unwrap_or("");
    join_nonempty(&[path, basename, parent])
}

/// `Project` → name, description, and root path.
fn project_fact_text(fields: &std::collections::BTreeMap<String, String>) -> String {
    let name = fields.get("name").map(String::as_str).unwrap_or("");
    let description = fields.get("description").map(String::as_str).unwrap_or("");
    let root_path = fields.get("root_path").map(String::as_str).unwrap_or("");
    join_nonempty(&[name, description, root_path])
}

/// Unknown label: flatten every non-internal field value in key order (the map
/// is sorted, so this is deterministic). `id` and `_`-prefixed reserved fields
/// are excluded, as they are identity/bookkeeping, not searchable content.
fn generic_fact_text(fields: &std::collections::BTreeMap<String, String>) -> String {
    let values: Vec<&str> = fields
        .iter()
        .filter(|(k, _)| k.as_str() != "id" && !k.starts_with('_'))
        .map(|(_, v)| v.as_str())
        .collect();
    join_nonempty(&values)
}

/// Join the non-empty parts with a single space.
fn join_nonempty(parts: &[&str]) -> String {
    parts
        .iter()
        .filter(|s| !s.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join(" ")
}

/// The validity confirm (§7.3): keep only the candidate ids that have a current
/// graph presence, preserving the fused order.
///
/// FTS5 and the vector index are atemporal (they index every fact ever seen), so
/// a fused candidate may name a node that no longer exists. One batched,
/// label-agnostic graph read filters the candidates to those actually present.
/// This is the presence form; the full temporal liveness join (a node's
/// `expired_at` against `T_asof`, the §4.4 endpoint-node clause) lands with the
/// node temporal stamps in R6, at which point this gains a `WHERE` on the node's
/// transaction axis. Returns the candidates that are present, in input order, so
/// the RRF ranking survives the filter. An empty input is an empty result with
/// no query.
pub async fn confirm_present(graph: &GraphHandle, candidates: &[String]) -> Result<Vec<String>> {
    if candidates.is_empty() {
        return Ok(Vec::new());
    }
    let list = candidates
        .iter()
        .map(|id| format!("'{}'", escape_cypher(id)))
        .collect::<Vec<_>>()
        .join(", ");
    // Label-agnostic: a candidate id may belong to any node table.
    let cypher = format!("MATCH (n) WHERE n.id IN [{list}] RETURN n.id AS id");
    let rs = graph.query_rows(cypher).await?;
    let present: HashSet<String> = rs
        .rows
        .iter()
        .filter_map(|row| row.first().map(|cell| cell.as_str().to_string()))
        .collect();
    Ok(candidates
        .iter()
        .filter(|id| present.contains(*id))
        .cloned()
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_single_list_keeps_its_order() {
        let fused = rrf_rank(&[ids(&["a", "b", "c"])], K_RRF);
        assert_eq!(fused, ids(&["a", "b", "c"]));
    }

    #[test]
    fn an_id_in_more_lists_outranks_one_in_fewer() {
        // `b` is mid-rank in both lists; `a` is top of only the first. Appearing
        // in both should let `b` overtake a single-list top.
        let l1 = ids(&["a", "b"]);
        let l2 = ids(&["c", "b"]);
        let fused = rrf_rank(&[l1, l2], K_RRF);
        assert_eq!(fused[0], "b", "the id surfaced by both primitives wins");
    }

    #[test]
    fn higher_rank_in_both_lists_wins() {
        let l1 = ids(&["x", "y"]);
        let l2 = ids(&["x", "y"]);
        let fused = rrf_fuse(&[l1, l2], K_RRF);
        assert_eq!(fused[0].0, "x");
        assert!(fused[0].1 > fused[1].1, "x scores strictly higher than y");
    }

    #[test]
    fn a_duplicate_within_a_list_uses_only_its_best_rank() {
        // `a` repeated in one list must not be counted twice.
        let with_dup = rrf_fuse(&[ids(&["a", "a", "b"])], K_RRF);
        let no_dup = rrf_fuse(&[ids(&["a", "b"])], K_RRF);
        let a_dup = with_dup.iter().find(|(id, _)| id == "a").unwrap().1;
        let a_no = no_dup.iter().find(|(id, _)| id == "a").unwrap().1;
        assert!((a_dup - a_no).abs() < 1e-12, "a duplicate id does not inflate the score");
    }

    #[test]
    fn ties_break_by_id_for_determinism() {
        // Same rank in symmetric lists -> equal scores -> deterministic id order.
        let fused = rrf_rank(&[ids(&["b"]), ids(&["a"])], K_RRF);
        assert_eq!(fused, ids(&["a", "b"]), "equal scores order by id ascending");
    }

    #[test]
    fn empty_input_is_empty() {
        assert!(rrf_rank(&[], K_RRF).is_empty());
        assert!(rrf_rank(&[vec![]], K_RRF).is_empty());
    }

    fn fields(pairs: &[(&str, &str)]) -> std::collections::BTreeMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn file_fact_text_includes_path_basename_and_parent() {
        let t = fact_text("File", &fields(&[("path", "/home/tim/proj/main.rs")]));
        assert!(t.contains("/home/tim/proj/main.rs"), "full path: {t}");
        assert!(t.contains("main.rs"), "basename: {t}");
        assert!(t.contains("/home/tim/proj"), "parent dir: {t}");
    }

    #[test]
    fn project_fact_text_joins_name_description_and_root() {
        let t = fact_text(
            "Project",
            &fields(&[
                ("name", "Arlen"),
                ("description", "a desktop OS"),
                ("root_path", "/home/tim/arlen"),
            ]),
        );
        assert_eq!(t, "Arlen a desktop OS /home/tim/arlen");
    }

    #[test]
    fn unknown_label_flattens_non_internal_values_deterministically() {
        let f = fields(&[("id", "x"), ("_owner", "app"), ("alpha", "one"), ("beta", "two")]);
        // id and _-prefixed reserved fields excluded; values in key order.
        assert_eq!(fact_text("Widget", &f), "one two");
    }

    #[test]
    fn synthesis_is_idempotent_and_omits_empty_fields() {
        let f = fields(&[("path", "/a/b.txt")]);
        assert_eq!(fact_text("File", &f), fact_text("File", &f), "same fields -> same text");
        // A Project with only a name (no description/root) omits the empty parts.
        assert_eq!(fact_text("Project", &fields(&[("name", "Solo")])), "Solo");
    }

    #[tokio::test]
    async fn retrieve_fuses_keyword_hits_with_their_graph_neighbours() {
        use sqlx::sqlite::SqlitePoolOptions;
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        crate::fts::create_fact_text_index(&pool).await.unwrap();

        let tmp = tempfile::TempDir::new().unwrap();
        let graph = crate::graph::spawn(tmp.path().join("graph").to_str().unwrap()).unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        // A File linked to a Project, with the File indexed by keyword.
        graph
            .write("CREATE (:File {id: '/a/main.rs', path: '/a/main.rs'})".into())
            .await
            .unwrap();
        graph.write("CREATE (:Project {id: 'p1'})".into()).await.unwrap();
        graph
            .write(
                "MATCH (f:File {id: '/a/main.rs'}), (p:Project {id: 'p1'}) \
                 CREATE (f)-[:FILE_PART_OF]->(p)"
                    .into(),
            )
            .await
            .unwrap();
        crate::fts::upsert_fact_text(&pool, "/a/main.rs", "/a/main.rs main.rs a source")
            .await
            .unwrap();

        let results = retrieve(&pool, &graph, "main.rs", 10).await.unwrap();
        assert!(
            results.contains(&"/a/main.rs".to_string()),
            "the keyword hit is returned: {results:?}"
        );
        assert!(
            results.contains(&"p1".to_string()),
            "its one-hop graph neighbour is fused in: {results:?}"
        );
        // A query matching nothing returns nothing (LLM-free, no fallback call).
        assert!(retrieve(&pool, &graph, "nonexistent", 10).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn confirm_present_keeps_only_existing_nodes_in_fused_order() {
        let tmp = tempfile::TempDir::new().unwrap();
        let graph = crate::graph::spawn(tmp.path().join("graph").to_str().unwrap()).unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        graph.write("CREATE (:File {id: 'f1', path: '/a'})".into()).await.unwrap();
        graph.write("CREATE (:Project {id: 'p1'})".into()).await.unwrap();

        // Fused candidates: f1 present, "absent" not in the graph, p1 present.
        let candidates = ids(&["f1", "absent", "p1"]);
        let confirmed = confirm_present(&graph, &candidates).await.unwrap();
        assert_eq!(confirmed, ids(&["f1", "p1"]), "absent dropped, fused order preserved");

        // An empty candidate set returns empty without a query.
        assert!(confirm_present(&graph, &[]).await.unwrap().is_empty());
    }
}

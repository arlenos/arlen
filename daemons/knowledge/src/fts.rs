//! The SQLite FTS5 keyword index for LLM-free retrieval
//! (bitemporal-knowledge-graph.md §7.1).
//!
//! `φ_bm25`, the keyword primitive, lives on the SQLite side beside `events`,
//! not in Kuzu, for three reasons: the promotion pass can populate it in the
//! same transaction that advances the HWM (so the index never drifts from the
//! graph), FTS5 ships in SQLite with no new native dependency (Kuzu's FTS is a
//! `LOAD EXTENSION` the read socket's write guard blocks), and each store keeps
//! one job. The index is keyed by graph node id, so its ranked hits share the
//! one key space the other primitives use, which is what makes RRF fusion
//! meaningful. BM25 is FTS5's built-in `rank`; this returns node ids best-first,
//! ready for [`crate::retrieval::rrf_fuse`].

use anyhow::Result;
use sqlx::SqlitePool;

/// Create the `fact_text` FTS5 virtual table if absent. `node_id` is stored but
/// `UNINDEXED` (it is the join key, not searchable content); `text` is the
/// deterministically synthesised, searchable fact text (see
/// [`crate::retrieval::fact_text`]).
pub async fn create_fact_text_index(pool: &SqlitePool) -> Result<()> {
    sqlx::query("CREATE VIRTUAL TABLE IF NOT EXISTS fact_text USING fts5(node_id UNINDEXED, text)")
        .execute(pool)
        .await?;
    Ok(())
}

/// Upsert a node's indexed text: replace any existing row for `node_id`, then
/// insert the new text. Idempotent on re-promotion (the same node re-synthesised
/// yields the same row, never a duplicate). FTS5 has no native upsert, so this is
/// delete-then-insert; the promotion pass that calls it is serial, and in the
/// real path both run inside the promotion transaction, so the pair is atomic
/// with the graph write.
pub async fn upsert_fact_text(pool: &SqlitePool, node_id: &str, text: &str) -> Result<()> {
    sqlx::query("DELETE FROM fact_text WHERE node_id = ?1")
        .bind(node_id)
        .execute(pool)
        .await?;
    sqlx::query("INSERT INTO fact_text (node_id, text) VALUES (?1, ?2)")
        .bind(node_id)
        .bind(text)
        .execute(pool)
        .await?;
    Ok(())
}

/// Remove a node's indexed text (when it is summarised away or compacted, §7.2),
/// so an index entry cannot outlive the node it names.
///
/// Called by the retention compaction path, which deletes the File nodes it has
/// summarised and then drops the ones the graph confirms are gone. Until 27
/// August it had no caller at all: compaction removed the nodes and left their
/// rows, so the index only ever grew and a search spent part of its ranking on
/// files that were not there.
pub async fn delete_fact_text(pool: &SqlitePool, node_id: &str) -> Result<()> {
    sqlx::query("DELETE FROM fact_text WHERE node_id = ?1")
        .bind(node_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Keyword-search the index, returning matching node ids ranked best-first by
/// BM25 (SQLite's `bm25()` is more negative for a better match, so ascending
/// order is best-first). The ranked ids are one of RRF's input lists.
///
/// EACH TERM is bound as its own FTS5 phrase (double-quoted, embedded quotes
/// doubled), joined by the implicit AND. Quoting is what makes a `.`, `-` or `/`
/// in a filename a literal token rather than an FTS5 operator that errors the
/// query - the reason it was introduced - and quoting the query WHOLE kept that
/// safety while quietly turning every multi-word search into an exact-phrase
/// search. "main.rs notes" then matched only a document with those two words
/// adjacent and in that order, which is not what a person types and not what the
/// assistant sends. Per term, both properties hold: every term is still literal,
/// and a document matches when it contains all of them anywhere.
///
/// A query of nothing but whitespace has no terms, and an empty `MATCH` is an
/// error rather than an empty result, so it returns nothing without asking.
pub async fn search_fact_text(pool: &SqlitePool, query: &str, limit: i64) -> Result<Vec<String>> {
    let terms: Vec<String> = query
        .split_whitespace()
        .map(|term| format!("\"{}\"", term.replace('"', "\"\"")))
        .collect();
    if terms.is_empty() {
        return Ok(Vec::new());
    }
    let ids = sqlx::query_scalar::<_, String>(
        "SELECT node_id FROM fact_text WHERE fact_text MATCH ?1 ORDER BY bm25(fact_text) LIMIT ?2",
    )
    .bind(terms.join(" "))
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn mem_pool() -> SqlitePool {
        // One connection so the in-memory database is shared across queries.
        SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("open in-memory sqlite")
    }

    #[tokio::test]
    async fn fts_index_searches_and_ranks_by_keyword() {
        // This also probes that SQLite ships FTS5 (the CREATE VIRTUAL TABLE would
        // error otherwise).
        let pool = mem_pool().await;
        create_fact_text_index(&pool).await.expect("FTS5 available");
        upsert_fact_text(&pool, "n1", "/home/tim/proj/main.rs main.rs proj source")
            .await
            .unwrap();
        upsert_fact_text(&pool, "n2", "/home/tim/proj/readme.md readme.md proj docs")
            .await
            .unwrap();

        let hits = search_fact_text(&pool, "source", 10).await.unwrap();
        assert_eq!(hits, vec!["n1".to_string()], "the keyword matches only the source file");

        let both = search_fact_text(&pool, "proj", 10).await.unwrap();
        assert_eq!(both.len(), 2, "a shared term matches both nodes");
    }

    /// The shape the whole-query quoting broke: two words a person types, sitting
    /// in one document but not next to each other.
    #[tokio::test]
    async fn several_terms_match_a_document_that_holds_them_all_apart() {
        let pool = mem_pool().await;
        create_fact_text_index(&pool).await.unwrap();
        upsert_fact_text(&pool, "/a/main.rs", "/a/main.rs main.rs a rust source with notes")
            .await
            .unwrap();
        upsert_fact_text(&pool, "/b/other.rs", "/b/other.rs other.rs a rust source")
            .await
            .unwrap();

        let hits = search_fact_text(&pool, "main.rs notes", 10).await.unwrap();
        assert_eq!(hits, vec!["/a/main.rs".to_string()], "both terms, not adjacent");

        // Every term still has to be there: the second document has neither.
        let none = search_fact_text(&pool, "other.rs notes", 10).await.unwrap();
        assert!(none.is_empty(), "a term the document lacks excludes it: {none:?}");

        // And a term full of FTS5 punctuation is still a literal token, which is
        // what the quoting was for.
        let dotted = search_fact_text(&pool, "main.rs", 10).await.unwrap();
        assert_eq!(dotted, vec!["/a/main.rs".to_string()]);

        // A query of nothing but spaces has no terms and asks nothing.
        assert!(search_fact_text(&pool, "   ", 10).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn upsert_is_idempotent_and_delete_removes() {
        let pool = mem_pool().await;
        create_fact_text_index(&pool).await.unwrap();
        upsert_fact_text(&pool, "n1", "alpha beta gamma").await.unwrap();
        // Re-upserting the same node must not create a duplicate row.
        upsert_fact_text(&pool, "n1", "alpha beta gamma").await.unwrap();
        assert_eq!(search_fact_text(&pool, "alpha", 10).await.unwrap().len(), 1);

        delete_fact_text(&pool, "n1").await.unwrap();
        assert!(
            search_fact_text(&pool, "alpha", 10).await.unwrap().is_empty(),
            "a deleted node leaves no index entry"
        );
    }
}

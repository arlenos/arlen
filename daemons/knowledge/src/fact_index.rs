//! Keyword text for every node type a caller may read, not only files and projects.
//!
//! The keyword arm of retrieval (`fts.rs`) was fed by two indexers, one for `File`
//! and one for `Project`, so a `Command`, a `Meeting`, a commit or a code symbol
//! could exist in the graph and never be found by a word in it. This is the rest,
//! as a table rather than one function per type: each [`Spec`] names a label and
//! the columns that carry its words.
//!
//! Two ways to know what is new, because the types differ:
//! - a type with a time column is read forward from a per-label mark, `(time,
//!   id)`, so every pass reads only what arrived since the last one and equal
//!   times across a batch boundary are neither lost nor read forever;
//! - a type without one is small or slow-changing (apps, branches, endpoints,
//!   action items, code symbols) and is re-read whole, bounded, but only when its
//!   count has changed. An edit that keeps the count, an app renamed in place, is
//!   picked up the next time anything of that type is added; named, not hidden.
//!
//! The marks live in SQLite beside the index they describe. Authority labels
//! (`Grant` and its relations) are not here: nothing outside `access_grants` may
//! read them. Which of the indexed ids a caller may SEE is decided at retrieval,
//! per label, as it always was.

use anyhow::Result;
use sqlx::SqlitePool;

use crate::graph::GraphHandle;

/// One indexed node type.
pub(crate) struct Spec {
    /// The node label.
    pub label: &'static str,
    /// An INT64 column that only grows as nodes arrive, or `None`.
    pub time: Option<&'static str>,
    /// The columns whose text is indexed, as `(column, key in the fact text)`.
    pub text: &'static [(&'static str, &'static str)],
}

/// The node types this pass indexes. `File` and `Project` have their own
/// indexers in `promotion.rs` and are not repeated here.
pub(crate) const SPECS: &[Spec] = &[
    Spec { label: "Command", time: Some("ran_at"), text: &[("command", "command"), ("cwd", "cwd")] },
    Spec {
        label: "Meeting",
        time: Some("started_at"),
        text: &[("title", "title"), ("summary", "summary"), ("participants", "participants")],
    },
    Spec { label: "Commit", time: Some("committed_at"), text: &[("message", "message"), ("author", "author")] },
    Spec {
        label: "Event",
        time: Some("timestamp"),
        text: &[("title", "title"), ("type", "type"), ("app_id", "app_id"), ("service", "service")],
    },
    Spec {
        label: "UserAction",
        time: Some("timestamp"),
        text: &[("category", "category"), ("action", "action"), ("subject", "subject")],
    },
    Spec { label: "AnnotationVersion", time: Some("recorded_at"), text: &[("data", "data")] },
    Spec { label: "Directory", time: Some("created_at"), text: &[("path", "path"), ("name", "name")] },
    Spec { label: "App", time: None, text: &[("id", "app"), ("name", "name")] },
    Spec { label: "Branch", time: None, text: &[("name", "name")] },
    Spec { label: "NetworkEndpoint", time: None, text: &[("id", "host"), ("protocol", "protocol")] },
    Spec { label: "ActionItem", time: None, text: &[("text", "text"), ("owner", "owner")] },
    Spec {
        label: "CodeSymbol",
        time: None,
        text: &[("name", "name"), ("kind", "kind"), ("source_file", "source_file")],
    },
];

/// How many nodes of one type a single pass reads. A backlog drains over several
/// passes instead of holding the graph thread for one long one.
const BATCH: i64 = 500;

/// How many nodes of an untimed type are re-read when its count changes.
const UNTIMED_CAP: i64 = 5000;

async fn ensure_marks(pool: &SqlitePool) -> Result<()> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS fact_index_mark (
            label TEXT PRIMARY KEY, mark_time INTEGER NOT NULL, mark_id TEXT NOT NULL)",
    )
    .execute(pool)
    .await?;
    Ok(())
}

async fn mark(pool: &SqlitePool, label: &str) -> Result<(i64, String)> {
    Ok(sqlx::query_as::<_, (i64, String)>(
        "SELECT mark_time, mark_id FROM fact_index_mark WHERE label = ?1",
    )
    .bind(label)
    .fetch_optional(pool)
    .await?
    .unwrap_or((i64::MIN, String::new())))
}

async fn set_mark(pool: &SqlitePool, label: &str, time: i64, id: &str) -> Result<()> {
    sqlx::query(
        "INSERT INTO fact_index_mark (label, mark_time, mark_id) VALUES (?1, ?2, ?3)
         ON CONFLICT(label) DO UPDATE SET mark_time = ?2, mark_id = ?3",
    )
    .bind(label)
    .bind(time)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Index what is new of every type in [`SPECS`]. Returns how many nodes it wrote.
/// One type failing is logged and does not stop the others.
pub(crate) async fn index_nodes(pool: &SqlitePool, graph: &GraphHandle) -> Result<usize> {
    ensure_marks(pool).await?;
    let mut written = 0;
    for spec in SPECS {
        match index_one(pool, graph, spec).await {
            Ok(n) => written += n,
            Err(e) => tracing::warn!(label = spec.label, error = %e, "could not index keyword text"),
        }
    }
    Ok(written)
}

async fn index_one(pool: &SqlitePool, graph: &GraphHandle, spec: &Spec) -> Result<usize> {
    let cols: Vec<String> = spec.text.iter().map(|(c, _)| format!("n.{c}")).collect();
    let label = spec.label;
    let rows = match spec.time {
        Some(t) => {
            let (mt, mid) = mark(pool, label).await?;
            let mid = crate::utils::escape_cypher(&mid);
            graph
                .query_rows(format!(
                    "MATCH (n:{label}) WITH n, coalesce(n.{t}, 0) AS t \
                     WHERE t > {mt} OR (t = {mt} AND n.id > '{mid}') \
                     RETURN n.id, t, {} ORDER BY t, n.id LIMIT {BATCH}",
                    cols.join(", ")
                ))
                .await?
        }
        None => {
            let count = graph
                .query_rows(format!("MATCH (n:{label}) RETURN count(n)"))
                .await?
                .rows
                .first()
                .and_then(|r| r.first())
                .map(|c| c.as_i64())
                .unwrap_or(0);
            if mark(pool, label).await?.0 == count {
                return Ok(0);
            }
            let rs = graph
                .query_rows(format!(
                    "MATCH (n:{label}) RETURN n.id, 0, {} ORDER BY n.id LIMIT {UNTIMED_CAP}",
                    cols.join(", ")
                ))
                .await?;
            set_mark(pool, label, count, "").await?;
            rs
        }
    };
    let mut last: Option<(i64, String)> = None;
    for row in &rows.rows {
        let id = row[0].as_str().to_string();
        if id.is_empty() {
            continue;
        }
        let mut fields = std::collections::BTreeMap::new();
        for (i, (_, key)) in spec.text.iter().enumerate() {
            let v = match &row[2 + i] {
                crate::graph::CellValue::String(s) => s.clone(),
                crate::graph::CellValue::Int64(n) => n.to_string(),
                _ => String::new(),
            };
            fields.insert((*key).to_string(), v);
        }
        let text = crate::retrieval::fact_text(label, &fields);
        if !text.is_empty() {
            crate::fts::upsert_fact_text(pool, &id, &text).await?;
        }
        last = Some((row[1].as_i64(), id));
    }
    if let (Some(_), Some((t, id))) = (spec.time, last) {
        set_mark(pool, label, t, &id).await?;
    }
    Ok(rows.rows.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn setup() -> (SqlitePool, GraphHandle, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let pool = crate::db::open(dir.path().join("e.db").to_str().unwrap()).await.unwrap();
        crate::fts::create_fact_text_index(&pool).await.unwrap();
        let graph = crate::graph::spawn(dir.path().join("g").to_str().unwrap()).unwrap();
        (pool, graph, dir)
    }

    /// A command, a meeting and a code symbol become findable by a word in them,
    /// which none of them was before this pass existed.
    #[tokio::test]
    async fn the_other_node_types_are_findable_by_their_words() {
        let (pool, graph, _d) = setup().await;
        for q in [
            "CREATE (:Command {id: 'c1', command: 'cargo build --release', cwd: '/src/arlen', ran_at: 10})",
            "CREATE (:Meeting {id: 'm1', title: 'Quartalsplanung', summary: 'budget', participants: 'Ana', started_at: 5})",
            "CREATE (:CodeSymbol {id: 's1', name: 'persist_retract', kind: 'function', source_file: 'daemon.rs'})",
        ] {
            graph.write(q.into()).await.unwrap();
        }
        assert!(index_nodes(&pool, &graph).await.unwrap() >= 3);
        assert_eq!(crate::fts::search_fact_text(&pool, "release", 10).await.unwrap(), vec!["c1".to_string()]);
        assert_eq!(crate::fts::search_fact_text(&pool, "Quartalsplanung", 10).await.unwrap(), vec!["m1".to_string()]);
        assert_eq!(crate::fts::search_fact_text(&pool, "persist_retract", 10).await.unwrap(), vec!["s1".to_string()]);
    }

    /// A timed type is read forward: a second pass with nothing new writes
    /// nothing, and a node arriving at the SAME time as the mark is still read.
    #[tokio::test]
    async fn a_timed_type_reads_only_what_is_new() {
        let (pool, graph, _d) = setup().await;
        graph.write("CREATE (:Command {id: 'a', command: 'first', ran_at: 10})".into()).await.unwrap();
        index_nodes(&pool, &graph).await.unwrap();
        assert_eq!(index_one(&pool, &graph, &SPECS[0]).await.unwrap(), 0, "nothing new");
        graph.write("CREATE (:Command {id: 'b', command: 'second', ran_at: 10})".into()).await.unwrap();
        assert_eq!(index_one(&pool, &graph, &SPECS[0]).await.unwrap(), 1, "a tie on time is read once");
        assert_eq!(crate::fts::search_fact_text(&pool, "second", 10).await.unwrap(), vec!["b".to_string()]);
    }

    /// An untimed type is re-read only when its count changes.
    #[tokio::test]
    async fn an_untimed_type_is_reread_when_its_count_changes() {
        let (pool, graph, _d) = setup().await;
        let branch = SPECS.iter().find(|s| s.label == "Branch").unwrap();
        graph.write("CREATE (:Branch {id: 'b1', name: 'feature-journal'})".into()).await.unwrap();
        ensure_marks(&pool).await.unwrap();
        assert_eq!(index_one(&pool, &graph, branch).await.unwrap(), 1);
        assert_eq!(index_one(&pool, &graph, branch).await.unwrap(), 0, "same count, not re-read");
        graph.write("CREATE (:Branch {id: 'b2', name: 'main'})".into()).await.unwrap();
        assert_eq!(index_one(&pool, &graph, branch).await.unwrap(), 2);
    }

    /// Every spec names a label and columns the schema declares, so a renamed
    /// column fails here rather than as an empty index on a running machine.
    #[tokio::test]
    async fn every_spec_reads_columns_the_schema_has() {
        let (pool, graph, _d) = setup().await;
        ensure_marks(&pool).await.unwrap();
        for spec in SPECS {
            index_one(&pool, &graph, spec).await.unwrap_or_else(|e| panic!("{}: {e}", spec.label));
        }
    }
}

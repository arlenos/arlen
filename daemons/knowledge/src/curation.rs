//! A person's identity decisions, stored as commands and replayed
//! (bitemporal-knowledge-graph.md §4.12).
//!
//! "These two are the same person", "these two are not", "this is not an entity at
//! all": a person decides these once, and a rebuild must not forget them. A
//! rebuild here is a rescan, a re-extraction or a model upgrade, after which every
//! mention and every suggestion is new. So the decisions are not edits to the
//! graph. They are commands in the SQLite record beside the fact journal, and
//! [`replay`] applies them to whatever the graph holds now, as often as it is
//! asked to.
//!
//! The graph side of a decision is a `SAME_AS` edge between two entities of one
//! type, never a merge of nodes: identity is an edge, and an edge can be closed,
//! which is how a wrong decision is undone. Each shared entity type gets its own
//! `SAME_AS` table, because the engine needs typed endpoints and entity types are
//! dynamic tables (`write::entity_table_name`).
//!
//! The commands are bitemporal like everything else: withdrawing one closes it
//! (`expired_at`), nothing is deleted, and a closed command stops applying.

use anyhow::{bail, Result};
use sqlx::SqlitePool;

use crate::graph::GraphHandle;
use crate::utils::escape_cypher;

/// One decision a person made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// `a` and `b` are the same entity.
    MustLink { entity_type: String, a: String, b: String },
    /// `a` and `b` are different entities, whatever they look like. Duplicate
    /// detection consults this before it proposes anything.
    CannotLink { entity_type: String, a: String, b: String },
    /// Every identity claim on `entity` is undone.
    Split { entity_type: String, entity: String },
    /// `entity` is called `name`.
    Rename { entity_type: String, entity: String, name: String },
    /// `mention` names no entity. Applied once mentions exist; recorded now so
    /// the decision is not lost before then.
    NotAnEntity { mention: String },
}

impl Command {
    fn columns(&self) -> (&'static str, &str, &str, &str, &str) {
        match self {
            Command::MustLink { entity_type, a, b } => ("must_link", entity_type, a, b, ""),
            Command::CannotLink { entity_type, a, b } => ("cannot_link", entity_type, a, b, ""),
            Command::Split { entity_type, entity } => ("split", entity_type, entity, "", ""),
            Command::Rename { entity_type, entity, name } => ("rename", entity_type, entity, "", name),
            Command::NotAnEntity { mention } => ("not_an_entity", "", mention, "", ""),
        }
    }

    fn from_columns(kind: &str, entity_type: String, a: String, b: String, arg: String) -> Result<Self> {
        Ok(match kind {
            "must_link" => Command::MustLink { entity_type, a, b },
            "cannot_link" => Command::CannotLink { entity_type, a, b },
            "split" => Command::Split { entity_type, entity: a },
            "rename" => Command::Rename { entity_type, entity: a, name: arg },
            "not_an_entity" => Command::NotAnEntity { mention: a },
            other => bail!("unknown curation command {other:?}"),
        })
    }
}

/// Create the command table. Idempotent.
pub async fn ensure_schema(pool: &SqlitePool) -> Result<()> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS curation (
            seq         INTEGER PRIMARY KEY AUTOINCREMENT,
            command     TEXT    NOT NULL,
            entity_type TEXT    NOT NULL,
            a           TEXT    NOT NULL,
            b           TEXT    NOT NULL,
            arg         TEXT    NOT NULL,
            decided_by  TEXT    NOT NULL,
            created_at  INTEGER NOT NULL,
            expired_at  INTEGER
        )",
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Record a decision. Returns its sequence number, which names it from then on.
pub async fn record(pool: &SqlitePool, command: &Command, decided_by: &str) -> Result<i64> {
    ensure_schema(pool).await?;
    let (kind, entity_type, a, b, arg) = command.columns();
    if a.is_empty() {
        bail!("a curation command names nothing");
    }
    let seq = sqlx::query_scalar::<_, i64>(
        "INSERT INTO curation (command, entity_type, a, b, arg, decided_by, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?) RETURNING seq",
    )
    .bind(kind)
    .bind(entity_type)
    .bind(a)
    .bind(b)
    .bind(arg)
    .bind(decided_by)
    .bind(crate::time::now().0)
    .fetch_one(pool)
    .await?;
    Ok(seq)
}

/// Withdraw a decision: close it, so it stops applying. Kept as history.
pub async fn withdraw(pool: &SqlitePool, seq: i64) -> Result<()> {
    sqlx::query("UPDATE curation SET expired_at = ? WHERE seq = ? AND expired_at IS NULL")
        .bind(crate::time::now().0)
        .bind(seq)
        .execute(pool)
        .await?;
    Ok(())
}

/// Every decision still in force, oldest first.
pub async fn live(pool: &SqlitePool) -> Result<Vec<(i64, Command)>> {
    ensure_schema(pool).await?;
    let rows: Vec<(i64, String, String, String, String, String)> = sqlx::query_as(
        "SELECT seq, command, entity_type, a, b, arg FROM curation
         WHERE expired_at IS NULL ORDER BY seq",
    )
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(|(seq, k, t, a, b, arg)| Ok((seq, Command::from_columns(&k, t, a, b, arg)?)))
        .collect()
}

/// Whether a person said `a` and `b` are different, in either order. Duplicate
/// detection asks this before it proposes the pair.
pub async fn cannot_link(pool: &SqlitePool, entity_type: &str, a: &str, b: &str) -> Result<bool> {
    ensure_schema(pool).await?;
    let n: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM curation WHERE command = 'cannot_link' AND expired_at IS NULL
         AND entity_type = ?1 AND ((a = ?2 AND b = ?3) OR (a = ?3 AND b = ?2))",
    )
    .bind(entity_type)
    .bind(a)
    .bind(b)
    .fetch_one(pool)
    .await?;
    Ok(n > 0)
}

/// The `SAME_AS` table for one entity type.
pub fn same_as_table(entity_type: &str) -> String {
    crate::write::entity_rel_table_name("SAME_AS", entity_type, entity_type)
}

/// Create the `SAME_AS` table for `entity_type`, with the four stamps and an op
/// id. Needs the entity table to exist; a type with no entity yet has nothing to
/// link, so its table is left until there is.
async fn ensure_same_as(graph: &GraphHandle, entity_type: &str) -> Result<bool> {
    let node = crate::write::entity_table_name(entity_type);
    let rel = same_as_table(entity_type);
    let made = graph
        .write(format!(
            "CREATE REL TABLE IF NOT EXISTS {rel}(FROM {node} TO {node}, op_id STRING, \
             valid_at INT64, invalid_at INT64, created_at INT64, expired_at INT64)"
        ))
        .await;
    Ok(made.is_ok())
}

/// Apply every decision in force to the graph as it stands. Idempotent: a
/// decision already applied changes nothing, so this runs after every rescan.
///
/// In order, so a later decision wins: a `must_link` followed by a `cannot_link`
/// of the same pair ends with no live `SAME_AS`, and the reverse ends with one.
pub async fn replay(pool: &SqlitePool, graph: &GraphHandle) -> Result<usize> {
    let now = crate::time::now().0;
    let mut applied = 0;
    for (seq, command) in live(pool).await? {
        let statements = match &command {
            Command::MustLink { entity_type, a, b } => {
                if !ensure_same_as(graph, entity_type).await? {
                    continue;
                }
                let (node, rel) = (crate::write::entity_table_name(entity_type), same_as_table(entity_type));
                let (a, b) = (escape_cypher(a), escape_cypher(b));
                vec![format!(
                    "MATCH (x:{node} {{id: '{a}'}}), (y:{node} {{id: '{b}'}}) \
                     OPTIONAL MATCH (x)-[l:{rel}]-(y) WHERE l.invalid_at IS NULL AND l.expired_at IS NULL \
                     WITH x, y, l WHERE l IS NULL \
                     CREATE (x)-[:{rel} {{op_id: 'must_link:{seq}', valid_at: {now}, created_at: {now}}}]->(y)"
                )]
            }
            Command::CannotLink { entity_type, a, b } => {
                if !ensure_same_as(graph, entity_type).await? {
                    continue;
                }
                let (node, rel) = (crate::write::entity_table_name(entity_type), same_as_table(entity_type));
                let (a, b) = (escape_cypher(a), escape_cypher(b));
                vec![format!(
                    "MATCH (x:{node} {{id: '{a}'}})-[l:{rel}]-(y:{node} {{id: '{b}'}}) \
                     WHERE l.invalid_at IS NULL AND l.expired_at IS NULL \
                     SET l.invalid_at = {now}, l.expired_at = {now}"
                )]
            }
            Command::Split { entity_type, entity } => {
                if !ensure_same_as(graph, entity_type).await? {
                    continue;
                }
                let (node, rel) = (crate::write::entity_table_name(entity_type), same_as_table(entity_type));
                let e = escape_cypher(entity);
                vec![format!(
                    "MATCH (x:{node} {{id: '{e}'}})-[l:{rel}]-() \
                     WHERE l.invalid_at IS NULL AND l.expired_at IS NULL \
                     SET l.invalid_at = {now}, l.expired_at = {now}"
                )]
            }
            Command::Rename { entity_type, entity, name } => {
                let node = crate::write::entity_table_name(entity_type);
                vec![format!(
                    "MATCH (x:{node} {{id: '{}'}}) SET x.name = '{}'",
                    escape_cypher(entity),
                    escape_cypher(name)
                )]
            }
            // No mentions exist yet; the decision waits in the record for them.
            Command::NotAnEntity { .. } => continue,
        };
        for s in statements {
            // A decision about an entity that is not in this graph (yet, or any
            // more) is not an error: it waits until the entity is there.
            if graph.write(s).await.is_ok() {
                applied += 1;
            }
        }
    }
    Ok(applied)
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: &str = "shared.Person";

    async fn setup() -> (SqlitePool, GraphHandle, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let pool = crate::db::open(dir.path().join("e.db").to_str().unwrap()).await.unwrap();
        let graph = crate::graph::spawn(dir.path().join("g").to_str().unwrap()).unwrap();
        let node = crate::write::entity_table_name(T);
        graph
            .write(format!("CREATE NODE TABLE IF NOT EXISTS {node}(id STRING, name STRING, PRIMARY KEY(id))"))
            .await
            .unwrap();
        for (id, name) in [("anna-1", "Anna Maier"), ("anna-2", "A. Maier"), ("anna-3", "Anna Moser")] {
            graph.write(format!("CREATE (:{node} {{id: '{id}', name: '{name}'}})")).await.unwrap();
        }
        (pool, graph, dir)
    }

    async fn live_links(graph: &GraphHandle) -> i64 {
        let rel = same_as_table(T);
        graph
            .query_rows(format!(
                "MATCH ()-[l:{rel}]->() WHERE l.invalid_at IS NULL AND l.expired_at IS NULL RETURN count(l)"
            ))
            .await
            .unwrap()
            .rows[0][0]
            .as_i64()
    }

    /// A `must_link` becomes one live `SAME_AS`, and replaying it again adds nothing.
    #[tokio::test]
    async fn a_must_link_is_one_edge_however_often_it_replays() {
        let (pool, graph, _d) = setup().await;
        record(&pool, &Command::MustLink { entity_type: T.into(), a: "anna-1".into(), b: "anna-2".into() }, "person")
            .await
            .unwrap();
        replay(&pool, &graph).await.unwrap();
        replay(&pool, &graph).await.unwrap();
        assert_eq!(live_links(&graph).await, 1);
        // Both nodes are still there: identity is an edge, never a merge.
        let node = crate::write::entity_table_name(T);
        let n = graph.query_rows(format!("MATCH (x:{node}) RETURN count(x)")).await.unwrap();
        assert_eq!(n.rows[0][0].as_i64(), 3);
    }

    /// The later decision wins, and the closed edge stays as history.
    #[tokio::test]
    async fn a_later_cannot_link_closes_the_earlier_link() {
        let (pool, graph, _d) = setup().await;
        let pair = |c: fn(String, String, String) -> Command| c(T.into(), "anna-1".into(), "anna-2".into());
        record(&pool, &pair(|entity_type, a, b| Command::MustLink { entity_type, a, b }), "person").await.unwrap();
        record(&pool, &pair(|entity_type, a, b| Command::CannotLink { entity_type, a, b }), "person").await.unwrap();
        replay(&pool, &graph).await.unwrap();
        assert_eq!(live_links(&graph).await, 0);
        assert!(cannot_link(&pool, T, "anna-2", "anna-1").await.unwrap(), "either order");
        assert!(!cannot_link(&pool, T, "anna-1", "anna-3").await.unwrap());
    }

    /// The point of commands: a rebuilt graph gets the decision back.
    #[tokio::test]
    async fn a_decision_survives_a_rebuild() {
        let (pool, graph, dir) = setup().await;
        record(&pool, &Command::MustLink { entity_type: T.into(), a: "anna-1".into(), b: "anna-2".into() }, "person")
            .await
            .unwrap();
        replay(&pool, &graph).await.unwrap();

        let fresh = crate::graph::spawn(dir.path().join("g2").to_str().unwrap()).unwrap();
        let node = crate::write::entity_table_name(T);
        fresh
            .write(format!("CREATE NODE TABLE IF NOT EXISTS {node}(id STRING, name STRING, PRIMARY KEY(id))"))
            .await
            .unwrap();
        for id in ["anna-1", "anna-2"] {
            fresh.write(format!("CREATE (:{node} {{id: '{id}'}})")).await.unwrap();
        }
        replay(&pool, &fresh).await.unwrap();
        assert_eq!(live_links(&fresh).await, 1);
    }

    /// Split undoes every link on an entity; a withdrawn decision stops applying.
    #[tokio::test]
    async fn split_and_withdraw() {
        let (pool, graph, _d) = setup().await;
        let link = |b: &str| Command::MustLink { entity_type: T.into(), a: "anna-1".into(), b: b.into() };
        record(&pool, &link("anna-2"), "person").await.unwrap();
        let second = record(&pool, &link("anna-3"), "person").await.unwrap();
        replay(&pool, &graph).await.unwrap();
        assert_eq!(live_links(&graph).await, 2);

        let split = record(&pool, &Command::Split { entity_type: T.into(), entity: "anna-1".into() }, "person")
            .await
            .unwrap();
        replay(&pool, &graph).await.unwrap();
        assert_eq!(live_links(&graph).await, 0);

        // Withdraw the split and the second link: the first link comes back as a
        // new edge on the next replay, the second does not.
        withdraw(&pool, split).await.unwrap();
        withdraw(&pool, second).await.unwrap();
        replay(&pool, &graph).await.unwrap();
        assert_eq!(live_links(&graph).await, 1);
    }

    #[tokio::test]
    async fn rename_and_an_unknown_command_kind() {
        let (pool, graph, _d) = setup().await;
        record(&pool, &Command::Rename { entity_type: T.into(), entity: "anna-2".into(), name: "Anna Maier".into() }, "person")
            .await
            .unwrap();
        replay(&pool, &graph).await.unwrap();
        let node = crate::write::entity_table_name(T);
        let n = graph.query_rows(format!("MATCH (x:{node} {{id: 'anna-2'}}) RETURN x.name")).await.unwrap();
        assert_eq!(n.rows[0][0].as_str(), "Anna Maier");
        assert!(Command::from_columns("merge", String::new(), String::new(), String::new(), String::new()).is_err());
    }
}

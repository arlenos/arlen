//! The fact journal: the graph's canonical record, in `SQLite`.
//!
//! Decided in `kg-engine-decision.md`, "The write side is the record" (D11). The
//! SQLite event log keeps raw events for 30 days and then deletes them, and the
//! entity layer wrote straight into Ladybug, so for anything older than a month
//! the query engine had become the only copy of what the graph knew - and it is
//! the part of the stack with one-way storage upgrades and one maintainer. This
//! module turns that around: every node the graph upserts and every edge it
//! appends or closes is written here first, and Ladybug is a projection that can
//! be dropped and rebuilt from these rows.
//!
//! Facts, not Cypher. Journaling the statements would rebuild Ladybug, but a
//! record that only one engine can replay keeps the engine as the authority in
//! all but name; the named fallback (Grafeo) would have to parse Kuzu's dialect.
//! A [`Fact`] says what became true, and [`project`] is the one place that knows
//! how Ladybug spells it.

use anyhow::{bail, Result};
use serde_json::Value;
use sqlx::{Sqlite, SqlitePool, Transaction};
use std::collections::BTreeMap;

use crate::utils::escape_cypher;

/// The metadata key for the last journal `seq` applied to Ladybug.
const PROJECTED_KEY: &str = "projected_seq";

/// The four bi-temporal stamps of an edge, microseconds since the epoch
/// (`bitemporal-knowledge-graph.md` §3). `None` is open: not yet invalid, not yet
/// expired.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stamps {
    /// When the fact became true in the world.
    pub valid_at: Option<i64>,
    /// When it stopped being true in the world.
    pub invalid_at: Option<i64>,
    /// When the system learned it.
    pub created_at: Option<i64>,
    /// When the system stopped believing it.
    pub expired_at: Option<i64>,
}

/// One thing the graph came to know. The variants are the only three ways a fact
/// enters it: a node's properties are set, an edge is appended, an edge is closed.
#[derive(Debug, Clone, PartialEq)]
pub enum Fact {
    /// Upsert a node by id and set these properties. `on_create` properties are
    /// set only when the node is new, which is how a first-seen stamp survives
    /// every later upsert.
    Node {
        label: String,
        id: String,
        props: BTreeMap<String, Value>,
        on_create: BTreeMap<String, Value>,
    },
    /// Ensure an edge between two existing nodes. With `op_id` set the edge is an
    /// identified one (a temporal membership); without, it is merged on its
    /// endpoints, which is what the observation edges have always been.
    Edge {
        rel: String,
        from: (String, String),
        to: (String, String),
        op_id: Option<String>,
        stamps: Stamps,
        props: BTreeMap<String, Value>,
    },
    /// Close the live edge carrying `op_id`: stamp `invalid_at` and `expired_at`.
    /// Close, never delete (§4.7).
    Close {
        rel: String,
        op_id: String,
        at: i64,
    },
    /// Close every live `rel` edge leaving node `from`: the "supersede whatever
    /// version is current" step, for a writer that does not know that version's
    /// id. Deterministic under replay because facts apply in `seq` order, so the
    /// live set at this fact is the one the original write saw.
    CloseFrom {
        rel: String,
        from: (String, String),
        at: i64,
    },
    /// Close every live `rel` edge INTO a node: a project that stops being one
    /// closes its members' memberships, which is what its delete used to remove.
    CloseTo {
        rel: String,
        to: (String, String),
        at: i64,
    },
    /// One occurrence of something that recurs between two nodes - an app
    /// launching another. The record keeps every occurrence under its own `id`;
    /// the graph keeps ONE edge per pair whose `count`, `first_seen` and
    /// `last_seen` the projection computes from the record, over DISTINCT ids, so
    /// a fact journaled twice cannot count twice and a rebuild arrives at the same
    /// numbers. One edge per pair is a privacy property, not an economy: an edge
    /// per exec would be the minute-by-minute activity map `provenance-halo.md`
    /// §7 forbids.
    Occurrence {
        rel: String,
        id: String,
        from: (String, String),
        to: (String, String),
        at: i64,
    },
}

impl Fact {
    /// A node upsert setting `props`, nothing set only on create.
    pub fn node(label: &str, id: &str, props: &[(&str, Value)]) -> Self {
        Fact::Node {
            label: label.into(),
            id: id.into(),
            props: props.iter().map(|(k, v)| (k.to_string(), v.clone())).collect(),
            on_create: BTreeMap::new(),
        }
    }

    /// An endpoint-merged observation edge with no stamps and no properties: the
    /// shape every `ACCESSED_*` / `ACTIVE_IN` edge has.
    pub fn link(rel: &str, from: (&str, &str), to: (&str, &str)) -> Self {
        Fact::Edge {
            rel: rel.into(),
            from: (from.0.into(), from.1.into()),
            to: (to.0.into(), to.1.into()),
            op_id: None,
            stamps: Stamps::default(),
            props: BTreeMap::new(),
        }
    }
}

impl Fact {
    /// An endpoint-merged edge whose properties are set on every observation
    /// (`CONNECTED_TO`'s direction and last-seen time).
    pub fn link_with(rel: &str, from: (&str, &str), to: (&str, &str), props: &[(&str, Value)]) -> Self {
        let Fact::Edge { rel, from, to, op_id, stamps, .. } = Fact::link(rel, from, to) else {
            unreachable!("link builds an edge")
        };
        Fact::Edge {
            rel,
            from,
            to,
            op_id,
            stamps,
            props: props.iter().map(|(k, v)| (k.to_string(), v.clone())).collect(),
        }
    }
}

/// Where a fact came from, recorded beside it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Origin {
    /// The provenance key (`provenance.rs`): `graph`, `agent`, `user`...
    pub provenance: String,
    /// What fixed the valid time, one of [`VALID_TIME_SOURCES`]
    /// (bitemporal-knowledge-graph.md §3.1). Empty when the fact carries no valid
    /// time; [`record`] refuses anything else.
    pub valid_time_source: String,
    /// The raw event the fact was derived from, if any.
    pub event_id: Option<String>,
}

/// Where a valid time may come from, and nothing else (§3.1):
/// - `observed`: the event is its own evidence (a focus, an open, a command);
/// - `source_field`: the source states the time (git author date, a mail `Date`);
/// - `text_extracted`: a date read out of content;
/// - `ingest_fallback`: nothing better existed, and the fact says so.
///
/// A closed list on purpose. The column used to take whatever a writer passed,
/// and two writers passed `event` and `write`, which name where a fact came from
/// rather than how its time was known. An import that wants a new source has to
/// add it here, where the next reader of the table will see it.
pub const VALID_TIME_SOURCES: [&str; 4] = ["observed", "source_field", "text_extracted", "ingest_fallback"];

impl Origin {
    /// A fact the promotion pipeline derived from raw event `event_id`, whose
    /// valid time is the event's own: it was observed.
    pub fn event(event_id: &str) -> Self {
        Origin {
            provenance: "graph".into(),
            valid_time_source: "observed".into(),
            event_id: Some(event_id.into()),
        }
    }
}

/// Create the journal table. Idempotent.
pub async fn ensure_schema(pool: &SqlitePool) -> Result<()> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS facts (
            seq               INTEGER PRIMARY KEY AUTOINCREMENT,
            op                TEXT    NOT NULL,
            label             TEXT    NOT NULL,
            id                TEXT    NOT NULL,
            from_label        TEXT,
            from_id           TEXT,
            to_label          TEXT,
            to_id             TEXT,
            props             TEXT    NOT NULL DEFAULT '{}',
            on_create         TEXT    NOT NULL DEFAULT '{}',
            valid_at          INTEGER,
            invalid_at        INTEGER,
            created_at        INTEGER,
            expired_at        INTEGER,
            provenance        TEXT    NOT NULL,
            valid_time_source TEXT    NOT NULL DEFAULT '',
            event_id          TEXT
        )",
    )
    .execute(pool)
    .await?;
    // The two names written before the list was closed, mapped once: `event` was
    // the promotion pipeline (observed), `write` the agent's own assertion, which
    // carries no time of its own (ingest_fallback).
    sqlx::query(
        "UPDATE facts SET valid_time_source = CASE valid_time_source \
           WHEN 'event' THEN 'observed' WHEN 'write' THEN 'ingest_fallback' END \
         WHERE valid_time_source IN ('event', 'write')",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS metadata (key TEXT PRIMARY KEY, value TEXT NOT NULL)",
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Append `facts` inside `tx`, in order. The caller commits: the point is that the
/// facts and the high-water mark they were derived up to land together or not at
/// all.
pub async fn record(tx: &mut Transaction<'_, Sqlite>, facts: &[(Fact, Origin)]) -> Result<()> {
    for (fact, origin) in facts {
        let source = origin.valid_time_source.as_str();
        if !source.is_empty() && !VALID_TIME_SOURCES.contains(&source) {
            bail!("journal valid_time_source {source:?} is not one of {VALID_TIME_SOURCES:?}");
        }
        let q = sqlx::query(
            "INSERT INTO facts (op, label, id, from_label, from_id, to_label, to_id,
                                props, on_create, valid_at, invalid_at, created_at,
                                expired_at, provenance, valid_time_source, event_id)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        );
        let q = match fact {
            Fact::Node { label, id, props, on_create } => q
                .bind("node")
                .bind(label)
                .bind(id)
                .bind(None::<String>)
                .bind(None::<String>)
                .bind(None::<String>)
                .bind(None::<String>)
                .bind(serde_json::to_string(props)?)
                .bind(serde_json::to_string(on_create)?)
                .bind(None::<i64>)
                .bind(None::<i64>)
                .bind(None::<i64>)
                .bind(None::<i64>),
            Fact::Edge { rel, from, to, op_id, stamps, props } => q
                .bind("edge")
                .bind(rel)
                .bind(op_id.clone().unwrap_or_default())
                .bind(&from.0)
                .bind(&from.1)
                .bind(&to.0)
                .bind(&to.1)
                .bind(serde_json::to_string(props)?)
                .bind("{}")
                .bind(stamps.valid_at)
                .bind(stamps.invalid_at)
                .bind(stamps.created_at)
                .bind(stamps.expired_at),
            Fact::Occurrence { rel, id, from, to, at } => q
                .bind("occurrence")
                .bind(rel)
                .bind(id)
                .bind(&from.0)
                .bind(&from.1)
                .bind(&to.0)
                .bind(&to.1)
                .bind("{}")
                .bind("{}")
                .bind(Some(*at))
                .bind(None::<i64>)
                .bind(None::<i64>)
                .bind(None::<i64>),
            Fact::CloseTo { rel, to, at } => q
                .bind("close_to")
                .bind(rel)
                .bind("")
                .bind(None::<String>)
                .bind(None::<String>)
                .bind(&to.0)
                .bind(&to.1)
                .bind("{}")
                .bind("{}")
                .bind(None::<i64>)
                .bind(Some(*at))
                .bind(None::<i64>)
                .bind(Some(*at)),
            Fact::CloseFrom { rel, from, at } => q
                .bind("close_from")
                .bind(rel)
                .bind("")
                .bind(&from.0)
                .bind(&from.1)
                .bind(None::<String>)
                .bind(None::<String>)
                .bind("{}")
                .bind("{}")
                .bind(None::<i64>)
                .bind(Some(*at))
                .bind(None::<i64>)
                .bind(Some(*at)),
            Fact::Close { rel, op_id, at } => q
                .bind("close")
                .bind(rel)
                .bind(op_id)
                .bind(None::<String>)
                .bind(None::<String>)
                .bind(None::<String>)
                .bind(None::<String>)
                .bind("{}")
                .bind("{}")
                .bind(None::<i64>)
                .bind(Some(*at))
                .bind(None::<i64>)
                .bind(Some(*at)),
        };
        q.bind(&origin.provenance)
            .bind(&origin.valid_time_source)
            .bind(&origin.event_id)
            .execute(&mut **tx)
            .await?;
    }
    Ok(())
}

/// One journal row as stored.
type Row = (
    i64,
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    String,
    String,
    Option<i64>,
    Option<i64>,
    Option<i64>,
    Option<i64>,
);

/// The facts after `after`, oldest first, at most `limit` of them, with their `seq`.
pub async fn read_since(pool: &SqlitePool, after: i64, limit: i64) -> Result<Vec<(i64, Fact)>> {
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT seq, op, label, id, from_label, from_id, to_label, to_id, props, on_create,
                valid_at, invalid_at, created_at, expired_at
         FROM facts WHERE seq > ? ORDER BY seq ASC LIMIT ?",
    )
    .bind(after)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(|r| Ok((r.0, row_to_fact(r)?))).collect()
}

fn row_to_fact(r: Row) -> Result<Fact> {
    let (_, op, label, id, fl, fi, tl, ti, props, on_create, va, ia, ca, ea) = r;
    Ok(match op.as_str() {
        "node" => Fact::Node {
            label,
            id,
            props: serde_json::from_str(&props)?,
            on_create: serde_json::from_str(&on_create)?,
        },
        "edge" => {
            let (Some(fl), Some(fi), Some(tl), Some(ti)) = (fl, fi, tl, ti) else {
                bail!("journal edge row without both endpoints");
            };
            Fact::Edge {
                rel: label,
                from: (fl, fi),
                to: (tl, ti),
                op_id: (!id.is_empty()).then_some(id),
                stamps: Stamps { valid_at: va, invalid_at: ia, created_at: ca, expired_at: ea },
                props: serde_json::from_str(&props)?,
            }
        }
        "occurrence" => {
            let (Some(fl), Some(fi), Some(tl), Some(ti)) = (fl, fi, tl, ti) else {
                bail!("journal occurrence row without both endpoints");
            };
            Fact::Occurrence {
                rel: label,
                id,
                from: (fl, fi),
                to: (tl, ti),
                at: va.ok_or_else(|| anyhow::anyhow!("journal occurrence row without its time"))?,
            }
        }
        "close_to" => {
            let (Some(tl), Some(ti)) = (tl, ti) else {
                bail!("journal close_to row without its node");
            };
            Fact::CloseTo {
                rel: label,
                to: (tl, ti),
                at: ia.ok_or_else(|| anyhow::anyhow!("journal close_to row without its time"))?,
            }
        }
        "close_from" => {
            let (Some(fl), Some(fi)) = (fl, fi) else {
                bail!("journal close_from row without its node");
            };
            Fact::CloseFrom {
                rel: label,
                from: (fl, fi),
                at: ia.ok_or_else(|| anyhow::anyhow!("journal close_from row without its time"))?,
            }
        }
        "close" => Fact::Close {
            rel: label,
            op_id: id,
            at: ia.ok_or_else(|| anyhow::anyhow!("journal close row without its time"))?,
        },
        other => bail!("journal row with unknown op {other:?}"),
    })
}

/// The last `seq` projected into Ladybug; 0 before the first projection.
pub async fn projected_seq(pool: &SqlitePool) -> Result<i64> {
    let row: Option<(String,)> = sqlx::query_as("SELECT value FROM metadata WHERE key = ?")
        .bind(PROJECTED_KEY)
        .fetch_optional(pool)
        .await?;
    Ok(row.and_then(|(v,)| v.parse().ok()).unwrap_or(0))
}

/// Record that everything up to `seq` is in Ladybug.
pub async fn set_projected_seq(pool: &SqlitePool, seq: i64) -> Result<()> {
    sqlx::query(
        "INSERT INTO metadata (key, value) VALUES (?, ?)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(PROJECTED_KEY)
    .bind(seq.to_string())
    .execute(pool)
    .await?;
    Ok(())
}

/// A label, relationship type or property name the projection may interpolate.
/// The journal is written by this daemon, but a projector that trusts its rows
/// would turn a corrupted row into a Cypher statement.
fn ident(s: &str) -> Result<&str> {
    let ok = !s.is_empty()
        && s.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if ok {
        Ok(s)
    } else {
        bail!("journal identifier {s:?} is not a plain name")
    }
}

/// A JSON scalar as a Cypher literal. Floats are refused: the graph stores times
/// and counts as INT64, and a float in the journal would round-trip differently
/// through each engine.
fn literal(v: &Value) -> Result<String> {
    Ok(match v {
        Value::Null => "NULL".into(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => match n.as_i64() {
            Some(i) => i.to_string(),
            None => bail!("journal number {n} is not an integer"),
        },
        Value::String(s) => format!("'{}'", escape_cypher(s)),
        _ => bail!("journal property is not a scalar"),
    })
}

fn opt(v: Option<i64>) -> String {
    v.map_or_else(|| "NULL".into(), |i| i.to_string())
}

fn assignments(var: &str, props: &BTreeMap<String, Value>) -> Result<Vec<String>> {
    props
        .iter()
        .map(|(k, v)| Ok(format!("{var}.{} = {}", ident(k)?, literal(v)?)))
        .collect()
}

/// The Cypher that makes `fact` true in Ladybug. Every statement is idempotent,
/// so replaying a fact that is already projected changes nothing.
pub fn project(fact: &Fact) -> Result<String> {
    match fact {
        Fact::Node { label, id, props, on_create } => {
            let mut s = format!("MERGE (n:{} {{id: '{}'}})", ident(label)?, escape_cypher(id));
            let created = assignments("n", on_create)?;
            if !created.is_empty() {
                s.push_str(&format!(" ON CREATE SET {}", created.join(", ")));
            }
            let set = assignments("n", props)?;
            if !set.is_empty() {
                s.push_str(&format!(" SET {}", set.join(", ")));
            }
            Ok(s)
        }
        Fact::Edge { rel, from, to, op_id, stamps, props } => {
            let head = format!(
                "MATCH (a:{} {{id: '{}'}}), (b:{} {{id: '{}'}})",
                ident(&from.0)?,
                escape_cypher(&from.1),
                ident(&to.0)?,
                escape_cypher(&to.1),
            );
            let rel = ident(rel)?;
            let mut set = assignments("r", props)?;
            match op_id {
                Some(op) => {
                    set.push(format!("r.valid_at = {}", opt(stamps.valid_at)));
                    set.push(format!("r.invalid_at = {}", opt(stamps.invalid_at)));
                    set.push(format!("r.created_at = {}", opt(stamps.created_at)));
                    set.push(format!("r.expired_at = {}", opt(stamps.expired_at)));
                    Ok(format!(
                        "{head} MERGE (a)-[r:{rel} {{op_id: '{}'}}]->(b) SET {}",
                        escape_cypher(op),
                        set.join(", ")
                    ))
                }
                None if set.is_empty() => Ok(format!("{head} MERGE (a)-[r:{rel}]->(b)")),
                None => Ok(format!("{head} MERGE (a)-[r:{rel}]->(b) SET {}", set.join(", "))),
            }
        }
        Fact::Occurrence { .. } => {
            bail!("an occurrence is projected from the record's aggregate; see project_occurrence")
        }
        Fact::CloseTo { rel, to, at } => Ok(format!(
            "MATCH ()-[r:{}]->(b:{} {{id: '{}'}}) \
             WHERE r.invalid_at IS NULL AND r.expired_at IS NULL \
             SET r.invalid_at = {at}, r.expired_at = {at}",
            ident(rel)?,
            ident(&to.0)?,
            escape_cypher(&to.1),
        )),
        Fact::CloseFrom { rel, from, at } => Ok(format!(
            "MATCH (a:{} {{id: '{}'}})-[r:{}]->() \
             WHERE r.invalid_at IS NULL AND r.expired_at IS NULL \
             SET r.invalid_at = {at}, r.expired_at = {at}",
            ident(&from.0)?,
            escape_cypher(&from.1),
            ident(rel)?,
        )),
        Fact::Close { rel, op_id, at } => Ok(format!(
            "MATCH ()-[r:{} {{op_id: '{}'}}]->() WHERE r.invalid_at IS NULL \
             SET r.invalid_at = {at}, r.expired_at = {at}",
            ident(rel)?,
            escape_cypher(op_id),
        )),
    }
}

/// Record `facts` in one transaction, then bring the projection up to date.
///
/// The record commits first: if the projection fails, the facts are already the
/// truth and the next call applies them. The other order would leave a graph
/// holding things the record never heard of.
pub async fn commit_and_project(
    pool: &SqlitePool,
    graph: &crate::graph::GraphHandle,
    facts: Vec<Fact>,
    origin: &Origin,
) -> Result<()> {
    let tagged: Vec<(Fact, Origin)> = facts.into_iter().map(|f| (f, origin.clone())).collect();
    let mut tx = pool.begin().await?;
    record(&mut tx, &tagged).await?;
    tx.commit().await?;
    project_pending(pool, graph).await?;
    Ok(())
}

/// How often `rel` has occurred between `from` and `to` in the record up to and
/// including `seq`, and the first and last time: counted over distinct ids.
async fn occurrence_tally(
    pool: &SqlitePool,
    rel: &str,
    from: &(String, String),
    to: &(String, String),
    seq: i64,
) -> Result<(i64, i64, i64)> {
    let row: (i64, Option<i64>, Option<i64>) = sqlx::query_as(
        "SELECT count(DISTINCT id), min(valid_at), max(valid_at) FROM facts
         WHERE op = 'occurrence' AND label = ? AND from_label = ? AND from_id = ?
           AND to_label = ? AND to_id = ? AND seq <= ?",
    )
    .bind(rel)
    .bind(&from.0)
    .bind(&from.1)
    .bind(&to.0)
    .bind(&to.1)
    .bind(seq)
    .fetch_one(pool)
    .await?;
    Ok((row.0, row.1.unwrap_or(0), row.2.unwrap_or(0)))
}

/// The Cypher for an occurrence: the pair's one edge, carrying the tally.
fn project_occurrence(fact: &Fact, tally: (i64, i64, i64)) -> Result<String> {
    let Fact::Occurrence { rel, from, to, .. } = fact else {
        bail!("not an occurrence");
    };
    let (count, first, last) = tally;
    Ok(format!(
        "MATCH (a:{} {{id: '{}'}}), (b:{} {{id: '{}'}}) MERGE (a)-[l:{}]->(b) \
         SET l.count = {count}, l.first_seen = {first}, l.last_seen = {last}",
        ident(&from.0)?,
        escape_cypher(&from.1),
        ident(&to.0)?,
        escape_cypher(&to.1),
        ident(rel)?,
    ))
}

/// The statement that makes the fact at `seq` true in Ladybug.
async fn statement(pool: &SqlitePool, seq: i64, fact: &Fact) -> Result<String> {
    match fact {
        Fact::Occurrence { rel, from, to, .. } => {
            project_occurrence(fact, occurrence_tally(pool, rel, from, to, seq).await?)
        }
        other => project(other),
    }
}

/// Apply every fact after the projected mark to `graph`, in order. Returns how
/// many facts were applied.
///
/// The mark moves once per batch, not once per fact: every projection is
/// idempotent, so a failure halfway re-applies the head of the batch on the next
/// call and changes nothing, while a per-fact mark cost one more SQLite write for
/// every fact the graph learns. On failure the mark still moves to the last fact
/// that did land, so a fact that cannot be projected is retried, not skipped.
pub async fn project_pending(pool: &SqlitePool, graph: &crate::graph::GraphHandle) -> Result<usize> {
    let mut applied = 0;
    loop {
        let from = projected_seq(pool).await?;
        let batch = read_since(pool, from, 500).await?;
        if batch.is_empty() {
            return Ok(applied);
        }
        let mut last = from;
        for (seq, fact) in batch {
            if let Err(e) = async { graph.write(statement(pool, seq, &fact).await?).await }.await {
                if last > from {
                    set_projected_seq(pool, last).await?;
                }
                return Err(e);
            }
            last = seq;
            applied += 1;
        }
        set_projected_seq(pool, last).await?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn props(pairs: &[(&str, Value)]) -> BTreeMap<String, Value> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
    }

    async fn pool() -> (SqlitePool, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let pool = crate::db::open(dir.path().join("j.db").to_str().unwrap()).await.unwrap();
        (pool, dir)
    }

    fn origin() -> Origin {
        Origin { provenance: "graph".into(), valid_time_source: "observed".into(), event_id: Some("e1".into()) }
    }

    #[tokio::test]
    async fn facts_round_trip_through_the_journal_in_order() {
        let (pool, _d) = pool().await;
        let facts = vec![
            Fact::Node {
                label: "File".into(),
                id: "/a.rs".into(),
                props: props(&[("path", json!("/a.rs")), ("last_cgroup_id", json!(7))]),
                on_create: props(&[("last_accessed", json!(100))]),
            },
            Fact::Edge {
                rel: "FILE_PART_OF".into(),
                from: ("File".into(), "/a.rs".into()),
                to: ("Project".into(), "p1".into()),
                op_id: Some("op-1".into()),
                stamps: Stamps { valid_at: Some(5), created_at: Some(6), ..Default::default() },
                props: BTreeMap::new(),
            },
            Fact::Close { rel: "FILE_PART_OF".into(), op_id: "op-1".into(), at: 9 },
            Fact::CloseFrom { rel: "HAS_VERSION".into(), from: ("Annotation".into(), "a1".into()), at: 11 },
            Fact::CloseTo { rel: "FILE_PART_OF".into(), to: ("Project".into(), "p1".into()), at: 11 },
            Fact::Occurrence {
                rel: "LAUNCHED".into(),
                id: "ev-9".into(),
                from: ("App".into(), "shell".into()),
                to: ("App".into(), "files".into()),
                at: 12,
            },
        ];
        let tagged: Vec<_> = facts.iter().cloned().map(|f| (f, origin())).collect();
        let mut tx = pool.begin().await.unwrap();
        record(&mut tx, &tagged).await.unwrap();
        tx.commit().await.unwrap();
        let back: Vec<Fact> = read_since(&pool, 0, 100).await.unwrap().into_iter().map(|(_, f)| f).collect();
        assert_eq!(back, facts);
    }

    #[tokio::test]
    async fn an_uncommitted_transaction_leaves_no_fact_behind() {
        let (pool, _d) = pool().await;
        {
            let mut tx = pool.begin().await.unwrap();
            record(&mut tx, &[(Fact::Close { rel: "R".into(), op_id: "x".into(), at: 1 }, origin())])
                .await
                .unwrap();
            // dropped without commit: a pass whose HWM write failed
        }
        assert!(read_since(&pool, 0, 10).await.unwrap().is_empty());
    }

    /// The journal's whole claim: a graph dropped and rebuilt from it is the graph
    /// it was. Two files, a project, a membership appended and then closed, and a
    /// node upserted twice, which must keep its first-seen stamp.
    #[tokio::test]
    async fn a_graph_rebuilt_from_the_journal_is_the_graph_it_was() {
        let (pool, dir) = pool().await;
        let node = |label: &str, id: &str, p: &[(&str, Value)], c: &[(&str, Value)]| Fact::Node {
            label: label.into(),
            id: id.into(),
            props: props(p),
            on_create: props(c),
        };
        let facts = vec![
            node("Project", "p1", &[("name", json!("arlen")), ("root_path", json!("/r"))], &[]),
            node("File", "/r/a.rs", &[("path", json!("/r/a.rs"))], &[("last_accessed", json!(100))]),
            node("File", "/r/b.rs", &[("path", json!("/r/b.rs"))], &[("last_accessed", json!(110))]),
            node("File", "/r/a.rs", &[("path", json!("/r/a.rs"))], &[("last_accessed", json!(999))]),
            Fact::Edge {
                rel: "FILE_PART_OF".into(),
                from: ("File".into(), "/r/a.rs".into()),
                to: ("Project".into(), "p1".into()),
                op_id: Some("op-a".into()),
                stamps: Stamps { valid_at: Some(100), created_at: Some(100), ..Default::default() },
                props: BTreeMap::new(),
            },
            Fact::Edge {
                rel: "FILE_PART_OF".into(),
                from: ("File".into(), "/r/b.rs".into()),
                to: ("Project".into(), "p1".into()),
                op_id: Some("op-b".into()),
                stamps: Stamps { valid_at: Some(110), created_at: Some(110), ..Default::default() },
                props: BTreeMap::new(),
            },
            Fact::Close { rel: "FILE_PART_OF".into(), op_id: "op-b".into(), at: 200 },
        ];
        let tagged: Vec<_> = facts.into_iter().map(|f| (f, origin())).collect();
        let mut tx = pool.begin().await.unwrap();
        record(&mut tx, &tagged).await.unwrap();
        tx.commit().await.unwrap();

        let snapshot = |g: crate::graph::GraphHandle| async move {
            let files = g
                .query_rows("MATCH (f:File) RETURN f.id, f.last_accessed ORDER BY f.id".into())
                .await
                .unwrap();
            let edges = g
                .query_rows(
                    "MATCH (f:File)-[r:FILE_PART_OF]->(p:Project) \
                     RETURN f.id, r.op_id, r.valid_at, r.invalid_at ORDER BY r.op_id"
                        .into(),
                )
                .await
                .unwrap();
            (format!("{:?}", files.rows), format!("{:?}", edges.rows))
        };

        let first = crate::graph::spawn(dir.path().join("g1").to_str().unwrap()).unwrap();
        assert_eq!(project_pending(&pool, &first).await.unwrap(), 7);
        assert_eq!(project_pending(&pool, &first).await.unwrap(), 0, "nothing left to apply");
        let before = snapshot(first).await;
        assert!(before.0.contains("Int64(100)"), "the first-seen stamp survived the re-upsert: {}", before.0);
        assert!(before.1.contains("Int64(200)"), "the closed membership carries its close: {}", before.1);

        // Drop it: a fresh store, the projection mark reset, the same journal.
        set_projected_seq(&pool, 0).await.unwrap();
        let rebuilt = crate::graph::spawn(dir.path().join("g2").to_str().unwrap()).unwrap();
        project_pending(&pool, &rebuilt).await.unwrap();
        assert_eq!(snapshot(rebuilt).await, before);
    }

    /// What journaling costs on the promotion path, measured rather than guessed:
    /// one transaction per file open with the five facts a session-bearing open
    /// makes. Ignored because it times things; run with `--ignored --nocapture`.
    #[tokio::test]
    #[ignore = "a measurement, not a check: it prints timings"]
    async fn measure_the_cost_of_journaling_a_file_open() {
        let (pool, dir) = pool().await;
        let graph = crate::graph::spawn(dir.path().join("g").to_str().unwrap()).unwrap();
        let opens = 200;
        let mut record_ns = 0u128;
        let mut project_ns = 0u128;
        for i in 0..opens {
            let path = format!("/r/f{i}.rs");
            let facts: Vec<(Fact, Origin)> = vec![
                Fact::Node { label: "App".into(), id: "app".into(), props: props(&[("name", json!("x"))]), on_create: BTreeMap::new() },
                Fact::Node { label: "File".into(), id: path.clone(), props: props(&[("path", json!(path)), ("last_accessed", json!(i))]), on_create: BTreeMap::new() },
                Fact::Edge { rel: "ACCESSED_BY".into(), from: ("File".into(), path.clone()), to: ("App".into(), "app".into()), op_id: None, stamps: Stamps::default(), props: BTreeMap::new() },
                Fact::Node { label: "Session".into(), id: "s".into(), props: BTreeMap::new(), on_create: BTreeMap::new() },
                Fact::Edge { rel: "ACCESSED_IN".into(), from: ("File".into(), path.clone()), to: ("Session".into(), "s".into()), op_id: None, stamps: Stamps::default(), props: BTreeMap::new() },
            ]
            .into_iter()
            .map(|f| (f, origin()))
            .collect();
            let t = std::time::Instant::now();
            let mut tx = pool.begin().await.unwrap();
            record(&mut tx, &facts).await.unwrap();
            tx.commit().await.unwrap();
            record_ns += t.elapsed().as_nanos();
            let t = std::time::Instant::now();
            project_pending(&pool, &graph).await.unwrap();
            project_ns += t.elapsed().as_nanos();
        }
        let bytes = std::fs::metadata(dir.path().join("j.db")).unwrap().len();
        println!(
            "journal: {:.2} ms/open recording, {:.2} ms/open projecting, {} facts, {} bytes on disk ({} per fact)",
            record_ns as f64 / opens as f64 / 1e6,
            project_ns as f64 / opens as f64 / 1e6,
            opens * 5,
            bytes,
            bytes / (opens as u64 * 5),
        );
    }

    /// A launch journaled twice - the retry path does that - still counts once,
    /// and the pair keeps one edge however often it recurs.
    #[tokio::test]
    async fn an_occurrence_counts_distinct_ids_on_one_edge() {
        let (pool, dir) = pool().await;
        let occ = |id: &str, at: i64| Fact::Occurrence {
            rel: "LAUNCHED".into(),
            id: id.into(),
            from: ("App".into(), "shell".into()),
            to: ("App".into(), "files".into()),
            at,
        };
        let facts = vec![
            Fact::node("App", "shell", &[]),
            Fact::node("App", "files", &[]),
            occ("e1", 10),
            occ("e2", 20),
            occ("e2", 20),
            occ("e3", 30),
        ];
        let mut tx = pool.begin().await.unwrap();
        record(&mut tx, &facts.into_iter().map(|f| (f, origin())).collect::<Vec<_>>()).await.unwrap();
        tx.commit().await.unwrap();
        let graph = crate::graph::spawn(dir.path().join("g").to_str().unwrap()).unwrap();
        project_pending(&pool, &graph).await.unwrap();
        let rs = graph
            .query_rows("MATCH ()-[l:LAUNCHED]->() RETURN l.count, l.first_seen, l.last_seen".into())
            .await
            .unwrap();
        assert_eq!(rs.rows.len(), 1, "one edge for the pair");
        assert_eq!(rs.rows[0][0].as_i64(), 3, "e2 journaled twice counts once");
        assert_eq!(rs.rows[0][1].as_i64(), 10);
        assert_eq!(rs.rows[0][2].as_i64(), 30);
    }

    /// A valid-time source outside the four is refused, so a new import cannot
    /// slip in a name of its own.
    #[tokio::test]
    async fn an_unknown_valid_time_source_is_refused() {
        let (pool, _dir) = pool().await;
        let bad = Origin { valid_time_source: "event".into(), ..origin() };
        let mut tx = pool.begin().await.unwrap();
        let err = record(&mut tx, &[(Fact::node("App", "a", &[]), bad)]).await.unwrap_err();
        assert!(err.to_string().contains("not one of"), "{err}");
        for ok in VALID_TIME_SOURCES.iter().copied().chain([""]) {
            let o = Origin { valid_time_source: ok.into(), ..origin() };
            record(&mut tx, &[(Fact::node("App", "a", &[]), o)]).await.unwrap();
        }
    }

    #[test]
    fn the_projection_refuses_what_is_not_a_plain_name() {
        let bad = Fact::Node {
            label: "File) DETACH DELETE (x".into(),
            id: "a".into(),
            props: BTreeMap::new(),
            on_create: BTreeMap::new(),
        };
        assert!(project(&bad).is_err());
        let bad_key = Fact::Node {
            label: "File".into(),
            id: "a".into(),
            props: props(&[("p = 1, n.q", json!(1))]),
            on_create: BTreeMap::new(),
        };
        assert!(project(&bad_key).is_err());
        let float = Fact::Node {
            label: "File".into(),
            id: "a".into(),
            props: props(&[("x", json!(1.5))]),
            on_create: BTreeMap::new(),
        };
        assert!(project(&float).is_err());
    }

    #[test]
    fn a_quote_in_an_id_stays_inside_its_literal() {
        let f = Fact::Node {
            label: "File".into(),
            id: "/it's.rs".into(),
            props: BTreeMap::new(),
            on_create: BTreeMap::new(),
        };
        assert_eq!(project(&f).unwrap(), "MERGE (n:File {id: '/it\\'s.rs'})");
    }
}

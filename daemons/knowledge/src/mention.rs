//! Mentions and what they refer to (bitemporal-knowledge-graph.md §4.12).
//!
//! "Anna", "Anna M." and `a.m@uni.example` in three documents are three mentions,
//! not three people and not one. A [`Mention`] is a span an extractor read, keyed
//! by source, span and extractor version; it points at an anchored entity through a
//! scored `REFERS_TO` edge, and that edge is the claim, so it can be closed again.
//! Nothing here extracts text yet. This is where an extractor will write, with the
//! rule for when it may link on its own already in place.
//!
//! Mentions are not journaled: a re-extraction recovers every one of them. What a
//! person decided about one (`not_an_entity`) is, in `crate::curation`, and its
//! replay closes the mention's references whenever the mention comes back.

use anyhow::{bail, Result};

use crate::graph::GraphHandle;
use crate::utils::escape_cypher;

/// A span of text an extractor read as naming something.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mention {
    /// The node the text came from (a file, a mail, a note).
    pub source_id: String,
    /// Character offsets of the span in the source.
    pub span: (i64, i64),
    /// The text of the span.
    pub text: String,
    /// What the extractor took it for (`person`, `organization`...).
    pub label: String,
    /// The extractor's confidence, in basis points (0 to 10000).
    pub score: i64,
    /// Which extractor, and which version of it.
    pub extractor: String,
    pub extractor_version: String,
}

impl Mention {
    /// The mention's id: the same source, span and extractor version always give
    /// the same id, so a re-extraction finds the mention it made before, and a new
    /// extractor version makes new ones beside the old.
    pub fn id(&self) -> String {
        let key = format!(
            "{}\u{0}{}\u{0}{}\u{0}{}\u{0}{}",
            self.source_id, self.span.0, self.span.1, self.extractor, self.extractor_version
        );
        format!("mention:{}", uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, key.as_bytes()))
    }
}

/// Write a mention (an upsert on its id). Returns the id.
pub async fn upsert(graph: &GraphHandle, m: &Mention) -> Result<String> {
    if !(0..=10_000).contains(&m.score) {
        bail!("a mention score is basis points, 0 to 10000, not {}", m.score);
    }
    let id = m.id();
    graph
        .write(format!(
            "MERGE (n:Mention {{id: '{}'}}) SET n.source_id = '{}', n.span_start = {}, n.span_end = {}, \
             n.text = '{}', n.label = '{}', n.score = {}, n.extractor = '{}', n.extractor_version = '{}'",
            escape_cypher(&id),
            escape_cypher(&m.source_id),
            m.span.0,
            m.span.1,
            escape_cypher(&m.text),
            escape_cypher(&m.label),
            m.score,
            escape_cypher(&m.extractor),
            escape_cypher(&m.extractor_version),
        ))
        .await?;
    Ok(id)
}

/// The `REFERS_TO` table from mentions to one entity type.
pub fn refers_to_table(entity_type: &str) -> String {
    crate::write::entity_rel_table_name("REFERS_TO", "Mention", entity_type)
}

/// How a mention came to point at an entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Evidence {
    /// A key that names one entity and nothing else: an email address, a phone
    /// number, a contact card id, a git author email.
    StrongKey,
    /// A name that looks like the entity's.
    Name,
}

/// Whether a mention may be linked without a person looking (§4.12). A strong key
/// may. A name only when it carries enough to tell people apart: at least six
/// characters, at least two tokens, and not one character repeated. The gate is
/// Graphiti's; "Anna" or "A. M." never links on its own, however good the score.
pub fn may_link_automatically(text: &str, evidence: Evidence) -> bool {
    if evidence == Evidence::StrongKey {
        return true;
    }
    let t = text.trim();
    let tokens = t.split_whitespace().filter(|w| w.chars().any(char::is_alphanumeric)).count();
    let letters: Vec<char> = t.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect();
    let distinct = letters.iter().collect::<std::collections::BTreeSet<_>>().len();
    t.chars().count() >= 6 && tokens >= 2 && distinct >= 4
}

/// Point a mention at an entity. Refused when [`may_link_automatically`] says the
/// evidence is too thin; a person's `must_link` goes through `crate::curation`
/// instead. Idempotent: a live reference from the mention to the entity is left as
/// it is.
pub async fn refer(
    graph: &GraphHandle,
    mention: &Mention,
    entity_type: &str,
    entity_id: &str,
    evidence: Evidence,
) -> Result<bool> {
    if !may_link_automatically(&mention.text, evidence) {
        return Ok(false);
    }
    let node = crate::write::entity_table_name(entity_type);
    let rel = refers_to_table(entity_type);
    graph
        .write(format!(
            "CREATE REL TABLE IF NOT EXISTS {rel}(FROM Mention TO {node}, score INT64, \
             valid_at INT64, invalid_at INT64, created_at INT64, expired_at INT64)"
        ))
        .await?;
    let now = crate::time::now().0;
    graph
        .write(format!(
            "MATCH (m:Mention {{id: '{}'}}), (e:{node} {{id: '{}'}}) \
             OPTIONAL MATCH (m)-[r:{rel}]->(e) WHERE r.invalid_at IS NULL AND r.expired_at IS NULL \
             WITH m, e, r WHERE r IS NULL \
             CREATE (m)-[:{rel} {{score: {}, valid_at: {now}, created_at: {now}}}]->(e)",
            escape_cypher(&mention.id()),
            escape_cypher(entity_id),
            mention.score,
        ))
        .await?;
    Ok(true)
}

/// Close every live reference from a mention, whatever entity type it points at:
/// what `not_an_entity` does on replay.
pub async fn close_references(graph: &GraphHandle, mention_id: &str) -> Result<()> {
    let now = crate::time::now().0;
    for table in graph.rel_tables_involving("Mention").await? {
        if table.source != "Mention" {
            continue;
        }
        graph
            .write(format!(
                "MATCH (m:Mention {{id: '{}'}})-[r:{}]->() \
                 WHERE r.invalid_at IS NULL AND r.expired_at IS NULL \
                 SET r.invalid_at = {now}, r.expired_at = {now}",
                escape_cypher(mention_id),
                table.name,
            ))
            .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn anna(text: &str, start: i64) -> Mention {
        Mention {
            source_id: "/notes/meeting.md".into(),
            span: (start, start + text.len() as i64),
            text: text.into(),
            label: "person".into(),
            score: 9100,
            extractor: "gliner".into(),
            extractor_version: "1".into(),
        }
    }

    #[test]
    fn the_id_is_the_source_the_span_and_the_extractor_version() {
        let a = anna("Anna Maier", 10);
        assert_eq!(a.id(), anna("Anna Maier", 10).id(), "the same extraction finds the same mention");
        assert_ne!(a.id(), anna("Anna Maier", 40).id(), "another span is another mention");
        let newer = Mention { extractor_version: "2".into(), ..anna("Anna Maier", 10) };
        assert_ne!(a.id(), newer.id(), "a new extractor version makes new mentions");
    }

    #[test]
    fn a_short_or_thin_name_never_links_on_its_own() {
        assert!(!may_link_automatically("Anna", Evidence::Name), "one token");
        assert!(!may_link_automatically("A. M.", Evidence::Name), "under six characters");
        assert!(!may_link_automatically("aa aaaa", Evidence::Name), "too little to tell apart");
        assert!(may_link_automatically("Anna Maier", Evidence::Name));
        assert!(may_link_automatically("a@b.c", Evidence::StrongKey), "a strong key always may");
    }

    async fn setup() -> (GraphHandle, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let graph = crate::graph::spawn(dir.path().join("g").to_str().unwrap()).unwrap();
        let node = crate::write::entity_table_name("shared.Person");
        graph
            .write(format!("CREATE NODE TABLE IF NOT EXISTS {node}(id STRING, PRIMARY KEY(id))"))
            .await
            .unwrap();
        graph.write(format!("CREATE (:{node} {{id: 'anna'}})")).await.unwrap();
        (graph, dir)
    }

    async fn live_refs(graph: &GraphHandle) -> i64 {
        let rel = refers_to_table("shared.Person");
        graph
            .query_rows(format!(
                "MATCH ()-[r:{rel}]->() WHERE r.invalid_at IS NULL AND r.expired_at IS NULL RETURN count(r)"
            ))
            .await
            .unwrap()
            .rows[0][0]
            .as_i64()
    }

    /// A good name links once however often it is asserted; a thin one does not
    /// link at all; closing leaves the edge as history.
    #[tokio::test]
    async fn refer_links_once_and_close_keeps_history() {
        let (graph, _d) = setup().await;
        let full = anna("Anna Maier", 10);
        let short = anna("Anna", 60);
        upsert(&graph, &full).await.unwrap();
        upsert(&graph, &short).await.unwrap();
        assert!(refer(&graph, &full, "shared.Person", "anna", Evidence::Name).await.unwrap());
        assert!(refer(&graph, &full, "shared.Person", "anna", Evidence::Name).await.unwrap());
        assert!(!refer(&graph, &short, "shared.Person", "anna", Evidence::Name).await.unwrap());
        assert_eq!(live_refs(&graph).await, 1);

        close_references(&graph, &full.id()).await.unwrap();
        assert_eq!(live_refs(&graph).await, 0);
        let rel = refers_to_table("shared.Person");
        let all = graph.query_rows(format!("MATCH ()-[r:{rel}]->() RETURN count(r)")).await.unwrap();
        assert_eq!(all.rows[0][0].as_i64(), 1, "closed, not deleted");
    }

    #[tokio::test]
    async fn a_score_outside_basis_points_is_refused() {
        let (graph, _d) = setup().await;
        let bad = Mention { score: 91, ..anna("Anna Maier", 10) };
        assert!(upsert(&graph, &bad).await.is_ok());
        let worse = Mention { score: 12_000, ..anna("Anna Maier", 10) };
        assert!(upsert(&graph, &worse).await.is_err());
    }
}

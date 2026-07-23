/// Merge suggestions for duplicate shared entities.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::duplicate::{check_duplicate, DuplicateCandidate, DuplicateConfig};

/// A suggestion to merge two entities that appear to be duplicates.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergeSuggestion {
    pub id: String,
    pub entity_type: String,
    pub source_id: String,
    pub target_id: String,
    pub match_score: f64,
    pub match_fields: Vec<String>,
    pub status: SuggestionStatus,
    pub created_at: DateTime<Utc>,
    pub created_by: String,
}

/// Current status of a merge suggestion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SuggestionStatus {
    Pending,
    Accepted,
    Rejected,
    Expired,
}

impl MergeSuggestion {
    /// Create a new pending suggestion from a duplicate candidate.
    pub fn new(
        entity_type: &str,
        source_id: &str,
        candidate: &DuplicateCandidate,
        created_by: &str,
    ) -> Self {
        Self {
            id: uuid::Uuid::now_v7().to_string(),
            entity_type: entity_type.into(),
            source_id: source_id.into(),
            target_id: candidate.existing_id.clone(),
            match_score: candidate.match_score,
            match_fields: candidate.match_fields.clone(),
            status: SuggestionStatus::Pending,
            created_at: Utc::now(),
            created_by: created_by.into(),
        }
    }
}

/// Action to take after accepting or rejecting a merge.
#[derive(Debug)]
pub enum MergeAction {
    /// Delete source, keep target, re-point relations.
    Merge {
        delete_id: String,
        keep_id: String,
        update_relations: bool,
    },
    /// Keep both entities as separate (mark not-duplicate).
    KeepBoth {
        mark_not_duplicate: bool,
    },
}

/// The graph-stored string for a suggestion status (matches the serde lowercase
/// rename, so a persisted status round-trips with the pending-list query's filter).
fn status_str(status: SuggestionStatus) -> &'static str {
    match status {
        SuggestionStatus::Pending => "pending",
        SuggestionStatus::Accepted => "accepted",
        SuggestionStatus::Rejected => "rejected",
        SuggestionStatus::Expired => "expired",
    }
}

/// Persist a merge suggestion as a `MergeSuggestion` graph node, idempotent on the
/// suggestion id (MERGE). `match_fields` is stored as a JSON array string and
/// `created_at` as RFC3339 (lexically sortable for the pending query's ORDER BY),
/// matching what [`pending_suggestions_query`] reads back. The producer calls this
/// after [`detect_duplicate`]; the accept/reject op updates `status`.
pub async fn persist_suggestion(
    graph: &crate::graph::GraphHandle,
    s: &MergeSuggestion,
) -> anyhow::Result<()> {
    use crate::utils::escape_cypher;
    let id = escape_cypher(&s.id);
    let entity_type = escape_cypher(&s.entity_type);
    let source_id = escape_cypher(&s.source_id);
    let target_id = escape_cypher(&s.target_id);
    let match_fields = escape_cypher(&serde_json::to_string(&s.match_fields).unwrap_or_default());
    let status = escape_cypher(status_str(s.status));
    let created_at = escape_cypher(&s.created_at.to_rfc3339());
    let created_by = escape_cypher(&s.created_by);
    graph
        .write(format!(
            "MERGE (s:MergeSuggestion {{id: '{id}'}}) \
             SET s.entity_type = '{entity_type}', s.source_id = '{source_id}', \
             s.target_id = '{target_id}', s.match_score = {}, \
             s.match_fields = '{match_fields}', s.status = '{status}', \
             s.created_at = '{created_at}', s.created_by = '{created_by}'",
            s.match_score
        ))
        .await?;
    Ok(())
}

/// Detect whether a newly-written shared entity duplicates an existing one and, if
/// so, build the pending [`MergeSuggestion`] for it. Applies the entity type's
/// [`DuplicateConfig`] via [`check_duplicate`] against each supplied existing entity
/// of the same type (never against the new one itself), and returns a suggestion for
/// the highest-scoring match above the type's `min_score`, or `None` if nothing is
/// close enough. Pure over the supplied `existing` set - the caller fetches the
/// same-type entities from the graph and persists the returned suggestion - so the
/// detection logic is unit-tested without the graph. This is the core the write-path
/// producer calls; a `min_score` of `1.0` for a type with no unique fields never
/// matches, so those types produce no suggestions.
pub fn detect_duplicate(
    entity_type: &str,
    new_id: &str,
    new_data: &serde_json::Map<String, serde_json::Value>,
    existing: &[(String, serde_json::Map<String, serde_json::Value>)],
    created_by: &str,
) -> Option<MergeSuggestion> {
    let config = DuplicateConfig::for_type(entity_type);
    let best = existing
        .iter()
        .filter(|(id, _)| id != new_id)
        .filter_map(|(id, data)| check_duplicate(&config, new_data, id, data))
        .max_by(|a, b| {
            a.match_score
                .partial_cmp(&b.match_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        })?;
    Some(MergeSuggestion::new(entity_type, new_id, &best, created_by))
}

/// Build the action for accepting a merge suggestion.
pub fn accept_merge(suggestion: &MergeSuggestion) -> MergeAction {
    MergeAction::Merge {
        delete_id: suggestion.source_id.clone(),
        keep_id: suggestion.target_id.clone(),
        update_relations: true,
    }
}

/// Build the action for rejecting a merge suggestion.
pub fn reject_merge(_suggestion: &MergeSuggestion) -> MergeAction {
    MergeAction::KeepBoth {
        mark_not_duplicate: true,
    }
}

/// Cypher to list pending suggestions.
pub fn pending_suggestions_query(entity_type: Option<&str>, limit: usize) -> String {
    // Explicit RETURN fields (not `RETURN s`): the daemon's typed JSON read path has
    // no whole-node cell, so each field is projected under its own alias.
    const FIELDS: &str = "s.id AS id, s.entity_type AS entity_type, s.source_id AS source_id, \
         s.target_id AS target_id, s.match_score AS match_score, \
         s.match_fields AS match_fields, s.status AS status, \
         s.created_at AS created_at, s.created_by AS created_by";
    match entity_type {
        Some(t) => format!(
            "MATCH (s:MergeSuggestion) WHERE s.status = 'pending' AND s.entity_type = '{}' \
             RETURN {FIELDS} ORDER BY created_at DESC LIMIT {}",
            crate::utils::escape_cypher(t),
            limit,
        ),
        None => format!(
            "MATCH (s:MergeSuggestion) WHERE s.status = 'pending' \
             RETURN {FIELDS} ORDER BY created_at DESC LIMIT {}",
            limit,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::duplicate::DuplicateCandidate;

    fn candidate() -> DuplicateCandidate {
        DuplicateCandidate {
            existing_id: "existing-1".into(),
            match_score: 0.95,
            match_fields: vec!["email".into()],
        }
    }

    #[test]
    fn test_create_suggestion() {
        let s = MergeSuggestion::new("shared.Person", "new-1", &candidate(), "com.test");
        assert_eq!(s.entity_type, "shared.Person");
        assert_eq!(s.source_id, "new-1");
        assert_eq!(s.target_id, "existing-1");
        assert_eq!(s.status, SuggestionStatus::Pending);
        assert!(!s.id.is_empty());
    }

    #[test]
    fn test_accept_merge() {
        let s = MergeSuggestion::new("shared.Person", "new-1", &candidate(), "com.test");
        match accept_merge(&s) {
            MergeAction::Merge { delete_id, keep_id, update_relations } => {
                assert_eq!(delete_id, "new-1");
                assert_eq!(keep_id, "existing-1");
                assert!(update_relations);
            }
            _ => panic!("expected Merge"),
        }
    }

    #[test]
    fn test_reject_merge() {
        let s = MergeSuggestion::new("shared.Person", "new-1", &candidate(), "com.test");
        match reject_merge(&s) {
            MergeAction::KeepBoth { mark_not_duplicate } => {
                assert!(mark_not_duplicate);
            }
            _ => panic!("expected KeepBoth"),
        }
    }

    fn person(email: &str) -> serde_json::Map<String, serde_json::Value> {
        let mut m = serde_json::Map::new();
        m.insert("email".into(), serde_json::Value::String(email.into()));
        m
    }

    #[test]
    fn detect_duplicate_flags_the_matching_existing_person() {
        // A new Person with the same email as an existing one -> a pending merge
        // suggestion targeting that existing id.
        let existing = vec![
            ("p-other".to_string(), person("someone@else.org")),
            ("p-dup".to_string(), person("tim@x.org")),
        ];
        let s = detect_duplicate("shared.Person", "p-new", &person("tim@x.org"), &existing, "com.test")
            .expect("a duplicate is detected");
        assert_eq!(s.source_id, "p-new");
        assert_eq!(s.target_id, "p-dup", "the matching existing id, not the other");
        assert_eq!(s.status, SuggestionStatus::Pending);
    }

    #[test]
    fn detect_duplicate_returns_none_without_a_match_and_ignores_self() {
        // No existing person shares the email -> no suggestion.
        let existing = vec![("p-other".to_string(), person("someone@else.org"))];
        assert!(detect_duplicate("shared.Person", "p-new", &person("tim@x.org"), &existing, "c").is_none());
        // An entity is never a duplicate of ITSELF even with identical data.
        let same = vec![("p-new".to_string(), person("tim@x.org"))];
        assert!(detect_duplicate("shared.Person", "p-new", &person("tim@x.org"), &same, "c").is_none());
    }

    #[tokio::test]
    async fn persist_suggestion_writes_a_pending_merge_node() {
        // Real graph: a detected suggestion persists as a MergeSuggestion node the
        // pending-list query reads back, idempotent on re-persist.
        let tmp = tempfile::TempDir::new().unwrap();
        let graph = crate::graph::spawn(tmp.path().join("g").to_str().unwrap()).unwrap();
        let s = detect_duplicate(
            "shared.Person",
            "p-new",
            &person("tim@x.org"),
            &[("p-dup".to_string(), person("tim@x.org"))],
            "com.test",
        )
        .expect("a duplicate");
        persist_suggestion(&graph, &s).await.unwrap();
        persist_suggestion(&graph, &s).await.unwrap(); // idempotent

        let rows = graph
            .query_rows(
                "MATCH (s:MergeSuggestion) WHERE s.status = 'pending' \
                 RETURN s.source_id AS src, s.target_id AS tgt, s.entity_type AS ty"
                    .into(),
            )
            .await
            .unwrap();
        assert_eq!(rows.rows.len(), 1, "one pending suggestion node, not duplicated");
        assert_eq!(rows.rows[0][0].as_str(), "p-new");
        assert_eq!(rows.rows[0][1].as_str(), "p-dup");
        assert_eq!(rows.rows[0][2].as_str(), "shared.Person");
    }

    #[tokio::test]
    async fn the_list_query_returns_the_persisted_suggestion_as_json() {
        // The 0x0F list op runs `pending_suggestions_query` through the typed JSON
        // path, so its explicit-field RETURN must yield a usable object per row.
        let tmp = tempfile::TempDir::new().unwrap();
        let graph = crate::graph::spawn(tmp.path().join("g").to_str().unwrap()).unwrap();
        let s = detect_duplicate(
            "shared.Person",
            "p-new",
            &person("tim@x.org"),
            &[("p-dup".to_string(), person("tim@x.org"))],
            "com.test",
        )
        .expect("a duplicate");
        persist_suggestion(&graph, &s).await.unwrap();

        let json = graph
            .query_rows_json(pending_suggestions_query(Some("shared.Person"), 10))
            .await
            .unwrap();
        // The typed path returns `{columns, rows}` with positional rows; the RETURN
        // order is id, entity_type, source_id, target_id, ..., status, ...
        let v: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
        let rows = v["rows"].as_array().expect("rows array");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0][2], "p-new", "source_id");
        assert_eq!(rows[0][3], "p-dup", "target_id");
        assert_eq!(rows[0][6], "pending", "status");
        // A different type filter returns nothing.
        let none = graph
            .query_rows_json(pending_suggestions_query(Some("shared.Organization"), 10))
            .await
            .unwrap();
        let nv: serde_json::Value = serde_json::from_str(&none).unwrap();
        assert_eq!(nv["rows"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn test_pending_query_with_type() {
        let q = pending_suggestions_query(Some("shared.Person"), 10);
        assert!(q.contains("shared.Person"));
        assert!(q.contains("LIMIT 10"));
    }

    #[test]
    fn test_pending_query_all() {
        let q = pending_suggestions_query(None, 50);
        // No entity_type FILTER when unfiltered (the field is still projected in
        // RETURN, so check the WHERE clause specifically).
        assert!(!q.contains("AND s.entity_type"));
        assert!(q.contains("LIMIT 50"));
    }
}

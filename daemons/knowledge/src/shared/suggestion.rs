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

/// Parse a graph-stored status string back to the enum (inverse of [`status_str`]);
/// an unrecognised value is treated as `Expired` (fail-closed to a non-actionable
/// state, so a corrupt status can never be acted on as pending).
fn status_from_str(s: &str) -> SuggestionStatus {
    match s {
        "pending" => SuggestionStatus::Pending,
        "accepted" => SuggestionStatus::Accepted,
        "rejected" => SuggestionStatus::Rejected,
        _ => SuggestionStatus::Expired,
    }
}

/// The stored fields the accept/reject path needs: the duplicate PAIR + the type
/// (for owner-gating + table resolution) + the current status (only a `Pending`
/// suggestion may be acted on). The merge targets come from HERE, never from the
/// caller's request, so a caller can only name a suggestion id, not an arbitrary
/// pair to merge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuggestionCore {
    /// The qualified shared type (e.g. `shared.Person`).
    pub entity_type: String,
    /// The duplicate to fold away (deleted on merge).
    pub source_id: String,
    /// The canonical entity kept on merge.
    pub target_id: String,
    /// Current lifecycle status.
    pub status: SuggestionStatus,
}

/// Fetch a stored [`MergeSuggestion`]'s core fields by id, or `None` if absent.
/// Read-only. The pair + type it returns are the daemon-authored values persisted
/// when the suggestion was detected, never caller input.
pub async fn fetch_suggestion(
    graph: &crate::graph::GraphHandle,
    suggestion_id: &str,
) -> anyhow::Result<Option<SuggestionCore>> {
    use crate::utils::escape_cypher;
    let id = escape_cypher(suggestion_id);
    let cypher = format!(
        "MATCH (s:MergeSuggestion {{id: '{id}'}}) \
         RETURN s.entity_type AS entity_type, s.source_id AS source_id, \
         s.target_id AS target_id, s.status AS status LIMIT 1"
    );
    let json = graph.query_rows_json(cypher).await?;
    let v: serde_json::Value = serde_json::from_str(&json)?;
    let row = match v.get("rows").and_then(|r| r.as_array()).and_then(|r| r.first()) {
        Some(row) => row,
        None => return Ok(None),
    };
    let cell = |i: usize| row.get(i).and_then(|c| c.as_str()).unwrap_or_default().to_string();
    let entity_type = cell(0);
    let source_id = cell(1);
    let target_id = cell(2);
    // A stored suggestion missing its pair or type is corrupt; treat as absent.
    if entity_type.is_empty() || source_id.is_empty() || target_id.is_empty() {
        return Ok(None);
    }
    Ok(Some(SuggestionCore {
        entity_type,
        source_id,
        target_id,
        status: status_from_str(&cell(3)),
    }))
}

/// Re-validate that a suggestion's stored pair is STILL a live duplicate, just
/// before the destructive merge. Entity ids are natural-key-derived and REUSABLE
/// after deletion, and multiple pending suggestions for one source can accumulate,
/// so a stale suggestion could otherwise fold an unrelated (reused-id) or edited
/// entity. This re-fetches both entities' CURRENT scorer fields and re-runs
/// [`detect_duplicate`]; it returns `true` only if both are still present AND still
/// score as duplicates. Read-only. `false` => refuse the merge (stale / one gone /
/// no longer matching).
pub async fn suggestion_still_valid(
    graph: &crate::graph::GraphHandle,
    entity_type: &str,
    source_id: &str,
    target_id: &str,
) -> anyhow::Result<bool> {
    use crate::utils::escape_cypher;
    let config = DuplicateConfig::for_type(entity_type);
    // The fields the scorer compares (unique + fuzzy). No scorer fields => the type
    // has no duplicate model, so there is nothing to re-validate => refuse.
    let mut fields: Vec<String> = config.unique_fields.clone();
    for (f, _) in &config.fuzzy_fields {
        if !fields.contains(f) {
            fields.push(f.clone());
        }
    }
    if fields.is_empty() {
        return Ok(false);
    }
    let projection: Vec<String> = fields.iter().map(|f| format!("n.{f} AS {f}")).collect();
    let table = crate::write::entity_table_name(entity_type);
    let cypher = format!(
        "MATCH (n:{table}) WHERE n.id IN ['{}', '{}'] RETURN n.id AS id, {} LIMIT 2",
        escape_cypher(source_id),
        escape_cypher(target_id),
        projection.join(", "),
    );
    let json = graph.query_rows_json(cypher).await?;
    let found = parse_candidate_rows(&json);
    let src = found.iter().find(|(id, _)| id == source_id);
    let tgt = found.iter().find(|(id, _)| id == target_id);
    let (Some((_, src_data)), Some((_, tgt_data))) = (src, tgt) else {
        // One (or both) of the pair no longer exists at that id.
        return Ok(false);
    };
    // Re-run detection with the target as the reference and the source as the sole
    // candidate; a still-matching pair yields a suggestion, a no-longer-matching one
    // (reused/edited id) yields None.
    Ok(detect_duplicate(
        entity_type,
        target_id,
        tgt_data,
        &[(source_id.to_string(), src_data.clone())],
        "revalidate",
    )
    .is_some())
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

/// Transition an existing merge suggestion to a new status (accepted/rejected/
/// expired) - what the accept/reject review ops record. A `MATCH ... SET` on the
/// suggestion id (not a MERGE: it must not resurrect a deleted suggestion), so a
/// missing id is a no-op. The accept op ALSO executes the graph merge (delete the
/// duplicate, re-point its relations); this only records the review decision.
pub async fn update_suggestion_status(
    graph: &crate::graph::GraphHandle,
    suggestion_id: &str,
    status: SuggestionStatus,
) -> anyhow::Result<()> {
    use crate::utils::escape_cypher;
    let id = escape_cypher(suggestion_id);
    let status = escape_cypher(status_str(status));
    graph
        .write(format!(
            "MATCH (s:MergeSuggestion {{id: '{id}'}}) SET s.status = '{status}'"
        ))
        .await?;
    Ok(())
}

/// Parse a `{columns, rows}` typed-JSON result into `(id, fields)` candidate
/// entities for [`detect_duplicate`]: column 0 is the id, the rest become the field
/// map keyed by column alias. Tolerant - a malformed shape yields no candidates.
fn parse_candidate_rows(
    json: &str,
) -> Vec<(String, serde_json::Map<String, serde_json::Value>)> {
    let v: serde_json::Value = match serde_json::from_str(json) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let (Some(columns), Some(rows)) = (v["columns"].as_array(), v["rows"].as_array()) else {
        return Vec::new();
    };
    let col_names: Vec<&str> = columns.iter().filter_map(|c| c.as_str()).collect();
    rows.iter()
        .filter_map(|row| {
            let cells = row.as_array()?;
            let id = cells.first()?.as_str()?.to_string();
            let mut map = serde_json::Map::new();
            for (i, name) in col_names.iter().enumerate().skip(1) {
                if let Some(val) = cells.get(i) {
                    map.insert((*name).to_string(), val.clone());
                }
            }
            Some((id, map))
        })
        .collect()
}

/// The write-path producer (SHARED-ENTITIES.md): after a shared entity is written,
/// find whether it duplicates an existing one and record a pending merge suggestion.
/// Queries the existing same-type entities by the new one's unique-field VALUES
/// (string exact-match, bounded, so it is scale-safe and correct regardless of the
/// total count - NOT a fetch-all), scores them via [`detect_duplicate`], and
/// [`persist_suggestion`]s the best match. Only ever WRITES a `MergeSuggestion` node
/// (never mutates the entities), so it is safe to run on every shared-entity write.
/// A type with no unique fields, or a new entity with no matchable unique-field
/// value, yields no suggestion (no query). The entity table must already exist (it
/// does, since the entity was just written).
pub async fn dedup_shared_entity_on_write(
    graph: &crate::graph::GraphHandle,
    entity_type: &str,
    new_id: &str,
    new_data: &serde_json::Map<String, serde_json::Value>,
    created_by: &str,
) -> anyhow::Result<Option<MergeSuggestion>> {
    use crate::utils::escape_cypher;
    let config = DuplicateConfig::for_type(entity_type);

    // Match on the unique-field VALUES the new entity carries (strings only, e.g.
    // email/domain/place_id/name). No matchable value -> no dedup query.
    let clauses: Vec<String> = config
        .unique_fields
        .iter()
        .filter_map(|f| match new_data.get(f) {
            Some(serde_json::Value::String(v)) if !v.is_empty() => {
                Some(format!("n.{f} = '{}'", escape_cypher(v)))
            }
            _ => None,
        })
        .collect();
    if clauses.is_empty() {
        return Ok(None);
    }

    // Project the id + every field the scorer compares (unique + fuzzy).
    let mut fields: Vec<String> = config.unique_fields.clone();
    for (f, _) in &config.fuzzy_fields {
        if !fields.contains(f) {
            fields.push(f.clone());
        }
    }
    let projection: Vec<String> = fields.iter().map(|f| format!("n.{f} AS {f}")).collect();
    let table = crate::write::entity_table_name(entity_type);
    let cypher = format!(
        "MATCH (n:{table}) WHERE {} RETURN n.id AS id, {} LIMIT 100",
        clauses.join(" OR "),
        projection.join(", "),
    );

    let json = graph.query_rows_json(cypher).await?;
    let existing = parse_candidate_rows(&json);
    let suggestion = detect_duplicate(entity_type, new_id, new_data, &existing, created_by);
    if let Some(s) = &suggestion {
        persist_suggestion(graph, s).await?;
    }
    Ok(suggestion)
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
    async fn fetch_suggestion_reads_back_the_pair_type_and_status() {
        // The merge-accept op (0x10) reads the pair + type + status from HERE, never
        // from the caller. A persisted suggestion fetches back its stored core; an
        // absent id is None.
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

        let core = fetch_suggestion(&graph, &s.id).await.unwrap().expect("the suggestion");
        assert_eq!(core.entity_type, "shared.Person");
        assert_eq!(core.source_id, "p-new");
        assert_eq!(core.target_id, "p-dup");
        assert_eq!(core.status, SuggestionStatus::Pending);

        assert!(
            fetch_suggestion(&graph, "no-such-id").await.unwrap().is_none(),
            "an absent suggestion id is None"
        );
    }

    #[tokio::test]
    async fn suggestion_still_valid_guards_a_stale_or_reused_pair() {
        // The merge-accept re-validation: a merge only proceeds while the stored
        // pair is STILL a live duplicate, so a stale suggestion cannot fold an
        // absent or reused-id entity.
        let tmp = tempfile::TempDir::new().unwrap();
        let graph = crate::graph::spawn(tmp.path().join("g").to_str().unwrap()).unwrap();
        let table = crate::write::entity_table_name("shared.Person");
        graph
            .write(format!(
                "CREATE NODE TABLE {table}(id STRING, email STRING, \
                 normalized_name STRING, PRIMARY KEY(id))"
            ))
            .await
            .unwrap();
        graph
            .write(format!(
                "CREATE (:{table} {{id:'shared.Person:a', email:'tim@x.org', normalized_name:'tim'}})"
            ))
            .await
            .unwrap();
        graph
            .write(format!(
                "CREATE (:{table} {{id:'shared.Person:b', email:'tim@x.org', normalized_name:'tim'}})"
            ))
            .await
            .unwrap();

        // Both present + same email -> a live duplicate, mergeable.
        assert!(
            suggestion_still_valid(&graph, "shared.Person", "shared.Person:a", "shared.Person:b")
                .await
                .unwrap(),
            "a live matching pair is valid"
        );
        // Target absent -> not valid (a stale suggestion for a gone entity).
        assert!(
            !suggestion_still_valid(&graph, "shared.Person", "shared.Person:a", "shared.Person:gone")
                .await
                .unwrap(),
            "an absent pair member refuses the merge"
        );
        // The id is reused by a DIFFERENT entity (email changed) -> no longer a
        // duplicate -> refuse (the stale-mis-merge guard).
        graph
            .write(format!(
                "MATCH (n:{table} {{id:'shared.Person:b'}}) \
                 SET n.email='other@x.org', n.normalized_name='someone-else'"
            ))
            .await
            .unwrap();
        assert!(
            !suggestion_still_valid(&graph, "shared.Person", "shared.Person:a", "shared.Person:b")
                .await
                .unwrap(),
            "a reused/edited id that no longer matches refuses the merge"
        );
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

    #[tokio::test]
    async fn update_status_transitions_a_suggestion_out_of_pending() {
        // Marking a suggestion rejected removes it from the pending list; a missing
        // id is a harmless no-op (no resurrection).
        let tmp = tempfile::TempDir::new().unwrap();
        let graph = crate::graph::spawn(tmp.path().join("g").to_str().unwrap()).unwrap();
        let s = detect_duplicate(
            "shared.Person",
            "p-new",
            &person("tim@x.org"),
            &[("p-dup".to_string(), person("tim@x.org"))],
            "c",
        )
        .unwrap();
        persist_suggestion(&graph, &s).await.unwrap();

        update_suggestion_status(&graph, &s.id, SuggestionStatus::Rejected).await.unwrap();
        let pending = graph
            .query_rows_json(pending_suggestions_query(None, 10))
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&pending).unwrap();
        assert_eq!(v["rows"].as_array().unwrap().len(), 0, "rejected -> not pending");

        // A no-op on an unknown id (no error, nothing created).
        update_suggestion_status(&graph, "does-not-exist", SuggestionStatus::Accepted)
            .await
            .unwrap();
        let all = graph
            .query_rows("MATCH (s:MergeSuggestion) RETURN s.id AS id".into())
            .await
            .unwrap();
        assert_eq!(all.rows.len(), 1, "no suggestion resurrected by a missing-id update");
    }

    #[tokio::test]
    async fn the_producer_detects_and_persists_a_duplicate_from_the_graph() {
        // The write-path producer, end to end against a real graph: existing
        // entities of the type live in the dynamic `e_` table; a new one sharing a
        // unique field is detected + persisted; a unique new one is not.
        let tmp = tempfile::TempDir::new().unwrap();
        let graph = crate::graph::spawn(tmp.path().join("g").to_str().unwrap()).unwrap();
        let table = crate::write::entity_table_name("shared.Person");
        graph
            .write(format!(
                "CREATE NODE TABLE {table} \
                 (id STRING, email STRING, normalized_name STRING, PRIMARY KEY(id))"
            ))
            .await
            .unwrap();
        graph
            .write(format!("CREATE (:{table} {{id: 'p-existing', email: 'tim@x.org'}})"))
            .await
            .unwrap();
        graph
            .write(format!("CREATE (:{table} {{id: 'p-other', email: 'else@x.org'}})"))
            .await
            .unwrap();

        // A newly-written person sharing p-existing's email -> a persisted suggestion.
        let s = dedup_shared_entity_on_write(
            &graph,
            "shared.Person",
            "p-new",
            &person("tim@x.org"),
            "com.test",
        )
        .await
        .unwrap()
        .expect("the email duplicate is detected");
        assert_eq!(s.source_id, "p-new");
        assert_eq!(s.target_id, "p-existing", "the matching existing person, not p-other");
        let pending = graph
            .query_rows_json(pending_suggestions_query(None, 10))
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&pending).unwrap();
        assert_eq!(v["rows"].as_array().unwrap().len(), 1, "the suggestion persisted");

        // A unique new email -> no candidate, no suggestion.
        let none = dedup_shared_entity_on_write(
            &graph,
            "shared.Person",
            "p-uniq",
            &person("brand@new.org"),
            "c",
        )
        .await
        .unwrap();
        assert!(none.is_none(), "a non-duplicate produces no suggestion");
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

//! The meetings note store (agent-work-surfaces): file a produced meeting note
//! into the graph as a `Meeting` node with its `ActionItem` children, and read
//! them back for the recent-meetings home and a single note view.
//!
//! A meeting note stays on-device by design (the Otter/Granola trap we avoid),
//! so this is the durable home for the summary + action items the AI produced
//! from the transcript. The store operates directly on the serial graph thread
//! (the same mechanism `lcg.rs`/`promotion.rs` use), never the `0x02` write
//! socket, and takes plain params rather than the wire `MeetingNote` so the
//! knowledge daemon keeps no dependency on the note contract (its transcript is
//! not stored as a node here; the note document and the later per-claim span
//! provenance carry it).
//!
//! Participants are stored JSON-encoded in the node's `participants` STRING so a
//! name containing a comma round-trips faithfully. Action items are structured
//! `ActionItem` nodes (not a blob) so they stay answerable and later linkable to
//! their owner. Filing MERGEs by id, so re-filing the same meeting updates in
//! place; a re-file with fewer items leaves the extra `ActionItem` nodes
//! orphaned, which is acceptable for the file-once-per-note flow.
//!
//! Lib-only until the daemon wires a socket op (the `meetings_list`/`meeting_note`
//! reads and the note-filing write); the bin tree carries no consumer yet, so the
//! module allows dead code until that op lands, matching `capsule`/`typed_read`.
#![allow(dead_code)]

use anyhow::Result;

use crate::cypher::{match_two_nodes, merge_node_set, SetValue};
use crate::graph::{CellValue, GraphHandle};
use crate::utils::escape_cypher;

/// One action item to file with a meeting.
pub struct ActionItemRecord<'a> {
    /// The task text.
    pub text: &'a str,
    /// The owner the extractor attributed, when any.
    pub owner: Option<&'a str>,
}

/// The meeting facts to file: the produced note plus the recording start. The
/// transcript is not stored here (it lives in the note document).
pub struct MeetingRecord<'a> {
    /// The note title.
    pub title: &'a str,
    /// The prose summary.
    pub summary: &'a str,
    /// Participant display names, in listing order.
    pub participants: &'a [String],
    /// The extracted action items.
    pub action_items: &'a [ActionItemRecord<'a>],
}

/// One row of the recent-meetings home (`meetings_list`).
#[derive(Debug, Clone, PartialEq)]
pub struct MeetingSummaryRow {
    /// The meeting id.
    pub id: String,
    /// The note title.
    pub title: String,
    /// The prose summary.
    pub summary: String,
    /// Participant display names.
    pub participants: Vec<String>,
    /// The recording start, microseconds since epoch.
    pub started_at: i64,
}

/// A single filed action item read back from the graph.
#[derive(Debug, Clone, PartialEq)]
pub struct FiledActionItem {
    /// The task text.
    pub text: String,
    /// The owner, when one was attributed.
    pub owner: Option<String>,
}

/// A whole meeting note read back from the graph (`meeting_note {id}`).
#[derive(Debug, Clone, PartialEq)]
pub struct MeetingDetail {
    /// The summary row fields.
    pub summary: MeetingSummaryRow,
    /// The action items, in filing order.
    pub action_items: Vec<FiledActionItem>,
}

/// File a produced meeting note as a `Meeting` node with its `ActionItem`
/// children, atomically. Idempotent on `id`.
pub async fn file_meeting(
    graph: &GraphHandle,
    id: &str,
    record: MeetingRecord<'_>,
    started_at: i64,
) -> Result<()> {
    let participants_json = serde_json::to_string(record.participants)?;

    // One transaction so a mid-file failure rolls back rather than leaving a
    // Meeting node with a partial set of action-item edges.
    let mut stmts: Vec<String> = Vec::new();

    stmts.push(merge_node_set(
        "m",
        "Meeting",
        id,
        &[
            ("title", SetValue::Text(record.title)),
            ("summary", SetValue::Text(record.summary)),
            ("participants", SetValue::Text(&participants_json)),
            ("started_at", SetValue::Int(started_at)),
        ],
    ));

    for (i, item) in record.action_items.iter().enumerate() {
        // A deterministic per-meeting id, so a re-file MERGEs the same item in
        // place rather than duplicating.
        let ai_id = format!("{id}#action-{i}");
        stmts.push(merge_node_set(
            "ai",
            "ActionItem",
            &ai_id,
            &[
                ("text", SetValue::Text(item.text)),
                ("owner", SetValue::Text(item.owner.unwrap_or(""))),
            ],
        ));
        stmts.push(format!(
            "{} MERGE (m)-[:HAS_ACTION_ITEM]->(ai)",
            match_two_nodes("m", "Meeting", id, "ai", "ActionItem", &ai_id)
        ));
    }

    graph.transaction(stmts).await
}

/// The i64 in a cell, or 0 for a non-int cell (the stored fields are typed, so
/// this only guards an unexpected shape).
fn cell_i64(cell: Option<&CellValue>) -> i64 {
    match cell {
        Some(CellValue::Int64(i)) => *i,
        _ => 0,
    }
}

/// The string in a cell, or empty.
fn cell_str(cell: Option<&CellValue>) -> String {
    cell.map(|c| c.as_str().to_string()).unwrap_or_default()
}

/// Parse the JSON-encoded participants string, tolerating a malformed value as
/// no participants (a stored field is trusted, so this only guards corruption).
fn parse_participants(s: &str) -> Vec<String> {
    serde_json::from_str(s).unwrap_or_default()
}

/// The recent meetings, newest first, for the home surface.
pub async fn list_meetings(graph: &GraphHandle) -> Result<Vec<MeetingSummaryRow>> {
    let rs = graph
        .query_rows(
            "MATCH (m:Meeting) \
             RETURN m.id, m.title, m.summary, m.participants, m.started_at \
             ORDER BY m.started_at DESC"
                .to_string(),
        )
        .await?;
    Ok(rs
        .rows
        .iter()
        .map(|row| MeetingSummaryRow {
            id: cell_str(row.first()),
            title: cell_str(row.get(1)),
            summary: cell_str(row.get(2)),
            participants: parse_participants(&cell_str(row.get(3))),
            started_at: cell_i64(row.get(4)),
        })
        .collect())
}

/// A single meeting note by id, or `None` when unknown.
pub async fn get_meeting(graph: &GraphHandle, id: &str) -> Result<Option<MeetingDetail>> {
    let id_esc = escape_cypher(id);
    let rs = graph
        .query_rows(format!(
            "MATCH (m:Meeting {{id: '{id_esc}'}}) \
             RETURN m.id, m.title, m.summary, m.participants, m.started_at"
        ))
        .await?;
    let Some(row) = rs.rows.first() else {
        return Ok(None);
    };
    let summary = MeetingSummaryRow {
        id: cell_str(row.first()),
        title: cell_str(row.get(1)),
        summary: cell_str(row.get(2)),
        participants: parse_participants(&cell_str(row.get(3))),
        started_at: cell_i64(row.get(4)),
    };

    // Action items in filing order (the deterministic `#action-<i>` id sorts
    // them, so a lexical sort keeps them in order for < 10 items; a numeric
    // suffix past 9 would need padding, noted for a future large-note pass).
    let items = graph
        .query_rows(format!(
            "MATCH (m:Meeting {{id: '{id_esc}'}})-[:HAS_ACTION_ITEM]->(ai:ActionItem) \
             RETURN ai.text, ai.owner ORDER BY ai.id"
        ))
        .await?;
    let action_items = items
        .rows
        .iter()
        .map(|row| {
            let owner = cell_str(row.get(1));
            FiledActionItem {
                text: cell_str(row.first()),
                owner: (!owner.is_empty()).then_some(owner),
            }
        })
        .collect();

    Ok(Some(MeetingDetail {
        summary,
        action_items,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graph() -> GraphHandle {
        let tmp = tempfile::TempDir::new().unwrap();
        // Keep the tempdir alive for the process by leaking it: the test graph
        // thread holds the path open, and the test is short-lived.
        let path = tmp.path().join("graph");
        std::mem::forget(tmp);
        crate::graph::spawn(path.to_str().unwrap()).unwrap()
    }

    fn record<'a>(items: &'a [ActionItemRecord<'a>]) -> MeetingRecord<'a> {
        MeetingRecord {
            title: "Editor direction",
            summary: "we shipped the parser",
            participants: PARTICIPANTS.as_slice(),
            action_items: items,
        }
    }

    static PARTICIPANTS: std::sync::LazyLock<Vec<String>> =
        std::sync::LazyLock::new(|| vec!["Tim".to_string(), "Ada, the reviewer".to_string()]);

    #[tokio::test]
    async fn file_then_read_back_the_meeting_and_its_action_items() {
        let g = graph();
        let items = [
            ActionItemRecord { text: "land the store", owner: Some("Tim") },
            ActionItemRecord { text: "review the schema", owner: None },
        ];
        file_meeting(&g, "m-1", record(&items), 1_000).await.unwrap();

        let detail = get_meeting(&g, "m-1").await.unwrap().expect("filed");
        assert_eq!(detail.summary.title, "Editor direction");
        assert_eq!(detail.summary.summary, "we shipped the parser");
        // The comma-bearing participant round-trips via JSON.
        assert_eq!(detail.summary.participants, vec!["Tim", "Ada, the reviewer"]);
        assert_eq!(detail.summary.started_at, 1_000);
        assert_eq!(detail.action_items.len(), 2);
        assert_eq!(detail.action_items[0].text, "land the store");
        assert_eq!(detail.action_items[0].owner.as_deref(), Some("Tim"));
        assert_eq!(detail.action_items[1].owner, None);
    }

    #[tokio::test]
    async fn list_returns_meetings_newest_first() {
        let g = graph();
        file_meeting(&g, "old", record(&[]), 100).await.unwrap();
        file_meeting(&g, "new", record(&[]), 900).await.unwrap();

        let rows = list_meetings(&g).await.unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].id, "new");
        assert_eq!(rows[1].id, "old");
    }

    #[tokio::test]
    async fn re_filing_updates_in_place_not_duplicating() {
        let g = graph();
        file_meeting(&g, "m", record(&[]), 100).await.unwrap();
        let updated = MeetingRecord {
            title: "Renamed",
            summary: "revised",
            participants: &[],
            action_items: &[],
        };
        file_meeting(&g, "m", updated, 200).await.unwrap();

        let rows = list_meetings(&g).await.unwrap();
        assert_eq!(rows.len(), 1, "re-file must not duplicate the meeting");
        assert_eq!(rows[0].title, "Renamed");
        assert_eq!(rows[0].started_at, 200);
    }

    #[tokio::test]
    async fn get_unknown_meeting_is_none() {
        let g = graph();
        assert!(get_meeting(&g, "nope").await.unwrap().is_none());
    }
}

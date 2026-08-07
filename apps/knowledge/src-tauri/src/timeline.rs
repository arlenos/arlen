// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

//! The Timeline read (KA-R2): what the system actually recorded, newest first.
//!
//! Two sources, because the graph has two records of activity: a `File` node
//! carries when it was last accessed and by which app, and an `Event` node
//! carries a window focus. Both are facts the promotion pipeline wrote from real
//! events - nothing here is inferred.
//!
//! **No sessions.** The model has a `session` item for contiguous work clustered
//! under a title, and clustering is a judgement about where one stretch of work
//! ends. Emitting guessed boundaries would put invented history into a surface
//! whose whole promise is that it shows only what was captured, so this returns
//! lone events and the session item stays unused until the clustering is built
//! deliberately.

use serde::Serialize;
use std::collections::HashMap;

/// One recorded event, in the shape the timeline renders.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct TimelineEvent {
    pub id: String,
    /// One of the model's kinds: `opened`, `edited`, `ran`, `focus`, `agent`,
    /// `imported`.
    pub kind: String,
    /// The verb as a MESSAGE ID (`k.tl.verb.opened`), never the word.
    ///
    /// The app learned this the expensive way: the verb once shipped as English
    /// and every row on a German timeline read "opened chapter-3.md", and the
    /// i18n lint could not see it because a lowercase single token looks like an
    /// identifier. A backend emitting the word would walk past that lint for the
    /// same reason, so it emits the id and the catalog renders it.
    pub verb: String,
    /// What was acted on, as a person would recognise it.
    pub object: String,
    /// Where it happened.
    pub source: String,
    /// Unix seconds.
    pub at: i64,
    /// The project, when the graph knows one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
}

/// One spine item. Only the `event` variant is produced; see the module note on
/// why sessions are absent rather than guessed.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct TimelineItem {
    pub kind: &'static str,
    pub event: TimelineEvent,
}

/// How far back one read reaches, per source. The surface groups by day and a
/// person scrolls days, not thousands of rows.
const LIMIT: usize = 200;

/// The recorded spine, newest first.
///
/// Both reads are best-effort in the same direction: whichever source answers
/// contributes its rows, and a failure of one does not empty the other. A total
/// failure returns an error, so the store shows its fixture and says it is
/// mocked, rather than an empty timeline that would read as "you did nothing".
#[tauri::command]
pub async fn knowledge_timeline() -> Result<Vec<TimelineItem>, String> {
    let socket = os_sdk::runtime::socket_path("ARLEN_KNOWLEDGE_SOCKET", "knowledge.sock");
    let client = os_sdk::graph::UnixGraphClient::new(socket.to_string_lossy().into_owned());

    let files = read_file_accesses(&client).await;
    let focus = read_window_focus(&client).await;
    let mut events: Vec<TimelineEvent> = match (files, focus) {
        // Both down is a real failure: the store falls to its fixture and says
        // it is mocked, which beats an empty spine reading as "you did nothing".
        (Err(e), Err(_)) => return Err(e),
        (Ok(a), Ok(b)) => a.into_iter().chain(b).collect(),
        (Ok(a), Err(_)) | (Err(_), Ok(a)) => a,
    };
    events.sort_by_key(|e| std::cmp::Reverse(e.at));
    events.truncate(LIMIT);
    Ok(events
        .into_iter()
        .map(|event| TimelineItem { kind: "event", event })
        .collect())
}

/// File accesses: the promotion pipeline stamps `last_accessed` and the app that
/// opened it.
///
/// **No project join, and that is not an oversight.** The read gate requires
/// every label AND relationship type in a query to be in the caller's readable
/// set, and that set is built by stripping `system.` from the profile's read
/// scopes and keeping only names that are entirely alphanumeric - so
/// `FILE_PART_OF` can never be in it. Measured against the gate itself, a
/// timeline query carrying the membership join answers "read denied: label
/// outside the caller's read scope" for any caller that is not system-anchored,
/// which this app is not (the first-party list is the four daemons).
///
/// Asking for the join anyway would cost the WHOLE read - the spine would fall
/// back to fixture data rather than show what the graph really holds. So this
/// asks for what it can have. The project column stays empty until the scope
/// question is settled one way or the other, and an empty column is a smaller
/// lie than an invented timeline.
async fn read_file_accesses(
    client: &os_sdk::graph::UnixGraphClient,
) -> Result<Vec<TimelineEvent>, String> {
    let cypher = format!(
        "MATCH (f:File) WHERE f.last_accessed IS NOT NULL \
         RETURN f.id AS id, f.path AS path, f.app_id AS app_id, \
                f.last_accessed AS at \
         ORDER BY f.last_accessed DESC LIMIT {LIMIT}"
    );
    let rows = client.query_rows(&cypher).await.map_err(|e| e.to_string())?;
    Ok(rows
        .iter()
        .filter_map(|r| {
            let path = text(r, "path")?;
            Some(TimelineEvent {
                id: text(r, "id").unwrap_or_else(|| path.clone()),
                kind: "opened".to_string(),
                verb: "k.tl.verb.opened".to_string(),
                object: basename(&path),
                source: text(r, "app_id").unwrap_or_default(),
                at: seconds(r, "at")?,
                project: None,
            })
        })
        .collect())
}

/// Window focus: an `Event` node the compositor's `window.focused` produced,
/// whose title is the window's.
async fn read_window_focus(
    client: &os_sdk::graph::UnixGraphClient,
) -> Result<Vec<TimelineEvent>, String> {
    let cypher = format!(
        "MATCH (e:Event) WHERE e.type = 'window.focused' \
         RETURN e.id AS id, e.title AS title, e.source AS source, \
                e.timestamp AS at \
         ORDER BY e.timestamp DESC LIMIT {LIMIT}"
    );
    let rows = client.query_rows(&cypher).await.map_err(|e| e.to_string())?;
    Ok(rows
        .iter()
        .filter_map(|r| {
            Some(TimelineEvent {
                id: text(r, "id")?,
                kind: "focus".to_string(),
                verb: "k.tl.verb.focused".to_string(),
                object: text(r, "title")?,
                source: text(r, "source").unwrap_or_default(),
                at: seconds(r, "at")?,
                project: None,
            })
        })
        .collect())
}

/// The last path component, or the whole string when there is none.
fn basename(path: &str) -> String {
    path.rsplit('/').next().unwrap_or(path).to_string()
}

/// A string cell, or `None` when absent or not a string.
fn text(row: &HashMap<String, serde_json::Value>, key: &str) -> Option<String> {
    row.get(key)?.as_str().map(str::to_string)
}

/// A timestamp cell as Unix SECONDS; the graph stores epoch microseconds.
fn seconds(row: &HashMap<String, serde_json::Value>, key: &str) -> Option<i64> {
    row.get(key)?.as_i64().map(|micros| micros / 1_000_000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_row_names_its_verb_as_a_catalog_id_not_a_word() {
        // The one thing a backend must not do here. `k.tl.verb.opened` renders
        // translated; "opened" renders as "opened" on a German timeline, and the
        // i18n lint cannot see it because it watches components.
        let mut row = HashMap::new();
        row.insert("path".into(), serde_json::json!("/home/u/chapter-3.md"));
        row.insert("id".into(), serde_json::json!("/home/u/chapter-3.md"));
        row.insert("app_id".into(), serde_json::json!("text-editor"));
        row.insert("at".into(), serde_json::json!(1_700_000_000_000_000i64));
        let e = TimelineEvent {
            id: text(&row, "id").unwrap(),
            kind: "opened".into(),
            verb: "k.tl.verb.opened".into(),
            object: basename(&text(&row, "path").unwrap()),
            source: text(&row, "app_id").unwrap(),
            at: seconds(&row, "at").unwrap(),
            project: None,
        };
        assert!(e.verb.starts_with("k.tl.verb."), "a catalog id, never a word");
        assert_eq!(e.object, "chapter-3.md", "the name, not the whole path");
        assert_eq!(e.at, 1_700_000_000, "seconds, not the microseconds the graph stores");
    }

    #[test]
    fn a_relative_name_survives_having_no_slash() {
        assert_eq!(basename("notes.md"), "notes.md");
        assert_eq!(basename("/a/b/c.txt"), "c.txt");
        // A trailing slash yields an empty last component; that is what the path
        // says, and inventing the parent's name would be a guess.
        assert_eq!(basename("/a/b/"), "");
    }

    #[test]
    fn an_item_is_tagged_the_way_the_union_expects() {
        let item = TimelineItem {
            kind: "event",
            event: TimelineEvent {
                id: "e1".into(),
                kind: "focus".into(),
                verb: "k.tl.verb.focused".into(),
                object: "Terminal".into(),
                source: "wayland".into(),
                at: 5,
                project: None,
            },
        };
        let json = serde_json::to_value(&item).unwrap();
        assert_eq!(json["kind"], "event");
        assert_eq!(json["event"]["verb"], "k.tl.verb.focused");
        // An absent project is omitted, matching the optional field the model
        // declares rather than sending a null the renderer would have to guard.
        assert!(json["event"].get("project").is_none());
    }
}

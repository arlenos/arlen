// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

//! The Projects browser read (KA-R3): projects, then their members, as the
//! Miller columns drill down.
//!
//! The columns walk a VIRTUAL slash path rather than a filesystem one - `/` is
//! the set of projects, `/Thesis` is that project's members. Two levels is the
//! whole of it here, because a third column is a member's relationship hops and
//! that is a different read.

use serde::Serialize;
use std::collections::HashMap;

/// One browser row, matching the kit's `FileEntry` so the Miller columns render
/// a project exactly as they render a directory. Snake-case on the wire: the
/// shape is the kit's, not this app's, and renaming it here would fork it.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct BrowserEntry {
    /// What the column shows.
    pub name: String,
    /// `"directory"` for a project, `"file"` for a member.
    pub kind: String,
    /// Always null: a project has no byte size, and a member's size belongs to
    /// the filesystem read rather than the graph.
    pub size: Option<u64>,
    /// Seconds since the epoch, when the graph recorded one.
    pub modified_unix: Option<i64>,
    pub is_hidden: bool,
    pub readonly: bool,
    pub symlink_target: Option<String>,
    /// The member's real path, so "reveal in containing folder" can work from a
    /// virtual listing. Absent for a project, which has no single home.
    pub full_path: Option<String>,
}

/// The Projects columns: `/` lists projects, `/<project>` lists its members.
///
/// `as_of` is accepted and, when set, **refused** rather than answered with
/// present-day rows. The as-of read is the daemon's bitemporal `valid_as_of`
/// and this command does not perform it yet; returning today's members under a
/// past timestamp would be the one failure the scrubber cannot survive, since
/// the whole point of dragging the control is to trust that the view changed.
/// An error sends the frontend to its fixture, which says it is mocked.
#[tauri::command]
pub async fn knowledge_projects_list(
    path: String,
    as_of: Option<i64>,
) -> Result<Vec<BrowserEntry>, String> {
    if as_of.is_some() {
        return Err("as-of reads are not wired yet".to_string());
    }
    let socket = os_sdk::runtime::socket_path("ARLEN_KNOWLEDGE_SOCKET", "knowledge.sock");
    let client = os_sdk::graph::UnixGraphClient::new(socket.to_string_lossy().into_owned());

    let trimmed = path.trim_matches('/');
    if trimmed.is_empty() {
        list_projects(&client).await
    } else {
        list_members(&client, trimmed).await
    }
}

/// The browser's place listing (the `knowledge_list` intent in `adapter.ts`):
/// one virtual place per sidebar entry - timeline, projects, searches, library,
/// capsules.
///
/// **Only `projects` answers for real.** The other four are reads this app does
/// not have yet, and each one refuses so the store marks that place mocked and
/// serves its fixture. Refusing per place rather than per command is what lets
/// the Projects place go live while the rest stay honestly labelled: a command
/// that answered them all with empty lists would show four places as "your graph
/// has nothing" when the truth is "nobody asked it".
#[tauri::command]
pub async fn knowledge_list(location: String) -> Result<Vec<BrowserEntry>, String> {
    if location != "projects" {
        return Err(format!("the {location} place is not wired yet"));
    }
    let socket = os_sdk::runtime::socket_path("ARLEN_KNOWLEDGE_SOCKET", "knowledge.sock");
    let client = os_sdk::graph::UnixGraphClient::new(socket.to_string_lossy().into_owned());
    list_projects(&client).await
}

/// Every live project, newest first.
async fn list_projects(
    client: &os_sdk::graph::UnixGraphClient,
) -> Result<Vec<BrowserEntry>, String> {
    let rows = client
        .query_rows(
            "MATCH (p:Project) WHERE p.expired_at IS NULL \
             RETURN p.name AS name, p.created_at AS created_at \
             ORDER BY p.created_at DESC LIMIT 500",
        )
        .await
        .map_err(|e| e.to_string())?;
    Ok(rows
        .iter()
        .filter_map(|r| {
            let name = text(r, "name")?;
            Some(BrowserEntry {
                name,
                kind: "directory".to_string(),
                size: None,
                modified_unix: seconds(r, "created_at"),
                is_hidden: false,
                readonly: true,
                symlink_target: None,
                full_path: None,
            })
        })
        .collect())
}

/// One project's live members, by the bitemporal FILE_PART_OF edge.
///
/// Liveness comes from the EDGE stamps alone, and that is not a shortcut: a
/// `File` node has no `expired_at` column - only `Project` does - and this
/// engine refuses a labelled match that names a column the table lacks
/// ("Binder exception: Cannot find property expired_at for f"). An earlier cut
/// of this query filtered on it and would have failed every members listing,
/// silently, with the frontend showing its fixture instead. Membership is what
/// the question is about anyway, and the edge carries it.
async fn list_members(
    client: &os_sdk::graph::UnixGraphClient,
    project: &str,
) -> Result<Vec<BrowserEntry>, String> {
    let cypher = format!(
        "MATCH (f:File)-[r:FILE_PART_OF]->(p:Project {{name: '{}'}}) \
         WHERE r.invalid_at IS NULL AND r.expired_at IS NULL \
         RETURN f.path AS path, f.last_accessed AS last_accessed \
         ORDER BY f.path LIMIT 2000",
        escape_cypher_literal(project)
    );
    let rows = client.query_rows(&cypher).await.map_err(|e| e.to_string())?;
    Ok(rows
        .iter()
        .filter_map(|r| {
            let path = text(r, "path")?;
            let name = path.rsplit('/').next().unwrap_or(&path).to_string();
            Some(BrowserEntry {
                name,
                kind: "file".to_string(),
                size: None,
                modified_unix: seconds(r, "last_accessed"),
                is_hidden: false,
                readonly: true,
                symlink_target: None,
                full_path: Some(path),
            })
        })
        .collect())
}

/// A string cell, or `None` when the column is absent or not a string. A row
/// missing its name is dropped rather than shown as an empty entry.
fn text(row: &HashMap<String, serde_json::Value>, key: &str) -> Option<String> {
    row.get(key)?.as_str().map(str::to_string)
}

/// A timestamp cell as Unix SECONDS. The graph stores microseconds since the
/// epoch and the kit's `modified_unix` is seconds, so this converts rather than
/// passing a number that would render as a date fifty thousand years out.
fn seconds(row: &HashMap<String, serde_json::Value>, key: &str) -> Option<i64> {
    row.get(key)?.as_i64().map(|micros| micros / 1_000_000)
}

/// Escape a string for a single-quoted Cypher literal: backslash first, so an
/// escaped quote is not double-escaped, then the quote. A project name is a
/// user-chosen directory name and can contain either.
fn escape_cypher_literal(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\'', "\\'")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_project_name_cannot_break_out_of_its_literal() {
        // The name comes from a directory on disk, so a quote in it is ordinary
        // rather than hostile - but it would end the literal all the same.
        assert_eq!(escape_cypher_literal("Tim's thesis"), "Tim\\'s thesis");
        assert_eq!(escape_cypher_literal(r"back\slash"), r"back\\slash");
        // Backslash first: an already-escaped quote must not double-escape.
        assert_eq!(escape_cypher_literal(r"a\'b"), r"a\\\'b");
    }

    #[test]
    fn a_microsecond_stamp_reads_back_as_seconds() {
        let mut row = HashMap::new();
        row.insert("t".to_string(), serde_json::json!(1_700_000_000_000_000i64));
        assert_eq!(seconds(&row, "t"), Some(1_700_000_000));
        // An absent or non-numeric cell is absent, never zero - a zero would
        // render as 1970 and look like a fact.
        assert_eq!(seconds(&row, "missing"), None);
        row.insert("s".to_string(), serde_json::json!("nope"));
        assert_eq!(seconds(&row, "s"), None);
    }

    #[tokio::test]
    async fn an_unwired_place_refuses_so_it_is_marked_mocked_not_empty() {
        // The store flips `mocked` per call, so a refusal is what keeps the four
        // unwired places labelled. Answering them with an empty list would read
        // as "the graph knows nothing about your library".
        for place in ["timeline", "searches", "library", "capsules"] {
            assert!(
                knowledge_list(place.to_string()).await.is_err(),
                "{place} must refuse rather than answer empty"
            );
        }
    }

    #[tokio::test]
    async fn an_as_of_read_is_refused_rather_than_answered_with_today() {
        // The scrubber's whole promise is that the view changed. Answering a
        // past timestamp with present rows is the one lie it cannot survive.
        let r = knowledge_projects_list("/".to_string(), Some(1_700_000_000)).await;
        assert!(r.is_err(), "an as-of read must refuse until it is wired");
    }
}

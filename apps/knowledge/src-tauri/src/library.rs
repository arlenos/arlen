// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

//! The Library read: what the bridges have put in the graph, grouped by source.
//!
//! Nothing is decided here. The daemon owns the read and the display contract
//! (`bridge-architecture.md`: a type declares its own name, its class and its
//! title field), so this command dials the socket and hands the sections through.
//! A section name or a layout invented on this side would be the frontend lookup
//! table that decision ruled out, one layer down.

use serde::Serialize;

/// One item, as the page renders it. The daemon has already resolved the title
/// (the declared field, or the entry's own key when there is none), so nothing
/// here chooses what to show.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LibraryEntry {
    pub id: String,
    pub title: String,
    pub sub: Option<String>,
    /// When this machine last wrote the row. **Not when the item was made** - no
    /// bridge declares that - so the page says "added".
    pub added: Option<i64>,
}

/// One section: one declared type from one source.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LibrarySection {
    /// The namespace, which is the origin tag a per-source revoke severs.
    pub source: String,
    #[serde(rename = "type")]
    pub qualified_type: String,
    /// The declared display name, or the qualified type when none was declared.
    pub label: String,
    /// One of the closed set: notes, documents, messages, media, other.
    pub class: String,
    pub entries: Vec<LibraryEntry>,
}

/// The Library sections.
///
/// An empty list means nothing has been bridged, which is the ordinary state of a
/// fresh machine and not a failure. The daemon probes before it reads, so a graph
/// that could not be reached arrives here as an error rather than as an empty
/// library, and the page says a different sentence for each.
#[tauri::command]
pub async fn knowledge_library() -> Result<Vec<LibrarySection>, String> {
    let socket = crate::service::socket_or_absent()?;
    let client = os_sdk::graph::UnixGraphClient::new(socket.to_string_lossy().into_owned());
    let sections = client.library().await.map_err(|e| e.to_string())?;
    Ok(sections
        .into_iter()
        .map(|s| LibrarySection {
            source: s.source,
            qualified_type: s.qualified_type,
            label: s.label,
            class: s.class,
            entries: s
                .entries
                .into_iter()
                .map(|e| LibraryEntry {
                    id: e.id,
                    title: e.title,
                    sub: e.sub,
                    added: e.added,
                })
                .collect(),
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_section_names_its_type_on_the_wire_as_type() {
        // `type` is a keyword in Rust and not in JSON, so the rename is the one
        // place this shape can silently disagree with the page that parses it.
        let json = serde_json::to_string(&LibrarySection {
            source: "md.obsidian".into(),
            qualified_type: "md.obsidian.Note".into(),
            label: "Notes".into(),
            class: "notes".into(),
            entries: vec![],
        })
        .unwrap();
        assert!(json.contains(r#""type":"md.obsidian.Note""#), "{json}");
        assert!(!json.contains("qualified_type"), "{json}");
    }

    #[test]
    fn an_absent_sub_line_is_null_rather_than_an_empty_string() {
        // The page leaves the line out when it is null. An empty string would
        // render as a gap under every entry that has no sub-line.
        let json = serde_json::to_string(&LibraryEntry {
            id: "md.obsidian.Note:a.md".into(),
            title: "Deep work".into(),
            sub: None,
            added: None,
        })
        .unwrap();
        assert!(json.contains(r#""sub":null"#), "{json}");
        assert!(json.contains(r#""added":null"#), "{json}");
    }
}

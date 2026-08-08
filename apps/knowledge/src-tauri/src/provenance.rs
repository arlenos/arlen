// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Where a file came from, as far as the graph can actually say (KA-R7).
//!
//! The detail pane asked for this through `knowledge_provenance` and no host
//! registered it, so every call threw and the store answered with a five-hop
//! lineage it had made up - "created in Text editor, part of Arlen OS, read by
//! just dev". That is the one thing a provenance surface must never do.
//!
//! What the graph really holds for a file, and what this returns, is thinner:
//! the promotion pipeline stamps a `File` node with the app that opened it and
//! when. So one hop, and it is true.
//!
//! **Why not more.** The lineage anyone wants - which project it belongs to, the
//! session it was worked in, which bridge imported it - lives on the edges, and
//! the daemon's read gate rejects any query naming a relationship type for a
//! caller that is not system-anchored (the readable set is built by stripping
//! `system.` from the profile's read scopes and keeping alphanumeric names, so
//! `FILE_PART_OF` can never be in it). The timeline hit the same wall and made the
//! same choice, with the reasoning in its own module: ask for what the caller can
//! have, because a query carrying the join fails whole and takes the honest part
//! down with it. This grows when that scope question is settled, not before.

use std::collections::HashMap;

use serde::Serialize;
use serde_json::Value;

/// One lineage hop, in the shape the detail pane renders.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ProvenanceHop {
    /// The verb as a MESSAGE ID (`k.pv.verb.openedIn`), never the word - the
    /// pane resolves it through the catalogue. The timeline learned this the
    /// expensive way: a verb shipped as English put "opened chapter-3.md" on a
    /// German surface, and the i18n lint could not see it because a lowercase
    /// single token looks like an identifier.
    pub verb: String,
    /// What the verb points at, as a person would recognise it. Data, not chrome,
    /// so it is not a message id.
    pub subject: String,
    /// Unix seconds, when the graph knows.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub when: Option<i64>,
}

/// The lineage for a node, named as the surface names it (a path, or the
/// basename the timeline shows).
///
/// An unknown node answers with an empty list rather than a guess: the pane
/// renders nothing at all for an empty one, which is the honest state for a file
/// the graph has no record of. An unreachable daemon is an error, so the caller
/// can say the read failed instead of showing "we know nothing about this" for a
/// file it may know plenty about.
#[tauri::command]
pub async fn knowledge_provenance(node: String) -> Result<Vec<ProvenanceHop>, String> {
    let socket = os_sdk::runtime::socket_path("ARLEN_KNOWLEDGE_SOCKET", "knowledge.sock");
    let client = os_sdk::graph::UnixGraphClient::new(socket.to_string_lossy().into_owned());
    let rows = client
        .query_rows(&file_query(&node))
        .await
        .map_err(|e| e.to_string())?;
    Ok(hops_from_rows(&rows))
}

/// Match the file by exact path or by basename, since the surface shows a name
/// and the graph keys on the path.
///
/// The name is escaped rather than trusted: it arrives from the frontend, and a
/// quote in a filename would otherwise end the literal and leave the rest of the
/// name as Cypher. Filenames with quotes are legal on every filesystem here.
fn file_query(node: &str) -> String {
    let safe = node.replace('\\', "\\\\").replace('\'', "\\'");
    format!(
        "MATCH (f:File) WHERE f.path = '{safe}' OR f.path ENDS WITH '/{safe}' \
         RETURN f.path AS path, f.app_id AS app_id, f.last_accessed AS at LIMIT 1"
    )
}

/// Turn the file row into hops. Pure, so the shape is tested without a daemon.
fn hops_from_rows(rows: &[HashMap<String, Value>]) -> Vec<ProvenanceHop> {
    let Some(row) = rows.first() else {
        return Vec::new();
    };
    let mut hops = Vec::new();
    if let Some(app) = text(row, "app_id") {
        hops.push(ProvenanceHop {
            verb: "k.pv.verb.openedIn".into(),
            subject: app,
            when: seconds(row, "at"),
        });
    }
    hops
}

/// A non-empty string cell, or None. Empty is None on purpose: a hop whose
/// subject is the empty string renders as a verb pointing at nothing.
fn text(row: &HashMap<String, Value>, key: &str) -> Option<String> {
    match row.get(key)?.as_str() {
        Some(s) if !s.is_empty() => Some(s.to_string()),
        _ => None,
    }
}

/// A timestamp cell as Unix SECONDS; the graph stores epoch microseconds. Same
/// conversion the timeline uses, so two surfaces cannot disagree about when the
/// same access happened.
fn seconds(row: &HashMap<String, Value>, key: &str) -> Option<i64> {
    row.get(key)?.as_i64().map(|micros| micros / 1_000_000)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn row(pairs: &[(&str, Value)]) -> HashMap<String, Value> {
        pairs.iter().map(|(k, v)| ((*k).to_string(), v.clone())).collect()
    }

    #[test]
    fn a_quote_in_a_filename_cannot_end_the_literal() {
        let q = file_query("it's a file.md");
        assert!(q.contains("it\\'s a file.md"), "{q}");
        // One statement, and the name never leaves the two quoted literals.
        assert_eq!(q.matches("RETURN").count(), 1);
    }

    #[test]
    fn a_file_the_graph_does_not_know_gets_no_hops_rather_than_a_guess() {
        assert!(hops_from_rows(&[]).is_empty());
    }

    #[test]
    fn the_opening_app_becomes_one_true_hop() {
        let r = row(&[
            ("path", json!("/home/t/notes.md")),
            ("app_id", json!("text-editor")),
            ("at", json!(1_786_000_000_000_000i64)),
        ]);
        let hops = hops_from_rows(&[r]);
        assert_eq!(hops.len(), 1);
        assert_eq!(hops[0].verb, "k.pv.verb.openedIn", "a message id, never a word");
        assert_eq!(hops[0].subject, "text-editor");
        assert_eq!(hops[0].when, Some(1_786_000_000), "micros are shown as seconds");
    }

    #[test]
    fn a_row_without_an_app_yields_nothing_rather_than_an_empty_subject() {
        let r = row(&[("path", json!("/home/t/notes.md"))]);
        assert!(hops_from_rows(&[r]).is_empty());
    }
}

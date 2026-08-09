// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

//! The editor's own lens reads.
//!
//! The lens panel invoked `provenance_of`, `related_of` and `project_of` - names
//! defined in the FILES app's binary. A Tauri command lives in one app's process,
//! so those calls were rejected at runtime and the panel fell to its fixture on
//! every load. Nobody noticed because the fixture is labelled and the failure is
//! silent, and until this app had a host at all the scope gate filed it under "no
//! backend" rather than "calling the wrong app".
//!
//! `provenance_of` is answered here, from this app's own read of the graph. The
//! other two are not, and that is the finding rather than an omission: backlinks
//! and project membership are EDGES, and the knowledge daemon's read gate refuses
//! any query naming a relationship type to a caller that is not system-anchored.
//! The knowledge app's own timeline and provenance chain hit the same wall today.
//! Asking anyway would fail the whole read, so the panel keeps its labelled
//! sample for those two sections and this one is real.

use std::collections::HashMap;

use serde::Serialize;
use serde_json::Value;

/// One lineage step, in the shape the lens renders.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ProvenanceStep {
    /// A verb MESSAGE ID, never the word: the panel resolves it through the
    /// catalogue, so an English string here would ship untranslated.
    pub verb: String,
    /// What the verb points at. Data, not chrome.
    pub subject: String,
    /// Unix seconds, when the graph knows.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub when: Option<i64>,
}

/// Where the open file came from, as far as the graph can say.
///
/// An unknown file answers with an empty list rather than a guess; an unreachable
/// daemon is an error, so the panel can tell "nothing recorded" from "could not
/// ask".
#[tauri::command]
pub async fn provenance_of(r#ref: String) -> Result<Vec<ProvenanceStep>, String> {
    let socket = os_sdk::runtime::socket_path("ARLEN_KNOWLEDGE_SOCKET", "knowledge.sock");
    let client = os_sdk::graph::UnixGraphClient::new(socket.to_string_lossy().into_owned());
    let rows = client
        .query_rows(&file_query(&r#ref))
        .await
        .map_err(|e| e.to_string())?;
    Ok(steps_from_rows(&rows))
}

/// Match by exact path or basename, since the lens is given whichever the
/// surface has. The name is escaped: it arrives from the frontend, and a quote in
/// a filename would otherwise end the literal and leave the rest as Cypher.
fn file_query(node: &str) -> String {
    let safe = node.replace('\\', "\\\\").replace('\'', "\\'");
    format!(
        "MATCH (f:File) WHERE f.path = '{safe}' OR f.path ENDS WITH '/{safe}' \
         RETURN f.path AS path, f.app_id AS app_id, f.last_accessed AS at LIMIT 1"
    )
}

/// Pure, so the shape is tested without a daemon.
fn steps_from_rows(rows: &[HashMap<String, Value>]) -> Vec<ProvenanceStep> {
    let Some(row) = rows.first() else {
        return Vec::new();
    };
    let mut steps = Vec::new();
    if let Some(app) = row.get("app_id").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
        steps.push(ProvenanceStep {
            verb: "te.pv.verb.openedIn".into(),
            subject: app.to_string(),
            // Epoch micros in the graph, seconds on the surface - the same
            // conversion the knowledge app uses, so two panels cannot disagree
            // about when one access happened.
            when: row.get("at").and_then(|v| v.as_i64()).map(|micros| micros / 1_000_000),
        });
    }
    steps
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
    }

    #[test]
    fn an_unknown_file_gets_no_steps_rather_than_a_guess() {
        assert!(steps_from_rows(&[]).is_empty());
    }

    #[test]
    fn the_opening_app_becomes_one_true_step() {
        let r = row(&[
            ("app_id", json!("text-editor")),
            ("at", json!(1_786_000_000_000_000i64)),
        ]);
        let steps = steps_from_rows(&[r]);
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].verb, "te.pv.verb.openedIn", "a message id, never a word");
        assert_eq!(steps[0].when, Some(1_786_000_000));
    }
}

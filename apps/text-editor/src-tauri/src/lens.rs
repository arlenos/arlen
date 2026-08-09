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

/// One lineage step, in exactly the shape the lens renders.
///
/// The field names are the frontend's, and that is not a detail: the first cut
/// of this returned `verb`/`subject`/`when`, which type-checked on both sides
/// and would have rendered "undefined undefined" in the panel. A command whose
/// answer does not fit its caller is a silent blank, not an error.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ProvenanceStep {
    /// The relation as a MESSAGE ID, never the word: the panel resolves it
    /// through the catalogue, so an English string here would ship untranslated.
    pub relation: String,
    /// Who or what acted, at the fidelity the graph actually has.
    pub actor: String,
    /// `user` | `graph` | `external` | `model` | `agent` - tints the dot only,
    /// never adds specificity the record does not carry.
    pub origin: &'static str,
    /// When, already phrased for display. The panel prints this verbatim, so an
    /// empty string is the honest form for "the graph did not record a time".
    pub when: String,
    /// `resolved` | `pid` | `proxy`: how confidently the actor is known.
    pub fidelity: &'static str,
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
            relation: "te.pv.verb.openedIn".into(),
            actor: app.to_string(),
            // The promotion pipeline wrote this from an observed file open, so
            // the origin is the graph's own observation, not a claim by a user
            // or a model.
            origin: "graph",
            // Epoch micros in the graph. The panel prints `when` verbatim, and
            // an ISO date is the one form that is unambiguous without knowing
            // the reader's locale; "3 weeks ago" would be the app's phrasing to
            // make, not this command's.
            when: row
                .get("at")
                .and_then(|v| v.as_i64())
                .map(|micros| iso_day(micros / 1_000_000))
                .unwrap_or_default(),
            // The graph records the app id it saw, so the actor is resolved -
            // not a pid we guessed a name for.
            fidelity: "resolved",
        });
    }
    steps
}

/// `YYYY-MM-DD` for Unix seconds, UTC. Civil-from-days, so no date dependency
/// for one field.
fn iso_day(seconds: i64) -> String {
    let days = seconds.div_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
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
    fn the_shape_carries_every_field_the_panel_reads() {
        let json = serde_json::to_string(&ProvenanceStep {
            relation: "x".into(),
            actor: "y".into(),
            origin: "graph",
            when: String::new(),
            fidelity: "resolved",
        })
        .unwrap();
        // The panel reads all five; a renamed field renders as a blank rather
        // than an error, so the names are asserted rather than trusted.
        for field in ["relation", "actor", "origin", "when", "fidelity"] {
            assert!(json.contains(&format!("\"{field}\"")), "{field} missing from {json}");
        }
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
        assert_eq!(steps[0].relation, "te.pv.verb.openedIn", "a message id, never a word");
        assert_eq!(steps[0].actor, "text-editor");
        assert_eq!(steps[0].when, "2026-08-06");
        assert_eq!(steps[0].origin, "graph");
        assert_eq!(steps[0].fidelity, "resolved");
    }
}

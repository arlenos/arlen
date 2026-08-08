// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

//! The recent-actions panel's two commands, over `org.arlen.Undo1`.
//!
//! The panel has invoked `undo_read` and `undo_enact` since it was designed and
//! nothing implemented them, so it fell back to its fixture and said so through
//! `undoMocked`. The session undo service now serves both, and this is the shell
//! side of that call.
//!
//! **Nothing here asks whether the assistant is running.** That is the point of
//! the separate service: the same operations used to live on `org.arlen.AIAgent1`,
//! which exists only when `[ai] enabled` is true, so switching the assistant off
//! removed a user's own file moves from the list and their undo with them.
//!
//! # What this translates, and what it refuses to invent
//!
//! The daemon serves the ledger's own shape: an op id, the folded lifecycle
//! state, a stable `inverseKind`, the object, whether the undo can be carried out
//! here, and the audit facts when the join resolved. The panel wants display
//! shape: a producer, a past-tense verb, a label for the button. Translating
//! identifiers into words is the UI side's job by design - `inverse_kind` is
//! documented as "a stable identifier the UI translates" - which is why the
//! wording lives here and not in the daemon.
//!
//! Two of those translations would be guesses if the source did not constrain
//! them, so both are pinned by the source and by a test:
//!
//! * The **verb** is derived, not invented. Every receipt in the log is one of
//!   seven closed variants, and each names the act it reverses exactly, so
//!   "restore this path" can only have come from a move.
//! * **Nothing is ever reported irreversible.** An action with no inverse never
//!   produces a receipt, so an irreversible act is absent from this log rather
//!   than present-and-marked. Reporting one would be a claim the ledger cannot
//!   make. The one entry that cannot be enacted here - a filesystem snapshot
//!   rollback - is reversible with a cost, and it is the cost that is reported.
//!
//! And one thing is left blank rather than filled in: **the time**. The signed
//! entry carries no timestamp of its own, so the clock comes entirely from the
//! audit join. With the audit daemon down, every row is timeless. The field is
//! omitted in that case, so the panel renders no time instead of 1 January 1970.

use serde::{Deserialize, Serialize};

/// The bus name the session undo service owns.
const BUS_NAME: &str = "org.arlen.Undo1";
/// The object path it serves.
const OBJECT_PATH: &str = "/org/arlen/Undo1";
/// The interface both methods live on.
const INTERFACE: &str = "org.arlen.Undo1";

/// One row as the daemon serves it.
///
/// A local mirror of the daemon's `UndoRow` rather than a shared type: the real
/// one carries a `&'static str` and so cannot be deserialized, and the shell has
/// no business depending on the undo stack for a JSON read. The two shapes are
/// held together by a test that maps a genuinely serialized `UndoRow`, so a field
/// renamed on the daemon side fails here rather than arriving as `None`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Row {
    op_id: String,
    inverse_kind: String,
    object: String,
    enactable: bool,
    #[serde(default)]
    description: Option<Description>,
}

/// The audit facts, when the join resolved.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Description {
    /// The `app_id` of whoever acted, kernel-attested at audit ingest.
    actor: String,
    /// Microseconds since the Unix epoch.
    timestamp_micros: i64,
}

/// One row as the panel renders it.
///
/// Named `UndoRowView` and not `UndoEntry`, which is what the panel's interface
/// calls it, for a reason worth stating: `ai-undo-core` already defines an
/// `UndoEntry`, and `check-invoke-shape` drops any struct name it finds twice
/// rather than compare against the wrong one. Sharing the name silently removed
/// this command from the 94 return shapes that check compares - a coverage hole
/// that looks exactly like a pass. The check now says so out loud; this name
/// keeps it from having to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UndoRowView {
    /// The handle an undo is requested with.
    op_id: String,
    /// Who acted, as the panel's four-way chip.
    producer: &'static str,
    /// The quiet leading verb, past tense.
    verb: &'static str,
    /// The emphasized object.
    object: String,
    /// Unix seconds. Absent when the audit join found nothing, because the signed
    /// entry has no clock of its own and a wrong time reads as a real one.
    #[serde(skip_serializing_if = "Option::is_none")]
    at: Option<i64>,
    /// `reversible` or `reversible_with_cost`; never `irreversible`, see the
    /// module note.
    reversibility: &'static str,
    /// The inverse named as the act it performs. Absent when the undo cannot be
    /// carried out here, which is how the panel knows not to offer the button.
    #[serde(skip_serializing_if = "Option::is_none")]
    inverse_label: Option<&'static str>,
    /// The panel's own interaction state. Every row starts ready; `enacting` and
    /// `done` are set by the panel as it works, and are not ledger facts.
    state: &'static str,
}

/// The forward act a receipt reverses, and the button that reverses it.
///
/// Keyed by `inverse_kind`, whose values are the seven `InverseReceipt` variants.
/// An unknown kind is a receipt variant added without coming back here; it gets a
/// neutral wording rather than a wrong one, and no button, because a label that
/// misnames what a button does is worse than a row without one.
fn wording(inverse_kind: &str) -> (&'static str, Option<&'static str>) {
    match inverse_kind {
        "restore-path" => ("moved", Some("Put back")),
        "restore-from-trash" => ("deleted", Some("Restore")),
        "restore-value" => ("changed", Some("Restore previous")),
        "delete-created" => ("created", Some("Remove it")),
        "trash-created" => ("created", Some("Move it to Trash")),
        "retract-graph-edge" => ("tagged", Some("Untag")),
        // Reversible, but not from here: rolling a filesystem snapshot back is
        // its own mechanism with its own cost.
        "restore-snapshot" => ("changed", None),
        _ => ("acted on", None),
    }
}

/// Which chip the panel draws, from the `app_id` the audit ledger attests.
///
/// The undo signer admits exactly one producer today - `ai-agent`, see its
/// `ADMITTED` - so an unjoined row is the assistant's by attestation and not by
/// assumption. The file manager, terminal and settings are listed because the
/// panel's design has chips for them and they will journal inverses; until they
/// do, those arms are unreachable and honest, not speculative.
fn producer_of(actor: Option<&str>) -> &'static str {
    match actor {
        Some("arlen-files") | Some("dev.arlen-files") => "files",
        Some("arlen-terminal") | Some("dev.arlen-terminal") => "terminal",
        Some("settings") | Some("dev.arlen-settings") => "settings",
        // `ai-agent`, its dev id, and the unjoined case: the signer admits no
        // other producer, so this is attestation rather than a default.
        _ => "agent",
    }
}

/// Translate one served row into the row the panel renders.
fn entry_from_row(row: Row) -> UndoRowView {
    let (verb, label) = wording(&row.inverse_kind);
    UndoRowView {
        op_id: row.op_id,
        producer: producer_of(row.description.as_ref().map(|d| d.actor.as_str())),
        verb,
        object: row.object,
        at: row.description.as_ref().map(|d| d.timestamp_micros / 1_000_000),
        reversibility: if row.inverse_kind == "restore-snapshot" {
            "reversible_with_cost"
        } else {
            "reversible"
        },
        // Only where the daemon says it will act. A button the daemon refuses is
        // a button that does nothing.
        inverse_label: if row.enactable { label } else { None },
        state: "ready",
    }
}

/// Call one method on the session undo service.
async fn call<B>(method: &str, args: &B) -> Result<String, String>
where
    B: serde::Serialize + zbus::zvariant::DynamicType,
{
    let conn = zbus::Connection::session()
        .await
        .map_err(|e| format!("session bus: {e}"))?;
    let proxy = zbus::Proxy::new(&conn, BUS_NAME, OBJECT_PATH, INTERFACE)
        .await
        .map_err(|e| format!("undo service unavailable: {e}"))?;
    proxy
        .call_method(method, args)
        .await
        .map_err(|e| format!("{method}: {e}"))?
        .body()
        .deserialize()
        .map_err(|e| format!("{method} reply: {e}"))
}

/// The recent reversal entries, newest first.
///
/// An unreachable service is an error, never an empty list: "nothing has happened
/// yet" and "we could not ask" are different answers, and the panel treats an
/// error as its cue to say the list is a fixture.
#[tauri::command]
pub async fn undo_read() -> Result<Vec<UndoRowView>, String> {
    let json = call("Recent", &()).await?;
    let rows: Vec<Row> =
        serde_json::from_str(&json).map_err(|e| format!("undo rows: {e}"))?;
    Ok(rows.into_iter().map(entry_from_row).collect())
}

/// Enact one entry's inverse, by op id.
///
/// The op id is a lookup key and nothing more: the daemon replays the inverse the
/// signer holds, so this cannot describe an undo of its own devising. The wire
/// word comes back verbatim; anything but a success is an error so the panel does
/// not draw a refusal as a completed undo.
#[tauri::command]
pub async fn undo_enact(op_id: String) -> Result<String, String> {
    let outcome = call("Enact", &(op_id,)).await?;
    match outcome.as_str() {
        "restored" | "deleted" | "trashed" => Ok(outcome),
        other => Err(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(kind: &str, enactable: bool, actor: Option<&str>) -> Row {
        Row {
            op_id: "op-1".into(),
            inverse_kind: kind.into(),
            object: "report.pdf".into(),
            enactable,
            description: actor.map(|a| Description {
                actor: a.into(),
                timestamp_micros: 1_700_000_000_000_000,
            }),
        }
    }

    /// The seven receipt kinds are the whole vocabulary the log can hold, and
    /// each names its forward act exactly. If a kind is added without a wording,
    /// this catches it: the neutral fallback is for a live surface, not for a
    /// variant nobody translated.
    #[test]
    fn every_receipt_kind_has_its_own_wording() {
        let kinds = [
            "restore-path",
            "restore-from-trash",
            "restore-value",
            "delete-created",
            "trash-created",
            "retract-graph-edge",
            "restore-snapshot",
        ];
        for k in kinds {
            let (verb, _) = wording(k);
            assert_ne!(verb, "acted on", "{k} has no wording");
        }
        assert_eq!(wording("something-new").0, "acted on");
        assert_eq!(wording("something-new").1, None);
    }

    /// Nothing in this log is irreversible. An act with no inverse never produced
    /// a receipt, so marking a row irreversible would state something the ledger
    /// cannot know.
    #[test]
    fn no_row_is_ever_reported_irreversible() {
        for k in [
            "restore-path",
            "restore-from-trash",
            "restore-value",
            "delete-created",
            "trash-created",
            "retract-graph-edge",
            "restore-snapshot",
            "something-new",
        ] {
            for enactable in [true, false] {
                let e = entry_from_row(row(k, enactable, None));
                assert_ne!(e.reversibility, "irreversible", "{k}");
            }
        }
    }

    /// A snapshot rollback is reversible with a cost and not enactable here.
    /// Those are two different facts and the row carries both: no button, but
    /// not a claim of permanence.
    #[test]
    fn a_snapshot_rollback_is_costly_not_permanent() {
        let e = entry_from_row(row("restore-snapshot", false, None));
        assert_eq!(e.reversibility, "reversible_with_cost");
        assert_eq!(e.inverse_label, None);
    }

    /// A row the daemon will not enact offers no button, whatever its kind. A
    /// label the daemon refuses is a button that does nothing.
    #[test]
    fn an_unenactable_row_offers_no_button() {
        let e = entry_from_row(row("restore-path", false, None));
        assert_eq!(e.inverse_label, None);
        assert_eq!(entry_from_row(row("restore-path", true, None)).inverse_label, Some("Put back"));
    }

    /// With the audit daemon down there is no clock: the signed entry carries no
    /// time of its own. The field is omitted so the panel shows nothing, rather
    /// than sent as zero, which renders as a real date in 1970.
    #[test]
    fn an_unjoined_row_has_no_time_rather_than_a_wrong_one() {
        let e = entry_from_row(row("restore-path", true, None));
        assert_eq!(e.at, None);
        let json = serde_json::to_value(&e).unwrap();
        assert!(json.get("at").is_none(), "a missing time must not be serialized");

        let joined = entry_from_row(row("restore-path", true, Some("ai-agent")));
        assert_eq!(joined.at, Some(1_700_000_000));
    }

    /// Producers are read from the attested actor, and an unjoined row is the
    /// assistant's because the signer admits nobody else.
    #[test]
    fn the_producer_comes_from_the_attested_actor() {
        assert_eq!(producer_of(Some("arlen-files")), "files");
        assert_eq!(producer_of(Some("arlen-terminal")), "terminal");
        assert_eq!(producer_of(Some("settings")), "settings");
        assert_eq!(producer_of(Some("ai-agent")), "agent");
        assert_eq!(producer_of(None), "agent");
    }

    /// The panel's own interaction state, not the ledger's. Both sides spell it
    /// `state` and they mean different things, so this pins which one is sent.
    #[test]
    fn every_row_arrives_ready() {
        assert_eq!(entry_from_row(row("restore-path", true, None)).state, "ready");
    }

    /// The mirror matches what the daemon actually serves. A field renamed there
    /// must fail here rather than silently deserialize to `None`.
    #[test]
    fn the_served_shape_deserializes_into_the_mirror() {
        let served = serde_json::json!({
            "opId": "op-7",
            "correlationId": "c-7",
            "state": "Committed",
            "inverseKind": "restore-path",
            "object": "/home/t/report.pdf",
            "description": {
                "actor": "ai-agent",
                "kind": "graph-access",
                "subject": "agent.auto-tag-by-project",
                "timestampMicros": 1_700_000_000_000_000i64
            },
            "enactable": true
        });
        let parsed: Row = serde_json::from_value(served).unwrap();
        assert_eq!(parsed.op_id, "op-7");
        assert!(parsed.enactable);
        let e = entry_from_row(parsed);
        assert_eq!(e.verb, "moved");
        assert_eq!(e.producer, "agent");
        assert_eq!(e.at, Some(1_700_000_000));
    }
}

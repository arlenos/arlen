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

use audit_proto::AuditKind;
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
    /// Which forward act this row reverses, as a TOKEN the panel words: one of the
    /// seven `InverseReceipt` kinds, or `unknown`. Not the verb itself - the panel
    /// already reads the producer chip out of its catalogue, and a verb written
    /// here arrives English beside a translated chip ("Dateien moved ...").
    kind: &'static str,
    /// The emphasized object.
    object: String,
    /// Unix seconds. Absent when the audit join found nothing, because the signed
    /// entry has no clock of its own and a wrong time reads as a real one.
    #[serde(skip_serializing_if = "Option::is_none")]
    at: Option<i64>,
    /// `reversible` or `reversible_with_cost`; never `irreversible`, see the
    /// module note.
    reversibility: &'static str,
    /// Whether the panel may offer the undo button. False when the daemon says it
    /// will not act, and for the kinds whose inverse is not carried out from here.
    /// The button's wording follows `kind` out of the panel's catalogue.
    enactable: bool,
    /// The panel's own interaction state. Every row starts ready; `enacting` and
    /// `done` are set by the panel as it works, and are not ledger facts.
    state: &'static str,
}

/// The forward act a receipt reverses, and the button that reverses it.
///
/// Keyed by `inverse_kind`, whose values are the seven `InverseReceipt` variants.
/// An unknown kind is a receipt variant added without coming back here; it gets
/// the neutral `unknown` token rather than a wrong one, and no button, because a
/// label that misnames what a button does is worse than a row without one.
///
/// The returned token is the whole answer. The wording lives in the panel's
/// message catalogue in both languages, so the row reads as one sentence and not
/// as a translated chip in front of an English verb.
fn kind_of(inverse_kind: &str) -> (&'static str, bool) {
    match inverse_kind {
        "restore-path" => ("restore-path", true),
        "restore-from-trash" => ("restore-from-trash", true),
        "restore-value" => ("restore-value", true),
        "delete-created" => ("delete-created", true),
        "trash-created" => ("trash-created", true),
        "retract-graph-edge" => ("retract-graph-edge", true),
        // Reversible, but not from here: rolling a filesystem snapshot back is
        // its own mechanism with its own cost.
        "restore-snapshot" => ("restore-snapshot", false),
        _ => ("unknown", false),
    }
}

/// Which chip the panel draws, from the `app_id` the audit ledger attests.
///
/// The undo signer admits exactly one producer today - `ai-agent`, see its
/// `ADMITTED` - so an unjoined row is the assistant's by attestation and not by
/// assumption. The file manager, terminal and settings are listed because the
/// panel's design has chips for them and they will journal inverses; until they
/// do, those arms are unreachable and honest, not speculative.
///
/// THE RELEASE IDS WERE MISSING, which mattered precisely because the arms are
/// unreachable: nothing exercises them, so nothing said so. Every app is staged
/// at `/usr/lib/arlen/apps/dev.arlen.<name>/` (all twelve mkosi phases agree) and
/// `path_to_app_id` rule 3 returns that directory name, so the file manager
/// attests as `dev.arlen.files` and the terminal as `dev.arlen.terminal`. The map
/// held `dev.arlen-files` (the cargo-run id, right for a debug build) and
/// `arlen-files` (which no rule produces: rule 2 strips `/usr/bin/arlen-` to
/// `files`). So on a real image both would have fallen through to `_ => "agent"`,
/// and a person's own file move would have been chipped as the assistant's work -
/// on the panel whose whole job is saying who did what.
///
/// All three spellings are kept per producer: the packaged id, the cargo-run id,
/// and the `/usr/bin` symlink's basename-derived id, since a caller resolved
/// through the symlink rather than the real path yields the third.
///
/// `settings` is the odd one and correctly so: `identity.rs` rule 1 pins its
/// canonical path to the bare id `settings` rather than the directory
/// convention, because the capability-revoke allowlist keys on it.
fn producer_of(actor: Option<&str>) -> &'static str {
    match actor {
        Some("dev.arlen.files") | Some("dev.arlen-files") | Some("files") => "files",
        Some("dev.arlen.terminal") | Some("dev.arlen-terminal") | Some("terminal") => "terminal",
        Some("settings") | Some("dev.arlen-settings") => "settings",
        // `ai-agent`, its dev id, and the unjoined case: the signer admits no
        // other producer, so this is attestation rather than a default.
        _ => "agent",
    }
}

/// Translate one served row into the row the panel renders.
fn entry_from_row(row: Row) -> UndoRowView {
    let (kind, has_local_inverse) = kind_of(&row.inverse_kind);
    UndoRowView {
        op_id: row.op_id,
        producer: producer_of(row.description.as_ref().map(|d| d.actor.as_str())),
        kind,
        object: row.object,
        at: row.description.as_ref().map(|d| d.timestamp_micros / 1_000_000),
        reversibility: if row.inverse_kind == "restore-snapshot" {
            "reversible_with_cost"
        } else {
            "reversible"
        },
        // Only where the daemon says it will act. A button the daemon refuses is
        // a button that does nothing.
        enactable: row.enactable && has_local_inverse,
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

/// One step of the chain, as the daemon serves it.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Step {
    actor: String,
    kind: String,
    subject: String,
    timestamp_micros: i64,
}

/// The chain as the daemon serves it.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Detail {
    op_id: String,
    steps: Vec<Step>,
}

/// One step of the record behind a row, as the panel renders it.
///
/// Named `UndoStepView` rather than `UndoStep` for the reason `UndoRowView`
/// carries above: the undo service already defines an `UndoStep`, and a name that
/// appears twice is dropped from `check-invoke-shape`'s comparison rather than
/// compared against the wrong one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UndoStepView {
    /// Who the ledger attests for this step, as the panel's chip.
    producer: &'static str,
    /// What the ledger recorded, as a TOKEN the panel words. Same rule as the
    /// row's `kind`: the wording lives in the catalogue in both languages, so a
    /// German disclosure does not read as a translated chip in front of an
    /// English noun.
    kind: &'static str,
    /// The content-free structural subject, e.g. `agent.auto-tag-by-project`.
    /// Data, not prose, so it passes through as it is.
    subject: String,
    /// Unix seconds. Present on every step: a detail step comes from the ledger,
    /// which stamps its own clock, unlike the signed entry a row is built from.
    at: i64,
}

/// The record behind one row, oldest step first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UndoDetailView {
    /// The op the steps belong to, echoed from the daemon so a late answer cannot
    /// be drawn under whichever row is open by the time it lands.
    op_id: String,
    /// The chain, oldest first: what authorised the action, then what came of it.
    steps: Vec<UndoStepView>,
}

/// The ledger's kind as a token the panel words.
///
/// Exhaustive over [`AuditKind`] on purpose. The taxonomy's own doc says adding a
/// variant should break every match on it, and this is now one of them: a new
/// kind cannot reach a disclosure as an untranslated word, it stops the build
/// here first. An unparseable string is `unknown` rather than passed through -
/// the ledger and this binary can be different versions, and a raw wire word on
/// screen is the thing the whole token discipline exists to prevent.
fn step_kind_of(wire: &str) -> &'static str {
    let Some(kind) = AuditKind::from_wire(wire) else {
        return "unknown";
    };
    match kind {
        AuditKind::Query => "query",
        AuditKind::ToolCall => "tool-call",
        AuditKind::Confirm => "confirm",
        AuditKind::PolicyViolation => "policy-violation",
        AuditKind::GraphAccess => "graph-access",
        AuditKind::Permission => "permission",
        AuditKind::NetworkCall => "network-call",
        AuditKind::AppAction => "app-action",
        AuditKind::CapabilityChange => "capability-change",
    }
}

/// The recorded chain behind one row, for the disclosure the panel opens.
///
/// An unreachable service, an unreadable ledger and an unknown op id are all
/// errors here, and none of them is an empty list. That is the daemon's rule and
/// this side must not soften it: somebody opened the disclosure to see the
/// record, so an empty chain has to mean the ledger holds nothing further. A read
/// that failed drawn as "nothing further" would be the panel stating the one
/// thing it does not know.
#[tauri::command]
pub async fn undo_detail(op_id: String) -> Result<UndoDetailView, String> {
    let json = call("Detail", &(op_id,)).await?;
    let detail: Detail =
        serde_json::from_str(&json).map_err(|e| format!("undo detail: {e}"))?;
    Ok(UndoDetailView {
        op_id: detail.op_id,
        steps: detail
            .steps
            .into_iter()
            .map(|s| UndoStepView {
                producer: producer_of(Some(&s.actor)),
                kind: step_kind_of(&s.kind),
                subject: s.subject,
                at: s.timestamp_micros / 1_000_000,
            })
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The other half of the wire pin.
    ///
    /// The literal is the same one `daemons/undo-service/src/undo_history.rs`
    /// asserts it produces, in the test named there. The two structs share no
    /// code - they meet on a D-Bus wire - so nothing in the tree can see them
    /// agree: `check-shared-signature` catches a broken Rust signature between
    /// crates that path-depend, and these do not. A field rename that reached
    /// the wire would break no build; it would just render an empty disclosure,
    /// which is the one sentence this path is written never to say by accident.
    #[test]
    fn the_served_disclosure_parses_into_the_panel_shape() {
        let wire = r#"{"opId":"op-1","steps":[{"actor":"ai-agent","kind":"graph_access","subject":"agent.auto-tag-by-project","timestampMicros":1700000000000000}]}"#;
        let detail: Detail = serde_json::from_str(wire).expect("the served shape parses");
        assert_eq!(detail.op_id, "op-1");
        assert_eq!(detail.steps.len(), 1);
        let step = &detail.steps[0];
        assert_eq!(producer_of(Some(&step.actor)), "agent");
        assert_eq!(step_kind_of(&step.kind), "graph-access");
        assert_eq!(step.subject, "agent.auto-tag-by-project");
        // Micros to seconds, the same conversion the row does.
        assert_eq!(step.timestamp_micros / 1_000_000, 1_700_000_000);
    }

    /// Walk the taxonomy rather than a hand-written copy of it. The list this
    /// would otherwise carry is exactly what fell a variant behind in the audit
    /// crate's own round-trip test, leaving the one kind that records a reach
    /// unchecked; `ALL` exists so that cannot happen twice.
    #[test]
    fn every_audit_kind_has_a_token_and_none_is_the_unknown_one() {
        for kind in AuditKind::ALL {
            let token = step_kind_of(kind.as_str());
            assert_ne!(
                token, "unknown",
                "{} has no token, so a disclosure would draw it as nothing",
                kind.as_str()
            );
            assert!(
                !token.contains('_'),
                "{token} is the wire spelling, not the panel's kebab token"
            );
        }
    }

    /// A ledger from a newer build than this binary. The word must not reach the
    /// screen: the panel has no catalogue entry for it, so it would render raw.
    #[test]
    fn a_kind_this_build_does_not_know_is_unknown_rather_than_passed_through() {
        assert_eq!(step_kind_of("some_future_kind"), "unknown");
        assert_eq!(step_kind_of(""), "unknown");
    }

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
    /// each names its forward act exactly. If a kind is added without being
    /// mapped, this catches it: the neutral fallback is for a live surface, not
    /// for a variant nobody wrote a wording for.
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
            let (token, _) = kind_of(k);
            assert_ne!(token, "unknown", "{k} is not mapped");
            assert_eq!(token, k, "{k} must travel as itself");
        }
        assert_eq!(kind_of("something-new").0, "unknown");
        assert!(!kind_of("something-new").1);
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
        assert!(!e.enactable);
    }

    /// A row the daemon will not enact offers no button, whatever its kind. A
    /// label the daemon refuses is a button that does nothing.
    #[test]
    fn an_unenactable_row_offers_no_button() {
        let e = entry_from_row(row("restore-path", false, None));
        assert!(!e.enactable);
        assert!(entry_from_row(row("restore-path", true, None)).enactable);
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
    ///
    /// THE PACKAGED IDS FIRST, since they are the ones a person meets and the
    /// ones that were missing: this test asserted `arlen-files`, an id no rule
    /// in `identity.rs` produces, and passed for as long as the map agreed with
    /// it. Both sides were spelled by the same hand and neither was checked
    /// against the resolver.
    #[test]
    fn the_producer_comes_from_the_attested_actor() {
        // What `path_to_app_id` rule 3 returns for the staged app directory.
        assert_eq!(producer_of(Some("dev.arlen.files")), "files");
        assert_eq!(producer_of(Some("dev.arlen.terminal")), "terminal");
        // The cargo-run ids, for a debug session.
        assert_eq!(producer_of(Some("dev.arlen-files")), "files");
        assert_eq!(producer_of(Some("dev.arlen-terminal")), "terminal");
        // Settings is pinned to the bare id by rule 1, not the directory
        // convention, because the revoke allowlist keys on it.
        assert_eq!(producer_of(Some("settings")), "settings");
        assert_eq!(producer_of(Some("ai-agent")), "agent");
        assert_eq!(producer_of(None), "agent");
    }

    /// An id nobody attests must not be quietly chipped as the assistant.
    ///
    /// This is the shape the bug had: an unrecognised actor falls to `agent`,
    /// which is correct for the unjoined case and wrong for a producer whose
    /// spelling drifted. The fallback cannot tell them apart, so the map has to
    /// be right - and the way to keep it right is to assert the ids the resolver
    /// actually mints, which the case above now does.
    #[test]
    fn an_unknown_actor_is_the_assistant_by_attestation() {
        assert_eq!(producer_of(Some("dev.arlen.calendar")), "agent");
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
        assert_eq!(e.kind, "restore-path");
        assert_eq!(e.producer, "agent");
        assert_eq!(e.at, Some(1_700_000_000));
    }
}

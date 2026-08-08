// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

//! `org.arlen.Undo1`: the session's undo surface.
//!
//! Two methods, both gated to the user-facing surfaces. `recent` lists what
//! happened and whether each is reversible; `enact` reverses one.
//!
//! **Nothing here asks whether the AI is running**, and that is the reason the
//! interface exists at all. The same three operations were on `org.arlen.AIAgent1`,
//! which is served only when `[ai] enabled` is true, so switching the assistant off
//! in Settings took a user's own file moves and setting changes out of the list and
//! took their undo with them. The records were never the AI's - the signed log and
//! the audit ledger are separate daemons - only the surface was.

use crate::{undo_enact, undo_history};

/// How many entries `recent` returns. The surface shows a recent list, not an
/// archive; the signed log keeps everything either way.
const RECENT_LIMIT: u32 = 50;

/// The object path this interface is served at.
pub const OBJECT_PATH: &str = "/org/arlen/Undo1";
/// The bus name the service owns.
pub const BUS_NAME: &str = "org.arlen.Undo1";

/// The undo surface. Holds no state: both methods read the two stores fresh, so a
/// restart loses nothing and two callers cannot disagree about what is undoable.
pub struct UndoInterface;

/// Resolve the calling app's Arlen identity from the D-Bus connection: the bus
/// attests the sender's PID (`GetConnectionUnixProcessID`, never a client value)
/// and the resolver maps `/proc/<pid>/exe` to an app id. Any failure is an `Err`
/// and the caller is refused.
///
/// Spelled here rather than shared: it is mechanism that either resolves or fails,
/// unlike the ADMISSION LIST, which is policy two daemons must agree on and so
/// lives in `arlen_permissions::identity::is_user_surface`.
async fn resolve_caller(
    header: &zbus::message::Header<'_>,
    connection: &zbus::Connection,
) -> Result<String, String> {
    let sender = header
        .sender()
        .ok_or_else(|| "no sender in message".to_string())?;
    let proxy = zbus::fdo::DBusProxy::new(connection)
        .await
        .map_err(|e| format!("DBusProxy: {e}"))?;
    let pid = proxy
        .get_connection_unix_process_id(sender.clone().into())
        .await
        .map_err(|e| format!("get caller pid: {e}"))?;
    arlen_permissions::identity::app_id_from_pid(pid).map_err(|e| format!("resolve app id: {e}"))
}

/// Whether the caller is a surface allowed to see and reverse the user's actions,
/// logging the refusal so a harness whose identity stopped resolving shows up in
/// the journal rather than silently seeing an empty list.
async fn admitted(
    method: &str,
    header: &zbus::message::Header<'_>,
    connection: &zbus::Connection,
) -> zbus::fdo::Result<()> {
    match resolve_caller(header, connection).await {
        Ok(caller) if arlen_permissions::identity::is_user_surface(&caller) => Ok(()),
        Ok(caller) => {
            tracing::warn!(%caller, %method, "refused: not a user surface");
            Err(zbus::fdo::Error::AccessDenied(format!(
                "{caller} may not read or reverse this session's actions"
            )))
        }
        Err(e) => {
            tracing::warn!(error = %e, %method, "refused: caller unresolved");
            Err(zbus::fdo::Error::AccessDenied(e))
        }
    }
}

#[zbus::interface(name = "org.arlen.Undo1")]
impl UndoInterface {
    /// The recent actions, newest first, as JSON: what happened, when, who did it
    /// and whether it can be reversed.
    ///
    /// A refused caller gets an ERROR, not an empty array. The first version
    /// returned `[]` on the grounds that the wire is JSON the caller parses and
    /// the journal carries the refusal - and then I ran it and read what a user
    /// would see: an empty recent-actions list, which says "you have done nothing"
    /// to someone who has. The journal line helps whoever reads journals. The
    /// panel treats an error as its cue to show its fixture and say so, which is
    /// the honest display.
    async fn recent(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> zbus::fdo::Result<String> {
        admitted("recent", &header, connection).await?;
        let rows = undo_history::recent_rows(
            &arlen_ai_undo_proto::socket_path(),
            &undo_history::LedgerChains::at_default_socket(),
            RECENT_LIMIT,
        )
        .await;
        Ok(serde_json::to_string(&rows).unwrap_or_else(|_| "[]".to_string()))
    }

    /// Reverse the action with this operation id, returning a one-word outcome.
    ///
    /// The inverse comes from the signed log, never from the caller: `op_id` is a
    /// lookup key and nothing more, so a caller cannot describe an undo of its own
    /// devising and have it enacted.
    async fn enact(
        &self,
        op_id: String,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> zbus::fdo::Result<String> {
        // A refusal is an error rather than an outcome word: the outcomes below
        // describe what an undo DID, and refusing to consider one is not among
        // them. Same reason `recent` errors.
        admitted("enact", &header, connection).await?;
        // The signer's protocol has no fetch-one, so the live set is fetched and
        // the entry found by key. The op id is a LOOKUP KEY and nothing else: the
        // inverse that gets replayed is the one the signer holds, so a caller
        // cannot describe an undo of its own devising and have it enacted.
        let socket = arlen_ai_undo_proto::socket_path();
        let entry = match crate::undo_signer::fetch_live(&socket).await {
            Ok(entries) => match entries.into_iter().find(|e| e.op_id == op_id) {
                Some(entry) => entry,
                None => return Ok("no-such-action".to_string()),
            },
            Err(e) => {
                tracing::warn!(error = %e, %op_id, "enact: could not read the undo log");
                return Ok("unavailable".to_string());
            }
        };
        if !undo_enact::is_enactable(&entry.inverse) {
            return Ok("not-reversible".to_string());
        }
        let inverse = entry.inverse.clone();
        match tokio::task::spawn_blocking(move || undo_enact::enact_inverse(&inverse)).await {
            Ok(Ok(outcome)) => Ok(outcome_wire(outcome).to_string()),
            Ok(Err(e)) => {
                tracing::warn!(error = %e, %op_id, "enact failed");
                Ok("failed".to_string())
            }
            Err(e) => {
                tracing::warn!(error = %e, %op_id, "enact panicked");
                Ok("failed".to_string())
            }
        }
    }
}

/// The wire word for an outcome. A small function so the vocabulary a caller sees
/// is stated in one place and can be unit-tested without a bus.
fn outcome_wire(outcome: undo_enact::EnactOutcome) -> &'static str {
    use undo_enact::EnactOutcome;
    match outcome {
        EnactOutcome::Restored => "restored",
        EnactOutcome::Deleted => "deleted",
        EnactOutcome::Trashed => "trashed",
        // The two refusals are their own words, not a generic failure: "we did not
        // undo this, and here is the reason you can act on" is a different thing to
        // tell a user than "it broke".
        EnactOutcome::RefusedIdentityMismatch => "refused-replaced",
        EnactOutcome::RefusedPriorOccupied => "refused-occupied",
        EnactOutcome::NotFilesystem => "not-a-file-change",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The vocabulary is closed and each word is distinct: a surface renders these
    /// verbatim, so two outcomes sharing a word would be two different things
    /// telling the user the same thing.
    #[test]
    fn every_outcome_has_its_own_word() {
        use undo_enact::EnactOutcome::*;
        let words = [
            outcome_wire(Restored),
            outcome_wire(Deleted),
            outcome_wire(Trashed),
            outcome_wire(RefusedIdentityMismatch),
            outcome_wire(RefusedPriorOccupied),
            outcome_wire(NotFilesystem),
        ];
        let mut seen = std::collections::BTreeSet::new();
        for w in words {
            assert!(!w.is_empty());
            assert!(seen.insert(w), "{w} is used twice");
        }
    }
}

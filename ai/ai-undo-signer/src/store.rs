//! The signer's local sealed store: the HMAC-chained undo-log it owns
//! (reversible-receipts-and-the-effect-model.md §6).
//!
//! [`SignerStore`] wires the two halves the signer holds: the key custody
//! ([`crate::key`]) and the chained `FileUndoLog` (from `arlen-ai-undo-core`).
//! Opening it resolves the private state directory, decides whether the log
//! already holds records *before* touching the key (so a chained log whose key
//! is missing fails closed rather than silently re-keying), loads-or-creates the
//! key, then opens and chain-verifies the log. Append and lookup go through this
//! store; the peer-authed socket that fronts it for the agent is a later
//! increment.

use std::path::Path;

use std::path::PathBuf;

use arlen_ai_undo_core::undo_log::{FileUndoLog, UndoEntry, UndoState};

use crate::checkpoint::{self, StartupCheck};
use crate::error::{Result, SignerError};
use crate::{key, paths};

/// The signer-owned, HMAC-chained undo-log plus its custodied key and head
/// checkpoint. Constructed by the signer process; the agent never holds this (it
/// reaches the store only through the signer's socket).
#[derive(Debug)]
pub struct SignerStore {
    log: FileUndoLog,
    checkpoint_path: PathBuf,
}

impl SignerStore {
    /// Open the store under the default private state directory
    /// (`paths::undo_log_dir`).
    pub fn open_default() -> Result<SignerStore> {
        let dir = paths::undo_log_dir()?;
        SignerStore::open_in(&dir)
    }

    /// Open the store under `dir`: ensure the 0700 private directory, decide
    /// whether the log already holds records, load-or-create the chain key, then
    /// open and verify the chained log. Fails closed if the log holds records but
    /// the key is gone (the chain would be unverifiable), or if the chain does
    /// not verify on load.
    pub fn open_in(dir: &Path) -> Result<SignerStore> {
        paths::ensure_private_dir(dir)?;
        let log_path = dir.join("undo.log");
        let key_path = paths::key_path(dir);

        // Whether the log already holds records is read from the file before the
        // key is touched: a non-empty log with a missing key must fail closed
        // (load_or_create refuses to mint a fresh key that would invalidate the
        // chain), never silently re-key.
        let log_has_records = log_has_records(&log_path);
        let key = key::load_or_create(&key_path, log_has_records)?;

        let log = FileUndoLog::open(&log_path, key.to_vec())
            .map_err(|e| SignerError::Storage(format!("opening the chained undo-log: {e}")))?;

        // Head checkpoint: the chain catches in-place tamper but not truncation
        // or whole-log erasure. Confirm the witnessed record is still present
        // with its recorded hash, fail-closed on a tampered/missing witness.
        let checkpoint_path = checkpoint::checkpoint_path(&log_path);
        let cp_read = checkpoint::read(&checkpoint_path);
        let head_at = match &cp_read {
            Ok(Some(cp)) if cp.count >= 1 => log
                .hash_at((cp.count - 1) as usize)
                .map(|h| checkpoint::hex32(&h)),
            _ => None,
        };
        match checkpoint::assess_startup(cp_read, log.record_count() == 0, head_at) {
            StartupCheck::Tampered { detail } => {
                return Err(SignerError::Storage(format!("undo-log integrity: {detail}")))
            }
            StartupCheck::Genesis | StartupCheck::Consistent => {}
        }

        let store = SignerStore { log, checkpoint_path };
        // Reseed the checkpoint to the live head: advances past a crash-ahead
        // append (the log may be one record ahead of the witness) so the witness
        // tracks reality. A no-op for an empty genesis log.
        if store.log.record_count() > 0 {
            store.write_checkpoint()?;
        }
        Ok(store)
    }

    /// Write the head checkpoint to the current record count and head hash.
    fn write_checkpoint(&self) -> Result<()> {
        checkpoint::write(
            &self.checkpoint_path,
            &checkpoint::Checkpoint {
                count: self.log.record_count() as u64,
                head_hex: checkpoint::hex32(&self.log.head_hash()),
            },
        )
    }

    /// Seal a newly-submitted entry into the chained log (its lifecycle begins
    /// `InFlight`). The append is fsynced write-ahead before this returns. A
    /// duplicate create for an existing `op_id` is refused (a second `Created`
    /// record would fold to an illegal sequence and wedge the entry to corrupt),
    /// so a buggy or hostile submitter cannot silently demote a sealed action.
    pub fn submit_created(&mut self, entry: UndoEntry) -> Result<()> {
        if self.log.entry(&entry.op_id).is_some() {
            return Err(SignerError::IllegalRecord(format!(
                "op_id {:?} already has a sealed entry",
                entry.op_id
            )));
        }
        self.log
            .append_created(entry)
            .map_err(|e| SignerError::Storage(format!("appending an undo-log entry: {e}")))?;
        self.write_checkpoint()
    }

    /// Record a lifecycle transition for an existing entry, chained and fsynced.
    /// The transition must be legal from the entry's current folded state (§6):
    /// an unknown `op_id`, an already-corrupt chain, or a forbidden transition is
    /// refused rather than sealed, so the chain can never be driven to a corrupt
    /// fold that the executor would read as non-reversible.
    pub fn transition(&mut self, op_id: &str, state: UndoState) -> Result<()> {
        match self.log.current_state(op_id) {
            None => {
                return Err(SignerError::IllegalRecord(format!(
                    "no sealed entry for op_id {op_id:?} to transition"
                )))
            }
            Some(Err(e)) => {
                return Err(SignerError::IllegalRecord(format!(
                    "op_id {op_id:?} chain is already corrupt: {e}"
                )))
            }
            Some(Ok(current)) if !current.can_transition_to(state) => {
                return Err(SignerError::IllegalRecord(format!(
                    "illegal transition {current:?} -> {state:?} for op_id {op_id:?}"
                )))
            }
            Some(Ok(_)) => {}
        }
        self.log
            .append_transition(op_id, state)
            .map_err(|e| SignerError::Storage(format!("appending an undo-log transition: {e}")))?;
        self.write_checkpoint()
    }

    /// The current folded state of `op_id`, or `None` if no entry with that id was
    /// ever sealed. The inner `Result` is `Err` if the record chain is illegal.
    pub fn state(&self, op_id: &str) -> Option<std::result::Result<UndoState, String>> {
        self.log.current_state(op_id)
    }

    /// The sealed entry for `op_id` (its immutable created data), if any.
    pub fn entry(&self, op_id: &str) -> Option<&UndoEntry> {
        self.log.entry(op_id)
    }
}

/// Whether the undo-log file at `path` already holds at least one record: it
/// exists and is non-empty. An absent or zero-length file has no records (the
/// genesis case), so the key may be minted.
fn log_has_records(path: &Path) -> bool {
    std::fs::metadata(path)
        .map(|m| m.len() > 0)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use arlen_ai_undo_core::effect_model::{CanonicalPath, InverseReceipt};

    fn entry(op: &str) -> UndoEntry {
        UndoEntry {
            op_id: op.to_string(),
            correlation_id: "run".to_string(),
            inverse: InverseReceipt::RestorePath {
                now: CanonicalPath::new("/b/x").unwrap(),
                prior: CanonicalPath::new("/a/x").unwrap(),
            },
        }
    }

    #[test]
    fn genesis_open_creates_key_and_seals_an_entry_across_reopen() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("undo-log");
        {
            let mut store = SignerStore::open_in(&dir).unwrap();
            store.submit_created(entry("op-1")).unwrap();
            store.transition("op-1", UndoState::Committed).unwrap();
            assert_eq!(store.state("op-1").unwrap().unwrap(), UndoState::Committed);
        }
        // A fresh signer process reopens: the key is read back and the chain
        // re-verifies, recovering the folded state.
        let reopened = SignerStore::open_in(&dir).unwrap();
        assert_eq!(reopened.state("op-1").unwrap().unwrap(), UndoState::Committed);
        assert_eq!(reopened.entry("op-1").unwrap().op_id, "op-1");
        assert!(reopened.state("absent").is_none());
    }

    #[test]
    fn a_populated_log_with_a_missing_key_fails_closed() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("undo-log");
        {
            let mut store = SignerStore::open_in(&dir).unwrap();
            store.submit_created(entry("op-1")).unwrap();
        }
        // Delete the key but keep the chained log: reopening must refuse rather
        // than mint a fresh key that would make the chain unverifiable.
        std::fs::remove_file(paths::key_path(&dir)).unwrap();
        match SignerStore::open_in(&dir) {
            Err(SignerError::KeyUnavailable(_)) => {}
            other => panic!("expected KeyUnavailable, got {other:?}"),
        }
    }

    #[test]
    fn a_tampered_log_fails_closed_on_reopen() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("undo-log");
        {
            let mut store = SignerStore::open_in(&dir).unwrap();
            store.submit_created(entry("op-1")).unwrap();
        }
        // Flip a byte in the sealed log: the chain no longer verifies.
        let log_path = dir.join("undo.log");
        let content = std::fs::read_to_string(&log_path).unwrap();
        std::fs::write(&log_path, content.replacen("op-1", "op-X", 1)).unwrap();
        assert!(
            SignerStore::open_in(&dir).is_err(),
            "a tampered sealed log must fail to open"
        );
    }

    #[test]
    fn a_duplicate_create_is_refused_not_sealed() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("undo-log");
        let mut store = SignerStore::open_in(&dir).unwrap();
        store.submit_created(entry("op-1")).unwrap();
        match store.submit_created(entry("op-1")) {
            Err(SignerError::IllegalRecord(_)) => {}
            other => panic!("expected IllegalRecord, got {other:?}"),
        }
        // The original entry is untouched and still folds to a clean InFlight.
        assert_eq!(store.state("op-1").unwrap().unwrap(), UndoState::InFlight);
    }

    #[test]
    fn an_illegal_or_orphan_transition_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("undo-log");
        let mut store = SignerStore::open_in(&dir).unwrap();
        // No entry yet: a transition is an orphan.
        assert!(matches!(
            store.transition("op-1", UndoState::Committed),
            Err(SignerError::IllegalRecord(_))
        ));
        store.submit_created(entry("op-1")).unwrap();
        // InFlight cannot jump straight to Compensated.
        assert!(matches!(
            store.transition("op-1", UndoState::Compensated),
            Err(SignerError::IllegalRecord(_))
        ));
        // A legal transition still works, and the state stays clean (not corrupt).
        store.transition("op-1", UndoState::Committed).unwrap();
        assert_eq!(store.state("op-1").unwrap().unwrap(), UndoState::Committed);
    }

    #[test]
    fn truncating_the_log_is_caught_by_the_head_checkpoint() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("undo-log");
        {
            let mut store = SignerStore::open_in(&dir).unwrap();
            store.submit_created(entry("op-1")).unwrap();
            store.submit_created(entry("op-2")).unwrap();
        }
        // Truncate the log to empty (an attacker erasing history). The chain alone
        // would read this as a clean genesis; the head checkpoint witnesses that 2
        // records existed, so the open fails closed.
        std::fs::write(dir.join("undo.log"), b"").unwrap();
        match SignerStore::open_in(&dir) {
            Err(SignerError::Storage(msg)) => assert!(msg.contains("integrity")),
            other => panic!("expected an integrity failure, got {other:?}"),
        }
    }

    #[test]
    fn removing_only_the_checkpoint_for_a_nonempty_log_fails_closed() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("undo-log");
        {
            let mut store = SignerStore::open_in(&dir).unwrap();
            store.submit_created(entry("op-1")).unwrap();
        }
        // Delete the witness while leaving the log: the missing checkpoint for a
        // non-empty log is tampering, not a fresh genesis.
        std::fs::remove_file(checkpoint::checkpoint_path(&dir.join("undo.log"))).unwrap();
        assert!(SignerStore::open_in(&dir).is_err());
    }

    #[test]
    fn an_empty_genesis_log_does_not_require_a_pre_existing_key() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("undo-log");
        // Two opens with no entries: the first mints the key, the second reads it
        // back (the empty log carries no records, so neither errors).
        SignerStore::open_in(&dir).unwrap();
        SignerStore::open_in(&dir).unwrap();
        assert!(!log_has_records(&dir.join("undo.log")), "empty log");
    }
}

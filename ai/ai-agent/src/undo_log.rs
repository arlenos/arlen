//! The durable undo-log's event-sourced lifecycle state machine
//! (reversible-receipts-and-the-effect-model.md §2, §6).
//!
//! An append-only, HMAC-chained log cannot flip an entry's state in place, so a
//! state transition is a new appended (chained) record, and the current state of
//! an `op_id` is the **fold** of its records on read. This module is the pure
//! state machine: the lifecycle states, their legal transitions, and the fold
//! that reconstructs the current state, rejecting an illegal sequence so a
//! corrupt or forged log fails closed rather than yielding a bogus state.
//!
//! The store itself (the fsync write-ahead append under
//! `~/.local/state/arlen/agent/undo-log/`, the HMAC chain, the access control,
//! retention by undo-window), and the separate-uid signer helper that seals it,
//! are later EM-R1 increments built on this core.

use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The lifecycle of an undo-log entry (§2). An entry is created [`InFlight`] (the
/// provisional record appended and fsynced *before* the externalised act,
/// carrying the captured inverse), then transitions as the act commits, aborts,
/// is compensated, or is superseded.
///
/// [`InFlight`]: UndoState::InFlight
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UndoState {
    /// Provisional: written ahead of the act. A crash leaves the entry here for
    /// the reconciler to resolve against reality (§2).
    InFlight,
    /// The act committed; the entry is undoable within the undo window.
    Committed,
    /// The act did not commit (a definite failure); there is nothing to undo.
    Aborted,
    /// Compensation (undo) has started but is not yet confirmed.
    Compensating,
    /// Compensation completed; the action was undone.
    Compensated,
    /// A user change to the same target superseded the agent's entry
    /// (the user-subsumes-agent rule §2), rather than the agent fighting it.
    Superseded,
}

impl UndoState {
    /// Whether this is a terminal state, from which no further transition is
    /// legal: the entry's lifecycle has ended.
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            UndoState::Aborted | UndoState::Compensated | UndoState::Superseded
        )
    }

    /// Whether a transition from `self` to `next` is legal (§6). A user
    /// supersession may interrupt any non-terminal entry; otherwise the only
    /// paths are `InFlight -> Committed | Aborted`, `Committed -> Compensating`,
    /// and `Compensating -> Compensated`. A terminal state admits nothing.
    pub fn can_transition_to(self, next: UndoState) -> bool {
        if self.is_terminal() {
            return false;
        }
        // User-subsumes-agent: a user change to the target supersedes any
        // non-terminal entry rather than being fought.
        if next == UndoState::Superseded {
            return true;
        }
        matches!(
            (self, next),
            (UndoState::InFlight, UndoState::Committed)
                | (UndoState::InFlight, UndoState::Aborted)
                | (UndoState::Committed, UndoState::Compensating)
                | (UndoState::Compensating, UndoState::Compensated)
        )
    }
}

/// Fold a sequence of an entry's appended lifecycle records into its current
/// state (§6: "the current state of an `op_id` is the fold of its records on
/// read"). The first record must be [`UndoState::InFlight`] (an entry is created
/// provisional); each subsequent record must be a legal transition from the
/// running state. Returns the final state, or `Err` describing the first illegal
/// record, so a corrupt or forged record chain fails closed.
pub fn fold_state(records: &[UndoState]) -> Result<UndoState, String> {
    let mut iter = records.iter().copied();
    let Some(first) = iter.next() else {
        return Err("empty record chain: an entry must begin InFlight".to_string());
    };
    if first != UndoState::InFlight {
        return Err(format!("first record must be InFlight, was {first:?}"));
    }
    let mut current = first;
    for next in iter {
        if !current.can_transition_to(next) {
            return Err(format!("illegal transition {current:?} -> {next:?}"));
        }
        current = next;
    }
    Ok(current)
}

/// The immutable data an entry is created with (§2): its operation identity, the
/// decision it came from, and the captured inverse that undoes it. The lifecycle
/// `state` is NOT stored here (it is the fold of the entry's transition records),
/// so this never changes after the create record is appended. The resolved
/// forward operation (for reconcile) is a later field, added with the reconcile
/// path; the undo only needs the inverse.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UndoEntry {
    /// The durable op id (the entry's key, the reconcile/compensate key).
    pub op_id: String,
    /// The gate decision this action came from.
    pub correlation_id: String,
    /// The captured inverse: replaying it is the undo.
    pub inverse: crate::effect_model::InverseReceipt,
}

/// One appended record in the event-sourced log: either an entry's creation (its
/// immutable data, implicitly `InFlight`) or a lifecycle transition of an
/// existing entry. The current state of an `op_id` is the fold of its records.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum LogRecord {
    /// An entry was created (state begins `InFlight`).
    Created(UndoEntry),
    /// An existing entry transitioned to a new state.
    Transition {
        /// The entry whose state changed.
        op_id: String,
        /// The new state.
        state: UndoState,
    },
}

/// An in-memory event-sourced undo-log: an append-only record sequence whose
/// per-entry state is folded on read (§6). This is the pure store core; the fsync
/// write-ahead, the on-disk HMAC chain, and the separate-uid signer that seals it
/// are later EM-R1 increments that persist exactly this record sequence.
#[derive(Debug, Default)]
pub struct UndoLog {
    records: Vec<LogRecord>,
}

impl UndoLog {
    /// A new, empty log.
    pub fn new() -> Self {
        Self::default()
    }

    /// Append an entry's creation record (state begins `InFlight`). In the durable
    /// store this is the provisional record written and fsynced before the act.
    pub fn append_created(&mut self, entry: UndoEntry) {
        self.records.push(LogRecord::Created(entry));
    }

    /// Append a lifecycle transition for an existing entry.
    pub fn append_transition(&mut self, op_id: &str, state: UndoState) {
        self.records.push(LogRecord::Transition {
            op_id: op_id.to_string(),
            state,
        });
    }

    /// The current state of `op_id`, folded from its records (§6), or `None` if no
    /// entry with that id was ever created. `Err` if the record chain is illegal
    /// (fails closed, never a bogus state).
    pub fn current_state(&self, op_id: &str) -> Option<Result<UndoState, String>> {
        let mut states: Vec<UndoState> = Vec::new();
        let mut created = false;
        for record in &self.records {
            match record {
                LogRecord::Created(entry) if entry.op_id == op_id => {
                    created = true;
                    states.push(UndoState::InFlight);
                }
                LogRecord::Transition { op_id: id, state } if id == op_id => {
                    states.push(*state);
                }
                _ => {}
            }
        }
        if !created {
            return None;
        }
        Some(fold_state(&states))
    }

    /// The created entry for `op_id`, if one was created (its immutable data).
    pub fn entry(&self, op_id: &str) -> Option<&UndoEntry> {
        self.records.iter().find_map(|r| match r {
            LogRecord::Created(entry) if entry.op_id == op_id => Some(entry),
            _ => None,
        })
    }
}

/// A file-backed undo-log: the event-sourced record sequence persisted as JSON
/// lines, appended write-ahead with an `fsync` before the call returns, and
/// folded on read like the in-memory [`UndoLog`] (§6). The provisional record is
/// durable before the externalised act, so a crash never loses an entry the
/// caller believes recorded. This is the persistence mechanism; the on-disk HMAC
/// chain and the separate-uid signer helper that will own this file (so a
/// same-uid agent can neither forge nor read it) are later EM-R1 increments.
#[derive(Debug)]
pub struct FileUndoLog {
    path: PathBuf,
    log: UndoLog,
}

impl FileUndoLog {
    /// Open the log at `path`, creating it durably if absent (the empty file is
    /// created and its parent directory fsynced, so the file's own appends need
    /// only an `fsync` of the file), then load and fold the existing records. A
    /// record that does not parse (corrupt, or a tampered path that fails the
    /// `CanonicalPath` shape check on deserialize) fails the open, fail-closed.
    pub fn open(path: impl Into<PathBuf>) -> std::io::Result<Self> {
        let path = path.into();
        if !path.exists() {
            std::fs::OpenOptions::new().create(true).append(true).open(&path)?;
            // fsync the directory so the new file's entry survives a crash; the
            // file's later appends then only fsync the file itself.
            if let Some(parent) = path.parent() {
                if let Ok(dir) = std::fs::File::open(parent) {
                    let _ = dir.sync_all();
                }
            }
        }
        let log = Self::load(&path)?;
        Ok(Self { path, log })
    }

    fn load(path: &Path) -> std::io::Result<UndoLog> {
        let content = std::fs::read_to_string(path)?;
        let mut log = UndoLog::new();
        for line in content.lines().filter(|l| !l.trim().is_empty()) {
            let record: LogRecord = serde_json::from_str(line)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
            log.records.push(record);
        }
        Ok(log)
    }

    fn append_record(&mut self, record: LogRecord) -> std::io::Result<()> {
        let line = serde_json::to_string(&record)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        let mut f = std::fs::OpenOptions::new().append(true).open(&self.path)?;
        writeln!(f, "{line}")?;
        // Write-ahead: the record is durable before the call returns.
        f.sync_all()?;
        self.log.records.push(record);
        Ok(())
    }

    /// Append an entry's creation record, durably (state begins `InFlight`).
    pub fn append_created(&mut self, entry: UndoEntry) -> std::io::Result<()> {
        self.append_record(LogRecord::Created(entry))
    }

    /// Append a lifecycle transition for an existing entry, durably.
    pub fn append_transition(&mut self, op_id: &str, state: UndoState) -> std::io::Result<()> {
        self.append_record(LogRecord::Transition {
            op_id: op_id.to_string(),
            state,
        })
    }

    /// The current state of `op_id`, folded from its records (§6).
    pub fn current_state(&self, op_id: &str) -> Option<Result<UndoState, String>> {
        self.log.current_state(op_id)
    }

    /// The created entry for `op_id`, if one was created.
    pub fn entry(&self, op_id: &str) -> Option<&UndoEntry> {
        self.log.entry(op_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use UndoState::*;

    #[test]
    fn a_committed_then_compensated_chain_folds_to_compensated() {
        assert_eq!(
            fold_state(&[InFlight, Committed, Compensating, Compensated]).unwrap(),
            Compensated
        );
    }

    #[test]
    fn an_aborted_chain_folds_to_aborted() {
        assert_eq!(fold_state(&[InFlight, Aborted]).unwrap(), Aborted);
    }

    #[test]
    fn a_supersession_can_interrupt_any_non_terminal_state() {
        assert_eq!(fold_state(&[InFlight, Superseded]).unwrap(), Superseded);
        assert_eq!(fold_state(&[InFlight, Committed, Superseded]).unwrap(), Superseded);
        assert_eq!(
            fold_state(&[InFlight, Committed, Compensating, Superseded]).unwrap(),
            Superseded
        );
    }

    #[test]
    fn an_empty_or_non_inflight_start_is_rejected() {
        assert!(fold_state(&[]).is_err());
        assert!(fold_state(&[Committed]).is_err(), "an entry must begin InFlight");
    }

    #[test]
    fn an_illegal_transition_fails_closed() {
        // InFlight cannot jump straight to Compensated (no commit, no compensate).
        assert!(fold_state(&[InFlight, Compensated]).is_err());
        // Committed cannot go back to InFlight.
        assert!(fold_state(&[InFlight, Committed, InFlight]).is_err());
        // A terminal state admits nothing further.
        assert!(fold_state(&[InFlight, Aborted, Committed]).is_err());
        assert!(fold_state(&[InFlight, Superseded, Committed]).is_err());
    }

    #[test]
    fn terminal_states_admit_no_transition() {
        for terminal in [Aborted, Compensated, Superseded] {
            assert!(terminal.is_terminal());
            for next in [InFlight, Committed, Aborted, Compensating, Compensated, Superseded] {
                assert!(!terminal.can_transition_to(next), "{terminal:?} -> {next:?} must be illegal");
            }
        }
    }

    fn entry(op: &str) -> UndoEntry {
        use crate::effect_model::{CanonicalPath, InverseReceipt};
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
    fn store_folds_an_entrys_current_state_on_read() {
        let mut log = UndoLog::new();
        log.append_created(entry("op-1"));
        assert_eq!(log.current_state("op-1").unwrap().unwrap(), InFlight);
        log.append_transition("op-1", Committed);
        assert_eq!(log.current_state("op-1").unwrap().unwrap(), Committed);
        log.append_transition("op-1", Compensating);
        log.append_transition("op-1", Compensated);
        assert_eq!(log.current_state("op-1").unwrap().unwrap(), Compensated);
    }

    #[test]
    fn store_isolates_entries_by_op_id() {
        let mut log = UndoLog::new();
        log.append_created(entry("a"));
        log.append_created(entry("b"));
        log.append_transition("a", Committed);
        assert_eq!(log.current_state("a").unwrap().unwrap(), Committed);
        assert_eq!(log.current_state("b").unwrap().unwrap(), InFlight, "b unaffected by a");
        assert!(log.current_state("c").is_none(), "an uncreated op id has no entry");
    }

    #[test]
    fn store_surfaces_an_illegal_chain_as_err() {
        let mut log = UndoLog::new();
        log.append_created(entry("op-1"));
        log.append_transition("op-1", Compensated); // illegal: InFlight -> Compensated
        assert!(log.current_state("op-1").unwrap().is_err());
    }

    #[test]
    fn store_returns_the_created_entry_data() {
        let mut log = UndoLog::new();
        log.append_created(entry("op-1"));
        assert_eq!(log.entry("op-1").unwrap().correlation_id, "run");
        assert!(log.entry("absent").is_none());
    }

    #[test]
    fn file_store_persists_and_folds_across_reopen() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("undo.log");
        {
            let mut log = FileUndoLog::open(&path).unwrap();
            log.append_created(entry("op-1")).unwrap();
            log.append_transition("op-1", Committed).unwrap();
            assert_eq!(log.current_state("op-1").unwrap().unwrap(), Committed);
        }
        // A fresh process re-opening the file recovers the same folded state.
        let reopened = FileUndoLog::open(&path).unwrap();
        assert_eq!(reopened.current_state("op-1").unwrap().unwrap(), Committed);
        assert_eq!(reopened.entry("op-1").unwrap().op_id, "op-1");
    }

    #[test]
    fn file_store_opens_a_fresh_path_empty() {
        let tmp = tempfile::TempDir::new().unwrap();
        let log = FileUndoLog::open(tmp.path().join("new.log")).unwrap();
        assert!(log.current_state("anything").is_none());
    }

    #[test]
    fn file_store_fails_closed_on_a_corrupt_or_tampered_record() {
        let tmp = tempfile::TempDir::new().unwrap();
        // A non-JSON line.
        let bad = tmp.path().join("corrupt.log");
        std::fs::write(&bad, "not json\n").unwrap();
        assert!(FileUndoLog::open(&bad).is_err());
        // A well-formed record whose CanonicalPath was tampered to a traversal
        // path must fail the load (the validating Deserialize rejects it).
        let tampered = tmp.path().join("tampered.log");
        std::fs::write(
            &tampered,
            "{\"Created\":{\"op_id\":\"o\",\"correlation_id\":\"r\",\"inverse\":{\"RestorePath\":{\"now\":\"/a/x\",\"prior\":\"/a/../etc\"}}}}\n",
        )
        .unwrap();
        assert!(FileUndoLog::open(&tampered).is_err(), "a tampered traversal path must fail the load");
    }
}

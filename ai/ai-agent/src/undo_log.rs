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

/// One persisted line: a record paired with its HMAC chain hash over the running
/// head (`HMAC(key, prev_hash || record_bytes)`, §6). The `record_bytes` chained
/// are the canonical `serde_json` encoding of `record`, so re-serializing a loaded
/// record reproduces the chained input and the chain re-verifies on load.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ChainedLine {
    record: LogRecord,
    hash: [u8; 32],
}

/// A file-backed undo-log: the event-sourced record sequence persisted as JSON
/// lines, each carrying its HMAC chain hash, appended write-ahead with an `fsync`
/// before the call returns, and folded on read like the in-memory [`UndoLog`]
/// (§6). The provisional record is durable before the externalised act, so a
/// crash never loses an entry the caller believes recorded; the chain makes a
/// tamper, reorder, or mid-log removal evident on the next load (fail-closed).
///
/// The HMAC key is a constructor parameter, not custodied here: this is the
/// persistence the separate-uid signer helper owns. A same-uid agent that held
/// the key could still forge the chain, so integrity *against the agent itself*
/// is the signer's job (different uid, key the agent never sees); against a
/// keyless same-uid process or accidental corruption the chain already bites.
#[derive(Debug)]
pub struct FileUndoLog {
    path: PathBuf,
    key: Vec<u8>,
    head: [u8; 32],
    log: UndoLog,
}

impl FileUndoLog {
    /// Open the log at `path` with chain `key`, creating it durably if absent (the
    /// empty file is created and its parent directory fsynced, so the file's own
    /// appends need only an `fsync` of the file), then load, verify the chain, and
    /// fold the records. A record that does not parse (corrupt, or a tampered path
    /// that fails the `CanonicalPath` shape check on deserialize) or a broken chain
    /// link fails the open, fail-closed.
    pub fn open(path: impl Into<PathBuf>, key: impl Into<Vec<u8>>) -> std::io::Result<Self> {
        let path = path.into();
        let key = key.into();
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
        let (log, head) = Self::load(&path, &key)?;
        Ok(Self {
            path,
            key,
            head,
            log,
        })
    }

    fn load(path: &Path, key: &[u8]) -> std::io::Result<(UndoLog, [u8; 32])> {
        let content = std::fs::read_to_string(path)?;
        let mut log = UndoLog::new();
        let mut head = GENESIS_PREV_HASH;
        for (i, line) in content
            .lines()
            .filter(|l| !l.trim().is_empty())
            .enumerate()
        {
            let chained: ChainedLine = serde_json::from_str(line)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
            let record_bytes = serde_json::to_vec(&chained.record)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
            let expected = compute_chain_hash(key, &head, &record_bytes);
            if expected != chained.hash {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("broken undo-log chain at record {i}"),
                ));
            }
            head = chained.hash;
            log.records.push(chained.record);
        }
        Ok((log, head))
    }

    fn append_record(&mut self, record: LogRecord) -> std::io::Result<()> {
        let record_bytes = serde_json::to_vec(&record)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        let hash = compute_chain_hash(&self.key, &self.head, &record_bytes);
        let chained = ChainedLine {
            record: record.clone(),
            hash,
        };
        let line = serde_json::to_string(&chained)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        let mut f = std::fs::OpenOptions::new().append(true).open(&self.path)?;
        writeln!(f, "{line}")?;
        // Write-ahead: the record is durable before the call returns.
        f.sync_all()?;
        self.head = hash;
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

/// The genesis previous-hash: 32 zero bytes, the chain's index-0 anchor (the same
/// scheme as the audit ledger).
pub const GENESIS_PREV_HASH: [u8; 32] = [0u8; 32];

/// Compute a record's HMAC chain hash: `HMAC-SHA256(key, prev_hash || record_bytes)`
/// (§6). Folding the previous hash into each record means any tamper, reorder, or
/// removal changes every subsequent hash, so the chain is tamper- and (with an
/// external head checkpoint) truncation-evident.
///
/// The HMAC key is the **separate-uid signer helper's**: a same-uid agent holds
/// no key it could not also forge, so integrity *against the agent itself*
/// requires the signer to be a different uid. This is the pure primitive that
/// signer uses; it is not the key custody.
pub fn compute_chain_hash(key: &[u8], prev_hash: &[u8; 32], record_bytes: &[u8]) -> [u8; 32] {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    let mut mac =
        Hmac::<Sha256>::new_from_slice(key).expect("HMAC accepts a key of any length");
    mac.update(prev_hash);
    mac.update(record_bytes);
    let out = mac.finalize().into_bytes();
    let mut hash = [0u8; 32];
    hash.copy_from_slice(&out);
    hash
}

/// Verify an HMAC chain of `(record_bytes, stored_hash)` pairs against `key`,
/// returning the index of the first broken link or `None` if the whole chain
/// verifies. A tampered record, a tampered hash, or a reordering breaks the link
/// at or after the change; a removed middle record breaks the next link. A
/// removed *tail* is NOT caught here (the remaining prefix still verifies) and is
/// the job of an external head checkpoint, as in the audit ledger.
pub fn verify_chain(key: &[u8], records: &[(Vec<u8>, [u8; 32])]) -> Option<usize> {
    let mut prev = GENESIS_PREV_HASH;
    for (i, (bytes, stored)) in records.iter().enumerate() {
        if compute_chain_hash(key, &prev, bytes) != *stored {
            return Some(i);
        }
        prev = *stored;
    }
    None
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

    const TEST_KEY: &[u8] = b"undo-log-test-key";

    #[test]
    fn file_store_persists_and_folds_across_reopen() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("undo.log");
        {
            let mut log = FileUndoLog::open(&path, TEST_KEY).unwrap();
            log.append_created(entry("op-1")).unwrap();
            log.append_transition("op-1", Committed).unwrap();
            assert_eq!(log.current_state("op-1").unwrap().unwrap(), Committed);
        }
        // A fresh process re-opening the file recovers the same folded state and
        // re-verifies the on-disk chain.
        let reopened = FileUndoLog::open(&path, TEST_KEY).unwrap();
        assert_eq!(reopened.current_state("op-1").unwrap().unwrap(), Committed);
        assert_eq!(reopened.entry("op-1").unwrap().op_id, "op-1");
    }

    #[test]
    fn file_store_opens_a_fresh_path_empty() {
        let tmp = tempfile::TempDir::new().unwrap();
        let log = FileUndoLog::open(tmp.path().join("new.log"), TEST_KEY).unwrap();
        assert!(log.current_state("anything").is_none());
    }

    #[test]
    fn file_store_fails_closed_on_a_corrupt_or_tampered_record() {
        let tmp = tempfile::TempDir::new().unwrap();
        // A non-JSON line.
        let bad = tmp.path().join("corrupt.log");
        std::fs::write(&bad, "not json\n").unwrap();
        assert!(FileUndoLog::open(&bad, TEST_KEY).is_err());
        // A well-formed envelope whose inner CanonicalPath is a traversal path
        // must fail the load (the validating Deserialize rejects it before the
        // chain is even checked).
        let tampered = tmp.path().join("tampered.log");
        std::fs::write(
            &tampered,
            "{\"record\":{\"Created\":{\"op_id\":\"o\",\"correlation_id\":\"r\",\"inverse\":{\"RestorePath\":{\"now\":\"/a/x\",\"prior\":\"/a/../etc\"}}}},\"hash\":[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]}\n",
        )
        .unwrap();
        assert!(FileUndoLog::open(&tampered, TEST_KEY).is_err(), "a tampered traversal path must fail the load");
    }

    #[test]
    fn file_store_fails_closed_when_a_persisted_record_is_tampered() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("undo.log");
        {
            let mut log = FileUndoLog::open(&path, TEST_KEY).unwrap();
            log.append_created(entry("op-1")).unwrap();
            log.append_transition("op-1", Committed).unwrap();
        }
        // Flip the op id in the first line: the record bytes no longer match the
        // stored chain hash, so the load detects the break and fails closed.
        let content = std::fs::read_to_string(&path).unwrap();
        let tampered = content.replacen("op-1", "op-X", 1);
        assert_ne!(content, tampered);
        std::fs::write(&path, tampered).unwrap();
        assert!(
            FileUndoLog::open(&path, TEST_KEY).is_err(),
            "a tampered persisted record must break the chain on load"
        );
    }

    #[test]
    fn file_store_fails_closed_under_a_wrong_key() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("undo.log");
        {
            let mut log = FileUndoLog::open(&path, TEST_KEY).unwrap();
            log.append_created(entry("op-1")).unwrap();
        }
        assert!(
            FileUndoLog::open(&path, b"different-key".as_slice()).is_err(),
            "the chain must not verify under a different key"
        );
    }

    // Build a chain of `(record_bytes, hash)` pairs from a sequence of byte
    // payloads, hashing each over the running previous hash (genesis-anchored).
    fn build_chain(key: &[u8], payloads: &[&[u8]]) -> Vec<(Vec<u8>, [u8; 32])> {
        let mut prev = GENESIS_PREV_HASH;
        let mut out = Vec::new();
        for p in payloads {
            let hash = compute_chain_hash(key, &prev, p);
            out.push((p.to_vec(), hash));
            prev = hash;
        }
        out
    }

    #[test]
    fn a_well_formed_chain_verifies() {
        let key = b"signer-key";
        let chain = build_chain(key, &[b"a", b"b", b"c"]);
        assert_eq!(verify_chain(key, &chain), None);
    }

    #[test]
    fn an_empty_chain_verifies() {
        assert_eq!(verify_chain(b"k", &[]), None);
    }

    #[test]
    fn a_tampered_record_breaks_its_link() {
        let key = b"signer-key";
        let mut chain = build_chain(key, &[b"a", b"b", b"c"]);
        // Tamper the payload of the middle record without re-hashing.
        chain[1].0 = b"B".to_vec();
        assert_eq!(verify_chain(key, &chain), Some(1));
    }

    #[test]
    fn a_tampered_hash_breaks_its_link() {
        let key = b"signer-key";
        let mut chain = build_chain(key, &[b"a", b"b", b"c"]);
        chain[2].1[0] ^= 0xff;
        assert_eq!(verify_chain(key, &chain), Some(2));
    }

    #[test]
    fn a_reorder_breaks_the_chain() {
        let key = b"signer-key";
        let mut chain = build_chain(key, &[b"a", b"b", b"c"]);
        chain.swap(0, 1);
        // The first record no longer hashes from genesis with its stored hash.
        assert_eq!(verify_chain(key, &chain), Some(0));
    }

    #[test]
    fn a_removed_middle_record_breaks_the_next_link() {
        let key = b"signer-key";
        let mut chain = build_chain(key, &[b"a", b"b", b"c"]);
        chain.remove(1); // the record that was index 2 now expects b"b"'s hash as prev
        assert_eq!(verify_chain(key, &chain), Some(1));
    }

    #[test]
    fn a_wrong_key_breaks_the_chain_at_the_head() {
        let chain = build_chain(b"signer-key", &[b"a", b"b"]);
        assert_eq!(verify_chain(b"other-key", &chain), Some(0));
    }

    #[test]
    fn a_truncated_tail_is_not_caught_by_the_chain_alone() {
        // Documented limitation: removing the tail leaves a verifying prefix.
        // Truncation evidence is the external head checkpoint's job.
        let key = b"signer-key";
        let mut chain = build_chain(key, &[b"a", b"b", b"c"]);
        chain.pop();
        assert_eq!(verify_chain(key, &chain), None);
    }
}

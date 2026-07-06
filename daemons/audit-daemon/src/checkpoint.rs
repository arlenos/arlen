//! Head checkpoint: truncation evidence outside the ledger rows.
//!
//! The HMAC hash chain makes edits, insertions, and reordering
//! tamper-evident — every present row is re-hashed and re-linked at
//! startup. What a chain cannot detect on its own is **truncation**:
//! deleting the newest rows, or the whole database file, leaves a
//! shorter-but-internally-consistent ledger (or a clean genesis), so
//! `verify()` succeeds and the loss is silent. An attacker needs no
//! HMAC key to `rm` a file.
//!
//! The checkpoint closes the common case. After every append the
//! daemon records the head — the latest committed `index` and its
//! `entry_hash` — to a small file *outside* the SQLite database. At
//! startup the **invariant checked is simple and strong**: the entry
//! the checkpoint points at must still be present in the ledger with
//! the same hash. Truncation deletes that entry (or the whole file);
//! tampering with the checkpoint changes the hash; either way the
//! invariant breaks and the daemon treats it exactly like a chain
//! break — `audit.tampered`, fail-closed ingest.
//!
//! Honest scope: the checkpoint file is itself owned by the same user
//! as the ledger, so a determined same-uid attacker who rewrites
//! *both* the database and the checkpoint consistently is not caught
//! here — that is the same trust boundary as the HMAC key on disk
//! (foundation §8.4 hardening: a TPM-sealed monotonic counter is the
//! robust closer, the file level is not). What the checkpoint does
//! buy is evidence for the cases that actually happen most: a naive
//! `rm` of the database, a partial restore, accidental corruption,
//! and any truncation by something that does not also rewrite the
//! checkpoint. It converts silent total loss into a loud alarm.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{AuditError, Result};

/// The durable record of the ledger head, stored beside the database.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Checkpoint {
    /// Index of the latest committed entry at the time of writing.
    pub index: u64,
    /// Hex-encoded `entry_hash` of that entry — lets startup confirm
    /// the checkpointed entry is still present and unchanged, not
    /// just that the count is right.
    pub entry_hash_hex: String,
    /// The TPM NV monotonic counter value at the time this checkpoint was
    /// sealed (`tpm_anchor`). `0` means no anchor was active (the default and
    /// the pre-anchor on-disk form; `#[serde(default)]` reads old checkpoints as
    /// `0`). At restart the live counter is compared against this so a same-uid
    /// truncate-and-rewrite of the log + checkpoint leaves a detectable gap.
    #[serde(default)]
    pub counter: u64,
}

/// The startup integrity decision for the checkpoint witness.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartupCheck {
    /// The checkpointed entry is present in the ledger with a matching
    /// hash. The caller reseeds the checkpoint to the live head, which
    /// also advances past a clean crash-ahead entry.
    Consistent,
    /// An empty ledger with no checkpoint: genesis. Nothing to protect
    /// yet; the first append writes the checkpoint.
    Genesis,
    /// Tampering: the checkpointed entry is gone (truncation), its hash
    /// no longer matches (checkpoint or entry mutated), a non-empty
    /// ledger has no checkpoint (witness removed), or the checkpoint
    /// file is present but unreadable. The caller freezes ingest and
    /// emits `audit.tampered`.
    Tampered {
        /// Reason, for the log and the `audit.tampered` payload.
        detail: String,
    },
}

/// Resolve the checkpoint path beside a ledger database.
pub fn checkpoint_path(db_path: &Path) -> PathBuf {
    // Same directory as the ledger (already mode 0700); a sibling
    // file keeps it in the per-user audit data dir.
    let dir = db_path.parent().unwrap_or_else(|| Path::new("."));
    dir.join("head.checkpoint")
}

/// Read the checkpoint, if present. A present-but-unparseable file
/// (a torn write should be impossible given the atomic rename below,
/// but out-of-band corruption is not) is an error, never silently
/// read as "no checkpoint" — the caller treats that as tampering.
pub fn read(path: &Path) -> Result<Option<Checkpoint>> {
    match std::fs::read(path) {
        Ok(bytes) => {
            let cp: Checkpoint = serde_json::from_slice(&bytes).map_err(|e| {
                AuditError::Storage(format!(
                    "checkpoint at {} is corrupt: {e}",
                    path.display()
                ))
            })?;
            Ok(Some(cp))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(AuditError::Storage(format!(
            "read checkpoint {}: {e}",
            path.display()
        ))),
    }
}

/// Write the checkpoint durably: write a temp file, fsync it, rename
/// over the target (atomic), then fsync the directory so the rename
/// itself survives a crash. Mode 0600. Directory-fsync failures are
/// propagated, not ignored: a non-durable checkpoint is no witness.
pub fn write(path: &Path, checkpoint: &Checkpoint) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;

    let dir = path.parent().ok_or_else(|| {
        AuditError::Storage(format!("checkpoint path {} has no parent", path.display()))
    })?;
    let tmp = path.with_extension("checkpoint.tmp");

    let body = serde_json::to_vec(checkpoint)
        .map_err(|e| AuditError::Storage(format!("encode checkpoint: {e}")))?;

    {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)
            .map_err(|e| AuditError::Storage(format!("open checkpoint tmp: {e}")))?;
        f.write_all(&body)
            .map_err(|e| AuditError::Storage(format!("write checkpoint tmp: {e}")))?;
        f.sync_all()
            .map_err(|e| AuditError::Storage(format!("fsync checkpoint tmp: {e}")))?;
    }

    std::fs::rename(&tmp, path)
        .map_err(|e| AuditError::Storage(format!("rename checkpoint: {e}")))?;

    let dir_file = std::fs::File::open(dir).map_err(|e| {
        AuditError::Storage(format!("open checkpoint dir {} for fsync: {e}", dir.display()))
    })?;
    dir_file
        .sync_all()
        .map_err(|e| AuditError::Storage(format!("fsync checkpoint dir: {e}")))?;
    Ok(())
}

/// Decide the startup integrity verdict.
///
/// * `read` — the [`read`] result for the checkpoint file.
/// * `ledger_empty` — whether the ledger has zero entries.
/// * `entry_hash_at_checkpoint` — the ledger's `entry_hash_hex` at the
///   checkpoint's `index`, or `None` if no entry exists there (looked
///   up only when a checkpoint is present).
///
/// The core invariant: a present checkpoint is consistent **iff** the
/// entry it points at is still in the ledger with the same hash.
/// Anything else with real data behind it is tampering. A read error
/// (the file exists but is unreadable) is always tampering — the
/// atomic writer cannot produce a malformed file, and an empty ledger
/// beside one may be the very deletion the witness should catch. The
/// only `Genesis` is a genuine first run: no file and an empty ledger.
pub fn assess_startup(
    read: Result<Option<Checkpoint>>,
    ledger_empty: bool,
    entry_hash_at_checkpoint: Option<String>,
) -> StartupCheck {
    match read {
        Ok(None) => {
            if ledger_empty {
                StartupCheck::Genesis
            } else {
                StartupCheck::Tampered {
                    detail: "head checkpoint missing for a non-empty ledger \
                             (the truncation witness was removed)"
                        .to_string(),
                }
            }
        }
        Ok(Some(cp)) => match entry_hash_at_checkpoint {
            None => StartupCheck::Tampered {
                detail: format!(
                    "ledger has no entry at checkpoint index {} (truncated)",
                    cp.index
                ),
            },
            Some(hash) if hash != cp.entry_hash_hex => StartupCheck::Tampered {
                detail: format!(
                    "ledger entry hash at checkpoint index {} does not match the \
                     checkpoint (checkpoint or entry mutated)",
                    cp.index
                ),
            },
            Some(_) => StartupCheck::Consistent,
        },
        Err(e) => StartupCheck::Tampered {
            detail: format!("head checkpoint present but unreadable: {e}"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cp(index: u64, hash: &str) -> Checkpoint {
        Checkpoint {
            index,
            entry_hash_hex: hash.to_string(),
            counter: 0,
        }
    }

    #[test]
    fn write_then_read_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = checkpoint_path(&dir.path().join("ledger.db"));
        let c = cp(7, "abcd");
        write(&path, &c).unwrap();
        assert_eq!(read(&path).unwrap(), Some(c));
    }

    #[test]
    fn absent_checkpoint_reads_as_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = checkpoint_path(&dir.path().join("ledger.db"));
        assert_eq!(read(&path).unwrap(), None);
    }

    #[test]
    fn corrupt_checkpoint_is_an_error_not_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = checkpoint_path(&dir.path().join("ledger.db"));
        std::fs::write(&path, b"not json").unwrap();
        assert!(read(&path).is_err());
    }

    #[test]
    fn genesis_is_empty_ledger_with_no_checkpoint() {
        assert_eq!(assess_startup(Ok(None), true, None), StartupCheck::Genesis);
    }

    #[test]
    fn missing_checkpoint_for_nonempty_ledger_is_tampered() {
        assert!(matches!(
            assess_startup(Ok(None), false, None),
            StartupCheck::Tampered { .. }
        ));
    }

    #[test]
    fn present_checkpointed_entry_with_matching_hash_is_consistent() {
        // The ledger may be exactly at, or ahead of, the checkpoint —
        // either way the checkpointed entry is intact, so it is
        // consistent. The hash, not the relative height, is the test.
        assert_eq!(
            assess_startup(Ok(Some(cp(3, "h3"))), false, Some("h3".into())),
            StartupCheck::Consistent
        );
    }

    #[test]
    fn missing_checkpointed_entry_is_truncation() {
        // No entry at the checkpoint index: the ledger was truncated
        // below it (or the whole database was replaced).
        assert!(matches!(
            assess_startup(Ok(Some(cp(5, "h5"))), false, None),
            StartupCheck::Tampered { .. }
        ));
    }

    #[test]
    fn checkpointed_entry_hash_mismatch_is_tampered_even_when_ahead() {
        // The ledger is ahead of the checkpoint but the entry at the
        // checkpoint index has a different hash than recorded: the
        // checkpoint was mutated out-of-band. Must alarm, not be
        // silently reseeded.
        assert!(matches!(
            assess_startup(Ok(Some(cp(3, "expected"))), false, Some("different".into())),
            StartupCheck::Tampered { .. }
        ));
    }

    #[test]
    fn corrupt_checkpoint_is_tampered_regardless_of_ledger() {
        // A corrupt checkpoint cannot arise from normal operation, so
        // its presence is tampering even beside an empty ledger — the
        // empty ledger may be the deletion the witness should catch.
        let corrupt = || Err(AuditError::Storage("corrupt".into()));
        assert!(matches!(
            assess_startup(corrupt(), true, None),
            StartupCheck::Tampered { .. }
        ));
        assert!(matches!(
            assess_startup(corrupt(), false, None),
            StartupCheck::Tampered { .. }
        ));
    }
}

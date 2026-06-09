//! Head checkpoint: truncation evidence outside the undo-log file
//! (reversible-receipts-and-the-effect-model.md §6; the audit-daemon
//! `checkpoint.rs` precedent).
//!
//! The HMAC chain in `arlen-ai-undo-core` makes edits, insertions, and
//! reordering tamper-evident: every record is re-hashed and re-linked on load.
//! What a chain cannot detect on its own is **truncation**: deleting the newest
//! records, or zeroing/removing the whole log file, leaves a shorter-but-
//! internally-consistent chain (or a clean genesis), so the load succeeds and the
//! loss is silent. An attacker needs no key to `rm` a file.
//!
//! The checkpoint closes the common case. After every append the signer records
//! the head, the record `count` and the head chain-hash, to a small file beside
//! the log. On open the invariant is simple and strong: the record the checkpoint
//! witnessed (the one at index `count - 1`) must still be present with the same
//! hash. Truncation removes it; mutation changes its hash; either way the
//! invariant breaks and the signer treats it like a chain break (fail closed).
//!
//! Honest scope: the checkpoint file is owned by the same uid as the log, so a
//! same-uid attacker who rewrites *both* the log and the checkpoint consistently
//! is not caught here (the acknowledged F3 residual; the robust closer is a
//! TPM-sealed monotonic counter). What it buys is evidence for the cases that
//! actually happen: a naive `rm`/truncate of the log, a partial restore,
//! accidental corruption, and any truncation by something that does not also
//! rewrite the checkpoint. It converts silent total loss into a loud failure.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Result, SignerError};

/// The durable record of the undo-log head, stored beside the log file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Checkpoint {
    /// The chain record count at the time of writing.
    pub count: u64,
    /// Hex of the head chain-hash (the hash of the record at index `count - 1`),
    /// so the open path confirms that record is still present and unchanged, not
    /// merely that the count is right.
    pub head_hex: String,
}

/// The startup integrity verdict for the checkpoint witness.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartupCheck {
    /// The witnessed record is present with a matching hash (the log may be exactly
    /// at, or a crash-ahead append past, the checkpoint). The caller reseeds the
    /// checkpoint to the live head.
    Consistent,
    /// An empty log with no checkpoint: genesis. The first append writes one.
    Genesis,
    /// Tampering: the witnessed record is gone (truncation), its hash no longer
    /// matches (checkpoint or log mutated), a non-empty log has no checkpoint (the
    /// witness was removed), or the checkpoint is present but unreadable. The
    /// caller refuses to open.
    Tampered {
        /// Reason, for the error and the log.
        detail: String,
    },
}

/// Hex-encode a 32-byte chain hash.
pub fn hex32(hash: &[u8; 32]) -> String {
    let mut s = String::with_capacity(64);
    for b in hash {
        use std::fmt::Write;
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// Resolve the checkpoint path beside the undo-log file.
pub fn checkpoint_path(log_path: &Path) -> PathBuf {
    let dir = log_path.parent().unwrap_or_else(|| Path::new("."));
    dir.join("head.checkpoint")
}

/// Read the checkpoint, if present. A present-but-unparseable file is an error,
/// never silently read as "no checkpoint": the atomic writer below cannot produce
/// a malformed file, so a corrupt one is out-of-band tampering.
pub fn read(path: &Path) -> Result<Option<Checkpoint>> {
    match std::fs::read(path) {
        Ok(bytes) => {
            let cp: Checkpoint = serde_json::from_slice(&bytes).map_err(|e| {
                SignerError::Storage(format!("checkpoint at {} is corrupt: {e}", path.display()))
            })?;
            Ok(Some(cp))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(SignerError::Storage(format!(
            "read checkpoint {}: {e}",
            path.display()
        ))),
    }
}

/// Write the checkpoint durably: temp file, fsync, atomic rename over the target,
/// then fsync the directory so the rename survives a crash. Mode 0600.
pub fn write(path: &Path, checkpoint: &Checkpoint) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;

    let dir = path.parent().ok_or_else(|| {
        SignerError::Storage(format!("checkpoint path {} has no parent", path.display()))
    })?;
    let tmp = path.with_extension("checkpoint.tmp");
    let body = serde_json::to_vec(checkpoint)
        .map_err(|e| SignerError::Storage(format!("encode checkpoint: {e}")))?;

    {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)
            .map_err(|e| SignerError::Storage(format!("open checkpoint tmp: {e}")))?;
        f.write_all(&body)
            .map_err(|e| SignerError::Storage(format!("write checkpoint tmp: {e}")))?;
        f.sync_all()
            .map_err(|e| SignerError::Storage(format!("fsync checkpoint tmp: {e}")))?;
    }

    std::fs::rename(&tmp, path)
        .map_err(|e| SignerError::Storage(format!("rename checkpoint: {e}")))?;

    let dir_file = std::fs::File::open(dir).map_err(|e| {
        SignerError::Storage(format!("open checkpoint dir {} for fsync: {e}", dir.display()))
    })?;
    dir_file
        .sync_all()
        .map_err(|e| SignerError::Storage(format!("fsync checkpoint dir: {e}")))?;
    Ok(())
}

/// Decide the startup integrity verdict.
///
/// * `read` is the [`read`] result for the checkpoint file.
/// * `log_empty` is whether the loaded log has zero records.
/// * `head_at_checkpoint` is the live log's head-hash hex at the checkpoint's
///   witnessed index (`count - 1`), or `None` if the log has fewer records than
///   that (looked up only when a checkpoint is present).
///
/// Core invariant: a present checkpoint is consistent iff the record it witnessed
/// is still in the log with the same hash. Anything else with real data behind it
/// is tampering. A read error is always tampering (the atomic writer cannot
/// produce a malformed file). The only `Genesis` is a genuine first run.
pub fn assess_startup(
    read: Result<Option<Checkpoint>>,
    log_empty: bool,
    head_at_checkpoint: Option<String>,
) -> StartupCheck {
    match read {
        Ok(None) => {
            if log_empty {
                StartupCheck::Genesis
            } else {
                StartupCheck::Tampered {
                    detail: "head checkpoint missing for a non-empty undo-log \
                             (the truncation witness was removed)"
                        .to_string(),
                }
            }
        }
        Ok(Some(cp)) => match head_at_checkpoint {
            None => StartupCheck::Tampered {
                detail: format!(
                    "undo-log has no record at checkpoint count {} (truncated)",
                    cp.count
                ),
            },
            Some(hash) if hash != cp.head_hex => StartupCheck::Tampered {
                detail: format!(
                    "undo-log head hash at checkpoint count {} does not match the \
                     checkpoint (checkpoint or log mutated)",
                    cp.count
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

    fn cp(count: u64, head: &str) -> Checkpoint {
        Checkpoint { count, head_hex: head.to_string() }
    }

    #[test]
    fn hex32_encodes_lowercase_64_chars() {
        let mut h = [0u8; 32];
        h[0] = 0xab;
        h[31] = 0x0f;
        let s = hex32(&h);
        assert_eq!(s.len(), 64);
        assert!(s.starts_with("ab"));
        assert!(s.ends_with("0f"));
    }

    #[test]
    fn write_then_read_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = checkpoint_path(&dir.path().join("undo.log"));
        let c = cp(7, "abcd");
        write(&path, &c).unwrap();
        assert_eq!(read(&path).unwrap(), Some(c));
    }

    #[test]
    fn absent_checkpoint_reads_as_none() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read(&checkpoint_path(&dir.path().join("undo.log"))).unwrap(), None);
    }

    #[test]
    fn corrupt_checkpoint_is_an_error_not_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = checkpoint_path(&dir.path().join("undo.log"));
        std::fs::write(&path, b"not json").unwrap();
        assert!(read(&path).is_err());
    }

    #[test]
    fn genesis_is_empty_log_with_no_checkpoint() {
        assert_eq!(assess_startup(Ok(None), true, None), StartupCheck::Genesis);
    }

    #[test]
    fn missing_checkpoint_for_a_nonempty_log_is_tampered() {
        assert!(matches!(
            assess_startup(Ok(None), false, None),
            StartupCheck::Tampered { .. }
        ));
    }

    #[test]
    fn witnessed_record_present_with_matching_hash_is_consistent() {
        assert_eq!(
            assess_startup(Ok(Some(cp(3, "h3"))), false, Some("h3".into())),
            StartupCheck::Consistent
        );
    }

    #[test]
    fn missing_witnessed_record_is_truncation() {
        assert!(matches!(
            assess_startup(Ok(Some(cp(5, "h5"))), false, None),
            StartupCheck::Tampered { .. }
        ));
    }

    #[test]
    fn witnessed_hash_mismatch_is_tampered() {
        assert!(matches!(
            assess_startup(Ok(Some(cp(3, "expected"))), false, Some("different".into())),
            StartupCheck::Tampered { .. }
        ));
    }

    #[test]
    fn corrupt_checkpoint_is_tampered_regardless_of_log() {
        let corrupt = || Err(SignerError::Storage("corrupt".into()));
        assert!(matches!(assess_startup(corrupt(), true, None), StartupCheck::Tampered { .. }));
        assert!(matches!(assess_startup(corrupt(), false, None), StartupCheck::Tampered { .. }));
    }
}

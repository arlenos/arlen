//! Enacting a filesystem/setting [`InverseReceipt`] - the undo side of the ACT
//! layer (ai-act-layer-plan.md). The engine captures an inverse write-ahead when
//! it performs a reversible action; replaying that inverse here IS the undo.
//!
//! The graph inverse ([`InverseReceipt::RetractGraphEdge`]) is enacted separately
//! by the graph compensation path (`compensation.rs` -> the knowledge retract op),
//! so this module handles only the filesystem/setting variants and reports the
//! graph one as [`EnactOutcome::NotFilesystem`]. Snapshot restore is gated on a
//! snapshot-capable filesystem and the setting-value restore rides the
//! format-preserving editor, so both report a clear unsupported reason until their
//! own slices land; the two content-safe filesystem arms - relocation-undo and
//! identity-bound delete - are enacted here.
//!
//! Safety property: an undo only ever touches exactly the entity the action
//! produced. [`InverseReceipt::DeleteCreated`] deletes ONLY when the file still
//! carries the commit-time fingerprint (so undo never deletes a replacement a user
//! put in its place); relocation-undo refuses to clobber an occupied prior path.

use std::io::Read;
use std::path::Path;

use arlen_ai_undo_core::effect_model::InverseReceipt;
use sha2::{Digest, Sha256};

/// What the enactment did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnactOutcome {
    /// A relocated entity was moved back to its prior path.
    Restored,
    /// The created entity was deleted (its fingerprint still matched).
    Deleted,
    /// The created entity no longer carries its commit-time fingerprint (a user
    /// replaced it), so undo left it in place rather than deleting a replacement.
    RefusedIdentityMismatch,
    /// The prior path is occupied, so relocation-undo refused rather than clobber.
    RefusedPriorOccupied,
    /// The receipt reverses a graph write, not a filesystem effect; the graph
    /// compensation path enacts it.
    NotFilesystem,
}

/// Why an enactment could not run.
#[derive(Debug)]
pub enum EnactError {
    /// A filesystem operation failed.
    Io(String),
    /// The variant's enactment is not built yet (with the reason).
    Unsupported(&'static str),
}

impl std::fmt::Display for EnactError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EnactError::Io(e) => write!(f, "undo enact io: {e}"),
            EnactError::Unsupported(why) => write!(f, "undo enact unsupported: {why}"),
        }
    }
}

impl std::error::Error for EnactError {}

/// The content fingerprint of a file: the lowercase hex SHA-256 of its bytes. This
/// is the identity a [`InverseReceipt::DeleteCreated`] undo checks against the
/// commit-time fingerprint, so the CAPTURE side (recording a created file) MUST
/// fingerprint the same way. `None` if the file is unreadable (e.g. already gone).
pub fn fingerprint_file(path: &Path) -> Option<String> {
    let mut file = std::fs::File::open(path).ok()?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Some(hex_lower(&hasher.finalize()))
}

/// Lowercase hex of a byte slice.
fn hex_lower(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// Enact a filesystem/setting inverse receipt. Reversible + identity-safe:
/// relocation-undo moves the entity back only into a free prior path;
/// creation-undo deletes only a still-fingerprint-matching file. The graph,
/// snapshot and setting variants report their status without touching anything.
pub fn enact_inverse(receipt: &InverseReceipt) -> Result<EnactOutcome, EnactError> {
    match receipt {
        InverseReceipt::RestorePath { now, prior } => enact_restore_path(now.as_str(), prior.as_str()),
        InverseReceipt::DeleteCreated { created } => {
            enact_delete_created(created.path().as_str(), created.fingerprint())
        }
        InverseReceipt::RestoreValue { .. } => Err(EnactError::Unsupported(
            "setting-value restore rides the format-preserving editor (next slice)",
        )),
        InverseReceipt::RestoreSnapshot { .. } => Err(EnactError::Unsupported(
            "snapshot restore is gated on a snapshot-capable filesystem",
        )),
        InverseReceipt::RetractGraphEdge { .. } => Ok(EnactOutcome::NotFilesystem),
    }
}

/// Move a relocated entity from `now` back to `prior`, refusing to clobber an
/// occupied prior path (a new file placed there since the action).
fn enact_restore_path(now: &str, prior: &str) -> Result<EnactOutcome, EnactError> {
    if Path::new(prior).exists() {
        return Ok(EnactOutcome::RefusedPriorOccupied);
    }
    std::fs::rename(now, prior).map_err(|e| EnactError::Io(e.to_string()))?;
    Ok(EnactOutcome::Restored)
}

/// Delete the created entity at `path` only if it still carries `fingerprint`
/// (the commit-time content identity); otherwise leave a user's replacement alone.
/// An already-absent file is treated as an identity mismatch (nothing of ours to
/// delete), never an error - undo is idempotent.
fn enact_delete_created(path: &str, fingerprint: &str) -> Result<EnactOutcome, EnactError> {
    match fingerprint_file(Path::new(path)) {
        Some(current) if current == fingerprint => {
            std::fs::remove_file(path).map_err(|e| EnactError::Io(e.to_string()))?;
            Ok(EnactOutcome::Deleted)
        }
        _ => Ok(EnactOutcome::RefusedIdentityMismatch),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arlen_ai_undo_core::effect_model::{CanonicalPath, CreatedIdentity};
    use std::sync::atomic::{AtomicU64, Ordering};

    fn tmp() -> std::path::PathBuf {
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let d = std::env::temp_dir().join(format!("undo-enact-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn canonical(p: &Path) -> CanonicalPath {
        // The receipt path is already canonical-absolute; the temp paths are.
        serde_json::from_value(serde_json::Value::String(p.to_string_lossy().into_owned())).unwrap()
    }

    #[test]
    fn fingerprint_is_the_content_sha256() {
        let d = tmp();
        let f = d.join("x");
        std::fs::write(&f, b"hello").unwrap();
        // SHA-256("hello") is a known vector.
        assert_eq!(
            fingerprint_file(&f).unwrap(),
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
        assert!(fingerprint_file(&d.join("absent")).is_none());
    }

    #[test]
    fn delete_created_removes_a_fingerprint_matching_file() {
        let d = tmp();
        let f = d.join("made.txt");
        std::fs::write(&f, b"agent made this").unwrap();
        let fp = fingerprint_file(&f).unwrap();
        let receipt = InverseReceipt::DeleteCreated {
            created: CreatedIdentity::new(canonical(&f), &fp).unwrap(),
        };
        assert_eq!(enact_inverse(&receipt).unwrap(), EnactOutcome::Deleted);
        assert!(!f.exists(), "the created file is gone");
    }

    #[test]
    fn delete_created_refuses_a_replaced_file() {
        let d = tmp();
        let f = d.join("made.txt");
        std::fs::write(&f, b"agent made this").unwrap();
        let fp = fingerprint_file(&f).unwrap();
        // The user replaces the file with different content after the action.
        std::fs::write(&f, b"user put something else here").unwrap();
        let receipt = InverseReceipt::DeleteCreated {
            created: CreatedIdentity::new(canonical(&f), &fp).unwrap(),
        };
        assert_eq!(enact_inverse(&receipt).unwrap(), EnactOutcome::RefusedIdentityMismatch);
        assert!(f.exists(), "the user's replacement is left alone");
    }

    #[test]
    fn delete_created_on_an_absent_file_is_a_no_op_mismatch() {
        let d = tmp();
        let f = d.join("gone.txt");
        let receipt = InverseReceipt::DeleteCreated {
            created: CreatedIdentity::new(canonical(&f), "deadbeef").unwrap(),
        };
        assert_eq!(enact_inverse(&receipt).unwrap(), EnactOutcome::RefusedIdentityMismatch);
    }

    #[test]
    fn restore_path_moves_the_entity_back() {
        let d = tmp();
        let now = d.join("moved-here");
        let prior = d.join("was-here");
        std::fs::write(&now, b"content").unwrap();
        let receipt = InverseReceipt::RestorePath { now: canonical(&now), prior: canonical(&prior) };
        assert_eq!(enact_inverse(&receipt).unwrap(), EnactOutcome::Restored);
        assert!(!now.exists());
        assert_eq!(std::fs::read(&prior).unwrap(), b"content");
    }

    #[test]
    fn restore_path_refuses_to_clobber_an_occupied_prior() {
        let d = tmp();
        let now = d.join("moved-here");
        let prior = d.join("was-here");
        std::fs::write(&now, b"a").unwrap();
        std::fs::write(&prior, b"someone-else").unwrap();
        let receipt = InverseReceipt::RestorePath { now: canonical(&now), prior: canonical(&prior) };
        assert_eq!(enact_inverse(&receipt).unwrap(), EnactOutcome::RefusedPriorOccupied);
        // Both files untouched.
        assert_eq!(std::fs::read(&prior).unwrap(), b"someone-else");
        assert_eq!(std::fs::read(&now).unwrap(), b"a");
    }

    #[test]
    fn graph_and_gated_variants_report_without_touching_anything() {
        let g = InverseReceipt::RetractGraphEdge {
            op_id: "op".into(),
            from_type: "system.File".into(),
            from_id: "/f".into(),
            to_type: "system.Project".into(),
            to_id: "p".into(),
            relation_type: "FILE_PART_OF".into(),
        };
        assert_eq!(enact_inverse(&g).unwrap(), EnactOutcome::NotFilesystem);
    }
}

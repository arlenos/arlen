//! Enacting a filesystem/setting [`InverseReceipt`] - the undo side of the ACT
//! layer (ai-act-layer-plan.md). The engine captures an inverse write-ahead when
//! it performs a reversible action; replaying that inverse here IS the undo.
//!
//! The graph inverse ([`InverseReceipt::RetractGraphEdge`]) is enacted separately
//! by the graph compensation path (`compensation.rs` -> the knowledge retract op),
//! so this module handles only the filesystem/setting variants and reports the
//! graph one as [`EnactOutcome::NotFilesystem`]. Snapshot restore is gated on a
//! snapshot-capable filesystem, so it reports a clear unsupported reason until its
//! own slice lands; the relocation-undo, identity-bound delete and the
//! setting-value restore (through the format-preserving editor) are enacted here.
//!
//! Safety property: an undo only ever touches exactly the entity the action
//! produced. [`InverseReceipt::DeleteCreated`] deletes ONLY when the file still
//! carries the commit-time fingerprint (so undo never deletes a replacement a user
//! put in its place); relocation-undo refuses to clobber an occupied prior path.

use std::path::Path;

use arlen_ai_undo_core::effect_model::{
    fingerprint_file, CanonicalPath, CreatedIdentity, InverseReceipt, SettingTarget,
};
use arlen_ai_undo_core::undo_log::UndoEntry;
use arlen_config_format::{checked_remove, checked_set, handler_for, ConfigValue, Format};

/// What the enactment did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnactOutcome {
    /// A relocated entity was moved back to its prior path.
    Restored,
    /// The created entity was deleted (its fingerprint still matched).
    Deleted,
    /// The created entity was moved to the trash (its fingerprint still matched);
    /// the trash-first undo-of-create, restorable rather than destroyed.
    Trashed,
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

/// The result of a restart-time compensation-recovery pass over the entries a crash
/// left mid-reversal ([`arlen_ai_undo_core::undo_log::UndoLog::compensating_entries`]).
#[derive(Debug, Default, PartialEq, Eq)]
pub struct RecoveryOutcome {
    /// op ids whose interrupted compensation reached a DEFINITIVE enact result (the
    /// entity moved back / was deleted, or a refusal the world has already settled
    /// past). The caller records `Compensated` for these so they are never re-driven.
    pub recovered: Vec<String>,
    /// op ids whose enact hit a transient I/O error, paired with the message. Left
    /// `Compensating` so the next restart retries them.
    pub deferred: Vec<(String, String)>,
}

/// Drive the crash-interrupted compensations to completion on restart: replay each
/// entry's inverse through `enact`. The inverse is idempotent, so a compensation the
/// crash left half-applied finishes cleanly. A definitive enact result - the entity
/// restored/deleted, OR a refusal the world has already moved past (an occupied
/// prior, an identity mismatch that will never match again) - marks the op
/// `recovered`, because re-driving it on every restart would repeat the same
/// outcome forever. Only a transient I/O error `defer`s the op for the next restart.
/// `enact` is injected (in production, [`enact_inverse`]) so the recovery POLICY is
/// unit-tested without touching the filesystem. The caller owns the durable log, so
/// it records the `Compensated` transitions for `recovered` after this returns.
pub fn drive_compensation_recovery<F>(entries: &[&UndoEntry], mut enact: F) -> RecoveryOutcome
where
    F: FnMut(&InverseReceipt) -> Result<EnactOutcome, EnactError>,
{
    let mut outcome = RecoveryOutcome::default();
    for entry in entries {
        match enact(&entry.inverse) {
            Ok(_) => outcome.recovered.push(entry.op_id.clone()),
            Err(e) => outcome.deferred.push((entry.op_id.clone(), e.to_string())),
        }
    }
    outcome
}

/// Whether this daemon can actually carry out `receipt`'s inverse.
///
/// A recent-actions surface offers an undo per row, and a row that offers one it
/// cannot perform is worse than a row that offers none: the user learns the
/// button lies. `RestoreSnapshot` is exactly that case today - the receipt is a
/// perfectly good record of how to reverse the action, and [`enact_inverse`]
/// refuses it because the snapshot seam belongs to the atomic-image strand.
///
/// Exhaustive on purpose, with no catch-all: a new receipt variant has to be
/// classified here rather than defaulting to "yes, offer it" and being found out
/// by a user pressing the button.
pub fn is_enactable(receipt: &InverseReceipt) -> bool {
    match receipt {
        InverseReceipt::RestorePath { .. }
        | InverseReceipt::RestoreFromTrash { .. }
        | InverseReceipt::DeleteCreated { .. }
        | InverseReceipt::TrashCreated { .. }
        | InverseReceipt::RestoreValue { .. }
        // Not enacted through THIS path (the graph store retracts it), but the
        // undo is real and the surface should offer it.
        | InverseReceipt::RetractGraphEdge { .. } => true,
        InverseReceipt::RestoreSnapshot { .. } => false,
    }
}

/// Enact a filesystem/setting inverse receipt. Reversible + identity-safe:
/// relocation-undo moves the entity back only into a free prior path;
/// creation-undo deletes only a still-fingerprint-matching file. The graph,
/// snapshot and setting variants report their status without touching anything.
pub fn enact_inverse(receipt: &InverseReceipt) -> Result<EnactOutcome, EnactError> {
    match receipt {
        InverseReceipt::RestorePath { now, prior } => enact_restore_path(now.as_str(), prior.as_str()),
        InverseReceipt::RestoreFromTrash { original, trashed, trash_info } => {
            enact_restore_from_trash(original.as_str(), trashed.as_str(), trash_info.as_str())
        }
        InverseReceipt::DeleteCreated { created } => {
            enact_delete_created(created.path().as_str(), created.fingerprint())
        }
        InverseReceipt::TrashCreated { created } => {
            enact_trash_created(created.path().as_str(), created.fingerprint())
        }
        InverseReceipt::RestoreValue { target, prior } => {
            enact_restore_value(target.file(), target.key(), prior.as_deref())
        }
        InverseReceipt::RestoreSnapshot { .. } => Err(EnactError::Unsupported(
            "snapshot restore is gated on a snapshot-capable filesystem",
        )),
        InverseReceipt::RetractGraphEdge { .. } => Ok(EnactOutcome::NotFilesystem),
    }
}

/// Atomically move `from` to `to` WITHOUT clobbering an existing `to`, via
/// `renameat2(RENAME_NOREPLACE)`. Returns `Ok(true)` when the move happened,
/// `Ok(false)` when `to` was already occupied (the kernel refused, no clobber), and
/// `Err` on any other I/O failure. This closes the check-then-rename TOCTOU: there is
/// no window between a liveness check and the move for another process to place a
/// file at `to`. On a filesystem or kernel that does not support the flag
/// (`EINVAL`/`ENOSYS`) it falls back to an exists-check + rename, where the race
/// window remains but only on that unusual target.
fn rename_noreplace(from: &str, to: &str) -> std::io::Result<bool> {
    use std::ffi::CString;
    let invalid = |_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "path contains a nul byte");
    let c_from = CString::new(from).map_err(invalid)?;
    let c_to = CString::new(to).map_err(invalid)?;
    // SAFETY: both pointers are valid C strings live for the call; AT_FDCWD resolves
    // the relative-or-absolute paths against the cwd exactly as `std::fs::rename`.
    let rc = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            c_from.as_ptr(),
            libc::AT_FDCWD,
            c_to.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if rc == 0 {
        return Ok(true);
    }
    let err = std::io::Error::last_os_error();
    match err.raw_os_error() {
        Some(libc::EEXIST) => Ok(false),
        // The flag is unsupported here; fall back to the (racy) check-then-rename so
        // undo still functions on an exotic target.
        Some(libc::EINVAL) | Some(libc::ENOSYS) => {
            if Path::new(to).exists() {
                Ok(false)
            } else {
                std::fs::rename(from, to)?;
                Ok(true)
            }
        }
        _ => Err(err),
    }
}

/// Move a relocated entity from `now` back to `prior`, refusing to clobber an
/// occupied prior path (a new file placed there since the action). The move is a
/// no-clobber atomic rename, so the occupied check and the move cannot race.
fn enact_restore_path(now: &str, prior: &str) -> Result<EnactOutcome, EnactError> {
    if rename_noreplace(now, prior).map_err(|e| EnactError::Io(e.to_string()))? {
        Ok(EnactOutcome::Restored)
    } else {
        Ok(EnactOutcome::RefusedPriorOccupied)
    }
}

/// Restore a trashed entity from `trashed` back to `original`, then remove the
/// companion `.trashinfo` sidecar so no orphan trash-view entry survives the undo.
/// Refuses to clobber an occupied `original` (same as [`enact_restore_path`]); the
/// file restoration is the load-bearing step, so the sidecar removal is best-effort
/// AFTER it (a leftover `.trashinfo` is a cosmetic trash-view artifact, never a lost
/// file), and an already-absent sidecar is fine - undo is idempotent.
fn enact_restore_from_trash(
    original: &str,
    trashed: &str,
    trash_info: &str,
) -> Result<EnactOutcome, EnactError> {
    if !rename_noreplace(trashed, original).map_err(|e| EnactError::Io(e.to_string()))? {
        return Ok(EnactOutcome::RefusedPriorOccupied);
    }
    // The file is back at its origin; clean the trash metadata best-effort.
    let _ = std::fs::remove_file(trash_info);
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

/// Trash the created entity at `path` (move it to the freedesktop home trash) only
/// if it still carries `fingerprint`; otherwise leave a user's replacement in place.
/// The trash-first undo-of-create - it NEVER permanently destroys, so an accidental
/// undo (e.g. of a folder the user has since filled) stays recoverable from the
/// trash. An already-absent file is an identity mismatch, never an error (idempotent).
fn enact_trash_created(path: &str, fingerprint: &str) -> Result<EnactOutcome, EnactError> {
    let trash = crate::file_executor::home_trash_dir()
        .ok_or_else(|| EnactError::Io("no home trash directory".to_string()))?;
    enact_trash_created_in(path, fingerprint, &trash)
}

/// [`enact_trash_created`] against an explicit trash root, so the trash-put is
/// unit-tested without touching the real home trash.
fn enact_trash_created_in(
    path: &str,
    fingerprint: &str,
    trash_dir: &Path,
) -> Result<EnactOutcome, EnactError> {
    match fingerprint_file(Path::new(path)) {
        Some(current) if current == fingerprint => {
            let files_dir = trash_dir.join("files");
            let info_dir = trash_dir.join("info");
            std::fs::create_dir_all(&files_dir)
                .and_then(|()| std::fs::create_dir_all(&info_dir))
                .map_err(|e| EnactError::Io(e.to_string()))?;
            let base_name = Path::new(path)
                .file_name()
                .and_then(|n| n.to_str())
                .ok_or_else(|| EnactError::Io("created path has no file name".to_string()))?;
            crate::file_executor::trash_into(&files_dir, &info_dir, base_name, path)
                .map(|_slot| EnactOutcome::Trashed)
                .map_err(|e| EnactError::Io(format!("trash: {e:?}")))
        }
        _ => Ok(EnactOutcome::RefusedIdentityMismatch),
    }
}

/// Restore a setting `key` in `file` to its `prior` value (or remove the key when
/// `prior` is `None` - it was absent before the action), through the
/// format-preserving editor with its read-after-write self-check. The edit is
/// written back atomically (temp + rename) so a crash never leaves a half-written
/// config. The format is detected from the file's name; an unknown format refuses
/// (never guesses + corrupts). An unset-of-an-absent-file is an idempotent no-op.
fn enact_restore_value(
    file: &str,
    key: &str,
    prior: Option<&str>,
) -> Result<EnactOutcome, EnactError> {
    let format = format_for_file(file)
        .ok_or(EnactError::Unsupported("unrecognized setting file format"))?;
    let handler = handler_for(format);

    let text = match std::fs::read_to_string(file) {
        Ok(t) => t,
        // An unset (prior None) of a key in an absent file is already the desired
        // state; a set into an absent file starts from empty (the natural place).
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if prior.is_none() {
                return Ok(EnactOutcome::Restored);
            }
            String::new()
        }
        Err(e) => return Err(EnactError::Io(e.to_string())),
    };

    let candidate = match prior {
        Some(v) => checked_set(handler.as_ref(), &text, key, &parse_prior(v))
            .map_err(|e| EnactError::Io(e.to_string()))?,
        None => checked_remove(handler.as_ref(), &text, key)
            .map_err(|e| EnactError::Io(e.to_string()))?,
    };

    atomic_write(file, candidate.as_bytes())?;
    Ok(EnactOutcome::Restored)
}

/// Apply a scalar setting: set `key` in `file` to `value` through the same
/// format-preserving editor + atomic write + read-after-write self-check the undo
/// path uses. This is the FORWARD of a settings write; the executor captures the
/// prior first ([`capture_prior_value`]) so a later undo restores it. `value` is
/// typed best-effort (`true`/`false`/int/float, else string) exactly as the restore
/// path types a prior, so the forward set and the undo restore round-trip identically.
pub fn apply_setting_value(file: &str, key: &str, value: &str) -> Result<EnactOutcome, EnactError> {
    // Setting to a concrete value is exactly the restore path with `Some(value)`.
    enact_restore_value(file, key, Some(value))
}

/// Detect the config format from a setting file's name. `prefs.js` and `.env` are
/// recognized by name; the rest by extension. `None` for an unrecognized name, so
/// the caller refuses rather than corrupting an unknown format.
fn format_for_file(file: &str) -> Option<Format> {
    let name = Path::new(file).file_name()?.to_str()?;
    if name == "prefs.js" {
        return Some(Format::FirefoxPrefs);
    }
    if name == ".env" || name.ends_with(".env") {
        return Some(Format::Env);
    }
    let ext = Path::new(file).extension()?.to_str()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "toml" => Format::Toml,
        "json" | "jsonc" => Format::Json,
        "ini" | "conf" | "cfg" => Format::Ini,
        _ => return None,
    })
}

/// Best-effort type a `prior` string back into a scalar [`ConfigValue`]: `true`/
/// `false` as a bool, an integer or float literal as that number, else a string.
/// A limitation of the receipt carrying the prior as text: a string setting whose
/// literal value is `"true"` or `"42"` restores as a bool/int - rare, and the fix
/// is a typed prior in a future receipt-model refinement. Pure.
fn parse_prior(s: &str) -> ConfigValue {
    match s {
        "true" => return ConfigValue::Bool(true),
        "false" => return ConfigValue::Bool(false),
        _ => {}
    }
    if let Ok(i) = s.parse::<i64>() {
        return ConfigValue::Int(i);
    }
    if let Ok(f) = s.parse::<f64>() {
        return ConfigValue::Float(f);
    }
    ConfigValue::String(s.to_string())
}

/// Write `bytes` to `path` atomically: a sibling temp file, fsync, then rename over
/// the target, so a crash never leaves a half-written config.
fn atomic_write(path: &str, bytes: &[u8]) -> Result<(), EnactError> {
    use std::io::Write;
    let target = Path::new(path);
    let dir = target.parent().unwrap_or_else(|| Path::new("."));
    let tmp = dir.join(format!(
        ".{}.undo-tmp",
        target.file_name().and_then(|n| n.to_str()).unwrap_or("cfg")
    ));
    let write = || -> std::io::Result<()> {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        std::fs::rename(&tmp, target)?;
        Ok(())
    };
    write().map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        EnactError::Io(e.to_string())
    })
}

// --- Capture: the forward half. A reversible executor arm records the inverse of
// the action it is about to perform (or just performed), so a later undo replays
// it through `enact_inverse`. Capture + enact are a pair: `capture -> perform ->
// enact` restores the pre-action state (the round-trip invariant tested below).

/// The inverse of a relocation `from -> to`: undo moves `to` back to `from`.
/// Captured trivially from the two paths (no read); `None` if either is not a
/// canonical absolute path (the executor works in canonical paths, so this only
/// guards a mis-supplied argument).
pub fn inverse_of_move(from: &str, to: &str) -> Option<InverseReceipt> {
    Some(InverseReceipt::RestorePath {
        now: CanonicalPath::new(to)?,
        prior: CanonicalPath::new(from)?,
    })
}

/// The inverse of creating `path`: undo deletes exactly that file, identity-bound
/// to the content it holds NOW. Call AFTER the create so the fingerprint witnesses
/// the created bytes. `Err` if the file is unreadable (nothing to bind to) or the
/// path is not canonical-absolute.
///
/// NB the identity is a CONTENT fingerprint, not the inode: undo deletes whatever
/// currently sits at `path` if its bytes still hash to the commit-time fingerprint.
/// Under the single-uid model that is the user's own file, and a byte-identical
/// replacement at the same path is near-impossible in practice; a future
/// multi-writer model would want inode/dev identity here.
pub fn capture_created(path: &str) -> Result<InverseReceipt, EnactError> {
    let fingerprint = fingerprint_file(Path::new(path))
        .ok_or_else(|| EnactError::Io(format!("created file unreadable: {path}")))?;
    let canon = CanonicalPath::new(path)
        .ok_or(EnactError::Unsupported("created path is not canonical-absolute"))?;
    let created = CreatedIdentity::new(canon, &fingerprint)
        .ok_or(EnactError::Unsupported("empty fingerprint"))?;
    Ok(InverseReceipt::DeleteCreated { created })
}

/// The inverse of setting `key` in `file`: undo restores the value the key holds
/// NOW (or removes the key if it is currently absent). Call BEFORE the set so the
/// captured prior is the pre-action value. `Err` for an unknown format or a
/// non-scalar (Opaque) current value that cannot be represented as a restorable
/// prior (the action is then not RestoreValue-reversible).
pub fn capture_prior_value(file: &str, key: &str) -> Result<InverseReceipt, EnactError> {
    let format = format_for_file(file)
        .ok_or(EnactError::Unsupported("unrecognized setting file format"))?;
    let handler = handler_for(format);
    let target =
        SettingTarget::new(file, key).ok_or(EnactError::Unsupported("empty file or key"))?;

    let prior = match std::fs::read_to_string(file) {
        Ok(text) => {
            let model = handler
                .read(&text)
                .map_err(|e| EnactError::Io(format!("read setting: {e}")))?;
            match model.get(key) {
                Some(ConfigValue::Opaque) => {
                    return Err(EnactError::Unsupported("current value is non-scalar"))
                }
                Some(v) => Some(value_to_prior_string(v)),
                None => None,
            }
        }
        // Absent file: the key is currently absent, so undo removes it.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(EnactError::Io(e.to_string())),
    };
    Ok(InverseReceipt::RestoreValue { target, prior })
}

/// A scalar [`ConfigValue`] as the textual prior a [`InverseReceipt::RestoreValue`]
/// carries. The inverse of [`parse_prior`], so a captured value round-trips back to
/// the same scalar on enactment.
fn value_to_prior_string(v: &ConfigValue) -> String {
    match v {
        ConfigValue::String(s) => s.clone(),
        ConfigValue::Bool(b) => b.to_string(),
        ConfigValue::Int(i) => i.to_string(),
        ConfigValue::Float(f) => f.to_string(),
        // Guarded before the call; a String rendering is the safe fallback.
        ConfigValue::Opaque => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
    fn recovery_marks_definitive_enacts_recovered_and_defers_transient_io() {
        let mk = |op: &str| UndoEntry {
            op_id: op.to_string(),
            correlation_id: "r".to_string(),
            inverse: InverseReceipt::RestorePath {
                now: canonical(Path::new("/a/now")),
                prior: canonical(Path::new("/a/prior")),
            },
        };
        let a = mk("done-restored");
        let b = mk("settled-refusal");
        let c = mk("transient");
        let entries = [&a, &b, &c];
        // Route by call order (entries are enacted in order): a definitive restore,
        // a settled refusal, then a transient I/O failure.
        let mut i = 0;
        let outcome = drive_compensation_recovery(&entries, |_inv| {
            i += 1;
            match i {
                1 => Ok(EnactOutcome::Restored),
                2 => Ok(EnactOutcome::RefusedPriorOccupied),
                _ => Err(EnactError::Io("disk gone".into())),
            }
        });
        // Both the restore and the settled refusal are recovered (never re-driven);
        // only the transient failure is deferred to the next restart.
        assert_eq!(outcome.recovered, ["done-restored", "settled-refusal"]);
        assert_eq!(outcome.deferred.len(), 1);
        assert_eq!(outcome.deferred[0].0, "transient");
        assert!(outcome.deferred[0].1.contains("disk gone"));
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
    fn rename_noreplace_moves_into_a_free_slot_and_refuses_an_occupied_one() {
        let d = tmp();
        let src = d.join("src");
        let free = d.join("free");
        std::fs::write(&src, b"x").unwrap();
        assert!(rename_noreplace(src.to_str().unwrap(), free.to_str().unwrap()).unwrap());
        assert_eq!(std::fs::read(&free).unwrap(), b"x");
        assert!(!src.exists());
        // An occupied destination is refused atomically, clobbering nothing.
        let src2 = d.join("src2");
        std::fs::write(&src2, b"new").unwrap();
        assert!(!rename_noreplace(src2.to_str().unwrap(), free.to_str().unwrap()).unwrap());
        assert_eq!(std::fs::read(&free).unwrap(), b"x", "destination not clobbered");
        assert_eq!(std::fs::read(&src2).unwrap(), b"new", "source left in place");
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
    fn trash_created_moves_the_file_to_trash_when_the_fingerprint_matches() {
        let d = tmp();
        let created = d.join("new.txt");
        std::fs::write(&created, b"content").unwrap();
        let fp = fingerprint_file(&created).unwrap();
        let trash = d.join("Trash");
        let outcome = enact_trash_created_in(created.to_str().unwrap(), &fp, &trash).unwrap();
        assert_eq!(outcome, EnactOutcome::Trashed);
        assert!(!created.exists(), "the created file was moved out of its place");
        // It landed in the trash with its sidecar - restorable, not destroyed.
        assert_eq!(std::fs::read(trash.join("files/new.txt")).unwrap(), b"content");
        assert!(trash.join("info/new.txt.trashinfo").exists());
    }

    #[test]
    fn trash_created_refuses_when_the_fingerprint_no_longer_matches() {
        let d = tmp();
        let created = d.join("new.txt");
        std::fs::write(&created, b"content").unwrap();
        let trash = d.join("Trash");
        // A user replaced the file (different content) -> undo leaves it alone.
        let outcome =
            enact_trash_created_in(created.to_str().unwrap(), "deadbeef", &trash).unwrap();
        assert_eq!(outcome, EnactOutcome::RefusedIdentityMismatch);
        assert!(created.exists(), "the replacement is left in place");
    }

    #[test]
    fn restore_from_trash_moves_back_and_cleans_the_sidecar() {
        let d = tmp();
        let original = d.join("notes.md");
        let trash_files = d.join("Trash/files");
        let trash_info = d.join("Trash/info");
        std::fs::create_dir_all(&trash_files).unwrap();
        std::fs::create_dir_all(&trash_info).unwrap();
        let trashed = trash_files.join("notes.md");
        let info = trash_info.join("notes.md.trashinfo");
        // Simulate a completed trash: the file sits in Trash/files with its sidecar.
        std::fs::write(&trashed, b"content").unwrap();
        std::fs::write(&info, b"[Trash Info]\nPath=/x\n").unwrap();

        let receipt = InverseReceipt::RestoreFromTrash {
            original: canonical(&original),
            trashed: canonical(&trashed),
            trash_info: canonical(&info),
        };
        assert_eq!(enact_inverse(&receipt).unwrap(), EnactOutcome::Restored);
        assert_eq!(std::fs::read(&original).unwrap(), b"content", "the file is back at its origin");
        assert!(!trashed.exists(), "the trash copy is gone");
        assert!(!info.exists(), "the sidecar was cleaned - no orphan trash entry");
    }

    #[test]
    fn restore_from_trash_refuses_an_occupied_origin() {
        let d = tmp();
        let original = d.join("notes.md");
        let trashed = d.join("trashed.md");
        let info = d.join("notes.md.trashinfo");
        std::fs::write(&trashed, b"trashed").unwrap();
        std::fs::write(&info, b"[Trash Info]\n").unwrap();
        // The user recreated something at the original path since the trash.
        std::fs::write(&original, b"user-new").unwrap();

        let receipt = InverseReceipt::RestoreFromTrash {
            original: canonical(&original),
            trashed: canonical(&trashed),
            trash_info: canonical(&info),
        };
        assert_eq!(enact_inverse(&receipt).unwrap(), EnactOutcome::RefusedPriorOccupied);
        assert_eq!(std::fs::read(&original).unwrap(), b"user-new", "the occupant is untouched");
        assert!(trashed.exists(), "the trash copy stays put");
        assert!(info.exists(), "the sidecar stays until a real restore");
    }

    fn restore_value(file: &Path, key: &str, prior: Option<&str>) -> InverseReceipt {
        InverseReceipt::RestoreValue {
            target: SettingTarget::new(file.to_str().unwrap(), key).unwrap(),
            prior: prior.map(|s| s.to_string()),
        }
    }

    #[test]
    fn restore_value_sets_a_prior_string_preserving_comments() {
        let d = tmp();
        let f = d.join("appearance.toml");
        std::fs::write(&f, "# theme\naccent = \"#ff0000\"\n").unwrap();
        // Undo restores the prior accent, keeping the comment.
        let r = restore_value(&f, "accent", Some("#6366f1"));
        assert_eq!(enact_inverse(&r).unwrap(), EnactOutcome::Restored);
        let out = std::fs::read_to_string(&f).unwrap();
        assert!(out.contains("accent = \"#6366f1\""), "prior value restored: {out}");
        assert!(out.contains("# theme"), "comment preserved: {out}");
    }

    #[test]
    fn restore_value_types_a_bool_prior() {
        let d = tmp();
        let f = d.join("shell.toml");
        std::fs::write(&f, "enabled = false\n").unwrap();
        let r = restore_value(&f, "enabled", Some("true"));
        assert_eq!(enact_inverse(&r).unwrap(), EnactOutcome::Restored);
        // Restored as a bool (enabled = true), not the string "true".
        assert!(std::fs::read_to_string(&f).unwrap().contains("enabled = true"));
    }

    #[test]
    fn restore_value_none_removes_a_key_that_was_absent_before() {
        let d = tmp();
        let f = d.join("shell.toml");
        std::fs::write(&f, "a = 1\nadded_by_agent = 2\n").unwrap();
        // The action added the key; prior None means undo removes it.
        let r = restore_value(&f, "added_by_agent", None);
        assert_eq!(enact_inverse(&r).unwrap(), EnactOutcome::Restored);
        let out = std::fs::read_to_string(&f).unwrap();
        assert!(!out.contains("added_by_agent"), "the added key is gone: {out}");
        assert!(out.contains("a = 1"), "the untouched key stays");
    }

    #[test]
    fn restore_value_unset_on_an_absent_file_is_a_no_op() {
        let d = tmp();
        let f = d.join("gone.toml");
        let r = restore_value(&f, "k", None);
        assert_eq!(enact_inverse(&r).unwrap(), EnactOutcome::Restored);
        assert!(!f.exists(), "no file is created for a no-op unset");
    }

    #[test]
    fn restore_value_refuses_an_unknown_format() {
        let d = tmp();
        let f = d.join("mystery.xyz");
        std::fs::write(&f, "k=v\n").unwrap();
        let r = restore_value(&f, "k", Some("old"));
        assert!(matches!(enact_inverse(&r), Err(EnactError::Unsupported(_))));
    }

    #[test]
    fn prior_typing_is_best_effort() {
        assert_eq!(parse_prior("true"), ConfigValue::Bool(true));
        assert_eq!(parse_prior("42"), ConfigValue::Int(42));
        assert_eq!(parse_prior("3.5"), ConfigValue::Float(3.5));
        assert_eq!(parse_prior("#6366f1"), ConfigValue::String("#6366f1".into()));
    }

    #[test]
    fn round_trip_move_capture_perform_enact_restores_the_origin() {
        let d = tmp();
        let from = d.join("a");
        let to = d.join("b");
        std::fs::write(&from, b"payload").unwrap();
        // Capture the inverse, then perform the move, then undo.
        let inv = inverse_of_move(from.to_str().unwrap(), to.to_str().unwrap()).unwrap();
        std::fs::rename(&from, &to).unwrap();
        assert_eq!(enact_inverse(&inv).unwrap(), EnactOutcome::Restored);
        assert!(from.exists() && !to.exists(), "the file is back at its origin");
        assert_eq!(std::fs::read(&from).unwrap(), b"payload");
    }

    #[test]
    fn round_trip_create_capture_after_enact_deletes_the_creation() {
        let d = tmp();
        let f = d.join("created");
        // Perform the create, then capture (fingerprint the created bytes), then undo.
        std::fs::write(&f, b"agent output").unwrap();
        let inv = capture_created(f.to_str().unwrap()).unwrap();
        assert_eq!(enact_inverse(&inv).unwrap(), EnactOutcome::Deleted);
        assert!(!f.exists());
    }

    #[test]
    fn round_trip_setting_capture_before_perform_enact_restores_the_prior() {
        let d = tmp();
        let f = d.join("shell.toml");
        std::fs::write(&f, "# keep\naccent = \"#6366f1\"\nflag = true\n").unwrap();
        // Capture the prior BEFORE changing, then perform the set, then undo.
        let inv_accent = capture_prior_value(f.to_str().unwrap(), "accent").unwrap();
        let inv_flag = capture_prior_value(f.to_str().unwrap(), "flag").unwrap();
        std::fs::write(&f, "# keep\naccent = \"#ff0000\"\nflag = false\n").unwrap();
        // Undo both: the string and the bool round-trip back to their pre-values.
        assert_eq!(enact_inverse(&inv_accent).unwrap(), EnactOutcome::Restored);
        assert_eq!(enact_inverse(&inv_flag).unwrap(), EnactOutcome::Restored);
        let out = std::fs::read_to_string(&f).unwrap();
        assert!(out.contains("accent = \"#6366f1\""), "string prior restored: {out}");
        assert!(out.contains("flag = true"), "bool prior restored: {out}");
    }

    #[test]
    fn capture_prior_of_an_absent_key_is_a_removal_inverse() {
        let d = tmp();
        let f = d.join("shell.toml");
        std::fs::write(&f, "a = 1\n").unwrap();
        // The agent is about to ADD `b`; its prior is absent, so the inverse removes it.
        let inv = capture_prior_value(f.to_str().unwrap(), "b").unwrap();
        match &inv {
            InverseReceipt::RestoreValue { prior, .. } => assert!(prior.is_none()),
            _ => panic!("expected RestoreValue"),
        }
        std::fs::write(&f, "a = 1\nb = 2\n").unwrap();
        assert_eq!(enact_inverse(&inv).unwrap(), EnactOutcome::Restored);
        assert!(!std::fs::read_to_string(&f).unwrap().contains("b ="), "the added key is undone");
    }

    #[test]
    fn capture_helpers_refuse_bad_input() {
        assert!(inverse_of_move("relative/from", "/abs/to").is_none());
        assert!(capture_created("/no/such/file/here").is_err());
        assert!(matches!(
            capture_prior_value("/x/mystery.xyz", "k"),
            Err(EnactError::Unsupported(_))
        ));
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

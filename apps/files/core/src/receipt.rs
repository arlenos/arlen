//! Map a recorded FM undo op to a durable [`InverseReceipt`] for the signed
//! undo-log (compensable-action-history-plan.md, CAH-3: the FM as a durable-log
//! producer, so an FM delete/create/rename/move is undoable across sessions, not
//! only through the in-memory `Ctrl+Z` stack).
//!
//! Pure: the caller supplies the FM root's absolute path (the file-manager host
//! roots at `/`, so its root-relative op paths are absolute-minus-leading-slash)
//! and the absolute home trash directory. A created DIRECTORY has no content
//! fingerprint, so it maps to `None` (the in-memory stack still undoes it; only the
//! durable cross-session record is skipped) - honest, not papered over.

use std::path::Path;

use arlen_ai_undo_core::effect_model::{
    fingerprint_file, CanonicalPath, CreatedIdentity, InverseReceipt,
};

use crate::undo::UndoableOp;

/// Map an [`UndoableOp`] to the durable [`InverseReceipt`] the signed undo-log
/// stores, resolving the op's root-relative paths against `root_abs` (the FM root's
/// absolute path) and `trash_abs` (the absolute home trash dir). `None` when the op
/// carries no content-identifiable durable inverse (a created directory), or a path
/// is not representable as a canonical absolute path.
pub fn to_inverse(op: &UndoableOp, root_abs: &Path, trash_abs: &Path) -> Option<InverseReceipt> {
    match op {
        // A created FILE trashes on undo, identity-bound to its commit-time content
        // (so undo never trashes a later replacement that reused the path). A created
        // directory has no content fingerprint -> None.
        UndoableOp::Created { path } => {
            let abs = root_abs.join(path);
            let fingerprint = fingerprint_file(&abs)?;
            let created = CreatedIdentity::new(canonical(&abs)?, &fingerprint)?;
            Some(InverseReceipt::TrashCreated { created })
        }
        // A rename's inverse renames back: the entry is now at parent/to_name, undo
        // restores parent/from_name.
        UndoableOp::Renamed { parent, from_name, to_name } => {
            let now = canonical(&root_abs.join(parent).join(to_name))?;
            let prior = canonical(&root_abs.join(parent).join(from_name))?;
            Some(InverseReceipt::RestorePath { now, prior })
        }
        // A move's inverse moves back into the original parent, basename preserved.
        UndoableOp::Moved { current, original_parent } => {
            let base = Path::new(current).file_name()?;
            let now = canonical(&root_abs.join(current))?;
            let prior = canonical(&root_abs.join(original_parent).join(base))?;
            Some(InverseReceipt::RestorePath { now, prior })
        }
        // A trash's inverse restores from the freedesktop trash slot the FM wrote.
        UndoableOp::Trashed { trashed_name, original } => {
            let original = canonical(&root_abs.join(original))?;
            let trashed = canonical(&trash_abs.join("files").join(trashed_name))?;
            let trash_info =
                canonical(&trash_abs.join("info").join(format!("{trashed_name}.trashinfo")))?;
            Some(InverseReceipt::RestoreFromTrash { original, trashed, trash_info })
        }
    }
}

/// A path as a `CanonicalPath` (absolute, no `.`/`..`), or `None` if it is not
/// representable that way (non-UTF-8, or a stray relative component).
fn canonical(p: &Path) -> Option<CanonicalPath> {
    CanonicalPath::new(p.to_str()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn a_created_file_maps_to_a_trash_created_inverse() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        // The FM records a root-relative path; here the root is the tempdir.
        std::fs::write(root.join("new.txt"), b"content").unwrap();
        let op = UndoableOp::Created { path: PathBuf::from("new.txt") };
        match to_inverse(&op, &root, Path::new("/tmp/Trash")) {
            Some(InverseReceipt::TrashCreated { created }) => {
                assert!(created.path().as_str().ends_with("/new.txt"));
                assert!(!created.fingerprint().is_empty());
            }
            other => panic!("expected TrashCreated, got {other:?}"),
        }
    }

    #[test]
    fn a_created_directory_has_no_durable_inverse() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::create_dir(root.join("folder")).unwrap();
        let op = UndoableOp::Created { path: PathBuf::from("folder") };
        assert!(to_inverse(&op, &root, Path::new("/tmp/Trash")).is_none());
    }

    #[test]
    fn a_rename_maps_to_restore_path() {
        let op = UndoableOp::Renamed {
            parent: PathBuf::from("home/tim/docs"),
            from_name: "old.txt".into(),
            to_name: "new.txt".into(),
        };
        match to_inverse(&op, Path::new("/"), Path::new("/home/tim/.local/share/Trash")) {
            Some(InverseReceipt::RestorePath { now, prior }) => {
                assert_eq!(now.as_str(), "/home/tim/docs/new.txt");
                assert_eq!(prior.as_str(), "/home/tim/docs/old.txt");
            }
            other => panic!("expected RestorePath, got {other:?}"),
        }
    }

    #[test]
    fn a_move_restores_into_the_original_parent() {
        let op = UndoableOp::Moved {
            current: PathBuf::from("home/tim/dst/report.pdf"),
            original_parent: PathBuf::from("home/tim/src"),
        };
        match to_inverse(&op, Path::new("/"), Path::new("/t")) {
            Some(InverseReceipt::RestorePath { now, prior }) => {
                assert_eq!(now.as_str(), "/home/tim/dst/report.pdf");
                assert_eq!(prior.as_str(), "/home/tim/src/report.pdf");
            }
            other => panic!("expected RestorePath, got {other:?}"),
        }
    }

    #[test]
    fn a_trash_maps_to_restore_from_trash() {
        let op = UndoableOp::Trashed {
            trashed_name: "notes.txt".into(),
            original: PathBuf::from("home/tim/notes.txt"),
        };
        let trash = Path::new("/home/tim/.local/share/Trash");
        match to_inverse(&op, Path::new("/"), trash) {
            Some(InverseReceipt::RestoreFromTrash { original, trashed, trash_info }) => {
                assert_eq!(original.as_str(), "/home/tim/notes.txt");
                assert_eq!(trashed.as_str(), "/home/tim/.local/share/Trash/files/notes.txt");
                assert_eq!(
                    trash_info.as_str(),
                    "/home/tim/.local/share/Trash/info/notes.txt.trashinfo"
                );
            }
            other => panic!("expected RestoreFromTrash, got {other:?}"),
        }
    }
}

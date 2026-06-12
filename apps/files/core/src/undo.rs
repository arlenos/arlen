//! FM operation undo (file-manager-plan.md bucket B, "the FM op-undo - `Ctrl+Z`
//! over the FM op log").
//!
//! Each mutating op the host runs is recorded as an [`UndoableOp`] capturing
//! exactly what its inverse needs; `Ctrl+Z` pops the most recent USER ACTION (a
//! batch - one `files_op` call may touch several entries) and applies each
//! inverse in reverse order through the existing [`crate::ops`]. The host knows
//! which op it dispatched, so it records the precise variant; the inverses reuse
//! the audited ops rather than re-implementing moves. A permanent delete is not
//! recorded (it has no inverse). Undo uses [`ConflictPolicy::Fail`] so it never
//! clobbers something that moved into the freed spot in the meantime - it reports
//! the conflict instead.

use std::path::{Path, PathBuf};

use cap_std::fs::Dir;

use crate::ops::{self, ConflictPolicy, OpResult};

/// A completed operation, recorded with what its inverse needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UndoableOp {
    /// A new entry was created at `path` (new-folder, copy, duplicate). Inverse:
    /// delete it.
    Created {
        /// The root-relative path of the created entry.
        path: PathBuf,
    },
    /// An entry was renamed in place. Inverse: rename it back.
    Renamed {
        /// The parent directory (root-relative) the rename happened in.
        parent: PathBuf,
        /// The original name.
        from_name: String,
        /// The new name.
        to_name: String,
    },
    /// An entry was moved into another folder (its basename preserved). Inverse:
    /// move it back to its original parent.
    Moved {
        /// The current root-relative path of the moved entry.
        current: PathBuf,
        /// The original parent folder (root-relative) it was moved out of.
        original_parent: PathBuf,
    },
    /// An entry was trashed. Inverse: restore it to where it was deleted from.
    Trashed {
        /// The basename inside the trash (`<Trash>/files/<name>`).
        trashed_name: String,
        /// The root-relative path the entry was trashed from.
        original: PathBuf,
    },
}

/// Apply the inverse of one recorded op, through the existing ops, against `dir`
/// (the FM root capability) and `trash` (the home trash capability). Conflicts
/// fail rather than clobber.
pub fn apply_inverse(op: &UndoableOp, dir: &Dir, trash: &Dir) -> OpResult<()> {
    match op {
        UndoableOp::Created { path } => ops::delete_permanent(dir, path),
        UndoableOp::Renamed {
            parent,
            from_name,
            to_name,
        } => ops::rename(dir, parent, to_name, from_name).map(|_| ()),
        UndoableOp::Moved {
            current,
            original_parent,
        } => ops::move_entry(dir, current, dir, original_parent, ConflictPolicy::Fail).map(|_| ()),
        UndoableOp::Trashed {
            trashed_name,
            original,
        } => ops::restore_entry(trash, trashed_name, dir, Path::new(original), ConflictPolicy::Fail)
            .map(|_| ()),
    }
}

/// An undo history of user actions. Each entry is a batch (one `files_op` call,
/// which may have touched several entries); `Ctrl+Z` undoes the whole batch.
#[derive(Debug, Default)]
pub struct UndoStack {
    batches: Vec<Vec<UndoableOp>>,
}

impl UndoStack {
    /// An empty stack.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a completed user action (a batch of one or more undoable ops). An
    /// empty batch (e.g. a permanent delete records nothing) is not pushed, so it
    /// never produces a no-op undo.
    pub fn record(&mut self, ops: Vec<UndoableOp>) {
        if !ops.is_empty() {
            self.batches.push(ops);
        }
    }

    /// Whether there is anything to undo.
    pub fn can_undo(&self) -> bool {
        !self.batches.is_empty()
    }

    /// Undo the most recent user action: pop the last batch and apply each
    /// inverse in reverse order (so a multi-step action unwinds correctly). On
    /// the first failing inverse the undo stops and reports the error; the
    /// already-applied inverses stand (the batch is removed regardless, matching
    /// the irreversible nature of a partially-applied undo). `Ok(false)` when
    /// there was nothing to undo.
    pub fn undo(&mut self, dir: &Dir, trash: &Dir) -> OpResult<bool> {
        let Some(batch) = self.batches.pop() else {
            return Ok(false);
        };
        for op in batch.iter().rev() {
            apply_inverse(op, dir, trash)?;
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cap_std::ambient_authority;

    fn cap(p: &std::path::Path) -> Dir {
        Dir::open_ambient_dir(p, ambient_authority()).unwrap()
    }

    /// A root + a trash (`files/` + `info/`) under one tempdir.
    fn fixture() -> (tempfile::TempDir, Dir, Dir) {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::create_dir(tmp.path().join("root")).unwrap();
        std::fs::create_dir_all(tmp.path().join("trash/files")).unwrap();
        std::fs::create_dir_all(tmp.path().join("trash/info")).unwrap();
        let root = cap(&tmp.path().join("root"));
        let trash = cap(&tmp.path().join("trash"));
        (tmp, root, trash)
    }

    #[test]
    fn undo_of_a_created_entry_deletes_it() {
        let (tmp, dir, trash) = fixture();
        std::fs::create_dir(tmp.path().join("root/newdir")).unwrap();
        let mut stack = UndoStack::new();
        stack.record(vec![UndoableOp::Created {
            path: PathBuf::from("newdir"),
        }]);
        assert!(stack.undo(&dir, &trash).unwrap());
        assert!(!tmp.path().join("root/newdir").exists());
        assert!(!stack.can_undo());
    }

    #[test]
    fn undo_of_a_rename_renames_back() {
        let (tmp, dir, trash) = fixture();
        std::fs::write(tmp.path().join("root/new.txt"), b"x").unwrap();
        let mut stack = UndoStack::new();
        // The op was: rename old.txt -> new.txt, in the root parent.
        stack.record(vec![UndoableOp::Renamed {
            parent: PathBuf::from("."),
            from_name: "old.txt".to_string(),
            to_name: "new.txt".to_string(),
        }]);
        assert!(stack.undo(&dir, &trash).unwrap());
        assert!(tmp.path().join("root/old.txt").exists());
        assert!(!tmp.path().join("root/new.txt").exists());
    }

    #[test]
    fn undo_of_a_move_moves_back_to_the_original_parent() {
        let (tmp, dir, trash) = fixture();
        std::fs::create_dir(tmp.path().join("root/sub")).unwrap();
        std::fs::write(tmp.path().join("root/sub/f.txt"), b"x").unwrap();
        let mut stack = UndoStack::new();
        // f.txt was moved from "." into "sub"; undo moves sub/f.txt back to ".".
        stack.record(vec![UndoableOp::Moved {
            current: PathBuf::from("sub/f.txt"),
            original_parent: PathBuf::from("."),
        }]);
        assert!(stack.undo(&dir, &trash).unwrap());
        assert!(tmp.path().join("root/f.txt").exists());
        assert!(!tmp.path().join("root/sub/f.txt").exists());
    }

    #[test]
    fn undo_of_a_trash_restores_from_the_trash() {
        let (tmp, dir, trash) = fixture();
        // Stage a trashed entry: doc.txt in files/, its .trashinfo in info/.
        std::fs::write(tmp.path().join("trash/files/doc.txt"), b"DATA").unwrap();
        std::fs::write(
            tmp.path().join("trash/info/doc.txt.trashinfo"),
            "[Trash Info]\nPath=/root/doc.txt\nDeletionDate=2026-01-01T00:00:00\n",
        )
        .unwrap();
        let mut stack = UndoStack::new();
        stack.record(vec![UndoableOp::Trashed {
            trashed_name: "doc.txt".to_string(),
            original: PathBuf::from("doc.txt"),
        }]);
        assert!(stack.undo(&dir, &trash).unwrap());
        assert_eq!(std::fs::read(tmp.path().join("root/doc.txt")).unwrap(), b"DATA");
        assert!(!tmp.path().join("trash/files/doc.txt").exists());
    }

    #[test]
    fn a_batch_of_creates_undoes_all_in_reverse() {
        let (tmp, dir, trash) = fixture();
        std::fs::write(tmp.path().join("root/a.txt"), b"a").unwrap();
        std::fs::write(tmp.path().join("root/b.txt"), b"b").unwrap();
        let mut stack = UndoStack::new();
        stack.record(vec![
            UndoableOp::Created { path: PathBuf::from("a.txt") },
            UndoableOp::Created { path: PathBuf::from("b.txt") },
        ]);
        assert!(stack.undo(&dir, &trash).unwrap());
        assert!(!tmp.path().join("root/a.txt").exists());
        assert!(!tmp.path().join("root/b.txt").exists());
    }

    #[test]
    fn an_empty_stack_undoes_nothing() {
        let (_tmp, dir, trash) = fixture();
        let mut stack = UndoStack::new();
        assert!(!stack.undo(&dir, &trash).unwrap());
    }

    #[test]
    fn an_empty_batch_is_not_recorded() {
        let mut stack = UndoStack::new();
        stack.record(vec![]);
        assert!(!stack.can_undo());
    }
}

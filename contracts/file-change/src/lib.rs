//! The proposed/applied file-change contract - the body of the harness's diff
//! card and the fused GateCard (surface 2 of `harness-file-refs-and-diffs-plan.md`).
//!
//! One definition shared by the producer (the ai-agent executor's predict/plan,
//! which proposes a change) and the consumer (the harness diff card, which shows
//! the exact change before approval and again as the done-receipt). The frontend
//! renders the generated ts-rs bindings (`bindings/*.ts`). It is settled as a
//! contract first - like `contracts/artifact` - so arlen-ui can design the diff
//! card against a real shape before the producer wiring lands.
//!
//! The crux the plan names: proposed and done are the SAME artifact. The
//! GateCard's body IS the diff; after approval the same card stays as a receipt
//! with a per-file undo. So a [`FileChangeSet`] is state-agnostic - the proposed
//! vs done state lives on the proposal/receipt envelope, not the change.
//!
//! What is producible today: a [`FileOp::Rename`] from the built `fs.move`
//! action (a path move, no content diff). [`FileOp::Edit`]'s unified-diff body
//! is reserved for when file-edit effects land; no built action emits it yet.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// A set of proposed (or applied) file changes. A multi-file change renders as a
/// list of per-file cards with a summary count; per-file accept/reject is the
/// plan's v1 granularity floor, so the unit the frontend toggles is one
/// [`FileChange`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../bindings/")]
pub struct FileChangeSet {
    /// The per-file changes, in proposal order.
    pub changes: Vec<FileChange>,
}

impl FileChangeSet {
    /// A change set from one or more changes.
    pub fn new(changes: Vec<FileChange>) -> Self {
        Self { changes }
    }

    /// A single-file change set (the common case - one `fs.move`, one edit).
    pub fn single(change: FileChange) -> Self {
        Self {
            changes: vec![change],
        }
    }
}

/// One file's proposed change: the path the card headers plus what happens to it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../bindings/")]
pub struct FileChange {
    /// The path the card headers - the destination for a rename/create, the
    /// target for an edit/delete. The card's file-path header.
    pub path: String,
    /// What happens to the file.
    pub op: FileOp,
}

impl FileChange {
    /// A move/rename of `from` to `to`. The bytes are unchanged, so the card
    /// shows the path move, not a content diff. This is what the built `fs.move`
    /// action proposes.
    pub fn rename(from: impl Into<String>, to: impl Into<String>) -> Self {
        Self {
            path: to.into(),
            op: FileOp::Rename { from: from.into() },
        }
    }

    /// A new file at `path` with `content` (the diff renders as all-additions).
    pub fn create(path: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            op: FileOp::Create {
                content: content.into(),
            },
        }
    }

    /// A deletion of `path`. `prior` is the file's body before deletion when
    /// known (for the removed-lines view), else empty.
    pub fn delete(path: impl Into<String>, prior: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            op: FileOp::Delete {
                prior: prior.into(),
            },
        }
    }

    /// A content edit of `path`, carried as a unified diff. Reserved - no built
    /// action emits this yet (it lands with file-edit effects).
    pub fn edit(path: impl Into<String>, unified_diff: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            op: FileOp::Edit {
                unified_diff: unified_diff.into(),
            },
        }
    }
}

/// What happens to a file. Internally tagged on `op` (like the artifact payload's
/// `kind`) so the frontend branches on the tag and the card picks its body view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../bindings/")]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum FileOp {
    /// Moved/renamed from `from` to the change's `path`. The bytes are unchanged,
    /// so there is no content diff; the card shows the path move. Produced today
    /// from the built `fs.move` action.
    Rename {
        /// The original path the file moved from.
        from: String,
    },
    /// A new file created with `content` (the diff renders as all-additions).
    Create {
        /// The new file's full content.
        content: String,
    },
    /// An existing file deleted (the diff renders as all-removals).
    Delete {
        /// The file's body before deletion when known, else empty.
        prior: String,
    },
    /// A content edit shown as a unified diff. Reserved for when file-edit
    /// effects land; not produced by any built action yet.
    Edit {
        /// The unified-diff text (the Shiki-highlighted card body).
        unified_diff: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    #[test]
    fn a_rename_serialises_with_the_destination_as_path_and_the_tagged_op() {
        let c = FileChange::rename("/dl/report.pdf", "/dl/2026/report.pdf");
        let v: Value = serde_json::from_str(&serde_json::to_string(&c).unwrap()).unwrap();
        assert_eq!(
            v,
            json!({
                "path": "/dl/2026/report.pdf",
                "op": { "op": "rename", "from": "/dl/report.pdf" }
            })
        );
    }

    #[test]
    fn each_op_round_trips() {
        for c in [
            FileChange::rename("/a", "/b"),
            FileChange::create("/c", "new body\n"),
            FileChange::delete("/d", "old body\n"),
            FileChange::edit("/e", "@@ -1 +1 @@\n-old\n+new\n"),
        ] {
            let bytes = serde_json::to_vec(&c).unwrap();
            let back: FileChange = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(c, back);
        }
    }

    #[test]
    fn a_change_set_carries_multiple_files() {
        let set = FileChangeSet::new(vec![
            FileChange::rename("/x", "/y"),
            FileChange::create("/z", "z\n"),
        ]);
        assert_eq!(set.changes.len(), 2);
        let back: FileChangeSet =
            serde_json::from_slice(&serde_json::to_vec(&set).unwrap()).unwrap();
        assert_eq!(set, back);

        // The single-change convenience is one entry.
        assert_eq!(FileChangeSet::single(FileChange::rename("/p", "/q")).changes.len(), 1);
    }
}

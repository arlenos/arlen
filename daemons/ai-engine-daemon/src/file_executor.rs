//! The filesystem forward executor (ai-act-layer-plan.md §⟳): the ACT layer's live
//! non-graph acts, `fs.move` and `fs.trash`, over the already-built capture/enact/
//! undo-signer machinery. `fs.trash` moves the entity into the freedesktop home
//! trash and captures a trash-aware inverse (undo restores it AND cleans the
//! `.trashinfo`), so a reversible delete leaves no orphaned trash-view entry.
//!
//! It mirrors [`crate::write_executor::GraphWriteExecutor`] exactly: tool-check ->
//! per-call executor-live gate (fail-closed) -> validate the required args ->
//! mint an op id -> S13 audit-before-act (fail-closed) -> WRITE-AHEAD capture the
//! inverse -> perform -> submit the compensation to the separate-uid undo signer.
//! `fs.move` is gate-classified `ReversibleAction`; its inverse is a
//! [`arlen_ai_undo_core::effect_model::InverseReceipt::RestorePath`] enacted by
//! [`crate::undo_enact::enact_restore_path`], so an undo moves the file back.
//!
//! The input contract is `{from, to}`, both CANONICAL-ABSOLUTE paths (the same form
//! [`crate::undo_enact::inverse_of_move`] and the enact path require); a relative or
//! `..`-bearing path is refused fail-closed, never guessed.
//!
//! NB unlike `graph.write`, filesystem compensation has no in-memory session store
//! yet: the graph [`crate::compensation::CompensationStore`] is graph-only by design
//! (its own doc: a filesystem inverse "belongs to a different undo path"). So the
//! durable, HMAC-chained undo signer IS this executor's compensation record; the
//! in-memory session-store parallel (for the activity-view undo trigger) is the
//! documented follow-up.
//!
//! ## Confinement posture (accepted: single-uid, reversibility-is-the-net)
//!
//! This executor does NOT confine which paths it may move. The Authorize proof
//! binds the exact `{from, to}` pair (a proof minted for one pair cannot execute a
//! different one), but it does not bound them to the user's home or the session's
//! project anchor. That is the design's deliberate posture for the reversible act
//! tier (ai-act-layer-plan.md, "Reversible autonomy still audits + is revocable"):
//! the move runs as the single session uid, hits the audit ledger, shows in the
//! pull activity view, is undoable (the `RestorePath` compensation) and the standing
//! grant is revocable. The blast radius is "any file this uid may rename", bounded
//! by `executor_live` (default off), the registered-tools-only dispatch and the
//! one-time args-bound proof, not by a filesystem scope.
//!
//! The path gate is SYNTACTIC (`CanonicalPath`: absolute, no `.`/`..`), not
//! symlink-resolved, so a component that is itself a symlink resolves wherever it
//! points at rename time. That is acceptable under the unconfined posture above;
//! should a future scope gate be added it MUST be driven off a symlink-resolved
//! path, or a symlinked parent would bypass it.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use ai_engine_contract::{ContractError, Execute, ExecuteOutcome};
use arlen_ai_core::audit::behaviour_action_event;
use arlen_ai_undo_core::effect_model::{CanonicalPath, InverseReceipt};
pub(crate) use arlen_freedesktop_trash::{
    home_trash_dir, rename_noreplace, trash_into, RenameError, TrashError,
};
use arlen_ai_undo_core::undo_log::UndoEntry;
use async_trait::async_trait;
use audit_proto::sink::AuditSink;

use crate::dispatch::Executor;
use crate::session::SessionGrant;

/// The relocation act.
const FS_MOVE_TOOL: &str = "fs.move";

/// The reversible-delete act (freedesktop home trash).
const FS_TRASH_TOOL: &str = "fs.trash";

/// The create act: write a NEW file with content; undo deletes exactly that file.
pub(crate) const FS_CREATE_TOOL: &str = "fs.create";

/// The forward producer for the reversible filesystem acts (`fs.move`, `fs.trash`).
/// Audit + undo-signer are optional (tests that exercise only the mechanics omit
/// them); the daemon always wires both. The same instance registers under both tool
/// names in the [`crate::proxy_executor::ProxyExecutor`].
pub struct FileSystemExecutor {
    audit: Option<Arc<dyn AuditSink>>,
    undo_signer: Option<PathBuf>,
    executor_live: fn() -> bool,
    /// A fixed home-trash root (tests, so a trash does not touch the developer's
    /// real `~/.local/share/Trash`); production leaves it `None` and resolves the
    /// XDG home trash per call.
    trash_root: Option<PathBuf>,
}

impl Default for FileSystemExecutor {
    fn default() -> Self {
        Self::new()
    }
}

impl FileSystemExecutor {
    /// Build the executor, gated by the on-disk `[agent] executor_live` per call,
    /// with no audit sink or undo signer yet.
    pub fn new() -> Self {
        Self {
            audit: None,
            undo_signer: None,
            executor_live: crate::engine_config::may_act,
            trash_root: None,
        }
    }

    /// Override the executor-live gate with a fixed source (tests, so a move does
    /// not depend on the developer's `ai.toml`).
    pub fn with_executor_live_gate(mut self, executor_live: fn() -> bool) -> Self {
        self.executor_live = executor_live;
        self
    }

    /// Pin the home-trash root (tests, so `fs.trash` uses a temp dir, not the
    /// developer's real trash).
    pub fn with_trash_root(mut self, root: PathBuf) -> Self {
        self.trash_root = Some(root);
        self
    }

    /// Attach the audit sink so the move is recorded content-free BEFORE it applies
    /// (S13 audit-before-act); a ledger that cannot record the intent refuses it.
    pub fn with_audit(mut self, audit: Arc<dyn AuditSink>) -> Self {
        self.audit = Some(audit);
        self
    }

    /// Attach the undo-signer socket so the move's `RestorePath` compensation is
    /// persisted to the signed, HMAC-chained undo log (best-effort).
    pub fn with_undo_signer(mut self, socket: PathBuf) -> Self {
        self.undo_signer = Some(socket);
        self
    }
}

#[async_trait]
impl Executor for FileSystemExecutor {
    async fn execute(&self, req: &Execute, _grant: &SessionGrant) -> ExecuteOutcome {
        match req.tool_name.as_str() {
            FS_MOVE_TOOL => self.execute_move(req).await,
            FS_TRASH_TOOL => self.execute_trash(req).await,
            FS_CREATE_TOOL => self.execute_create(req).await,
            other => ExecuteOutcome::Error {
                code: ContractError::UnknownTool,
                message: format!("{other} is not a filesystem tool this daemon runs"),
            },
        }
    }
}

impl FileSystemExecutor {
    /// Relocate `from` -> `to` reversibly (the `RestorePath` inverse moves it back).
    async fn execute_move(&self, req: &Execute) -> ExecuteOutcome {
        // Executor-live gate, re-read PER CALL (fail-closed): even an authorized
        // proof cannot move a file once executor_live is off; nothing is audited or
        // performed when it is off.
        if !(self.executor_live)() {
            return ExecuteOutcome::Error {
                code: ContractError::ExecutionFailed,
                message: "fs.move is not permitted: the executor is not live".to_string(),
            };
        }
        // Both paths are required; a missing one is a malformed request, never guessed.
        let field = |k: &str| req.tool_input.get(k).and_then(|v| v.as_str()).map(str::to_string);
        let (Some(from), Some(to)) = (field("from"), field("to")) else {
            return ExecuteOutcome::Error {
                code: ContractError::InvalidArguments,
                message: "fs.move needs string from + to (both canonical-absolute paths) in the \
                          tool input"
                    .to_string(),
            };
        };
        // WRITE-AHEAD: capture the inverse (RestorePath - undo moves `to` back to
        // `from`) BEFORE the move. A non-canonical-absolute path is refused
        // fail-closed here (never a relative / `..` path).
        let Some(inverse) = crate::undo_enact::inverse_of_move(&from, &to) else {
            return ExecuteOutcome::Error {
                code: ContractError::InvalidArguments,
                message: "fs.move paths must be canonical-absolute".to_string(),
            };
        };
        let op_id = match crate::write_executor::mint_op_id() {
            Ok(id) => id,
            Err(e) => {
                return ExecuteOutcome::Error {
                    code: ContractError::ExecutionFailed,
                    message: format!("could not mint an op id: {e}"),
                }
            }
        };
        // S13 audit-before-act: record the move intent content-free BEFORE it
        // applies, correlated by the daemon's own op id. Fail closed - a ledger that
        // cannot record the intent refuses the move.
        if let Some(audit) = &self.audit {
            let event = behaviour_action_event(FS_MOVE_TOOL, "fs-move", &op_id);
            if audit.submit(event).await.is_err() {
                return ExecuteOutcome::Error {
                    code: ContractError::ExecutionFailed,
                    message: "audit ledger unavailable; fs.move refused".to_string(),
                };
            }
        }
        // Perform the move ATOMICALLY with no-clobber (`RENAME_NOREPLACE`). A plain
        // `fs::rename` OVERWRITES an existing destination, and a check-then-rename
        // (`exists()` then `rename`) leaves a TOCTOU window in which a racing
        // same-uid process re-creates `to` and the rename clobbers it - either way
        // destroying a file the `RestorePath` inverse could never restore. The
        // kernel refuses to create `to` if it already exists, so the reversibility
        // invariant holds against the race. A cross-filesystem move (`EXDEV`) or a
        // filesystem without no-clobber-rename support is refused, never softened
        // into a clobbering fallback.
        match rename_noreplace(&from, &to) {
            Ok(()) => {}
            Err(RenameError::DestinationExists) => {
                return ExecuteOutcome::Error {
                    code: ContractError::ExecutionFailed,
                    message: "fs.move refused: the destination already exists (a clobber is not \
                              reversible)"
                        .to_string(),
                };
            }
            Err(RenameError::Unsupported) => {
                return ExecuteOutcome::Error {
                    code: ContractError::ExecutionFailed,
                    message: "fs.move refused: this filesystem cannot perform an atomic no-clobber \
                              move"
                        .to_string(),
                };
            }
            Err(RenameError::Other(m)) => {
                return ExecuteOutcome::Error {
                    code: ContractError::ExecutionFailed,
                    message: format!("fs.move failed: {m}"),
                };
            }
        }
        // Register the compensation to the durable, HMAC-chained undo signer (the
        // captured RestorePath, keyed on this op id). Best-effort: a signer that is
        // absent or failing does not fail an already-committed, reversible move.
        if let Some(signer) = &self.undo_signer {
            if signer.exists() {
                let entry = UndoEntry {
                    op_id: op_id.clone(),
                    correlation_id: op_id.clone(),
                    inverse,
                };
                if let Err(e) = crate::undo_signer::submit_created(signer, &entry).await {
                    tracing::debug!("undo signer submit failed for fs.move: {e}");
                }
            }
        }
        ExecuteOutcome::Ok {
            result: serde_json::json!({ "op_id": op_id, "from": from, "to": to }),
        }
    }

    /// Trash `path` into the freedesktop home trash reversibly. Writes the
    /// `.trashinfo` sidecar and moves the entity into `Trash/files/` atomically
    /// (no-clobber), capturing a [`InverseReceipt::RestoreFromTrash`] so an undo
    /// restores the file AND cleans the sidecar. Same discipline as `fs.move`:
    /// per-call executor-live gate, S13 audit-before-act, write-ahead inverse.
    async fn execute_trash(&self, req: &Execute) -> ExecuteOutcome {
        if !(self.executor_live)() {
            return exec_err(
                ContractError::ExecutionFailed,
                "fs.trash is not permitted: the executor is not live",
            );
        }
        let Some(path) = req.tool_input.get("path").and_then(|v| v.as_str()).map(str::to_string)
        else {
            return exec_err(
                ContractError::InvalidArguments,
                "fs.trash needs a string path (canonical-absolute) in the tool input",
            );
        };
        // The entity's original location - the inverse restores here. A
        // non-canonical-absolute path is refused fail-closed (never a `..`/relative).
        let Some(original) = CanonicalPath::new(&path) else {
            return exec_err(
                ContractError::InvalidArguments,
                "fs.trash path must be canonical-absolute",
            );
        };
        let Some(base_name) = Path::new(&path).file_name().and_then(|n| n.to_str()) else {
            return exec_err(ContractError::InvalidArguments, "fs.trash path has no file name");
        };
        let base_name = base_name.to_string();
        // Resolve the home trash (test override or XDG). Fail closed with no root.
        let Some(trash) = self.trash_root.clone().or_else(home_trash_dir) else {
            return exec_err(
                ContractError::ExecutionFailed,
                "fs.trash: no writable home trash ($HOME/$XDG_DATA_HOME unset)",
            );
        };
        let op_id = match crate::write_executor::mint_op_id() {
            Ok(id) => id,
            Err(e) => {
                return exec_err(
                    ContractError::ExecutionFailed,
                    format!("could not mint an op id: {e}"),
                )
            }
        };
        // S13 audit-before-act: record the trash intent content-free BEFORE any side
        // effect (sidecar write or move), correlated by the op id. Fail closed.
        if let Some(audit) = &self.audit {
            let event = behaviour_action_event(FS_TRASH_TOOL, "fs-trash", &op_id);
            if audit.submit(event).await.is_err() {
                return exec_err(
                    ContractError::ExecutionFailed,
                    "audit ledger unavailable; fs.trash refused",
                );
            }
        }
        let files_dir = trash.join("files");
        let info_dir = trash.join("info");
        if let Err(e) = std::fs::create_dir_all(&files_dir)
            .and_then(|()| std::fs::create_dir_all(&info_dir))
        {
            return exec_err(
                ContractError::ExecutionFailed,
                format!("fs.trash: could not prepare the trash directory: {e}"),
            );
        }
        // Reserve a unique slot, write the sidecar (info-first per spec), and move
        // the entity in atomically. The canonical trashed + sidecar paths are
        // validated BEFORE the move inside `trash_into`, so once the move lands the
        // inverse is always constructible (the write-ahead guarantee).
        let slot = match trash_into(&files_dir, &info_dir, &base_name, &path) {
            Ok(s) => s,
            Err(TrashError::NotFound) => {
                return exec_err(ContractError::ExecutionFailed, "fs.trash: the path does not exist")
            }
            Err(TrashError::Unsupported) => {
                return exec_err(
                    ContractError::ExecutionFailed,
                    "fs.trash refused: this filesystem cannot perform an atomic no-clobber move",
                )
            }
            Err(TrashError::NoSlot) => {
                return exec_err(
                    ContractError::ExecutionFailed,
                    "fs.trash: could not find a free trash name",
                )
            }
            Err(TrashError::NonCanonical) => {
                return exec_err(
                    ContractError::ExecutionFailed,
                    "fs.trash: the resolved trash path is not canonical",
                )
            }
            Err(TrashError::Io(m)) => {
                return exec_err(ContractError::ExecutionFailed, format!("fs.trash failed: {m}"))
            }
        };
        let (trashed, trash_info) = slot.into_parts();
        let trashed_display = trashed.as_str().to_string();
        let inverse = InverseReceipt::RestoreFromTrash {
            original,
            trashed,
            trash_info,
        };
        // Register the compensation to the durable, HMAC-chained undo signer.
        // Best-effort: an absent/failing signer never fails a committed trash.
        if let Some(signer) = &self.undo_signer {
            if signer.exists() {
                let entry = UndoEntry {
                    op_id: op_id.clone(),
                    correlation_id: op_id.clone(),
                    inverse,
                };
                if let Err(e) = crate::undo_signer::submit_created(signer, &entry).await {
                    tracing::debug!("undo signer submit failed for fs.trash: {e}");
                }
            }
        }
        ExecuteOutcome::Ok {
            result: serde_json::json!({ "op_id": op_id, "path": path, "trashed": trashed_display }),
        }
    }

    /// Create a NEW file at `path` with `content`, reversibly. The undo is a
    /// [`InverseReceipt::DeleteCreated`] (identity-bound: it deletes the file only
    /// while it still holds these bytes, never a later replacement).
    ///
    /// The create is NO-CLOBBER by construction (`O_EXCL`): it refuses if the path
    /// already exists, so it can never overwrite a file the `DeleteCreated` inverse
    /// could not restore. `content` is UTF-8 text (a binary create is a follow-up).
    ///
    /// `fs.create` is gate-classified `Confirm` (a create can be a persistence /
    /// code-execution vector - a new `~/.bashrc`, autostart entry or systemd-user
    /// unit - and "reversible" does not bound that at-write harm), so the user
    /// approves each create through the consent flow BEFORE this runs; the executor
    /// itself needs no path denylist (the confirm is the control). Same single-uid
    /// posture as `fs.move`: it may create at any canonical-absolute path this uid
    /// can write, bounded by the confirm + executor_live.
    async fn execute_create(&self, req: &Execute) -> ExecuteOutcome {
        if !(self.executor_live)() {
            return exec_err(
                ContractError::ExecutionFailed,
                "fs.create is not permitted: the executor is not live",
            );
        }
        let field = |k: &str| req.tool_input.get(k).and_then(|v| v.as_str()).map(str::to_string);
        let (Some(path), Some(content)) = (field("path"), field("content")) else {
            return exec_err(
                ContractError::InvalidArguments,
                "fs.create needs a string path (canonical-absolute) + content in the tool input",
            );
        };
        // The DeleteCreated inverse relies on a canonical-absolute path; refuse a
        // relative / `..` path fail-closed.
        if CanonicalPath::new(&path).is_none() {
            return exec_err(
                ContractError::InvalidArguments,
                "fs.create path must be canonical-absolute",
            );
        }
        let op_id = match crate::write_executor::mint_op_id() {
            Ok(id) => id,
            Err(e) => {
                return exec_err(
                    ContractError::ExecutionFailed,
                    format!("could not mint an op id: {e}"),
                )
            }
        };
        // S13 audit-before-act: record the create intent content-free BEFORE the
        // file is written, correlated by the op id. Fail closed.
        if let Some(audit) = &self.audit {
            let event = behaviour_action_event(FS_CREATE_TOOL, "fs-create", &op_id);
            if audit.submit(event).await.is_err() {
                return exec_err(
                    ContractError::ExecutionFailed,
                    "audit ledger unavailable; fs.create refused",
                );
            }
        }
        // Create NEW (O_EXCL): refuse if the path exists (an overwrite is not what
        // DeleteCreated bounds), so the create can never destroy existing data.
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut f) => {
                use std::io::Write;
                if let Err(e) = f.write_all(content.as_bytes()).and_then(|()| f.sync_all()) {
                    // Remove the partial file we created so a failed create leaves
                    // nothing behind.
                    let _ = std::fs::remove_file(&path);
                    return exec_err(
                        ContractError::ExecutionFailed,
                        format!("fs.create failed writing content: {e}"),
                    );
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                return exec_err(
                    ContractError::ExecutionFailed,
                    "fs.create refused: the path already exists (a create must not overwrite)",
                )
            }
            Err(e) => {
                return exec_err(ContractError::ExecutionFailed, format!("fs.create failed: {e}"))
            }
        }
        // Capture the DeleteCreated inverse AFTER the write (it fingerprints the
        // created bytes), then register it to the durable undo signer. A capture
        // failure never fails the committed create (the file is a new, user-
        // deletable file); it just leaves this create without an engine-undo record.
        match crate::undo_enact::capture_created(&path) {
            Ok(inverse) => {
                if let Some(signer) = &self.undo_signer {
                    if signer.exists() {
                        let entry = UndoEntry {
                            op_id: op_id.clone(),
                            correlation_id: op_id.clone(),
                            inverse,
                        };
                        if let Err(e) = crate::undo_signer::submit_created(signer, &entry).await {
                            tracing::debug!("undo signer submit failed for fs.create: {e}");
                        }
                    }
                }
            }
            Err(e) => {
                tracing::debug!("fs.create: could not capture the delete inverse for {path}: {e}");
            }
        }
        ExecuteOutcome::Ok {
            result: serde_json::json!({ "op_id": op_id, "path": path }),
        }
    }
}

/// A short constructor for an error outcome.
fn exec_err(code: ContractError, message: impl Into<String>) -> ExecuteOutcome {
    ExecuteOutcome::Error { code, message: message.into() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ai_engine_contract::{CapabilityContext, ReadTier};
    use std::sync::atomic::{AtomicU64, Ordering};

    /// A fresh canonical-absolute temp dir (the crate has no tempfile dev-dep, so
    /// this mirrors `undo_enact`'s own helper). Canonicalized so the paths are the
    /// canonical-absolute form the executor requires.
    fn tmp() -> std::path::PathBuf {
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let d = std::env::temp_dir().join(format!("fs-exec-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d.canonicalize().unwrap()
    }

    fn grant() -> SessionGrant {
        SessionGrant {
            capability_context: CapabilityContext { generic_tools: vec![], proxy_tools: vec![] },
            project_anchor: None,
            read_tier: ReadTier::None,
            externally_triggered: false,
            pid: 1,
        }
    }

    fn move_req(from: &str, to: &str) -> Execute {
        Execute {
            tool_name: FS_MOVE_TOOL.to_string(),
            tool_input: serde_json::json!({ "from": from, "to": to }),
            proof: None,
        }
    }

    fn live() -> FileSystemExecutor {
        FileSystemExecutor::new().with_executor_live_gate(|| true)
    }

    fn trash_req(path: &str) -> Execute {
        Execute {
            tool_name: FS_TRASH_TOOL.to_string(),
            tool_input: serde_json::json!({ "path": path }),
            proof: None,
        }
    }

    fn create_req(path: &str, content: &str) -> Execute {
        Execute {
            tool_name: FS_CREATE_TOOL.to_string(),
            tool_input: serde_json::json!({ "path": path, "content": content }),
            proof: None,
        }
    }

    #[tokio::test]
    async fn a_live_create_writes_the_new_file() {
        let dir = tmp();
        let target = dir.join("made.txt");
        let out = live().execute(&create_req(target.to_str().unwrap(), "agent output"), &grant()).await;
        match out {
            ExecuteOutcome::Ok { result } => {
                assert_eq!(result["path"], target.to_str().unwrap());
                assert!(result["op_id"].as_str().is_some_and(|s| !s.is_empty()));
            }
            other => panic!("expected Ok, got {other:?}"),
        }
        assert_eq!(std::fs::read(&target).unwrap(), b"agent output", "the file was created with its content");
    }

    #[tokio::test]
    async fn a_create_round_trips_through_its_captured_delete_inverse() {
        let dir = tmp();
        let target = dir.join("note.txt");
        live().execute(&create_req(target.to_str().unwrap(), "content"), &grant()).await;
        assert!(target.exists());
        // The DeleteCreated inverse the executor captures deletes exactly this file.
        let inverse = crate::undo_enact::capture_created(target.to_str().unwrap()).unwrap();
        crate::undo_enact::enact_inverse(&inverse).unwrap();
        assert!(!target.exists(), "undo deleted the created file");
    }

    #[tokio::test]
    async fn a_create_over_an_existing_file_is_refused_no_clobber() {
        // A create must never overwrite (DeleteCreated can't restore the clobbered
        // original); refuse if the path exists, leaving it untouched.
        let dir = tmp();
        let target = dir.join("existing.txt");
        std::fs::write(&target, b"do not clobber").unwrap();
        let out = live().execute(&create_req(target.to_str().unwrap(), "new"), &grant()).await;
        assert!(matches!(out, ExecuteOutcome::Error { .. }));
        assert_eq!(std::fs::read(&target).unwrap(), b"do not clobber", "the existing file is untouched");
    }

    #[tokio::test]
    async fn a_create_is_refused_when_the_executor_is_not_live() {
        let dir = tmp();
        let target = dir.join("nope.txt");
        let exec = FileSystemExecutor::new().with_executor_live_gate(|| false);
        let out = exec.execute(&create_req(target.to_str().unwrap(), "x"), &grant()).await;
        assert!(matches!(out, ExecuteOutcome::Error { .. }));
        assert!(!target.exists(), "nothing created when the executor is off");
    }

    #[tokio::test]
    async fn a_relative_create_path_is_refused() {
        match live().execute(&create_req("relative/x.txt", "x"), &grant()).await {
            ExecuteOutcome::Error { code, .. } => assert_eq!(code, ContractError::InvalidArguments),
            other => panic!("expected InvalidArguments, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_create_missing_content_is_a_malformed_request() {
        let req = Execute {
            tool_name: FS_CREATE_TOOL.to_string(),
            tool_input: serde_json::json!({ "path": "/tmp/x.txt" }),
            proof: None,
        };
        match live().execute(&req, &grant()).await {
            ExecuteOutcome::Error { code, .. } => assert_eq!(code, ContractError::InvalidArguments),
            other => panic!("expected InvalidArguments, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_live_trash_moves_the_file_into_the_trash_with_a_sidecar() {
        let dir = tmp();
        let trash_root = dir.join("trash");
        let target = dir.join("doc.txt");
        std::fs::write(&target, b"payload").unwrap();

        let exec = live().with_trash_root(trash_root.clone());
        let out = exec.execute(&trash_req(target.to_str().unwrap()), &grant()).await;
        let trashed = match out {
            ExecuteOutcome::Ok { result } => {
                assert_eq!(result["path"], target.to_str().unwrap());
                assert!(result["op_id"].as_str().is_some_and(|s| !s.is_empty()));
                result["trashed"].as_str().unwrap().to_string()
            }
            other => panic!("expected Ok, got {other:?}"),
        };
        assert!(!target.exists(), "the original is gone from its place");
        assert_eq!(std::fs::read(&trashed).unwrap(), b"payload", "the file is in the trash");
        // The sidecar exists and names the origin.
        let info = trash_root.join("info/doc.txt.trashinfo");
        let info_body = std::fs::read_to_string(&info).unwrap();
        assert!(info_body.contains("[Trash Info]"), "sidecar header: {info_body}");
        assert!(info_body.contains("DeletionDate="), "sidecar date: {info_body}");
    }

    #[tokio::test]
    async fn a_trash_round_trips_through_its_captured_inverse() {
        // The RestoreFromTrash inverse the trash captures actually restores the file
        // AND cleans the sidecar - the undo a later restore runs.
        let dir = tmp();
        let trash_root = dir.join("trash");
        let target = dir.join("notes.md");
        std::fs::write(&target, b"body").unwrap();

        let exec = live().with_trash_root(trash_root.clone());
        exec.execute(&trash_req(target.to_str().unwrap()), &grant()).await;
        let trashed = trash_root.join("files/notes.md");
        let info = trash_root.join("info/notes.md.trashinfo");
        assert!(trashed.exists() && info.exists());

        let inverse = InverseReceipt::RestoreFromTrash {
            original: CanonicalPath::new(target.to_str().unwrap()).unwrap(),
            trashed: CanonicalPath::new(trashed.to_str().unwrap()).unwrap(),
            trash_info: CanonicalPath::new(info.to_str().unwrap()).unwrap(),
        };
        crate::undo_enact::enact_inverse(&inverse).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"body", "the file is back");
        assert!(!trashed.exists(), "the trash copy is gone");
        assert!(!info.exists(), "the sidecar is cleaned - no orphan");
    }

    #[tokio::test]
    async fn trashing_a_same_named_file_dedups_the_name() {
        let dir = tmp();
        let trash_root = dir.join("trash");
        let exec = live().with_trash_root(trash_root.clone());

        // Trash two files that share a base name from different places.
        let a = dir.join("a/report.txt");
        let b = dir.join("b/report.txt");
        std::fs::create_dir_all(a.parent().unwrap()).unwrap();
        std::fs::create_dir_all(b.parent().unwrap()).unwrap();
        std::fs::write(&a, b"first").unwrap();
        std::fs::write(&b, b"second").unwrap();

        exec.execute(&trash_req(a.to_str().unwrap()), &grant()).await;
        exec.execute(&trash_req(b.to_str().unwrap()), &grant()).await;
        // Both survive in the trash under distinct names, neither clobbered.
        assert_eq!(std::fs::read(trash_root.join("files/report.txt")).unwrap(), b"first");
        assert_eq!(std::fs::read(trash_root.join("files/report.1.txt")).unwrap(), b"second");
    }

    #[tokio::test]
    async fn a_trash_is_refused_when_the_executor_is_not_live() {
        let dir = tmp();
        let target = dir.join("x.txt");
        std::fs::write(&target, b"h").unwrap();
        let exec = FileSystemExecutor::new()
            .with_executor_live_gate(|| false)
            .with_trash_root(dir.join("trash"));
        let out = exec.execute(&trash_req(target.to_str().unwrap()), &grant()).await;
        assert!(matches!(out, ExecuteOutcome::Error { .. }));
        assert!(target.exists(), "nothing trashed when the executor is off");
    }

    #[tokio::test]
    async fn a_relative_trash_path_is_refused() {
        let exec = live().with_trash_root(tmp());
        match exec.execute(&trash_req("relative/x.txt"), &grant()).await {
            ExecuteOutcome::Error { code, .. } => assert_eq!(code, ContractError::InvalidArguments),
            other => panic!("expected InvalidArguments, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_live_move_relocates_the_file_and_reports_the_op() {
        let dir = tmp();
        let from = dir.join("a.txt");
        let to = dir.join("b.txt");
        std::fs::write(&from, b"hello").unwrap();

        let out = live()
            .execute(&move_req(from.to_str().unwrap(), to.to_str().unwrap()), &grant())
            .await;
        match out {
            ExecuteOutcome::Ok { result } => {
                assert_eq!(result["from"], from.to_str().unwrap());
                assert_eq!(result["to"], to.to_str().unwrap());
                assert!(result["op_id"].as_str().is_some_and(|s| !s.is_empty()));
            }
            other => panic!("expected Ok, got {other:?}"),
        }
        assert!(!from.exists(), "the source is gone");
        assert_eq!(std::fs::read(&to).unwrap(), b"hello", "the file moved with its content");
    }

    #[tokio::test]
    async fn the_captured_inverse_restores_the_move() {
        // The RestorePath the executor captures actually undoes the move via the
        // built enact path - the round trip a later undo runs.
        let dir = tmp();
        let from = dir.join("orig.txt");
        let to = dir.join("moved.txt");
        std::fs::write(&from, b"x").unwrap();

        live()
            .execute(&move_req(from.to_str().unwrap(), to.to_str().unwrap()), &grant())
            .await;
        let inverse = crate::undo_enact::inverse_of_move(from.to_str().unwrap(), to.to_str().unwrap())
            .unwrap();
        crate::undo_enact::enact_inverse(&inverse).unwrap();
        assert!(from.exists(), "undo moved the file back to its source");
        assert!(!to.exists());
    }

    #[tokio::test]
    async fn a_move_is_refused_when_the_executor_is_not_live() {
        let dir = tmp();
        let from = dir.join("a.txt");
        let to = dir.join("b.txt");
        std::fs::write(&from, b"h").unwrap();

        let exec = FileSystemExecutor::new().with_executor_live_gate(|| false);
        let out = exec
            .execute(&move_req(from.to_str().unwrap(), to.to_str().unwrap()), &grant())
            .await;
        assert!(matches!(out, ExecuteOutcome::Error { .. }));
        assert!(from.exists(), "nothing moved when the executor is off");
    }

    #[tokio::test]
    async fn a_move_onto_an_existing_file_is_refused_no_clobber() {
        // A move whose destination exists would clobber it irreversibly; refuse it,
        // leaving BOTH files intact.
        let dir = tmp();
        let from = dir.join("a.txt");
        let to = dir.join("b.txt");
        std::fs::write(&from, b"src").unwrap();
        std::fs::write(&to, b"dst").unwrap();

        let out = live()
            .execute(&move_req(from.to_str().unwrap(), to.to_str().unwrap()), &grant())
            .await;
        assert!(matches!(out, ExecuteOutcome::Error { .. }));
        assert_eq!(std::fs::read(&from).unwrap(), b"src", "the source is untouched");
        assert_eq!(std::fs::read(&to).unwrap(), b"dst", "the destination was not clobbered");
    }

    #[tokio::test]
    async fn a_move_onto_an_existing_symlink_target_is_refused() {
        // `RENAME_NOREPLACE` refuses when `to` resolves to an existing file, so a
        // symlink at `to` pointing at a real file cannot be used to clobber it.
        let dir = tmp();
        let from = dir.join("a.txt");
        let real = dir.join("real.txt");
        let link = dir.join("link.txt");
        std::fs::write(&from, b"src").unwrap();
        std::fs::write(&real, b"real").unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();

        let out = live()
            .execute(&move_req(from.to_str().unwrap(), link.to_str().unwrap()), &grant())
            .await;
        assert!(matches!(out, ExecuteOutcome::Error { .. }));
        assert_eq!(std::fs::read(&from).unwrap(), b"src", "the source is untouched");
        assert_eq!(std::fs::read(&real).unwrap(), b"real", "the real file was not clobbered");
    }

    #[tokio::test]
    async fn a_missing_arg_is_a_malformed_request() {
        let req = Execute {
            tool_name: FS_MOVE_TOOL.to_string(),
            tool_input: serde_json::json!({ "from": "/a.txt" }),
            proof: None,
        };
        match live().execute(&req, &grant()).await {
            ExecuteOutcome::Error { code, .. } => assert_eq!(code, ContractError::InvalidArguments),
            other => panic!("expected InvalidArguments, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_relative_path_is_refused_before_moving() {
        // A non-canonical-absolute path fails closed at the inverse capture, never
        // guessed into a relative move.
        let req = move_req("relative/a.txt", "/tmp/b.txt");
        match live().execute(&req, &grant()).await {
            ExecuteOutcome::Error { code, .. } => assert_eq!(code, ContractError::InvalidArguments),
            other => panic!("expected InvalidArguments, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_foreign_tool_is_rejected() {
        let req = Execute {
            tool_name: "graph.write".to_string(),
            tool_input: serde_json::json!({}),
            proof: None,
        };
        match live().execute(&req, &grant()).await {
            ExecuteOutcome::Error { code, .. } => assert_eq!(code, ContractError::UnknownTool),
            other => panic!("expected UnknownTool, got {other:?}"),
        }
    }
}

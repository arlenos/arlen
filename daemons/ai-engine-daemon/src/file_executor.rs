//! The filesystem forward executor (ai-act-layer-plan.md §⟳): the ACT layer's one
//! live non-graph act, `fs.move`, over the already-built capture/enact/undo-signer
//! machinery.
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

use std::ffi::CString;
use std::path::PathBuf;
use std::sync::Arc;

use ai_engine_contract::{ContractError, Execute, ExecuteOutcome};
use arlen_ai_core::audit::behaviour_action_event;
use arlen_ai_undo_core::undo_log::UndoEntry;
use async_trait::async_trait;
use audit_proto::sink::AuditSink;

use crate::dispatch::Executor;
use crate::session::SessionGrant;

/// The one filesystem act this executor runs.
const FS_MOVE_TOOL: &str = "fs.move";

/// The forward producer for `fs.move`. Audit + undo-signer are optional (tests that
/// exercise only the move mechanics omit them); the daemon always wires both.
pub struct FileSystemExecutor {
    audit: Option<Arc<dyn AuditSink>>,
    undo_signer: Option<PathBuf>,
    executor_live: fn() -> bool,
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
            executor_live: crate::engine_config::executor_live,
        }
    }

    /// Override the executor-live gate with a fixed source (tests, so a move does
    /// not depend on the developer's `ai.toml`).
    pub fn with_executor_live_gate(mut self, executor_live: fn() -> bool) -> Self {
        self.executor_live = executor_live;
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
        if req.tool_name != FS_MOVE_TOOL {
            return ExecuteOutcome::Error {
                code: ContractError::UnknownTool,
                message: format!("{} is not a filesystem tool this daemon runs", req.tool_name),
            };
        }
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
}

/// Why an atomic no-clobber rename could not complete.
enum RenameError {
    /// `to` already exists; the kernel refused to clobber it (`EEXIST`).
    DestinationExists,
    /// The kernel or filesystem does not support `RENAME_NOREPLACE`. Refuse the
    /// move rather than fall back to a clobbering rename.
    Unsupported,
    /// Any other rename failure (`EXDEV`, permissions, a NUL in the path, ...).
    Other(String),
}

/// Rename `from` to `to`, refusing to overwrite an existing `to`
/// (`RENAME_NOREPLACE`). The kernel creates `to` only if it did not already
/// exist, so this closes the check-then-rename TOCTOU: a racing same-uid process
/// cannot make the move clobber (and thus irreversibly destroy) a file the
/// reversible tier promised to be able to restore. Both paths are canonical-
/// absolute, so `AT_FDCWD` is a placeholder the kernel ignores.
fn rename_noreplace(from: &str, to: &str) -> Result<(), RenameError> {
    let nul = |_| RenameError::Other("path contains an interior NUL byte".to_string());
    let cfrom = CString::new(from).map_err(nul)?;
    let cto = CString::new(to).map_err(nul)?;
    // SAFETY: both pointers are valid NUL-terminated C strings that outlive the
    // call; `renameat2` with `AT_FDCWD` and absolute paths ignores the dir fds.
    let rc = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            cfrom.as_ptr(),
            libc::AT_FDCWD,
            cto.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if rc == 0 {
        return Ok(());
    }
    let err = std::io::Error::last_os_error();
    match err.raw_os_error() {
        Some(libc::EEXIST) => Err(RenameError::DestinationExists),
        // The flag or the syscall is unavailable (old kernel / exotic fs).
        Some(libc::EINVAL | libc::ENOSYS | libc::EOPNOTSUPP) => Err(RenameError::Unsupported),
        _ => Err(RenameError::Other(err.to_string())),
    }
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

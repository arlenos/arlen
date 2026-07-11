//! The settings forward executor (ai-act-layer-plan.md §⟳): the ACT layer's
//! reversible scalar-setting write, `settings.set`, over the format-preserving
//! config editor and the `RestoreValue` capture/enact machinery.
//!
//! `settings.set` changes one scalar key in one config file under the user's Arlen
//! config (`~/.config/arlen`). It is confined by construction: the target is a bare
//! config filename (a single path component, no `/`/`.`/`..`), resolved under the
//! Arlen config dir, so a write can never escape it. Reversible: the executor
//! captures the prior value write-ahead (`RestoreValue`) BEFORE the write, so an
//! undo restores it (or removes the key if it was absent). Same discipline as
//! `fs.move`: per-call executor-live gate, S13 audit-before-act, write-ahead
//! inverse, undo-signer submit.
//!
//! ## Protected files (self-escalation guard)
//!
//! The AI master switches (`[ai] enabled`, `[agent] executor_live`, provider/model)
//! live in `~/.config/arlen/ai.toml` - the SAME directory this executor writes.
//! `settings.set` is gate-classified `ReversibleAction`, so under `executor_live` it
//! applies autonomously with no confirm; a `settings.set` to `ai.toml` would let the
//! agent tamper with its own gates (keep `executor_live` on, flip `enabled`, swap
//! the provider), the exact hole the separate-uid AI-switch daemon closes. So
//! `ai.toml` is a PROTECTED file this executor refuses outright: the AI master
//! switches change only through their own consent-gated daemon, never this path.

use std::path::PathBuf;
use std::sync::Arc;

use ai_engine_contract::{ContractError, Execute, ExecuteOutcome};
use arlen_ai_core::audit::behaviour_action_event;
use arlen_ai_undo_core::undo_log::UndoEntry;
use async_trait::async_trait;
use audit_proto::sink::AuditSink;

use crate::dispatch::Executor;
use crate::session::SessionGrant;
use crate::undo_enact::EnactError;

/// The scalar-setting write act.
const SETTINGS_SET_TOOL: &str = "settings.set";

/// Config files `settings.set` refuses to write: they gate the AI/security posture,
/// so they change only through their own consent-gated path, never an autonomous
/// reversible act. `ai.toml` holds the AI master switches, so a write to it would be
/// self-escalation.
const PROTECTED_CONFIG_FILES: &[&str] = &["ai.toml"];

/// The forward producer for `settings.set`. Audit + undo-signer are optional (tests
/// that exercise only the mechanics omit them); the daemon always wires both.
pub struct SettingsExecutor {
    audit: Option<Arc<dyn AuditSink>>,
    undo_signer: Option<PathBuf>,
    executor_live: fn() -> bool,
    /// A fixed Arlen config root (tests, so a set does not touch the developer's
    /// real `~/.config/arlen`); production leaves it `None` and resolves it per call.
    config_root: Option<PathBuf>,
}

impl Default for SettingsExecutor {
    fn default() -> Self {
        Self::new()
    }
}

impl SettingsExecutor {
    /// Build the executor, gated by the on-disk `[agent] executor_live` per call,
    /// with no audit sink or undo signer yet.
    pub fn new() -> Self {
        Self {
            audit: None,
            undo_signer: None,
            executor_live: crate::engine_config::executor_live,
            config_root: None,
        }
    }

    /// Override the executor-live gate with a fixed source (tests).
    pub fn with_executor_live_gate(mut self, executor_live: fn() -> bool) -> Self {
        self.executor_live = executor_live;
        self
    }

    /// Attach the audit sink so the write is recorded content-free BEFORE it applies
    /// (S13 audit-before-act); a ledger that cannot record the intent refuses it.
    pub fn with_audit(mut self, audit: Arc<dyn AuditSink>) -> Self {
        self.audit = Some(audit);
        self
    }

    /// Attach the undo-signer socket so the write's `RestoreValue` compensation is
    /// persisted to the signed, HMAC-chained undo log (best-effort).
    pub fn with_undo_signer(mut self, socket: PathBuf) -> Self {
        self.undo_signer = Some(socket);
        self
    }

    /// Pin the Arlen config root (tests, so `settings.set` uses a temp dir).
    pub fn with_config_root(mut self, root: PathBuf) -> Self {
        self.config_root = Some(root);
        self
    }
}

#[async_trait]
impl Executor for SettingsExecutor {
    async fn execute(&self, req: &Execute, _grant: &SessionGrant) -> ExecuteOutcome {
        if req.tool_name != SETTINGS_SET_TOOL {
            return err(
                ContractError::UnknownTool,
                format!("{} is not a settings tool this daemon runs", req.tool_name),
            );
        }
        // Executor-live gate, re-read PER CALL (fail-closed).
        if !(self.executor_live)() {
            return err(
                ContractError::ExecutionFailed,
                "settings.set is not permitted: the executor is not live",
            );
        }
        let field = |k: &str| req.tool_input.get(k).and_then(|v| v.as_str()).map(str::to_string);
        let (Some(file), Some(key), Some(value)) = (field("file"), field("key"), field("value"))
        else {
            return err(
                ContractError::InvalidArguments,
                "settings.set needs string file + key + value in the tool input",
            );
        };
        // The target must be a bare config filename (single component), so a write
        // can never escape the Arlen config dir.
        if !is_safe_config_filename(&file) {
            return err(
                ContractError::InvalidArguments,
                "settings.set file must be a bare config filename (no path components)",
            );
        }
        // Refuse the AI master-switch file: an autonomous write there is self-escalation.
        if PROTECTED_CONFIG_FILES.contains(&file.as_str()) {
            return err(
                ContractError::ExecutionFailed,
                "settings.set refused: this config file is protected (AI master switches)",
            );
        }
        if key.is_empty() {
            return err(ContractError::InvalidArguments, "settings.set key must be non-empty");
        }
        let Some(root) = self.config_root.clone().or_else(arlen_config_dir) else {
            return err(
                ContractError::ExecutionFailed,
                "settings.set: no Arlen config directory ($HOME/$XDG_CONFIG_HOME unset)",
            );
        };
        let resolved = root.join(&file);
        let Some(resolved_str) = resolved.to_str().map(str::to_string) else {
            return err(
                ContractError::ExecutionFailed,
                "settings.set: the resolved config path is not valid UTF-8",
            );
        };
        let op_id = match crate::write_executor::mint_op_id() {
            Ok(id) => id,
            Err(e) => {
                return err(
                    ContractError::ExecutionFailed,
                    format!("could not mint an op id: {e}"),
                )
            }
        };
        // S13 audit-before-act: record the setting write intent content-free BEFORE
        // any file mutation, correlated by the op id. Fail closed.
        if let Some(audit) = &self.audit {
            let event = behaviour_action_event(SETTINGS_SET_TOOL, "settings-set", &op_id);
            if audit.submit(event).await.is_err() {
                return err(
                    ContractError::ExecutionFailed,
                    "audit ledger unavailable; settings.set refused",
                );
            }
        }
        // WRITE-AHEAD: capture the prior value BEFORE the write (a read, no mutation),
        // so the RestoreValue inverse holds the pre-write value (or `None` if the key
        // was absent). A non-scalar current value / unknown format is refused here,
        // never overwritten blindly.
        let inverse = match crate::undo_enact::capture_prior_value(&resolved_str, &key) {
            Ok(inv) => inv,
            Err(EnactError::Unsupported(why)) => {
                return err(ContractError::InvalidArguments, format!("settings.set: {why}"))
            }
            Err(EnactError::Io(e)) => {
                return err(
                    ContractError::ExecutionFailed,
                    format!("settings.set could not read the config: {e}"),
                )
            }
        };
        // Apply the new value through the format-preserving editor (atomic write +
        // read-after-write self-check). Nothing is partially written on failure.
        match crate::undo_enact::apply_setting_value(&resolved_str, &key, &value) {
            Ok(_) => {}
            Err(EnactError::Unsupported(why)) => {
                return err(ContractError::InvalidArguments, format!("settings.set: {why}"))
            }
            Err(EnactError::Io(e)) => {
                return err(ContractError::ExecutionFailed, format!("settings.set failed: {e}"))
            }
        }
        // Register the compensation to the durable, HMAC-chained undo signer.
        // Best-effort: an absent/failing signer never fails a committed write.
        if let Some(signer) = &self.undo_signer {
            if signer.exists() {
                let entry = UndoEntry {
                    op_id: op_id.clone(),
                    correlation_id: op_id.clone(),
                    inverse,
                };
                if let Err(e) = crate::undo_signer::submit_created(signer, &entry).await {
                    tracing::debug!("undo signer submit failed for settings.set: {e}");
                }
            }
        }
        ExecuteOutcome::Ok {
            result: serde_json::json!({ "op_id": op_id, "file": file, "key": key }),
        }
    }
}

/// A short constructor for an error outcome.
fn err(code: ContractError, message: impl Into<String>) -> ExecuteOutcome {
    ExecuteOutcome::Error { code, message: message.into() }
}

/// Whether `name` is a bare config filename: non-empty, no path separator, not a
/// `.`/`..` traversal, no NUL. A `settings.set` target must be one of these, so
/// joining it under the Arlen config dir cannot escape it.
fn is_safe_config_filename(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains('/')
        && !name.contains('\0')
}

/// The user's Arlen config directory (`$XDG_CONFIG_HOME/arlen`, else
/// `$HOME/.config/arlen`). `None` if neither yields an absolute base, so a write
/// never lands at a relative path.
fn arlen_config_dir() -> Option<PathBuf> {
    let config_home = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
                .map(|h| h.join(".config"))
        })?;
    Some(config_home.join("arlen"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ai_engine_contract::{CapabilityContext, ReadTier};
    use arlen_ai_undo_core::effect_model::InverseReceipt;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn tmp() -> std::path::PathBuf {
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let d = std::env::temp_dir().join(format!("settings-exec-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
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

    fn set_req(file: &str, key: &str, value: &str) -> Execute {
        Execute {
            tool_name: SETTINGS_SET_TOOL.to_string(),
            tool_input: serde_json::json!({ "file": file, "key": key, "value": value }),
            proof: None,
        }
    }

    fn live(root: &std::path::Path) -> SettingsExecutor {
        SettingsExecutor::new()
            .with_executor_live_gate(|| true)
            .with_config_root(root.to_path_buf())
    }

    #[tokio::test]
    async fn a_live_set_writes_the_value_preserving_comments() {
        let root = tmp();
        std::fs::write(root.join("appearance.toml"), "# theme\naccent = \"#000000\"\n").unwrap();

        let out = live(&root)
            .execute(&set_req("appearance.toml", "accent", "#6366f1"), &grant())
            .await;
        match out {
            ExecuteOutcome::Ok { result } => {
                assert_eq!(result["file"], "appearance.toml");
                assert_eq!(result["key"], "accent");
                assert!(result["op_id"].as_str().is_some_and(|s| !s.is_empty()));
            }
            other => panic!("expected Ok, got {other:?}"),
        }
        let body = std::fs::read_to_string(root.join("appearance.toml")).unwrap();
        assert!(body.contains("accent = \"#6366f1\""), "value set: {body}");
        assert!(body.contains("# theme"), "comment preserved: {body}");
    }

    #[tokio::test]
    async fn a_set_round_trips_through_its_captured_inverse() {
        let root = tmp();
        let path = root.join("shell.toml");
        std::fs::write(&path, "layout = \"floating\"\n").unwrap();

        // The inverse the executor captures write-ahead (the prior "floating").
        let inverse = crate::undo_enact::capture_prior_value(path.to_str().unwrap(), "layout").unwrap();
        match &inverse {
            InverseReceipt::RestoreValue { prior, .. } => {
                assert_eq!(prior.as_deref(), Some("floating"))
            }
            _ => panic!("expected RestoreValue"),
        }

        live(&root).execute(&set_req("shell.toml", "layout", "tiling"), &grant()).await;
        assert!(std::fs::read_to_string(&path).unwrap().contains("\"tiling\""));

        // Enacting that captured inverse restores the prior - the undo a later
        // restore runs.
        crate::undo_enact::enact_inverse(&inverse).unwrap();
        assert!(std::fs::read_to_string(&path).unwrap().contains("\"floating\""));
    }

    #[tokio::test]
    async fn a_set_is_refused_when_the_executor_is_not_live() {
        let root = tmp();
        std::fs::write(root.join("shell.toml"), "a = 1\n").unwrap();
        let exec = SettingsExecutor::new()
            .with_executor_live_gate(|| false)
            .with_config_root(root.clone());
        let out = exec.execute(&set_req("shell.toml", "a", "2"), &grant()).await;
        assert!(matches!(out, ExecuteOutcome::Error { .. }));
        assert_eq!(std::fs::read_to_string(root.join("shell.toml")).unwrap(), "a = 1\n");
    }

    #[tokio::test]
    async fn the_ai_master_switch_file_is_protected() {
        let root = tmp();
        std::fs::write(root.join("ai.toml"), "[agent]\nexecutor_live = false\n").unwrap();
        let out = live(&root)
            .execute(&set_req("ai.toml", "executor_live", "true"), &grant())
            .await;
        assert!(matches!(out, ExecuteOutcome::Error { .. }), "ai.toml refused");
        // Untouched: the agent cannot flip its own gate through settings.set.
        assert_eq!(
            std::fs::read_to_string(root.join("ai.toml")).unwrap(),
            "[agent]\nexecutor_live = false\n"
        );
    }

    #[tokio::test]
    async fn a_path_traversal_filename_is_refused() {
        let root = tmp();
        for bad in ["../escape.toml", "sub/dir.toml", "..", "."] {
            match live(&root).execute(&set_req(bad, "k", "v"), &grant()).await {
                ExecuteOutcome::Error { code, .. } => {
                    assert_eq!(code, ContractError::InvalidArguments, "refused {bad}")
                }
                other => panic!("expected InvalidArguments for {bad}, got {other:?}"),
            }
        }
    }

    #[tokio::test]
    async fn a_missing_arg_is_a_malformed_request() {
        let root = tmp();
        let req = Execute {
            tool_name: SETTINGS_SET_TOOL.to_string(),
            tool_input: serde_json::json!({ "file": "shell.toml", "key": "a" }),
            proof: None,
        };
        match live(&root).execute(&req, &grant()).await {
            ExecuteOutcome::Error { code, .. } => assert_eq!(code, ContractError::InvalidArguments),
            other => panic!("expected InvalidArguments, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn an_unknown_format_is_refused_not_corrupted() {
        let root = tmp();
        std::fs::write(root.join("mystery.xyz"), "k=v\n").unwrap();
        match live(&root).execute(&set_req("mystery.xyz", "k", "w"), &grant()).await {
            ExecuteOutcome::Error { .. } => {}
            other => panic!("expected an error for an unknown format, got {other:?}"),
        }
        assert_eq!(std::fs::read_to_string(root.join("mystery.xyz")).unwrap(), "k=v\n");
    }

    #[tokio::test]
    async fn a_foreign_tool_is_rejected() {
        let root = tmp();
        let req = Execute {
            tool_name: "fs.move".to_string(),
            tool_input: serde_json::json!({}),
            proof: None,
        };
        match live(&root).execute(&req, &grant()).await {
            ExecuteOutcome::Error { code, .. } => assert_eq!(code, ContractError::UnknownTool),
            other => panic!("expected UnknownTool, got {other:?}"),
        }
    }

    #[test]
    fn safe_config_filename_rejects_traversal_and_separators() {
        assert!(is_safe_config_filename("shell.toml"));
        assert!(is_safe_config_filename(".hidden.toml"));
        assert!(!is_safe_config_filename(""));
        assert!(!is_safe_config_filename("."));
        assert!(!is_safe_config_filename(".."));
        assert!(!is_safe_config_filename("a/b.toml"));
        assert!(!is_safe_config_filename("/etc/passwd"));
    }
}

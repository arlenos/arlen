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
//! ## Writable files (fail-closed cosmetic allowlist)
//!
//! `settings.set` is gate-classified `ReversibleAction`, so under `executor_live` it
//! applies autonomously with no confirm. Several files under `~/.config/arlen` gate
//! the AI or security posture, and a write to one is NOT what "reversible" bounds
//! (redirecting the AI's own egress or self-widening a read scope is not undone by a
//! later value restore): `ai.toml` (the AI master switches - the exact hole the
//! separate-uid AI-switch daemon closes), `ai-routing.toml` (the ai-proxy provider
//! endpoints - a write there redirects LLM traffic to an attacker host, exfiltration
//! that bypasses the egress gate), `file-manager-mcp.toml` (`[scope] roots`, the
//! agent's own filesystem read allowlist), `shell.toml` (`[launcher] confined`, app
//! confinement), `compositor.toml` (`[system_actions]`, name->command mappings), and
//! more. So the guard is a FAIL-CLOSED ALLOWLIST, not a denylist: only benign,
//! user-facing cosmetic preference files are writable, and a new posture file is
//! refused by default. Posture files change only through their own consent-gated path.

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

/// The config files `settings.set` MAY write: a fail-closed allowlist of benign,
/// user-facing COSMETIC preference files (theme / appearance / sound / panel
/// layout). Everything else is refused - notably every file under `~/.config/arlen`
/// that gates the AI or security posture (`ai.toml`, `ai-routing.toml`,
/// `file-manager-mcp.toml`, `shell.toml`, `compositor.toml`, `graph.toml`,
/// `terminal.toml`, ...). An allowlist (not a denylist) means a new posture file is
/// protected by default, matching the gate registry's "possession of an entry is the
/// trust proof" fail-closed discipline.
const ALLOWED_CONFIG_FILES: &[&str] = &[
    "appearance.toml",
    "theme.toml",
    "sounds.toml",
    "quicksettings.toml",
    "topbar.toml",
];

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
        // Fail-closed allowlist: only benign cosmetic preference files are writable.
        // Every AI/security-posture file (ai.toml, ai-routing.toml, file-manager-mcp
        // .toml, shell.toml, compositor.toml, ...) is refused, so an autonomous write
        // can never redirect the AI's egress, widen its read scope, or flip a gate.
        if !ALLOWED_CONFIG_FILES.contains(&file.as_str()) {
            return err(
                ContractError::ExecutionFailed,
                "settings.set refused: only cosmetic preference files are writable \
                 (this file is not on the allowlist)",
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
        let path = root.join("theme.toml");
        std::fs::write(&path, "accent = \"#000000\"\n").unwrap();

        // The inverse the executor captures write-ahead (the prior accent).
        let inverse =
            crate::undo_enact::capture_prior_value(path.to_str().unwrap(), "accent").unwrap();
        match &inverse {
            InverseReceipt::RestoreValue { prior, .. } => {
                assert_eq!(prior.as_deref(), Some("#000000"))
            }
            _ => panic!("expected RestoreValue"),
        }

        live(&root).execute(&set_req("theme.toml", "accent", "#6366f1"), &grant()).await;
        assert!(std::fs::read_to_string(&path).unwrap().contains("\"#6366f1\""));

        // Enacting that captured inverse restores the prior - the undo a later
        // restore runs.
        crate::undo_enact::enact_inverse(&inverse).unwrap();
        assert!(std::fs::read_to_string(&path).unwrap().contains("\"#000000\""));
    }

    #[tokio::test]
    async fn a_set_is_refused_when_the_executor_is_not_live() {
        let root = tmp();
        std::fs::write(root.join("appearance.toml"), "a = 1\n").unwrap();
        let exec = SettingsExecutor::new()
            .with_executor_live_gate(|| false)
            .with_config_root(root.clone());
        let out = exec.execute(&set_req("appearance.toml", "a", "2"), &grant()).await;
        assert!(matches!(out, ExecuteOutcome::Error { .. }));
        assert_eq!(std::fs::read_to_string(root.join("appearance.toml")).unwrap(), "a = 1\n");
    }

    #[tokio::test]
    async fn ai_and_security_posture_files_are_refused_and_untouched() {
        // The allowlist refuses every posture file: the AI master switches, the
        // proxy egress endpoints, the agent's read scope, app confinement, system
        // actions. The agent cannot flip its own gate, redirect its egress or widen
        // its scope through settings.set.
        let cases: &[(&str, &str, &str, &str)] = &[
            ("ai.toml", "[agent]\nexecutor_live = false\n", "executor_live", "true"),
            (
                "ai-routing.toml",
                "[providers.evil]\nendpoint_url = \"https://ok\"\n",
                "providers.evil.endpoint_url",
                "https://attacker.example/collect",
            ),
            ("file-manager-mcp.toml", "[scope]\nroots = []\n", "scope.roots", "/home"),
            ("shell.toml", "[launcher]\nconfined = true\n", "launcher.confined", "false"),
            ("compositor.toml", "[system_actions]\n", "system_actions.logout", "rm -rf ~"),
        ];
        for (file, body, key, value) in cases {
            let root = tmp();
            std::fs::write(root.join(file), body).unwrap();
            let out = live(&root).execute(&set_req(file, key, value), &grant()).await;
            assert!(matches!(out, ExecuteOutcome::Error { .. }), "{file} must be refused");
            assert_eq!(
                &std::fs::read_to_string(root.join(file)).unwrap(),
                body,
                "{file} must be untouched"
            );
        }
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
    async fn a_non_allowlisted_file_is_refused_even_if_it_is_a_known_format() {
        // graph.toml is a real .toml (recognized format) but gates KG ingestion, so
        // it is off the cosmetic allowlist and refused untouched.
        let root = tmp();
        std::fs::write(root.join("graph.toml"), "watch = []\n").unwrap();
        match live(&root).execute(&set_req("graph.toml", "watch", "/home"), &grant()).await {
            ExecuteOutcome::Error { .. } => {}
            other => panic!("expected a refusal for a non-allowlisted file, got {other:?}"),
        }
        assert_eq!(std::fs::read_to_string(root.join("graph.toml")).unwrap(), "watch = []\n");
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

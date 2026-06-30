//! The agent daemon's runtime configuration, read from `ai.toml`.
//!
//! Deliberately minimal and **fail-safe**: anything missing or malformed
//! yields the safe defaults (nothing enabled, no graph read, suggest-only),
//! so a broken config never leaves the agent enabled or over-granted.

use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use arlen_ai_core::capability::{access_tier_from_level, AccessTier, ActionPermissions, BaselineMode};
use arlen_config_broker::AiMasterSwitches;
use serde::Deserialize;

use crate::loader::Provenance;

/// The action modes the harness autonomy dial may set. `autonomous` is
/// deliberately absent: the baseline can never be autonomous (autonomy is the
/// per-app `[ai] autonomous_apps` grant), and [`AgentConfig::parse`] clamps any
/// unknown mode to `suggest`.
pub const SETTABLE_ACTION_MODES: [&str; 2] = ["suggest", "supervised"];

/// Set `[ai] action_mode` in the ai.toml at `path`, format-preserving and atomic
/// (write a sibling temp file, then rename). Creates the file and the `[ai]`
/// table if absent, and preserves every other key + comment. This is the
/// daemon-side setter for the harness autonomy dial (the harness must not write
/// ai.toml directly; Settings owns the file, the daemon exposes the setter). The
/// gate and the `action_state` getter re-read ai.toml on every call, so the
/// change is live with no restart - mirroring how `executor_live` is read.
/// Rejects any mode outside [`SETTABLE_ACTION_MODES`] before touching the file.
pub fn set_action_mode_in(path: &Path, mode: &str) -> Result<(), String> {
    if !SETTABLE_ACTION_MODES.contains(&mode) {
        return Err(format!(
            "action_mode must be one of {SETTABLE_ACTION_MODES:?}, not {mode:?}"
        ));
    }
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let mut doc: toml_edit::DocumentMut = text
        .parse()
        .map_err(|e| format!("parse ai.toml: {e}"))?;
    // Preserve an existing `[ai]` table; create it only if missing, so other
    // keys (enabled, access_level, provider, autonomous_apps) are untouched.
    if doc.get("ai").and_then(|item| item.as_table()).is_none() {
        doc["ai"] = toml_edit::Item::Table(toml_edit::Table::new());
    }
    doc["ai"]["action_mode"] = toml_edit::value(mode);

    let parent = path
        .parent()
        .ok_or_else(|| "ai.toml path has no parent directory".to_string())?;
    std::fs::create_dir_all(parent).map_err(|e| format!("create config dir: {e}"))?;
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, doc.to_string()).map_err(|e| format!("write temp config: {e}"))?;
    // Owner-only: ai.toml carries the AI master switches (executor_live,
    // access_level, autonomous_apps), so it must never be world-readable.
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))
        .map_err(|e| format!("secure temp config: {e}"))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("rename config into place: {e}"))?;
    Ok(())
}

/// Add (`enabled = true`) or remove (`false`) `app_id` from `[ai]
/// autonomous_apps` in the ai.toml at `path`, format-preserving and atomic
/// (sibling temp + rename), creating the file/`[ai]` table/array if absent and
/// preserving every other key. This is the per-app half of the harness autonomy
/// dial (the "More" grant): a listed app may act autonomously under the baseline
/// model, still bounded by `executor_live`. The gate re-reads ai.toml per call,
/// so the change is LIVE with no restart. Idempotent (adding a present app or
/// removing an absent one is a no-op write). The daemon-side setter the harness
/// calls (the harness must not write ai.toml; Settings owns the file). Rejects an
/// empty or control-bearing app id before touching the file.
///
/// The list is the authority the gate reads; surfacing each grant as an LCG
/// Grant node in the capability browser is a separate projection (a follow-up),
/// not a precondition for the dial to take effect.
pub fn set_autonomous_app_in(path: &Path, app_id: &str, enabled: bool) -> Result<(), String> {
    let app = app_id.trim();
    if app.is_empty() || app.chars().any(char::is_control) {
        return Err("app id must be non-empty and free of control characters".to_string());
    }
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let mut doc: toml_edit::DocumentMut = text
        .parse()
        .map_err(|e| format!("parse ai.toml: {e}"))?;
    if doc.get("ai").and_then(|item| item.as_table()).is_none() {
        doc["ai"] = toml_edit::Item::Table(toml_edit::Table::new());
    }
    // Read the current membership, compute the new set, write it back. Rebuilding
    // the array is fine for a flat string list (no per-element decor to keep).
    let mut apps: Vec<String> = doc["ai"]
        .get("autonomous_apps")
        .and_then(|item| item.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    let present = apps.iter().any(|a| a == app);
    if enabled && !present {
        apps.push(app.to_string());
    } else if !enabled {
        apps.retain(|a| a != app);
    }
    let mut arr = toml_edit::Array::new();
    for a in &apps {
        arr.push(a.as_str());
    }
    doc["ai"]["autonomous_apps"] = toml_edit::value(arr);

    let parent = path
        .parent()
        .ok_or_else(|| "ai.toml path has no parent directory".to_string())?;
    std::fs::create_dir_all(parent).map_err(|e| format!("create config dir: {e}"))?;
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, doc.to_string()).map_err(|e| format!("write temp config: {e}"))?;
    // Owner-only: ai.toml carries the AI master switches (executor_live,
    // access_level, autonomous_apps), so it must never be world-readable.
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))
        .map_err(|e| format!("secure temp config: {e}"))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("rename config into place: {e}"))?;
    Ok(())
}

/// The LLM provider the agent loop drives, resolved from `ai.toml`. A
/// `kind: agent` behaviour cannot run without one, so `None` keeps agent
/// behaviours skipped (the same fail-closed posture as a disabled daemon).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderSettings {
    /// Catalogued provider name the proxy forwards to (the shared
    /// `ai.provider` key).
    pub name: String,
    /// Model identifier (`[provider] model`).
    pub model: String,
    /// The model's usable input context window, in tokens
    /// (`[provider] context_window`). Defaults to a conservative low value
    /// when omitted, so an under-specified provider compacts early and fails
    /// closed rather than overflowing.
    pub context_window: u32,
    /// Capability token presented to the proxy (`[provider] audit_token`).
    /// Defaults to a fixed agent token; the proxy only records it until
    /// Phase 9-γ S15 validates it against the caller identity.
    pub audit_token: String,
}

/// The agent's resolved runtime configuration.
pub struct AgentConfig {
    /// Behaviour name to the provenance it was approved for (the loader
    /// only enables a behaviour matching this). Built-in only for now.
    pub enabled: BTreeMap<String, Provenance>,
    /// The global Knowledge-Graph read tier.
    pub read_tier: AccessTier,
    /// The per-application action permissions (baseline + autonomous apps).
    pub actions: ActionPermissions,
    /// The LLM provider for `kind: agent` behaviours, if one is configured
    /// and AI is enabled. `None` means agent behaviours cannot run.
    pub provider: Option<ProviderSettings>,
    /// Whether the agent may **execute** proven workflow decisions (write to
    /// the Knowledge Graph), not just surface them. Default `false`:
    /// suggest-mode, where a decision is gated, audited, and reported but never
    /// acted on. Opt in with `[agent] executor_live = true`; the write still
    /// passes the full predict -> gate -> re-validate -> audit chain.
    ///
    /// Status of the deployment prerequisites (it defaults off and nothing flips
    /// it yet): (1) the execution semantics are decided, (2) the cancellation
    /// behaviour is bounded and accepted, and only (3) full proof atomicity
    /// remains as a hard blocker. Detail:
    ///
    /// 1. **Execution semantics (decided).** The executor fires on a proven
    ///    `PreviewThenExecute`, which in the capability model is the *Supervised*
    ///    lift ("preview with a cancellation window, then execute"). For a safe,
    ///    reversible, invisible curation action via a *deterministic workflow*
    ///    (auto-tag's `FILE_PART_OF`), it executes **silently and immediately**,
    ///    with no per-action prompt: per-file confirmation is annoying, and these
    ///    workflows make no LLM call so they cost no tokens. The user inspects
    ///    what was curated after the fact via the read-only activity view (the
    ///    `silent curator + pull` interaction model), not a pre-action window.
    ///    This deliberately overrides the literal Supervised window for safe
    ///    workflow curation; it does NOT extend to `kind: agent` LLM behaviours
    ///    (which are not wired to execute) or to high-impact / external-triggered
    ///    actions (which always confirm regardless).
    /// 2. **Cancellation (bounded, accepted).** The dispatch loop stays
    ///    cancellable (a reload/shutdown can drop an in-flight dispatch), kept on
    ///    purpose: it aborts a long `kind: agent` loop promptly, and for the
    ///    workflow write a *drop is the correct revocation behaviour* (a config
    ///    change removing the grant means the write should not be forced through;
    ///    a dropped write is not re-authorised on the next run). The write is
    ///    pre-audited and idempotent, so if its request was already sent it is
    ///    durably recorded and reconcilable, never lost. The write also has its
    ///    own timeout, so a stalled knowledge socket cannot park the dispatch
    ///    (and the daemon) waiting on it. Residual: an already-sent write can
    ///    still commit under a just-revoked grant (the bounded D-2 class, at most
    ///    the one in-flight event). A narrower per-write completion shield is
    ///    possible but not clearly more correct, since forcing the write through
    ///    a revocation is the opposite of what a revocation wants.
    /// 3. **Proof atomicity.** The executor re-validates the full proof, then
    ///    performs a separate write; the daemon enforces only endpoint existence
    ///    and edge absence atomically, not the gate's `PathUnderField`. A fact
    ///    outside the write predicate can change in between (gap A2, needs a
    ///    graph snapshot/version).
    /// 4. **Authorisation grant (provisioned).** The write only succeeds if the
    ///    agent's permission profile is installed at
    ///    `~/.config/permissions/ai-agent.toml`: the knowledge daemon mints the
    ///    write token from it. The shipped template
    ///    (`ai/ai-agent/dist/permissions/ai-agent.toml`) grants exactly the one
    ///    `FILE_PART_OF` relation the auto-tag workflow writes, with the
    ///    all-instances scope that unowned system-to-system link requires, and
    ///    nothing else. Going live is: install that file, then set
    ///    `executor_live = true`. Without the profile the agent stays in
    ///    suggest-mode in effect (every write is refused for lack of a grant).
    ///    The agent's canonical binary `/usr/lib/arlen/libexec/arlen-ai-agent`
    ///    resolves to the `ai-agent` app id (the profile key) in
    ///    `arlen_permissions::identity`, so the grant loads for the installed
    ///    daemon, not only in tests.
    /// 5. **Read-scope enforcement (not yet at the read boundary).** The profile
    ///    declares field-level reads (`File.{id,path}`, `Project.{id,root_path}`),
    ///    but the knowledge read socket does not yet enforce per-query read scope:
    ///    it rate-limits and rejects write verbs, then runs the Cypher. The
    ///    executor's own re-validation reads stay within the declared fields by
    ///    construction, so the *grant* is honest, but a compromised agent process
    ///    could read more until per-query read-scope enforcement lands
    ///    (token-authenticated reads + label/field whitelisting, the knowledge
    ///    daemon's S16 read-path hardening). This bounds confidentiality, not the
    ///    write blast radius (the write path does enforce the relation scope), and
    ///    the agent already reads File/Project in suggest-mode today.
    /// 6. **Same-uid provenance (F3, documented).** The write tier is keyed on the
    ///    resolved app id (`daemon.rs` `tier_for_app`), and both the identity
    ///    resolver's user-app path rule and the user-writable profile dir mean a
    ///    same-uid process can present as `ai-agent` from a non-canonical path and
    ///    obtain this grant. Adding the canonical-path mapping does not close that
    ///    boundary; a hard canonical-path write gate is deliberately not added
    ///    because it would refuse the dev agent running from a `target/` tree. F3
    ///    is closed by installd's inode-keyed identity registry plus root-owned
    ///    profiles (a cross-component hardening sprint, root-owned packaging is
    ///    Tim's call). The blast radius a same-uid spoof gains here is exactly the
    ///    one reversible `FILE_PART_OF` curation link, no wider.
    pub executor_live: bool,
}

#[derive(Deserialize, Default)]
struct RawAi {
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    access_level: u8,
    #[serde(default)]
    action_mode: Option<String>,
    #[serde(default)]
    autonomous_apps: Vec<String>,
    /// The catalogued provider name, shared with the rest of the product
    /// (`ai.provider`, written by Settings, read by `ai-daemon`).
    #[serde(default)]
    provider: Option<String>,
}

#[derive(Deserialize, Default)]
struct RawAgent {
    #[serde(default)]
    enabled: Vec<String>,
    #[serde(default)]
    executor_live: bool,
}

#[derive(Deserialize, Default)]
struct RawProvider {
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    context_window: Option<u32>,
    #[serde(default)]
    audit_token: Option<String>,
}

#[derive(Deserialize, Default)]
struct RawConfig {
    #[serde(default)]
    ai: RawAi,
    #[serde(default)]
    agent: RawAgent,
    #[serde(default)]
    provider: RawProvider,
}

/// Default model when `[provider] model` is omitted. Matches the catalogued
/// `ollama-default` backend (and `ai-daemon`'s hardcoded model), so a config
/// that only names the provider works out of the box.
const DEFAULT_MODEL: &str = "llama3:8b";
/// Conservative fallback window when `[provider] context_window` is omitted:
/// low enough that an under-specified provider compacts early and never
/// overflows. A deployment sets the model's real window.
const DEFAULT_CONTEXT_WINDOW: u32 = 8_192;
/// Fixed token presented to the proxy when `[provider] audit_token` is omitted.
const DEFAULT_AUDIT_TOKEN: &str = "ai-agent-default-token";

impl AgentConfig {
    /// The safe default: disabled, no graph read, suggest-only actions, no
    /// provider (so agent behaviours cannot run).
    pub fn fail_closed() -> Self {
        Self {
            enabled: BTreeMap::new(),
            read_tier: AccessTier::Minimal,
            actions: ActionPermissions::suggest_only(),
            provider: None,
            executor_live: false,
        }
    }

    /// Parse from `ai.toml` text alone (no broker). Equivalent to
    /// [`AgentConfig::resolve`] with no broker switches: every field, including
    /// the six master switches, comes from the file. Retained for the tests and
    /// any caller that has no broker to consult.
    pub fn parse(toml_text: &str) -> Self {
        Self::resolve(toml_text, None)
    }

    /// Resolve the runtime config from `ai.toml` text, with the six
    /// security-load-bearing master switches sourced from the config broker
    /// when it is reachable.
    ///
    /// The switches - `enabled`, `access_level`, `executor_live`,
    /// `action_mode`, `provider`, `autonomous_apps` - are the things a same-uid
    /// process could silently flip in the user-writable `ai.toml`
    /// (`same-uid-isolation-plan.md` Tier-A #1). When `broker` is `Some`, the
    /// broker (the separate-uid owner of a store the user's normal uid cannot
    /// write) is authoritative for them and the file's copies are ignored; when
    /// it is `None` (the broker is unreachable, the pre-cutover fallback) they
    /// come from the file, preserving today's behaviour. The non-switch fields
    /// are not authority-bearing and always come from `ai.toml`: the `[agent]
    /// enabled` behaviour list, and the provider's `model` / `context_window` /
    /// `audit_token`.
    ///
    /// A malformed document falls back to the safe defaults rather than
    /// erroring (fail-closed). The read level is clamped by
    /// `access_tier_from_level`, and `action_mode` can never be autonomous (a
    /// [`BaselineMode`]); autonomy is per-app only.
    pub fn resolve(toml_text: &str, broker: Option<&AiMasterSwitches>) -> Self {
        let raw: RawConfig = toml::from_str(toml_text).unwrap_or_default();

        // `enabled` (default off) is the global AI master switch, the same flag
        // the ai-daemon gates on. With AI disabled the agent runs nothing,
        // whatever the per-behaviour `[agent] enabled` list says.
        let enabled_master = broker.map_or(raw.ai.enabled, |b| b.enabled);
        if !enabled_master {
            return Self::fail_closed();
        }
        // The remaining switches: broker-authoritative when present, else the
        // file's. Cloned out of `raw.ai`/`raw.agent` here so the non-switch
        // fields below can still be moved out of `raw`.
        let access_level = broker.map_or(raw.ai.access_level, |b| b.access_level);
        let executor_live = broker.map_or(raw.agent.executor_live, |b| b.executor_live);
        let action_mode: Option<String> = match broker {
            Some(b) => Some(b.action_mode.as_str().to_string()),
            None => raw.ai.action_mode.clone(),
        };
        let autonomous_apps: Vec<String> = match broker {
            Some(b) => b.autonomous_apps.iter().cloned().collect(),
            None => raw.ai.autonomous_apps.clone(),
        };
        let provider_name: Option<String> = match broker {
            Some(b) => Some(b.provider.clone()),
            None => raw.ai.provider.clone(),
        };

        // Only built-in behaviours exist for now, so an enabled name is
        // approved for the built-in provenance.
        let enabled = raw
            .agent
            .enabled
            .into_iter()
            .map(|name| (name, Provenance::BuiltIn))
            .collect();
        let baseline = action_mode
            .as_deref()
            .map(BaselineMode::parse)
            .unwrap_or(BaselineMode::Suggest);
        // A provider is wired only when one is named (so the standard
        // Settings-authored config wires it, and a bare `enabled` without a
        // provider stays workflow-only rather than guessing a backend). The
        // model, window, and token fall back to safe defaults matching the
        // catalogued backend when an optional `[provider]` section does not
        // override them.
        let provider = provider_name
            .filter(|name| !name.is_empty())
            .map(|name| ProviderSettings {
                name,
                model: raw.provider.model.unwrap_or_else(|| DEFAULT_MODEL.to_string()),
                context_window: raw.provider.context_window.unwrap_or(DEFAULT_CONTEXT_WINDOW),
                audit_token: raw
                    .provider
                    .audit_token
                    .unwrap_or_else(|| DEFAULT_AUDIT_TOKEN.to_string()),
            });
        Self {
            enabled,
            read_tier: access_tier_from_level(access_level),
            actions: ActionPermissions::new(baseline, autonomous_apps),
            provider,
            executor_live,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_action_mode_writes_supervised_and_preserves_other_keys() {
        let dir = std::env::temp_dir().join(format!("arlen-action-mode-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("ai.toml");
        std::fs::write(
            &path,
            "[ai]\nenabled = true\naccess_level = 2\naction_mode = \"suggest\"\nprovider = \"ollama-default\"\n",
        )
        .unwrap();

        set_action_mode_in(&path, "supervised").expect("set supervised");

        let cfg = AgentConfig::parse(&std::fs::read_to_string(&path).unwrap());
        assert_eq!(cfg.actions.default_mode().as_str(), "supervised");
        // The other [ai] keys survive the format-preserving write.
        let back = std::fs::read_to_string(&path).unwrap();
        assert!(back.contains("provider = \"ollama-default\""));
        assert!(back.contains("access_level = 2"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn set_action_mode_rejects_autonomous_and_garbage() {
        let dir = std::env::temp_dir().join(format!("arlen-action-mode-bad-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("ai.toml");
        assert!(set_action_mode_in(&path, "autonomous").is_err());
        assert!(set_action_mode_in(&path, "nonsense").is_err());
        // A rejected mode never created the file (validated before any write).
        assert!(!path.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn set_autonomous_app_adds_then_removes_preserving_other_keys() {
        let dir = std::env::temp_dir().join(format!("arlen-auto-app-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("ai.toml");
        std::fs::write(&path, "[ai]\nenabled = true\naction_mode = \"supervised\"\n").unwrap();

        set_autonomous_app_in(&path, "org.arlen.files", true).expect("add app");
        let cfg = AgentConfig::parse(&std::fs::read_to_string(&path).unwrap());
        assert!(cfg.actions.autonomous_apps().any(|a| a == "org.arlen.files"));
        // Idempotent add: still exactly one entry.
        set_autonomous_app_in(&path, "org.arlen.files", true).expect("add again");
        let cfg = AgentConfig::parse(&std::fs::read_to_string(&path).unwrap());
        assert_eq!(cfg.actions.autonomous_apps().filter(|a| *a == "org.arlen.files").count(), 1);
        // Other keys survive.
        assert_eq!(cfg.actions.default_mode().as_str(), "supervised");

        set_autonomous_app_in(&path, "org.arlen.files", false).expect("remove app");
        let cfg = AgentConfig::parse(&std::fs::read_to_string(&path).unwrap());
        assert!(!cfg.actions.autonomous_apps().any(|a| a == "org.arlen.files"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn set_autonomous_app_rejects_empty_or_control_chars() {
        let dir = std::env::temp_dir().join(format!("arlen-auto-app-bad-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("ai.toml");
        assert!(set_autonomous_app_in(&path, "  ", true).is_err());
        assert!(set_autonomous_app_in(&path, "bad\nid", true).is_err());
        assert!(!path.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn set_action_mode_creates_ai_table_when_absent() {
        let dir = std::env::temp_dir().join(format!("arlen-action-mode-new-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("ai.toml");
        // No file at all: the setter creates it with a `[ai]` table.
        set_action_mode_in(&path, "suggest").expect("create + set");
        let cfg = AgentConfig::parse(&std::fs::read_to_string(&path).unwrap());
        assert_eq!(cfg.actions.default_mode().as_str(), "suggest");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parses_enabled_read_tier_and_actions() {
        let cfg = AgentConfig::parse(
            r#"
[ai]
enabled = true
access_level = 2
action_mode = "supervised"
autonomous_apps = ["org.arlen.files"]

[agent]
enabled = ["auto-tag-by-project"]
"#,
        );
        assert_eq!(cfg.enabled.get("auto-tag-by-project"), Some(&Provenance::BuiltIn));
        assert_eq!(cfg.read_tier, AccessTier::ProjectScoped);
        assert!(cfg.actions.is_autonomous("org.arlen.files"));
        // The executor opt-in defaults off (suggest-mode) when unspecified.
        assert!(!cfg.executor_live);
    }

    #[test]
    fn executor_live_is_opt_in() {
        let cfg = AgentConfig::parse(
            "[ai]\nenabled = true\n[agent]\nexecutor_live = true\n",
        );
        assert!(cfg.executor_live, "[agent] executor_live = true opts into executing");
        // Fail-closed config never executes.
        assert!(!AgentConfig::fail_closed().executor_live);
    }

    #[test]
    fn malformed_or_empty_config_fails_closed() {
        for text in ["", "not valid toml = =", "[ai]\naccess_level = 99"] {
            let cfg = AgentConfig::parse(text);
            assert!(cfg.enabled.is_empty());
            // 99 is out of range -> clamped to Minimal; empty -> Minimal.
            assert_eq!(cfg.read_tier, AccessTier::Minimal);
        }
    }

    #[test]
    fn global_ai_disable_overrides_enabled_behaviours() {
        // AI off globally must run nothing, even with behaviours listed and a
        // read level requested, so the master switch genuinely stops the agent.
        for text in [
            "[agent]\nenabled = [\"auto-tag-by-project\"]\n",
            "[ai]\nenabled = false\naccess_level = 4\n\n[agent]\nenabled = [\"auto-tag-by-project\"]\n",
        ] {
            let cfg = AgentConfig::parse(text);
            assert!(cfg.enabled.is_empty(), "AI off must enable no behaviours");
            assert_eq!(cfg.read_tier, AccessTier::Minimal);
            assert!(!cfg.actions.is_autonomous("org.arlen.files"));
        }
    }

    #[test]
    fn action_mode_can_never_be_autonomous_globally() {
        let cfg = AgentConfig::parse("[ai]\nenabled = true\naction_mode = \"autonomous\"\n");
        // A global autonomous request collapses to the safe baseline.
        assert!(!cfg.actions.is_autonomous("any.app"));
    }

    #[test]
    fn config_projects_the_honest_autonomy_dial_state() {
        // The `action_state` D-Bus getter projects exactly these three values
        // from the live config; assert the projection so the dial shows the
        // honest baseline + per-app grants + the orthogonal executor master.
        let cfg = AgentConfig::parse(
            "[ai]\nenabled = true\naction_mode = \"supervised\"\nautonomous_apps = [\"org.arlen.files\"]\n[agent]\nexecutor_live = true\n",
        );
        assert_eq!(cfg.actions.default_mode().as_str(), "supervised");
        let apps: Vec<&str> = cfg.actions.autonomous_apps().collect();
        assert_eq!(apps, vec!["org.arlen.files"]);
        assert!(cfg.executor_live);

        // Fail-closed config (a missing/unreadable file) projects the safe shape
        // the getter falls back to: suggest, no autonomous apps, executor off.
        let safe = AgentConfig::fail_closed();
        assert_eq!(safe.actions.default_mode().as_str(), "suggest");
        assert_eq!(safe.actions.autonomous_apps().count(), 0);
        assert!(!safe.executor_live);
    }

    #[test]
    fn the_standard_settings_config_wires_a_provider() {
        // The shape the Settings UI writes: [ai] enabled + provider. The agent
        // must wire a provider from it, with safe default model/window/token.
        let cfg = AgentConfig::parse("[ai]\nenabled = true\nprovider = \"ollama-default\"\n");
        let p = cfg.provider.expect("ai.provider wires a provider");
        assert_eq!(p.name, "ollama-default");
        assert_eq!(p.model, DEFAULT_MODEL);
        assert_eq!(p.context_window, DEFAULT_CONTEXT_WINDOW);
        assert_eq!(p.audit_token, DEFAULT_AUDIT_TOKEN);
    }

    #[test]
    fn a_provider_section_overrides_the_model_window_and_token() {
        let cfg = AgentConfig::parse(
            r#"
[ai]
enabled = true
provider = "my-cloud"

[provider]
model = "claude-opus-4-8"
context_window = 200000
audit_token = "tok-123"
"#,
        );
        let p = cfg.provider.expect("a provider is configured");
        assert_eq!(p.name, "my-cloud");
        assert_eq!(p.model, "claude-opus-4-8");
        assert_eq!(p.context_window, 200000);
        assert_eq!(p.audit_token, "tok-123");
    }

    #[test]
    fn no_provider_without_a_named_provider() {
        // A bare enabled config (no ai.provider), and an empty name, stay
        // workflow-only rather than guessing a backend.
        for text in [
            "[ai]\nenabled = true\n",
            "[ai]\nenabled = true\nprovider = \"\"\n",
        ] {
            assert!(AgentConfig::parse(text).provider.is_none(), "config: {text:?}");
        }
    }

    #[test]
    fn ai_disabled_yields_no_provider_even_when_named() {
        // The master switch off must leave nothing runnable, including a named
        // provider.
        let cfg = AgentConfig::parse("[ai]\nenabled = false\nprovider = \"ollama-default\"\n");
        assert!(cfg.provider.is_none());
    }

    use arlen_config_broker::ActionMode;
    use std::collections::BTreeSet;

    /// A broker state with strong switches, for the override tests.
    fn strong_switches() -> AiMasterSwitches {
        AiMasterSwitches {
            enabled: true,
            access_level: 3,
            executor_live: true,
            action_mode: ActionMode::Supervised,
            provider: "ollama-default".to_string(),
            autonomous_apps: BTreeSet::from(["org.arlen.files".to_string()]),
        }
    }

    #[test]
    fn resolve_with_no_broker_equals_parse() {
        // The fallback path (broker unreachable) must reproduce the file-only
        // behaviour exactly, so a down broker never changes the resolved config.
        let text = "[ai]\nenabled = true\naccess_level = 2\naction_mode = \"supervised\"\nautonomous_apps = [\"a.b\"]\nprovider = \"ollama-default\"\n[agent]\nenabled = [\"auto-tag-by-project\"]\nexecutor_live = true\n";
        let from_parse = AgentConfig::parse(text);
        let from_resolve = AgentConfig::resolve(text, None);
        assert_eq!(from_resolve.read_tier, from_parse.read_tier);
        assert_eq!(from_resolve.executor_live, from_parse.executor_live);
        assert_eq!(
            from_resolve.actions.default_mode().as_str(),
            from_parse.actions.default_mode().as_str()
        );
        assert_eq!(
            from_resolve.enabled.keys().collect::<Vec<_>>(),
            from_parse.enabled.keys().collect::<Vec<_>>()
        );
    }

    #[test]
    fn broker_switches_override_the_file_copies() {
        // The file carries weak/off switches; the broker carries strong ones.
        // The broker wins for all six, but the non-switch fields (the behaviour
        // list, the provider model/window) still come from the file.
        let text = r#"
[ai]
enabled = false
access_level = 0
action_mode = "suggest"
autonomous_apps = []
provider = ""

[agent]
enabled = ["auto-tag-by-project"]
executor_live = false

[provider]
model = "my-special-model"
context_window = 12345
"#;
        let switches = strong_switches();
        let cfg = AgentConfig::resolve(text, Some(&switches));
        // The six switches reflect the broker, not the (weaker) file.
        assert_eq!(cfg.read_tier, AccessTier::TimeScoped, "access_level 3 from broker");
        assert!(cfg.executor_live, "executor_live from broker");
        assert_eq!(cfg.actions.default_mode().as_str(), "supervised", "action_mode from broker");
        assert!(cfg.actions.is_autonomous("org.arlen.files"), "autonomous_apps from broker");
        // The non-switch fields still come from the file.
        assert_eq!(cfg.enabled.get("auto-tag-by-project"), Some(&Provenance::BuiltIn));
        let p = cfg.provider.expect("the broker provider name wires a provider");
        assert_eq!(p.name, "ollama-default", "provider name from broker");
        assert_eq!(p.model, "my-special-model", "model still from the file");
        assert_eq!(p.context_window, 12345, "context_window still from the file");
    }

    #[test]
    fn the_broker_master_switch_can_disable_a_file_that_says_enabled() {
        // A user-writable ai.toml flipping `enabled = true` cannot turn the AI on
        // if the broker (the canonical owner) says off - the whole point of
        // moving the master switch out of the writable file.
        let text = "[ai]\nenabled = true\naccess_level = 4\n[agent]\nenabled = [\"auto-tag-by-project\"]\nexecutor_live = true\n";
        let off = AiMasterSwitches {
            enabled: false,
            ..strong_switches()
        };
        let cfg = AgentConfig::resolve(text, Some(&off));
        assert!(cfg.enabled.is_empty(), "broker-off disables every behaviour");
        assert_eq!(cfg.read_tier, AccessTier::Minimal);
        assert!(!cfg.executor_live);
    }

    #[test]
    fn the_broker_master_switch_can_enable_a_file_that_says_disabled() {
        // The symmetric case: the file says off, the broker says on. The broker
        // is authoritative, so the agent runs.
        let text = "[ai]\nenabled = false\n[agent]\nenabled = [\"auto-tag-by-project\"]\n";
        let cfg = AgentConfig::resolve(text, Some(&strong_switches()));
        assert_eq!(cfg.enabled.get("auto-tag-by-project"), Some(&Provenance::BuiltIn));
        assert_eq!(cfg.read_tier, AccessTier::TimeScoped);
        assert!(cfg.executor_live);
    }

    #[test]
    fn a_broker_with_an_out_of_range_level_is_still_clamped() {
        // The broker store clamps on its own, but resolve must also clamp so a
        // hand-corrupted or future-widened value never widens the tier here.
        let switches = AiMasterSwitches {
            access_level: 99,
            ..strong_switches()
        };
        let cfg = AgentConfig::resolve("[agent]\nenabled = []\n", Some(&switches));
        assert_eq!(cfg.read_tier, AccessTier::Minimal, "99 clamps to the floor");
    }
}

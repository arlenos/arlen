//! Generic config CRUD commands.
//!
//! All commands operate on TOML files under `~/.config/arlen/<file>.toml`
//! using dot-notation keys (e.g. `theme.mode` -> `[theme] mode = ...`).

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use arlen_settings_core::config::{
    get_path, json_to_toml_edit, remove_dotted_in_doc, set_dotted_in_doc, toml_to_json,
};

/// Logical config file name, mapped to a path under `~/.config/arlen/`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConfigFile {
    Appearance,
    Compositor,
    Shell,
    Notifications,
    Modules,
    /// Knowledge daemon project-watch config (`graph.toml`).
    /// Settings UI for Focus Mode + Knowledge Graph pages writes
    /// `[projects]` here. The daemon reads this file on startup
    /// only — Settings warns the user that changes need a daemon
    /// restart until live-reload lands in a follow-up sprint.
    Graph,
    /// Quick Settings tile layout (`quicksettings.toml`). Schema:
    /// `[[tile]] id, visible, size`. The desktop-shell reads this on
    /// QS-panel mount via `qs_layout_get`; Settings writes it through
    /// the generic config API.
    QuickSettings,
    /// AI layer config (`ai.toml`). Schema: `[ai] enabled, provider`.
    /// The `arlen-ai-daemon` watches this file: toggling `enabled`
    /// switches the AI layer on/off live (Phase 9-α S7).
    Ai,
    /// Theme customization layer (`theme.toml`, sdk/theme's layer 3). The
    /// resolver merges it field-by-field over the active theme, so any
    /// `ArlenTheme` field (colours, the icon/cursor theme, radius, motion, ...)
    /// can be overridden here. The Appearance suite's per-field overrides write
    /// this file; the running shell re-resolves on change.
    Customization,
}

impl ConfigFile {
    fn filename(self) -> &'static str {
        match self {
            Self::Appearance => "appearance.toml",
            Self::Compositor => "compositor.toml",
            Self::Shell => "shell.toml",
            Self::Notifications => "notifications.toml",
            Self::Modules => "modules.toml",
            Self::Graph => "graph.toml",
            Self::QuickSettings => "quicksettings.toml",
            Self::Ai => "ai.toml",
            Self::Customization => "theme.toml",
        }
    }

    fn path(self) -> PathBuf {
        let dir = dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join("arlen");
        let _ = std::fs::create_dir_all(&dir);
        dir.join(self.filename())
    }
}

/// Read the file and parse as a generic TOML value. Returns an empty
/// table if the file does not exist yet.
fn read_file(file: ConfigFile) -> Result<toml::Value, String> {
    let path = file.path();
    if !path.exists() {
        return Ok(toml::Value::Table(toml::map::Map::new()));
    }
    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("read {}: {e}", path.display()))?;
    toml::from_str(&content).map_err(|e| format!("parse {}: {e}", path.display()))
}

/// Write a TOML value to disk atomically (write to .tmp, rename).
fn write_file(file: ConfigFile, value: &toml::Value) -> Result<(), String> {
    let path = file.path();
    let content = toml::to_string_pretty(value).map_err(|e| format!("serialize: {e}"))?;
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, content).map_err(|e| format!("write tmp: {e}"))?;
    // Owner-only (0600): user config carries security-bearing keys (ai.toml's
    // executor_live / access_level most of all), so it must never be world-
    // readable. Set on the temp before the rename so the live file is never 0644.
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))
        .map_err(|e| format!("secure tmp: {e}"))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("rename: {e}"))?;
    Ok(())
}


/// Read the whole file (when `key` is None) or a single dot-notation key.
#[tauri::command]
pub fn config_get(
    file: ConfigFile,
    key: Option<String>,
) -> Result<serde_json::Value, String> {
    let doc = read_file(file)?;
    match key.as_deref() {
        None | Some("") => Ok(toml_to_json(&doc)),
        Some(k) => match get_path(&doc, k) {
            Some(v) => Ok(toml_to_json(v)),
            None => Ok(serde_json::Value::Null),
        },
    }
}

/// The `ai.toml` keys that are AI master switches owned by the config broker
/// (the separate-uid store the user's normal uid cannot write). A `config_set`
/// to one of these on [`ConfigFile::Ai`] is routed to the broker, not the file,
/// so a malicious same-uid process cannot flip them by rewriting `ai.toml`.
fn is_ai_switch_key(key: &str) -> bool {
    matches!(
        key,
        "ai.enabled"
            | "ai.access_level"
            | "ai.provider"
            | "ai.action_mode"
            | "ai.autonomous_apps"
            | "agent.executor_live"
    )
}

/// Apply a single AI-switch `config_set` onto the broker's master switches,
/// mapping + validating the JSON value per key. Returns `Err` on a malformed
/// value (wrong type / out of range) so a bad write is refused rather than
/// papered over. Pure + testable.
fn apply_ai_switch(
    switches: &mut arlen_config_broker::AiMasterSwitches,
    key: &str,
    value: &serde_json::Value,
) -> Result<(), String> {
    use arlen_config_broker::ActionMode;
    match key {
        "ai.enabled" => {
            switches.enabled = value.as_bool().ok_or("ai.enabled must be a boolean")?
        }
        "agent.executor_live" => {
            switches.executor_live =
                value.as_bool().ok_or("agent.executor_live must be a boolean")?
        }
        "ai.access_level" => {
            let n = value
                .as_u64()
                .ok_or("ai.access_level must be a non-negative integer")?;
            switches.access_level = u8::try_from(n).map_err(|_| "ai.access_level out of range")?;
        }
        "ai.provider" => {
            switches.provider = value.as_str().ok_or("ai.provider must be a string")?.to_string()
        }
        "ai.action_mode" => {
            switches.action_mode = match value.as_str() {
                Some("suggest") => ActionMode::Suggest,
                Some("supervised") => ActionMode::Supervised,
                _ => return Err("ai.action_mode must be \"suggest\" or \"supervised\"".to_string()),
            }
        }
        "ai.autonomous_apps" => {
            let arr = value.as_array().ok_or("ai.autonomous_apps must be an array")?;
            let mut set = std::collections::BTreeSet::new();
            for v in arr {
                let app = v.as_str().ok_or("ai.autonomous_apps entries must be strings")?;
                // Match the agent's setter validation so Settings cannot write an
                // entry the agent would reject (an empty / control-char id).
                let trimmed = app.trim();
                if trimmed.is_empty() || trimmed.chars().any(char::is_control) {
                    return Err(
                        "ai.autonomous_apps entries must be non-empty and free of control characters"
                            .to_string(),
                    );
                }
                set.insert(trimmed.to_string());
            }
            switches.autonomous_apps = set;
        }
        other => return Err(format!("not an AI switch key: {other}")),
    }
    Ok(())
}

/// Write a value at a dot-notation key, preserving other sections
/// AND user comments + formatting in the file. Goes through
/// `toml_writer` (toml_edit-backed, atomic tmp+rename) so a save
/// from the Settings app doesn't erase the user's hand-edited
/// notes in `compositor.toml` / `shell.toml`.
///
/// The AI master switches are the exception: a write to one of them (on
/// [`ConfigFile::Ai`]) is routed to the config broker (the separate-uid owner)
/// when reachable, so a same-uid process cannot flip them by rewriting the
/// file. When the broker is unreachable it falls back to writing `ai.toml` (the
/// pre-cutover behaviour, kept coherent with the readers' fallback); the FINAL
/// cutover drops that fallback once the broker is deployed everywhere.
#[tauri::command]
pub async fn config_set(
    file: ConfigFile,
    key: String,
    value: serde_json::Value,
) -> Result<(), String> {
    if matches!(file, ConfigFile::Ai) && is_ai_switch_key(&key) {
        let client = arlen_config_broker::ConfigBrokerClient::default_socket();
        match client.get().await {
            Ok(mut switches) => {
                apply_ai_switch(&mut switches, &key, &value)?;
                return client
                    .set(&switches)
                    .await
                    .map_err(|e| format!("config broker set: {e}"));
            }
            // Genuinely unreachable: fall through to the ai.toml write below
            // (pre-cutover behaviour).
            Err(arlen_config_broker::ClientError::Transport(_)) => {}
            // Reachable but errored (corrupt store / refusal): surface it, never
            // write the switch to the file behind the broker (which the daemon
            // would then ignore, or which a same-uid attacker forced).
            Err(e) => return Err(format!("config broker: {e}")),
        }
    }
    let path = file.path();
    let item = json_to_toml_edit(value);
    arlen_settings_core::toml_writer::update(&path, |doc| {
        set_dotted_in_doc(doc, &key, item)
    })?;
    Ok(())
}

/// Set the default AI provider + model for the manager's Default-Models page
/// (`ai_defaults_set`). Writes `ai.provider` + `[provider] model` to `ai.toml`
/// in ONE format-preserving atomic write (Settings owns the `ai.toml` write; the
/// daemon reads it via `ai_defaults_get` and at startup). A typed wrapper over
/// the generic `config_set` so the manager makes one call, not two, and the two
/// fields never land half-written. The ranked-fallback + per-purpose
/// (query/agent/title) model schema is a deferred extension; this sets the
/// single default the daemon resolves today. Both must be non-empty.
#[tauri::command]
pub async fn ai_defaults_set(provider: String, model: String) -> Result<(), String> {
    if provider.trim().is_empty() || model.trim().is_empty() {
        return Err("provider and model must be non-empty".to_string());
    }
    // `ai.provider` is a broker-owned master switch (it decides where LLM
    // traffic goes); `provider.model` is not (it selects within a named
    // backend), so it stays in ai.toml. Route the provider to the broker when
    // reachable; on an unreachable broker write it to ai.toml too (the
    // pre-cutover fallback). The model always goes to ai.toml. The two are no
    // longer one atomic write, but they are independent fields the daemon reads
    // separately (provider from the broker, model from ai.toml), so a partial
    // apply is recoverable by a retry, not corrupting.
    let client = arlen_config_broker::ConfigBrokerClient::default_socket();
    let provider_to_file = match client.get().await {
        Ok(mut switches) => {
            switches.provider = provider.clone();
            client
                .set(&switches)
                .await
                .map_err(|e| format!("config broker set: {e}"))?;
            false
        }
        // Genuinely unreachable: write the provider to ai.toml too (pre-cutover).
        Err(arlen_config_broker::ClientError::Transport(_)) => true,
        // Reachable but errored: surface it rather than diverging from the broker.
        Err(e) => return Err(format!("config broker: {e}")),
    };
    let path = ConfigFile::Ai.path();
    arlen_settings_core::toml_writer::update(&path, |doc| {
        if provider_to_file {
            set_dotted_in_doc(doc, "ai.provider", toml_edit::value(provider.clone()))?;
        }
        set_dotted_in_doc(doc, "provider.model", toml_edit::value(model.clone()))?;
        Ok(())
    })?;
    Ok(())
}

/// The per-role default model ids (query/agent/title) the daemon resolves
/// (`ai_defaults_get_roles`). Each role falls back to the single default
/// (`provider.model`) when unset, so the Models hub always shows a resolved model.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleDefaults {
    /// The model answering interactive queries.
    query: String,
    /// The model driving the agent loop.
    agent: String,
    /// The model generating conversation titles.
    title: String,
}

/// The three roles the daemon resolves a model for.
const AI_ROLES: [&str; 3] = ["query", "agent", "title"];

/// Read the per-role default models from `ai.toml` (`ai_defaults_get_roles`), each
/// falling back to the single `provider.model` default when its role is unset.
#[tauri::command]
pub fn ai_defaults_get_roles() -> Result<RoleDefaults, String> {
    let ai = read_file(ConfigFile::Ai)?;
    let fallback = get_path(&ai, "provider.model")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let role = |name: &str| {
        get_path(&ai, &format!("defaults.{name}"))
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| fallback.clone())
    };
    Ok(RoleDefaults {
        query: role("query"),
        agent: role("agent"),
        title: role("title"),
    })
}

/// Assign a model to one role (`ai_defaults_set_role`). Writes `defaults.<role>`
/// to `ai.toml` format-preserving. A per-role model is not a master switch (it
/// selects within the configured backend), so it stays in `ai.toml`, not the
/// config broker. The role must be one of query/agent/title and the id non-empty.
#[tauri::command]
pub fn ai_defaults_set_role(role: String, id: String) -> Result<(), String> {
    if !AI_ROLES.contains(&role.as_str()) {
        return Err(format!("unknown role '{role}'"));
    }
    if id.trim().is_empty() {
        return Err("id must be non-empty".to_string());
    }
    let path = ConfigFile::Ai.path();
    arlen_settings_core::toml_writer::update(&path, |doc| {
        set_dotted_in_doc(doc, &format!("defaults.{role}"), toml_edit::value(id.clone()))
    })?;
    Ok(())
}

/// Reset a single key (delete it) or the whole file.
#[tauri::command]
pub fn config_reset(file: ConfigFile, key: Option<String>) -> Result<(), String> {
    match key.as_deref() {
        None | Some("") => {
            let path = file.path();
            if path.exists() {
                std::fs::remove_file(&path)
                    .map_err(|e| format!("remove {}: {e}", path.display()))?;
            }
            Ok(())
        }
        Some(k) => {
            let path = file.path();
            arlen_settings_core::toml_writer::update(&path, |doc| {
                remove_dotted_in_doc(doc, k);
                Ok(())
            })?;
            Ok(())
        }
    }
}


/// Return the built-in default for a config file (or a single key).
/// This is what the user sees when they "reset to default".
#[tauri::command]
pub fn config_get_default(
    file: ConfigFile,
    key: Option<String>,
) -> Result<serde_json::Value, String> {
    let doc = default_for(file);
    match key.as_deref() {
        None | Some("") => Ok(toml_to_json(&doc)),
        Some(k) => match get_path(&doc, k) {
            Some(v) => Ok(toml_to_json(v)),
            None => Ok(serde_json::Value::Null),
        },
    }
}

fn default_for(file: ConfigFile) -> toml::Value {
    let raw = match file {
        ConfigFile::Appearance => DEFAULT_APPEARANCE,
        ConfigFile::Notifications => DEFAULT_NOTIFICATIONS,
        ConfigFile::Ai => DEFAULT_AI,
        _ => return toml::Value::Table(toml::map::Map::new()),
    };
    toml::from_str::<toml::Value>(raw)
        .unwrap_or_else(|_| toml::Value::Table(toml::map::Map::new()))
}

/// Default `ai.toml`. The AI layer is opt-in (Foundation §5.1-5.2):
/// it ships disabled. `provider` names a catalogued provider on the
/// AI proxy; Phase 9-α ships only the local Ollama provider.
/// `access_level = 3` (TimeScoped, recent activity) is the generous
/// default: once enabled, the AI is useful out of the box and the user
/// narrows if they want, rather than starting blind and having to loosen
/// it to use it. The sovereignty guarantee is the audit + capability-scope
/// + local-only + visible/revocable reads, not a tiny default scope.
const DEFAULT_AI: &str = r##"
[ai]
enabled = false
access_level = 3
provider = "ollama-default"
"##;

/// Default appearance.toml shipped with the settings app. Matches the
/// dark theme values used by desktop-shell.
const DEFAULT_APPEARANCE: &str = r##"
[theme]
active = "dark"
mode = "dark"

[overrides]
# `radius_intensity` is the user's global radius multiplier (0.0..=2.0).
# Applied at emit time to chip/button/input/card/modal tokens; `full`
# and the per-corner window outline stay categorical. See
# docs/architecture/theme-system.md.
# radius_intensity = 1.0

[window]
border_width = 1
gap_size = 8

[fonts]
interface = "Inter Variable"
monospace = "JetBrains Mono"
size = 14

[accessibility]
reduce_motion = false
"##;

/// Default notifications.toml shipped with the settings app. Mirrors
/// the daemon's `notification_daemon::config::Config::default()`.
const DEFAULT_NOTIFICATIONS: &str = r##"
[general]
toast_duration_normal = 4000
toast_duration_high = 8000
max_visible_toasts = 5

[dnd]
mode = "off"
suppress_fullscreen = false
always_suppress = []
always_allow = []

[dnd.schedule]
start = "22:00"
end = "07:00"
days = []
mode = "priority"

[history]
enabled = true
max_age_days = 30
max_count = 1000

[grouping]
by_app = true
stack_similar = true
auto_collapse_after = 3
"##;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;


    // ── default configs ──────────────────────────────────────────────

    #[test]
    fn test_default_appearance_is_valid_toml() {
        let v: Result<toml::Value, _> = toml::from_str(DEFAULT_APPEARANCE);
        assert!(v.is_ok(), "DEFAULT_APPEARANCE parse error: {:?}", v.err());
        let table = v.unwrap();
        assert!(
            get_path(&table, "theme.mode").is_some(),
            "theme.mode must exist"
        );
    }

    #[test]
    fn test_default_notifications_is_valid_toml() {
        let v: Result<toml::Value, _> = toml::from_str(DEFAULT_NOTIFICATIONS);
        assert!(
            v.is_ok(),
            "DEFAULT_NOTIFICATIONS parse error: {:?}",
            v.err()
        );
        let table = v.unwrap();
        assert!(
            get_path(&table, "dnd.mode").is_some(),
            "dnd.mode must exist"
        );
        assert!(
            get_path(&table, "history.enabled").is_some(),
            "history.enabled must exist"
        );
    }

    #[test]
    fn test_default_ai_is_valid_toml() {
        let v: Result<toml::Value, _> = toml::from_str(DEFAULT_AI);
        assert!(v.is_ok(), "DEFAULT_AI parse error: {:?}", v.err());
        let table = v.unwrap();
        // The AI layer ships disabled (opt-in, Foundation §5.1-5.2).
        assert_eq!(
            get_path(&table, "ai.enabled").and_then(|x| x.as_bool()),
            Some(false),
            "ai.enabled must default to false"
        );
        assert!(
            get_path(&table, "ai.provider").is_some(),
            "ai.provider must exist"
        );
    }


    #[test]
    fn ai_switch_keys_are_recognised() {
        for k in [
            "ai.enabled",
            "ai.access_level",
            "ai.provider",
            "ai.action_mode",
            "ai.autonomous_apps",
            "agent.executor_live",
        ] {
            assert!(is_ai_switch_key(k), "{k} is a broker-owned switch");
        }
        // Non-switch AI keys + other files' keys are NOT routed to the broker.
        for k in ["provider.model", "provider.context_window", "ai.tool_routing", "theme.mode"] {
            assert!(!is_ai_switch_key(k), "{k} stays in the file");
        }
    }

    #[test]
    fn apply_ai_switch_maps_and_validates_each_key() {
        use arlen_config_broker::{ActionMode, AiMasterSwitches};
        let mut s = AiMasterSwitches::default();
        apply_ai_switch(&mut s, "ai.enabled", &serde_json::json!(true)).unwrap();
        apply_ai_switch(&mut s, "ai.access_level", &serde_json::json!(3)).unwrap();
        apply_ai_switch(&mut s, "ai.provider", &serde_json::json!("ollama-default")).unwrap();
        apply_ai_switch(&mut s, "ai.action_mode", &serde_json::json!("supervised")).unwrap();
        apply_ai_switch(&mut s, "agent.executor_live", &serde_json::json!(true)).unwrap();
        apply_ai_switch(&mut s, "ai.autonomous_apps", &serde_json::json!(["org.arlen.files"]))
            .unwrap();
        assert!(s.enabled);
        assert_eq!(s.access_level, 3);
        assert_eq!(s.provider, "ollama-default");
        assert_eq!(s.action_mode, ActionMode::Supervised);
        assert!(s.executor_live);
        assert!(s.autonomous_apps.contains("org.arlen.files"));
    }

    #[test]
    fn apply_ai_switch_rejects_malformed_values() {
        use arlen_config_broker::AiMasterSwitches;
        let mut s = AiMasterSwitches::default();
        // Wrong types + an out-of-range level + an unknown action mode all error,
        // so a bad write is refused rather than written.
        assert!(apply_ai_switch(&mut s, "ai.enabled", &serde_json::json!("yes")).is_err());
        assert!(apply_ai_switch(&mut s, "ai.access_level", &serde_json::json!(-1)).is_err());
        assert!(apply_ai_switch(&mut s, "ai.access_level", &serde_json::json!(9999)).is_err());
        assert!(apply_ai_switch(&mut s, "ai.action_mode", &serde_json::json!("autonomous")).is_err());
        assert!(apply_ai_switch(&mut s, "ai.autonomous_apps", &serde_json::json!("notarray")).is_err());
        // An empty or control-char app id is rejected (matches the agent setter).
        assert!(apply_ai_switch(&mut s, "ai.autonomous_apps", &serde_json::json!(["  "])).is_err());
        assert!(apply_ai_switch(&mut s, "ai.autonomous_apps", &serde_json::json!(["bad\nid"])).is_err());
        // Nothing was mutated by the failed calls.
        assert_eq!(s, AiMasterSwitches::default());
    }
}

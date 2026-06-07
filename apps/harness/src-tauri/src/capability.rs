//! AI capability context for the conversation surface (ai-app.md §2.1,
//! transparency principle #2: "capability is always visible").
//!
//! Reports the read tier and action mode the AI is operating under, plus
//! whether the layer is enabled. Read from `~/.config/arlen/ai.toml` —
//! the same file the ai-daemon resolves its read scope (`access_level`)
//! and the gate resolves its action mode from — so what the indicator
//! shows is what the daemon actually enforces. Read-only and advisory: a
//! missing or malformed config reports the fail-closed defaults
//! (disabled, Minimal, Suggest), matching the daemon's own posture.

use serde::Serialize;

/// The capability context shown in the conversation header.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Capability {
    /// Whether the AI layer is enabled at all.
    pub enabled: bool,
    /// Read-tier label (`Minimal`, `Session`, `Project`, `Time`, `Full`).
    pub tier: String,
    /// Action mode label (`Suggest`, `Supervised`).
    pub action_mode: String,
    /// The configured provider name (`ai.provider`), when set. Shown so the
    /// user can see which backend is answering without opening Settings.
    pub provider: Option<String>,
    /// The configured model (`[provider].model`), when set.
    pub model: Option<String>,
    /// Whether the agent's executor is live (`[agent] executor_live`). When
    /// false (the default), the agent computes and proposes curation but
    /// writes nothing; when true, proven reversible curation is written
    /// automatically and shown as activity to review and undo. This is the
    /// "what may the agent actually do right now" posture, surfaced on the
    /// agent dashboard. Fail-closed to false.
    pub executor_live: bool,
}

/// Read the `[agent] executor_live` flag from the parsed config. Absent,
/// non-boolean, or a missing `[agent]` table all read as `false` — the same
/// fail-closed default the agent daemon uses, so the posture shown matches
/// what the daemon enforces. Pure, so it is unit-tested without a config file.
fn executor_live(doc: &toml::Table) -> bool {
    doc.get("agent")
        .and_then(toml::Value::as_table)
        .and_then(|t| t.get("executor_live"))
        .and_then(toml::Value::as_bool)
        .unwrap_or(false)
}

/// Extract the provider name (`ai.provider`) and model (`[provider].model`)
/// from the parsed config. Either may be absent; both are display-only.
/// Pure, so the lookup is unit-tested without a config file.
fn provider_and_model(doc: &toml::Table) -> (Option<String>, Option<String>) {
    let provider = doc
        .get("ai")
        .and_then(toml::Value::as_table)
        .and_then(|t| t.get("provider"))
        .and_then(toml::Value::as_str)
        .map(str::to_string);
    let model = doc
        .get("provider")
        .and_then(toml::Value::as_table)
        .and_then(|t| t.get("model"))
        .and_then(toml::Value::as_str)
        .map(str::to_string);
    (provider, model)
}

/// Map the `ai.access_level` integer (0..=4, Foundation §8.4) to its
/// tier label. Out-of-range or absent floors to the most restrictive
/// (`Minimal`), matching the daemon's fail-closed `access_tier_from_level`.
fn tier_label(level: i64) -> &'static str {
    match level {
        1 => "Session",
        2 => "Project",
        3 => "Time",
        4 => "Full",
        _ => "Minimal",
    }
}

/// Map the `ai.action_mode` string to a display label. An unknown or
/// absent value floors to `Suggest` (the safe default; the daemon's gate
/// never widens past per-app modes anyway).
fn mode_label(mode: Option<&str>) -> String {
    match mode {
        Some("supervised") => "Supervised".to_string(),
        _ => "Suggest".to_string(),
    }
}

/// Resolve the path to `ai.toml`: `$XDG_CONFIG_HOME/arlen/ai.toml`
/// (via `dirs::config_dir`), falling back to a bare relative path if no
/// config dir is resolvable (then the read simply misses → defaults).
fn ai_config_path() -> std::path::PathBuf {
    dirs::config_dir()
        .unwrap_or_default()
        .join("arlen")
        .join("ai.toml")
}

/// Read the AI capability context. Never errors: an unreadable or
/// malformed config yields the fail-closed defaults so the indicator
/// always renders something truthful.
#[tauri::command]
pub async fn ai_capability() -> Capability {
    let defaults = Capability {
        enabled: false,
        tier: tier_label(0).to_string(),
        action_mode: mode_label(None),
        provider: None,
        model: None,
        executor_live: false,
    };

    let Ok(text) = std::fs::read_to_string(ai_config_path()) else {
        return defaults;
    };
    let Ok(doc) = text.parse::<toml::Table>() else {
        return defaults;
    };
    let ai = doc.get("ai").and_then(toml::Value::as_table);

    let enabled = ai
        .and_then(|t| t.get("enabled"))
        .and_then(toml::Value::as_bool)
        .unwrap_or(false);
    let level = ai
        .and_then(|t| t.get("access_level"))
        .and_then(toml::Value::as_integer)
        .unwrap_or(0);
    let mode = ai
        .and_then(|t| t.get("action_mode"))
        .and_then(toml::Value::as_str);
    let (provider, model) = provider_and_model(&doc);

    Capability {
        enabled,
        tier: tier_label(level).to_string(),
        action_mode: mode_label(mode),
        provider,
        model,
        executor_live: executor_live(&doc),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tier_label_maps_levels_and_floors_out_of_range() {
        assert_eq!(tier_label(0), "Minimal");
        assert_eq!(tier_label(1), "Session");
        assert_eq!(tier_label(4), "Full");
        assert_eq!(tier_label(9), "Minimal");
        assert_eq!(tier_label(-1), "Minimal");
    }

    #[test]
    fn mode_label_floors_unknown_to_suggest() {
        assert_eq!(mode_label(Some("supervised")), "Supervised");
        assert_eq!(mode_label(Some("suggest")), "Suggest");
        assert_eq!(mode_label(Some("autonomous")), "Suggest");
        assert_eq!(mode_label(None), "Suggest");
    }

    #[test]
    fn provider_and_model_read_from_their_keys() {
        let doc = r#"
            [ai]
            provider = "ollama"
            [provider]
            model = "llama3:8b"
        "#
        .parse::<toml::Table>()
        .unwrap();
        assert_eq!(provider_and_model(&doc), (Some("ollama".into()), Some("llama3:8b".into())));
    }

    #[test]
    fn provider_and_model_absent_are_none() {
        let doc = "[ai]\nenabled = true\n".parse::<toml::Table>().unwrap();
        assert_eq!(provider_and_model(&doc), (None, None));
    }

    #[test]
    fn executor_live_reads_the_agent_flag_and_fails_closed() {
        let on = "[agent]\nexecutor_live = true\n".parse::<toml::Table>().unwrap();
        assert!(executor_live(&on));
        let off = "[agent]\nexecutor_live = false\n".parse::<toml::Table>().unwrap();
        assert!(!executor_live(&off));
        // Missing [agent], missing key, and a non-bool all read as false.
        let absent = "[ai]\nenabled = true\n".parse::<toml::Table>().unwrap();
        assert!(!executor_live(&absent));
        let wrong_type = "[agent]\nexecutor_live = \"yes\"\n".parse::<toml::Table>().unwrap();
        assert!(!executor_live(&wrong_type));
    }
}

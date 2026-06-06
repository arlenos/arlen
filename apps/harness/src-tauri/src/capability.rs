//! AI capability context for the conversation surface (ai-app.md §2.1,
//! transparency principle #2: "capability is always visible").
//!
//! Reports the read tier and action mode the AI is operating under, plus
//! whether the layer is enabled. Read from `~/.config/lunaris/ai.toml` —
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

/// Resolve the path to `ai.toml`: `$XDG_CONFIG_HOME/lunaris/ai.toml`
/// (via `dirs::config_dir`), falling back to a bare relative path if no
/// config dir is resolvable (then the read simply misses → defaults).
fn ai_config_path() -> std::path::PathBuf {
    dirs::config_dir()
        .unwrap_or_default()
        .join("lunaris")
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

    Capability {
        enabled,
        tier: tier_label(level).to_string(),
        action_mode: mode_label(mode),
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
}

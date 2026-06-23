//! The daemon's minimal read of `ai.toml`: just the `[ai] enabled` master
//! switch that decides whether to spawn the pi sidecar (`pi-agent-adoption.md`
//! §D: `[ai] enabled=false` -> the daemon does not spawn pi).
//!
//! This deliberately does NOT depend on the ai-agent crate (the system being
//! replaced side-by-side); it reads only the one flag it needs, fail-closed:
//! a missing, unreadable or malformed config leaves AI disabled, so pi is never
//! spawned by accident.

use serde::Deserialize;
use std::path::PathBuf;

/// The `ai.toml` path: the `ARLEN_AI_CONFIG` override, else
/// `$HOME/.config/arlen/ai.toml`. Mirrors the ai-agent resolver so all the AI
/// components read the same file.
pub fn ai_config_path() -> PathBuf {
    if let Ok(path) = std::env::var("ARLEN_AI_CONFIG") {
        return PathBuf::from(path);
    }
    let home = std::env::var("HOME").unwrap_or_default();
    PathBuf::from(home).join(".config/arlen/ai.toml")
}

#[derive(Deserialize, Default)]
struct RawAi {
    #[serde(default)]
    enabled: bool,
}

#[derive(Deserialize, Default)]
struct RawConfig {
    #[serde(default)]
    ai: RawAi,
}

/// Whether `[ai] enabled` is true in the given `ai.toml` text. A malformed
/// document is treated as disabled (fail-closed), so a broken config never
/// causes the daemon to spawn pi.
pub fn ai_enabled_from_text(text: &str) -> bool {
    toml::from_str::<RawConfig>(text).map(|c| c.ai.enabled).unwrap_or(false)
}

/// Whether AI is enabled per the on-disk `ai.toml`. A missing/unreadable file is
/// disabled (fail-closed).
pub fn ai_enabled() -> bool {
    std::fs::read_to_string(ai_config_path()).map(|t| ai_enabled_from_text(&t)).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enabled_true_only_when_set() {
        assert!(ai_enabled_from_text("[ai]\nenabled = true\n"));
        assert!(!ai_enabled_from_text("[ai]\nenabled = false\n"));
        // Absent flag / absent section -> disabled.
        assert!(!ai_enabled_from_text("[ai]\naccess_level = 2\n"));
        assert!(!ai_enabled_from_text(""));
        // Other sections present, no [ai] enabled -> disabled.
        assert!(!ai_enabled_from_text("[agent]\nexecutor_live = true\n"));
    }

    #[test]
    fn malformed_config_is_disabled() {
        assert!(!ai_enabled_from_text("this is = not valid toml ["));
        // A wrong type for enabled fails the parse -> fail-closed disabled.
        assert!(!ai_enabled_from_text("[ai]\nenabled = \"yes\"\n"));
    }
}

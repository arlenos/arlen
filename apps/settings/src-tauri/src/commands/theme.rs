//! Theme-specific Tauri commands.
//!
//! These are thin convenience wrappers around the generic `config_*`
//! commands, kept separate so the frontend can call them without
//! building dot-notation keys itself.

use serde::{Deserialize, Serialize};

use super::config::{config_get, config_set, ConfigFile};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    Light,
    Dark,
    Auto,
}

impl ThemeMode {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Dark => "dark",
            Self::Auto => "auto",
        }
    }
}

/// Return the current appearance.toml as a JSON object.
#[tauri::command]
pub fn theme_get() -> Result<serde_json::Value, String> {
    config_get(ConfigFile::Appearance, None)
}

/// Set the theme mode. Also updates `theme.active` so the desktop-shell
/// theme watcher picks up the change (shell reads `active`, not `mode`).
#[tauri::command]
pub fn theme_set_mode(mode: ThemeMode) -> Result<(), String> {
    let mode_str = mode.as_str();
    config_set(
        ConfigFile::Appearance,
        "theme.mode".into(),
        serde_json::Value::String(mode_str.into()),
    )?;
    let active = if mode_str == "auto" { "dark" } else { mode_str };
    config_set(
        ConfigFile::Appearance,
        "theme.active".into(),
        serde_json::Value::String(active.into()),
    )?;
    Ok(())
}

/// Set the accent color (hex string like `#6366f1`).
#[tauri::command]
pub fn theme_set_accent(color: String) -> Result<(), String> {
    config_set(
        ConfigFile::Appearance,
        "colors.accent".into(),
        serde_json::Value::String(color),
    )
}

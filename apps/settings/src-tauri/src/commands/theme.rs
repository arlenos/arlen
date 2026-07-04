//! Theme-specific Tauri commands.
//!
//! These are thin convenience wrappers around the generic `config_*`
//! commands, kept separate so the frontend can call them without
//! building dot-notation keys itself.

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

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
pub async fn theme_set_mode(mode: ThemeMode) -> Result<(), String> {
    let mode_str = mode.as_str();
    config_set(
        ConfigFile::Appearance,
        "theme.mode".into(),
        serde_json::Value::String(mode_str.into()),
    )
    .await?;
    let active = if mode_str == "auto" { "dark" } else { mode_str };
    config_set(
        ConfigFile::Appearance,
        "theme.active".into(),
        serde_json::Value::String(active.into()),
    )
    .await?;
    Ok(())
}

/// Set the accent color (hex string like `#3b82f6`).
#[tauri::command]
pub async fn theme_set_accent(color: String) -> Result<(), String> {
    config_set(
        ConfigFile::Appearance,
        "overrides.accent".into(),
        serde_json::Value::String(color),
    )
    .await
}

/// A theme as the gallery lists it: identity + a resolved preview swatch.
/// Mirrors the desktop-shell `ThemeInfo` and adds `swatch` so the gallery
/// renders real colours instead of a fixture.
#[derive(Debug, Clone, Serialize)]
pub struct ThemeSummary {
    /// Theme id (the `[meta].id`, also the `theme.active` value).
    pub id: String,
    /// Display name.
    pub name: String,
    /// `"dark"` or `"light"`.
    pub variant: String,
    /// True for the built-in dark/light themes, false for user-installed.
    pub is_builtin: bool,
    /// Five representative resolved colours, hex: background, surface,
    /// accent, a structural mid-tone, foreground. The gallery paints these
    /// as the preview strip.
    pub swatch: Vec<String>,
}

/// The five preview colours for a resolved theme, in the gallery's order
/// (bg / surface / accent / secondary-structural / fg).
fn swatch_of(theme: &arlen_theme::ArlenTheme) -> Vec<String> {
    use arlen_theme::gtk::rgba_to_hex;
    let c = &theme.color;
    vec![
        rgba_to_hex(c.bg_app),
        rgba_to_hex(c.bg_card),
        rgba_to_hex(c.accent),
        rgba_to_hex(c.border_strong),
        rgba_to_hex(c.fg_primary),
    ]
}

/// Resolve a theme file's TOML into a gallery summary. Returns `None` if the
/// content does not resolve (a malformed user theme is skipped, not fatal).
fn summary_of(content: &str, is_builtin: bool) -> Option<ThemeSummary> {
    let theme = arlen_theme::ArlenTheme::from_bundled(content).ok()?;
    Some(ThemeSummary {
        id: theme.meta.id.clone(),
        name: theme.meta.name.clone(),
        variant: if theme.is_dark() { "dark" } else { "light" }.to_string(),
        is_builtin,
        swatch: swatch_of(&theme),
    })
}

/// List every available theme (built-in dark/light + user-installed under
/// `~/.local/share/arlen/themes/`), each resolved through `sdk/theme` so the
/// gallery previews are real. A user theme that fails to resolve is skipped.
#[tauri::command]
pub fn get_available_themes() -> Vec<ThemeSummary> {
    let mut out = Vec::new();
    for content in [arlen_theme::DARK_TOML, arlen_theme::LIGHT_TOML] {
        if let Some(summary) = summary_of(content, true) {
            out.push(summary);
        }
    }
    if let Ok(entries) = std::fs::read_dir(arlen_theme::ArlenTheme::user_themes_dir()) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("toml") {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    if let Some(summary) = summary_of(&content, false) {
                        out.push(summary);
                    }
                }
            }
        }
    }
    out
}

/// Switch the active theme: persist `appearance.toml [theme].active = id` and
/// emit `config:appearance:changed` so listeners re-resolve immediately. The
/// file watcher fires on the write too, but emitting directly makes the switch
/// feel instant instead of waiting on the debounce.
#[tauri::command]
pub async fn set_theme(id: String, app: AppHandle) -> Result<(), String> {
    config_set(
        ConfigFile::Appearance,
        "theme.active".into(),
        serde_json::Value::String(id),
    )
    .await?;
    if let Err(e) = app.emit("config:appearance:changed", ()) {
        log::warn!("set_theme: emit config:appearance:changed failed: {e}");
    }
    Ok(())
}

/// The currently active theme id (`appearance.toml [theme].active`), defaulting
/// to `"dark"` when the key is unset so the gallery always has a selection.
#[tauri::command]
pub fn get_active_theme_id() -> Result<String, String> {
    let value = config_get(ConfigFile::Appearance, Some("theme.active".into()))?;
    Ok(value
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| "dark".to_string()))
}

//! Desktop-shell-specific theme schema.
//!
//! The full token hierarchy lives in `lunaris-theme` (sdk/theme) and
//! is re-exported here for callers that previously imported from
//! this module. The types defined HERE are desktop-shell-side
//! configuration concerns:
//!
//! - `AppearanceConfig` / sub-structs: `~/.config/lunaris/appearance.toml`
//!   schema (theme selection + accessibility prefs + radius_intensity)
//! - `ThemeInfo`: lightweight summary for the Settings theme picker
//!
//! The `[radius]` and color tokens themselves are NEVER defined here
//! — they live in the canonical `lunaris-theme` schema. This module
//! only configures *which* theme is active and *how* the user
//! overlay layers on top.

use serde::{Deserialize, Serialize};

// Re-exports: callers that used to `use crate::theme::schema::ThemeTokens`
// transparently switch to the canonical `lunaris-theme` types.
pub use lunaris_theme::{
    ColorTokens, CursorTokens, DepthTokens, LunarisTheme, MotionTokens, RadiusTokens,
    SpacingTokens, ThemeMeta, ThemeVariant, TypographyTokens, WmTokens,
};

// ---------------------------------------------------------------------------
// User config (appearance.toml)
// ---------------------------------------------------------------------------

/// Top-level appearance config (`~/.config/lunaris/appearance.toml`).
///
/// Distinct from `~/.config/lunaris/theme.toml` (the full theme
/// customization overlay handled by `lunaris_theme`): this file
/// captures *preferences* like which theme is active, accessibility
/// options, and the user's radius-intensity multiplier. It does
/// **not** define theme data itself.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AppearanceConfig {
    /// Which theme to activate.
    #[serde(default)]
    pub theme: ThemeSelection,
    /// User overlay: pointers into the active theme.
    #[serde(default)]
    pub overrides: UserOverrides,
    /// Accessibility preferences.
    #[serde(default)]
    pub accessibility: AccessibilitySettings,
    /// Window decoration preferences (border width — radius lives
    /// in the theme, intensity is in `[overrides]`).
    #[serde(default)]
    pub window: WindowSection,
}

/// `[theme]` section.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemeSelection {
    /// Theme id to activate. Must match one of the bundled themes
    /// (`dark`, `light`) or a user theme at
    /// `~/.local/share/lunaris/themes/{id}.toml`.
    #[serde(default = "default_theme_active")]
    pub active: String,
}

fn default_theme_active() -> String {
    "dark".to_string()
}

impl Default for ThemeSelection {
    fn default() -> Self {
        Self {
            active: default_theme_active(),
        }
    }
}

/// `[overrides]` section. The `accent` and `font_scale` fields layer
/// on top of the active theme; `radius_intensity` is the user's
/// global radius multiplier (see `LunarisTheme::effective_*`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UserOverrides {
    /// Custom accent color (`#rrggbb`) or the sentinel
    /// `$foreground` to bind the accent to the active theme's
    /// primary text color (auto-flips with dark/light).
    #[serde(default)]
    pub accent: Option<String>,
    /// Font scale multiplier (1.0 = default).
    #[serde(default)]
    pub font_scale: Option<f32>,
    /// Radius intensity multiplier. `0.0` = sharp brutalist;
    /// `1.0` = theme defaults; `2.0` = max round. Clamped to
    /// `[0.0, 2.0]` at apply time. Defaults to `None` so a
    /// missing field falls through to the theme's own intensity.
    #[serde(default)]
    pub radius_intensity: Option<f32>,
}

/// `[accessibility]` section.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AccessibilitySettings {
    /// Disable all animations.
    #[serde(default)]
    pub reduce_motion: bool,
}

/// `[window]` section. Border width is a compositor-only setting
/// that doesn't fit the theme schema; it lives here. Corner radius
/// USED to live here as `corner_radius: u32`; that's been replaced
/// by `[overrides].radius_intensity` (semantic, percentage-based).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WindowSection {
    /// Window outline border width in pixels. Zero = no outline.
    #[serde(default)]
    pub border_width: Option<u32>,
}

// ---------------------------------------------------------------------------
// Theme info (UI summary)
// ---------------------------------------------------------------------------

/// Lightweight theme summary returned by the Tauri
/// `get_available_themes` command.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemeInfo {
    pub id: String,
    pub name: String,
    pub variant: ThemeVariant,
    pub is_builtin: bool,
}

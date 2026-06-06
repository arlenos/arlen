//! Theme loading + resolution wrapper.
//!
//! This module is a thin adapter around `lunaris_theme::LunarisTheme`
//! (the SSoT theme schema). It handles:
//!
//! - Compile-time embed of bundled themes (`dark.toml`, `light.toml`)
//!   from `desktop-shell/src-tauri/themes/` via `include_str!`. The
//!   compositor crate `include_str!`s the SAME files (cross-crate)
//!   so both binaries observe the same canonical bytes.
//! - User-installed-themes lookup at
//!   `~/.local/share/lunaris/themes/{id}.toml`.
//! - Resolution chain: bundled bytes → user theme overlay
//!   (via `LunarisTheme::resolve(...)`) → `appearance.toml`
//!   `[overrides]` (accent + font_scale + radius_intensity) →
//!   `[accessibility]` (reduce_motion).
//!
//! See `docs/architecture/theme-system.md` for the full architecture.

use std::path::PathBuf;

use lunaris_theme::{LunarisTheme, ThemeVariant};
use thiserror::Error;

use super::schema::{
    AccessibilitySettings, AppearanceConfig, ThemeInfo, UserOverrides,
};

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum ThemeError {
    #[error("theme not found: {0}")]
    NotFound(String),
    #[error("resolve {path}: {source}")]
    Resolve {
        path: String,
        source: lunaris_theme::ResolveError,
    },
    #[error("parse {path}: {source}")]
    Parse {
        path: String,
        source: toml::de::Error,
    },
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("serialize: {0}")]
    Serialize(String),
}

// ---------------------------------------------------------------------------
// Bundled themes (compile-time embedded)
// ---------------------------------------------------------------------------

const DARK_TOML: &str = include_str!("../../themes/dark.toml");
const LIGHT_TOML: &str = include_str!("../../themes/light.toml");

/// Sentinel value in `[overrides].accent` that binds the accent to
/// the active theme's primary foreground color. Lets users pick a
/// "monochrome" accent that automatically flips with dark/light
/// mode instead of freezing a single hex value.
pub const ACCENT_FOREGROUND_SENTINEL: &str = "$foreground";

// ---------------------------------------------------------------------------
// Loader
// ---------------------------------------------------------------------------

/// Manages bundled and user-installed themes.
pub struct ThemeLoader {
    user_dir: Option<PathBuf>,
}

impl ThemeLoader {
    /// Create a loader with the default user-themes directory.
    pub fn new() -> Result<Self, ThemeError> {
        let user_dir = dirs::data_dir()
            .map(|d| d.join("lunaris").join("themes"))
            .filter(|d| d.is_dir());
        Ok(Self { user_dir })
    }

    /// Create a loader with an explicit user-themes directory
    /// (used by tests + non-default config locations).
    pub fn new_with_user_dir(user_dir: PathBuf) -> Result<Self, ThemeError> {
        Ok(Self {
            user_dir: Some(user_dir).filter(|d| d.is_dir()),
        })
    }

    /// Get the bundled theme bytes for the given id.
    fn bundled_for(id: &str) -> Option<&'static str> {
        match id {
            "dark" => Some(DARK_TOML),
            "light" => Some(LIGHT_TOML),
            _ => None,
        }
    }

    /// Resolve an active theme id into a `LunarisTheme`. Layering:
    ///
    /// 1. Bundled bytes (matched by id; `dark` falls back if id is
    ///    unknown so a missing user theme still gives a usable shell).
    /// 2. User-installed theme overlay if present at
    ///    `{user_dir}/{id}.toml`.
    /// 3. User customization (`~/.config/lunaris/theme.toml`) is read
    ///    inside `lunaris_theme::resolve` itself when loaders pass it
    ///    via the `customization` arg — we read it here from the
    ///    standard path.
    pub fn load(&self, id: &str) -> Result<LunarisTheme, ThemeError> {
        // 1. Pick the bundled bytes. If the user requested an id
        // that isn't bundled, use dark as the floor — the user
        // theme overlay will rebrand from there.
        let bundled = Self::bundled_for(id).unwrap_or(DARK_TOML);

        // 2. User theme overlay.
        let user_overlay = if let Some(ref dir) = self.user_dir {
            let path = dir.join(format!("{id}.toml"));
            if path.exists() {
                Some(std::fs::read_to_string(&path)?)
            } else if Self::bundled_for(id).is_none() {
                // Asked for non-bundled, non-existent id.
                return Err(ThemeError::NotFound(id.into()));
            } else {
                None
            }
        } else if Self::bundled_for(id).is_none() {
            return Err(ThemeError::NotFound(id.into()));
        } else {
            None
        };

        // 3. ~/.config/lunaris/theme.toml customization.
        let custom_path = LunarisTheme::user_customization_path();
        let customization = if custom_path.exists() {
            Some(std::fs::read_to_string(&custom_path)?)
        } else {
            None
        };

        LunarisTheme::resolve(
            bundled,
            user_overlay.as_deref(),
            customization.as_deref(),
        )
        .map_err(|e| ThemeError::Resolve {
            path: format!("active={id}"),
            source: e,
        })
    }

    /// List all available themes (bundled + user).
    pub fn list_themes(&self) -> Vec<ThemeInfo> {
        let mut out: Vec<ThemeInfo> = ["dark", "light"]
            .iter()
            .filter_map(|id| {
                let bundled = Self::bundled_for(id)?;
                let theme = LunarisTheme::from_bundled(bundled).ok()?;
                Some(ThemeInfo {
                    id: theme.meta.id.clone(),
                    name: theme.meta.name.clone(),
                    variant: theme.meta.variant,
                    is_builtin: true,
                })
            })
            .collect();

        if let Some(ref dir) = self.user_dir {
            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().map(|e| e == "toml").unwrap_or(false) {
                        if let Ok(content) = std::fs::read_to_string(&path) {
                            if let Ok(theme) = LunarisTheme::from_bundled(&content) {
                                if !out.iter().any(|t| t.id == theme.meta.id) {
                                    out.push(ThemeInfo {
                                        id: theme.meta.id.clone(),
                                        name: theme.meta.name.clone(),
                                        variant: theme.meta.variant,
                                        is_builtin: false,
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }

        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }
}

// ---------------------------------------------------------------------------
// Override / accessibility application
// ---------------------------------------------------------------------------

/// Apply user `[overrides]` from `appearance.toml` to a resolved
/// theme. Accent + font_scale + radius_intensity all layer here.
///
/// Accent: re-derives `accent_hover` (lighten 15%) and
/// `accent_pressed` (darken 15%) so the theme doesn't show a
/// stale hover state with a brand-new accent.
///
/// Radius intensity: when `Some`, replaces the theme's
/// `radius.intensity`. When `None`, the theme's intensity stays.
pub fn apply_overrides(mut theme: LunarisTheme, overrides: &UserOverrides) -> LunarisTheme {
    if let Some(ref accent_raw) = overrides.accent {
        // Resolve the foreground sentinel.
        let resolved = if accent_raw == ACCENT_FOREGROUND_SENTINEL {
            Some(rgba_to_hex_string(&theme.color.fg_primary))
        } else if is_valid_hex_color(accent_raw) {
            Some(accent_raw.clone())
        } else {
            None
        };

        if let Some(hex) = resolved {
            if let Some(rgba) = lunaris_theme::parse_hex(&hex) {
                theme.color.accent = rgba;
                if let Some(h) = lighten_color(&hex, 0.15) {
                    if let Some(c) = lunaris_theme::parse_hex(&h) {
                        theme.color.accent_hover = c;
                    }
                }
                if let Some(d) = darken_color(&hex, 0.15) {
                    if let Some(c) = lunaris_theme::parse_hex(&d) {
                        theme.color.accent_pressed = c;
                    }
                }
            }
        }
    }

    if let Some(intensity) = overrides.radius_intensity {
        theme.radius.intensity = intensity;
    }

    theme
}

/// Apply accessibility settings (reduce_motion = "0ms" durations).
pub fn apply_accessibility(
    mut theme: LunarisTheme,
    settings: &AccessibilitySettings,
) -> LunarisTheme {
    if settings.reduce_motion {
        theme.motion.duration_fast = "0ms".into();
        theme.motion.duration_normal = "0ms".into();
        theme.motion.duration_slow = "0ms".into();
    }
    theme
}

/// Full resolution pipeline.
pub fn resolve_theme(
    loader: &ThemeLoader,
    config: &AppearanceConfig,
) -> Result<LunarisTheme, ThemeError> {
    let theme = loader.load(&config.theme.active)?;
    let theme = apply_overrides(theme, &config.overrides);
    let theme = apply_accessibility(theme, &config.accessibility);
    Ok(theme)
}

// ---------------------------------------------------------------------------
// Helpers (color manipulation + validation)
// ---------------------------------------------------------------------------

/// Validate a CSS hex color (3/4/6/8 digit).
pub fn is_valid_hex_color(color: &str) -> bool {
    if !color.starts_with('#') {
        return false;
    }
    let hex = &color[1..];
    matches!(hex.len(), 3 | 4 | 6 | 8) && hex.chars().all(|c| c.is_ascii_hexdigit())
}

fn rgba_to_hex_string(rgba: &lunaris_theme::Rgba) -> String {
    let r = (rgba[0] * 255.0).round().clamp(0.0, 255.0) as u8;
    let g = (rgba[1] * 255.0).round().clamp(0.0, 255.0) as u8;
    let b = (rgba[2] * 255.0).round().clamp(0.0, 255.0) as u8;
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// Lighten a hex color by `amount` (0.0-1.0). 6-digit hex only.
pub fn lighten_color(hex: &str, amount: f32) -> Option<String> {
    let (r, g, b) = parse_hex_rgb(hex)?;
    let r = (r as f32 + (255.0 - r as f32) * amount).round().min(255.0) as u8;
    let g = (g as f32 + (255.0 - g as f32) * amount).round().min(255.0) as u8;
    let b = (b as f32 + (255.0 - b as f32) * amount).round().min(255.0) as u8;
    Some(format!("#{r:02x}{g:02x}{b:02x}"))
}

/// Darken a hex color by `amount` (0.0-1.0). 6-digit hex only.
pub fn darken_color(hex: &str, amount: f32) -> Option<String> {
    let (r, g, b) = parse_hex_rgb(hex)?;
    let r = (r as f32 * (1.0 - amount)).round().max(0.0) as u8;
    let g = (g as f32 * (1.0 - amount)).round().max(0.0) as u8;
    let b = (b as f32 * (1.0 - amount)).round().max(0.0) as u8;
    Some(format!("#{r:02x}{g:02x}{b:02x}"))
}

fn parse_hex_rgb(hex: &str) -> Option<(u8, u8, u8)> {
    let hex = hex.strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some((r, g, b))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_themes_resolve() {
        let loader = ThemeLoader::new().unwrap();
        let dark = loader.load("dark").unwrap();
        assert_eq!(dark.meta.id, "dark");
        assert_eq!(dark.meta.variant, ThemeVariant::Dark);
        let light = loader.load("light").unwrap();
        assert_eq!(light.meta.id, "light");
        assert_eq!(light.meta.variant, ThemeVariant::Light);
    }

    #[test]
    fn unknown_id_with_no_user_dir_errors() {
        let loader = ThemeLoader::new_with_user_dir(PathBuf::from("/nonexistent")).unwrap();
        // user_dir filtered out by is_dir(), but bundled fallback
        // doesn't apply — unknown id returns NotFound.
        assert!(loader.load("nonexistent").is_err());
    }

    #[test]
    fn list_includes_builtins() {
        let loader = ThemeLoader::new().unwrap();
        let themes = loader.list_themes();
        assert!(themes.iter().any(|t| t.id == "dark"));
        assert!(themes.iter().any(|t| t.id == "light"));
    }

    #[test]
    fn hex_color_validation() {
        assert!(is_valid_hex_color("#ff00ff"));
        assert!(is_valid_hex_color("#F0F"));
        assert!(is_valid_hex_color("#ff00ff80"));
        assert!(!is_valid_hex_color("ff00ff"));
        assert!(!is_valid_hex_color("#xyz"));
        assert!(!is_valid_hex_color("#12345"));
    }

    #[test]
    fn lighten_darken() {
        assert_eq!(lighten_color("#000000", 0.5), Some("#808080".into()));
        assert_eq!(darken_color("#ffffff", 0.5), Some("#808080".into()));
    }

    #[test]
    fn intensity_override_applies() {
        let loader = ThemeLoader::new().unwrap();
        let theme = loader.load("dark").unwrap();
        let overrides = UserOverrides {
            accent: None,
            font_scale: None,
            radius_intensity: Some(1.5),
        };
        let result = apply_overrides(theme, &overrides);
        assert_eq!(result.radius.intensity, 1.5);
    }

    #[test]
    fn reduce_motion_zeroes_durations() {
        let loader = ThemeLoader::new().unwrap();
        let theme = loader.load("dark").unwrap();
        let result = apply_accessibility(
            theme,
            &AccessibilitySettings { reduce_motion: true },
        );
        assert_eq!(result.motion.duration_fast,   "0ms");
        assert_eq!(result.motion.duration_normal, "0ms");
        assert_eq!(result.motion.duration_slow,   "0ms");
    }
}

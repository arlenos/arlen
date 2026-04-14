/// Theme loading, resolution, and override application.
///
/// Built-in themes are embedded at compile time via `include_str!`. User themes
/// are loaded from `~/.local/share/lunaris/themes/`. The resolution chain:
///
/// 1. Load base theme by ID (built-in or user).
/// 2. Apply `extends` chain (if the theme inherits from another).
/// 3. Apply user overrides (accent color, font scale).
/// 4. Apply accessibility settings (reduce_motion).

use std::collections::HashMap;
use std::path::PathBuf;

use thiserror::Error;

use super::schema::{
    AccessibilitySettings, AppearanceConfig, ThemeInfo, ThemeTokens, UserOverrides,
};

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors that can occur during theme loading or resolution.
#[derive(Debug, Error)]
pub enum ThemeError {
    #[error("theme not found: {0}")]
    NotFound(String),
    #[error("parse error in {path}: {source}")]
    Parse {
        path: String,
        source: toml::de::Error,
    },
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("circular extends chain: {0}")]
    CircularExtends(String),
    #[error("serialize error: {0}")]
    Serialize(String),
}

// ---------------------------------------------------------------------------
// Built-in themes (embedded at compile time)
// ---------------------------------------------------------------------------

const DARK_TOML: &str = include_str!("../../themes/dark.toml");
const LIGHT_TOML: &str = include_str!("../../themes/light.toml");

// ---------------------------------------------------------------------------
// ThemeLoader
// ---------------------------------------------------------------------------

/// Manages built-in and user-installed themes.
pub struct ThemeLoader {
    builtin: HashMap<String, ThemeTokens>,
    user_dir: Option<PathBuf>,
}

impl ThemeLoader {
    /// Create a loader with compiled-in themes and the default user theme
    /// directory.
    pub fn new() -> Result<Self, ThemeError> {
        let mut builtin = HashMap::new();

        let dark: ThemeTokens = toml::from_str(DARK_TOML).map_err(|e| ThemeError::Parse {
            path: "builtin:dark.toml".into(),
            source: e,
        })?;
        let light: ThemeTokens = toml::from_str(LIGHT_TOML).map_err(|e| ThemeError::Parse {
            path: "builtin:light.toml".into(),
            source: e,
        })?;

        builtin.insert(dark.meta.id.clone(), dark);
        builtin.insert(light.meta.id.clone(), light);

        let user_dir = dirs::data_dir()
            .map(|d| d.join("lunaris").join("themes"))
            .filter(|d| d.is_dir());

        Ok(Self { builtin, user_dir })
    }

    /// Create a loader with an explicit user themes directory.
    pub fn new_with_user_dir(user_dir: PathBuf) -> Result<Self, ThemeError> {
        let mut loader = Self::new()?;
        loader.user_dir = Some(user_dir).filter(|d| d.is_dir());
        Ok(loader)
    }

    /// Load a theme by ID. Checks built-in themes first, then the user
    /// directory.
    pub fn load(&self, id: &str) -> Result<ThemeTokens, ThemeError> {
        if let Some(tokens) = self.builtin.get(id) {
            return Ok(tokens.clone());
        }

        if let Some(ref dir) = self.user_dir {
            let path = dir.join(format!("{id}.toml"));
            if path.exists() {
                let content = std::fs::read_to_string(&path)?;
                let tokens: ThemeTokens =
                    toml::from_str(&content).map_err(|e| ThemeError::Parse {
                        path: path.display().to_string(),
                        source: e,
                    })?;
                return Ok(tokens);
            }
        }

        Err(ThemeError::NotFound(id.into()))
    }

    /// List all available themes (built-in + user).
    pub fn list_themes(&self) -> Vec<ThemeInfo> {
        let mut themes: Vec<ThemeInfo> = self
            .builtin
            .values()
            .map(|t| ThemeInfo {
                id: t.meta.id.clone(),
                name: t.meta.name.clone(),
                variant: t.meta.variant,
                is_builtin: true,
            })
            .collect();

        if let Some(ref dir) = self.user_dir {
            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().map(|e| e == "toml").unwrap_or(false) {
                        if let Ok(content) = std::fs::read_to_string(&path) {
                            if let Ok(tokens) = toml::from_str::<ThemeTokens>(&content) {
                                if !themes.iter().any(|t| t.id == tokens.meta.id) {
                                    themes.push(ThemeInfo {
                                        id: tokens.meta.id.clone(),
                                        name: tokens.meta.name.clone(),
                                        variant: tokens.meta.variant,
                                        is_builtin: false,
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }

        themes.sort_by(|a, b| a.name.cmp(&b.name));
        themes
    }
}

// ---------------------------------------------------------------------------
// Override / accessibility application
// ---------------------------------------------------------------------------

/// Sentinel value in `[overrides].accent` that binds the accent to the
/// active theme's primary foreground. Lets users pick a "monochrome"
/// accent that automatically flips with dark/light mode instead of
/// freezing a single hex value.
pub const ACCENT_FOREGROUND_SENTINEL: &str = "$foreground";

/// Apply user overrides (accent color, font scale) to a set of tokens.
pub fn apply_overrides(mut tokens: ThemeTokens, overrides: &UserOverrides) -> ThemeTokens {
    if let Some(ref accent) = overrides.accent {
        // Resolve the sentinel before the hex check so users can opt into
        // a theme-bound monochrome accent from the Settings app.
        let resolved = if accent == ACCENT_FOREGROUND_SENTINEL {
            Some(tokens.colors.foreground.primary.clone())
        } else if is_valid_hex_color(accent) {
            Some(accent.clone())
        } else {
            None
        };

        if let Some(hex) = resolved {
            tokens.colors.semantic.accent_hover =
                lighten_color(&hex, 0.15).unwrap_or_else(|| hex.clone());
            tokens.colors.semantic.accent_pressed =
                darken_color(&hex, 0.15).unwrap_or_else(|| hex.clone());
            tokens.colors.semantic.accent = hex;
        }
    }

    if let Some(scale) = overrides.font_scale {
        if (0.5..=2.0).contains(&scale) {
            let base: f32 = tokens
                .typography
                .size_base
                .trim_end_matches("px")
                .parse()
                .unwrap_or(14.0);
            tokens.typography.size_base = format!("{:.0}px", base * scale);
        }
    }

    tokens
}

/// Apply accessibility settings (reduce_motion sets all durations to "0ms").
pub fn apply_accessibility(
    mut tokens: ThemeTokens,
    settings: &AccessibilitySettings,
) -> ThemeTokens {
    if settings.reduce_motion {
        tokens.motion.duration_fast = "0ms".into();
        tokens.motion.duration_normal = "0ms".into();
        tokens.motion.duration_slow = "0ms".into();
    }
    tokens
}

/// Full resolution pipeline: load theme, apply overrides, apply accessibility.
pub fn resolve_theme(
    loader: &ThemeLoader,
    config: &AppearanceConfig,
) -> Result<ThemeTokens, ThemeError> {
    let base = loader.load(&config.theme.active)?;

    // Resolve extends chain (max depth 4 to prevent infinite loops).
    let tokens = resolve_extends(loader, base, 4)?;

    let tokens = apply_overrides(tokens, &config.overrides);
    let tokens = apply_accessibility(tokens, &config.accessibility);
    let tokens = apply_window_overrides(tokens, &config.window);

    Ok(tokens)
}

/// Apply `[window]` overrides from appearance.toml. Today only
/// `corner_radius` is honoured — it overrides `radius.md` and derives
/// sm/lg from it so all three tiers stay consistent.
pub fn apply_window_overrides(
    mut tokens: ThemeTokens,
    window: &crate::theme::schema::WindowSection,
) -> ThemeTokens {
    if let Some(radius) = window.corner_radius {
        let md = radius;
        // Derive sm and lg so the whole scale shifts with the slider.
        // Minimum sm = 2px, maximum lg = 2x md (matches the built-in
        // dark.toml ratio of 4/8/12).
        let sm = md.saturating_sub(4).max(2);
        let lg = md.saturating_add(4);
        tokens.radius.sm = format!("{sm}px");
        tokens.radius.md = format!("{md}px");
        tokens.radius.lg = format!("{lg}px");
    }
    tokens
}

/// Resolve the `extends` chain by inheriting missing values from the parent.
/// Currently themes are self-contained, so this is a no-op unless `extends`
/// is set. Reserved for future use (e.g. a "nord" theme extending "dark").
fn resolve_extends(
    loader: &ThemeLoader,
    theme: ThemeTokens,
    depth: u8,
) -> Result<ThemeTokens, ThemeError> {
    if depth == 0 {
        return Err(ThemeError::CircularExtends(theme.meta.id.clone()));
    }

    // If no extends, return as-is.
    let Some(ref parent_id) = theme.meta.extends else {
        return Ok(theme);
    };

    let _parent = loader.load(parent_id)?;
    // For now, child themes are fully self-contained. Inheritance (merging
    // parent values for missing fields) is deferred until we have a concrete
    // use case.
    let _ = resolve_extends(loader, _parent, depth - 1)?;

    Ok(theme)
}

// ---------------------------------------------------------------------------
// Color helpers
// ---------------------------------------------------------------------------

/// Validate that a string is a 3, 4, 6, or 8 digit hex color with `#` prefix.
pub fn is_valid_hex_color(color: &str) -> bool {
    if !color.starts_with('#') {
        return false;
    }
    let hex = &color[1..];
    matches!(hex.len(), 3 | 4 | 6 | 8) && hex.chars().all(|c| c.is_ascii_hexdigit())
}

/// Lighten a hex color by `amount` (0.0 to 1.0). Returns None on parse error.
pub fn lighten_color(hex: &str, amount: f32) -> Option<String> {
    let (r, g, b) = parse_hex_rgb(hex)?;
    let r = (r as f32 + (255.0 - r as f32) * amount).round().min(255.0) as u8;
    let g = (g as f32 + (255.0 - g as f32) * amount).round().min(255.0) as u8;
    let b = (b as f32 + (255.0 - b as f32) * amount).round().min(255.0) as u8;
    Some(format!("#{r:02x}{g:02x}{b:02x}"))
}

/// Darken a hex color by `amount` (0.0 to 1.0). Returns None on parse error.
pub fn darken_color(hex: &str, amount: f32) -> Option<String> {
    let (r, g, b) = parse_hex_rgb(hex)?;
    let r = (r as f32 * (1.0 - amount)).round().max(0.0) as u8;
    let g = (g as f32 * (1.0 - amount)).round().max(0.0) as u8;
    let b = (b as f32 * (1.0 - amount)).round().max(0.0) as u8;
    Some(format!("#{r:02x}{g:02x}{b:02x}"))
}

/// Parse a 6-digit hex color into (R, G, B).
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
    fn builtin_themes_parse() {
        let loader = ThemeLoader::new().unwrap();
        assert!(loader.load("dark").is_ok());
        assert!(loader.load("light").is_ok());
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
        assert_eq!(lighten_color("#ff0000", 0.0), Some("#ff0000".into()));
    }

    #[test]
    fn apply_overrides_accent() {
        let loader = ThemeLoader::new().unwrap();
        let tokens = loader.load("dark").unwrap();
        let overrides = UserOverrides {
            accent: Some("#ff0000".into()),
            font_scale: None,
        };
        let result = apply_overrides(tokens, &overrides);
        assert_eq!(result.colors.semantic.accent, "#ff0000");
        assert_ne!(result.colors.semantic.accent_hover, "#ff0000");
    }

    #[test]
    fn apply_reduce_motion() {
        let loader = ThemeLoader::new().unwrap();
        let tokens = loader.load("dark").unwrap();
        let settings = AccessibilitySettings {
            reduce_motion: true,
        };
        let result = apply_accessibility(tokens, &settings);
        assert_eq!(result.motion.duration_fast, "0ms");
        assert_eq!(result.motion.duration_normal, "0ms");
        assert_eq!(result.motion.duration_slow, "0ms");
    }

    #[test]
    fn resolve_default_config() {
        let loader = ThemeLoader::new().unwrap();
        let config = AppearanceConfig::default();
        let tokens = resolve_theme(&loader, &config).unwrap();
        assert_eq!(tokens.meta.id, "dark");
    }

    #[test]
    fn font_scale_applied() {
        let loader = ThemeLoader::new().unwrap();
        let tokens = loader.load("dark").unwrap();
        let overrides = UserOverrides {
            accent: None,
            font_scale: Some(1.5),
        };
        let result = apply_overrides(tokens, &overrides);
        assert_eq!(result.typography.size_base, "21px");
    }
}

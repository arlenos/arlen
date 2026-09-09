//! Theme loading + resolution wrapper.
//!
//! This module is a thin adapter around `arlen_theme::ArlenTheme`
//! (the SSoT theme schema). It handles:
//!
//! - Compile-time embed of bundled themes (`dark.toml`, `light.toml`)
//!   from `desktop-shell/src-tauri/themes/` via `include_str!`. The
//!   compositor crate `include_str!`s the SAME files (cross-crate)
//!   so both binaries observe the same canonical bytes.
//! - User-installed-themes lookup at
//!   `~/.local/share/arlen/themes/{id}.toml`.
//! - Resolution chain: bundled bytes → user theme overlay
//!   (via `ArlenTheme::resolve(...)`) → `appearance.toml`
//!   `[overrides]` (accent + font_scale + radius_intensity) →
//!   `[accessibility]` (reduce_motion).
//!
//! See `docs/architecture/theme-system.md` for the full architecture.

use std::path::PathBuf;

use arlen_theme::ArlenTheme;
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
        source: arlen_theme::ResolveError,
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

// The bundled defaults live in the shared arlen-theme crate (single source for
// the shell and the compositor), not in this app's tree.
const DARK_TOML: &str = arlen_theme::DARK_TOML;
const LIGHT_TOML: &str = arlen_theme::LIGHT_TOML;

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
    /// Where the customization layer lives.
    ///
    /// Held rather than looked up per load, and that is the difference between
    /// a loader a test can drive and one that reads whoever is running it. The
    /// per-toolkit resolve made this matter: `[override.gtk]` decides what goes
    /// into a generated `gtk.css`, so a test that writes toolkit files while the
    /// loader reads the DEVELOPER's `~/.config/arlen/theme.toml` is asserting
    /// against their desktop.
    customization_path: PathBuf,
}

impl ThemeLoader {
    /// Create a loader with the default user-themes directory.
    pub fn new() -> Result<Self, ThemeError> {
        let user_dir = dirs::data_dir()
            .map(|d| d.join("arlen").join("themes"))
            .filter(|d| d.is_dir());
        Ok(Self {
            user_dir,
            customization_path: ArlenTheme::user_customization_path(),
        })
    }

    /// Create a loader with an explicit user-themes directory
    /// (used by tests + non-default config locations).
    pub fn new_with_user_dir(user_dir: PathBuf) -> Result<Self, ThemeError> {
        Ok(Self {
            user_dir: Some(user_dir).filter(|d| d.is_dir()),
            customization_path: ArlenTheme::user_customization_path(),
        })
    }

    /// Read the customization layer from `path` instead of the default.
    ///
    /// The shell knows its own config directory - it was handed one - so it says
    /// which `theme.toml` it means rather than asking `dirs` and hoping the two
    /// agree. Under a test they do not: the config dir is a temp one and `dirs`
    /// answers with the developer's home.
    pub fn with_customization_path(mut self, path: PathBuf) -> Self {
        self.customization_path = path;
        self
    }

    /// Get the bundled theme bytes for the given id.
    fn bundled_for(id: &str) -> Option<&'static str> {
        match id {
            "dark" => Some(DARK_TOML),
            "light" => Some(LIGHT_TOML),
            _ => None,
        }
    }

    /// Resolve an active theme id into a `ArlenTheme`. Layering:
    ///
    /// 1. Bundled bytes (matched by id; `dark` falls back if id is
    ///    unknown so a missing user theme still gives a usable shell).
    /// 2. User-installed theme overlay if present at
    ///    `{user_dir}/{id}.toml`.
    /// 3. User customization (`~/.config/arlen/theme.toml`) is read
    ///    inside `arlen_theme::resolve` itself when loaders pass it
    ///    via the `customization` arg — we read it here from the
    ///    standard path.
    pub fn load(&self, id: &str) -> Result<ArlenTheme, ThemeError> {
        self.load_for(id, None)
    }

    /// The same load, resolved AS ONE TOOLKIT'S GENERATOR SEES IT: the shared
    /// theme plus that toolkit's `[override.<toolkit>]` block.
    ///
    /// A theme with no override for `toolkit` resolves identically, so this is
    /// safe to call for every generator - which is what lets the apply step ask
    /// per toolkit rather than deciding whether an override exists first.
    pub fn load_for(
        &self,
        id: &str,
        toolkit: Option<arlen_theme::Toolkit>,
    ) -> Result<ArlenTheme, ThemeError> {
        // 1. User theme overlay. Read first so a non-bundled theme can
        // declare which bundled variant it extends.
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

        // 2. Pick the bundled base bytes. A bundled id uses its own
        // bytes. A user theme uses the bundled variant named by its
        // `[meta] extends` field (default dark), so a light user theme
        // does not inherit dark tokens for the fields it leaves unset.
        let bundled = match Self::bundled_for(id) {
            Some(bytes) => bytes,
            None => {
                let extends = user_overlay
                    .as_deref()
                    .and_then(extends_base)
                    .unwrap_or_else(|| "dark".to_string());
                Self::bundled_for(&extends).unwrap_or(DARK_TOML)
            }
        };

        // 3. The customization layer (`theme.toml`), from wherever this loader
        // was told it lives.
        let customization = if self.customization_path.exists() {
            Some(std::fs::read_to_string(&self.customization_path)?)
        } else {
            None
        };

        match toolkit {
            Some(tk) => ArlenTheme::resolve_toolkit(
                bundled,
                user_overlay.as_deref(),
                customization.as_deref(),
                tk,
            ),
            None => ArlenTheme::resolve(
                bundled,
                user_overlay.as_deref(),
                customization.as_deref(),
            ),
        }
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
                let theme = ArlenTheme::from_bundled(bundled).ok()?;
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
                            if let Ok(theme) = ArlenTheme::from_bundled(&content) {
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

/// Read the `[meta] extends` field from a user theme's TOML, naming the
/// bundled variant it builds on. Returns `None` if the field is absent or
/// the TOML does not parse, so the caller falls back to the dark base.
fn extends_base(overlay: &str) -> Option<String> {
    let value: toml::Value = overlay.parse().ok()?;
    value
        .get("meta")?
        .get("extends")?
        .as_str()
        .map(str::to_owned)
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
pub fn apply_overrides(mut theme: ArlenTheme, overrides: &UserOverrides) -> ArlenTheme {
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
            if let Some(rgba) = arlen_theme::parse_hex(&hex) {
                theme.color.accent = rgba;
                if let Some(h) = lighten_color(&hex, 0.15) {
                    if let Some(c) = arlen_theme::parse_hex(&h) {
                        theme.color.accent_hover = c;
                    }
                }
                if let Some(d) = darken_color(&hex, 0.15) {
                    if let Some(c) = arlen_theme::parse_hex(&d) {
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
    mut theme: ArlenTheme,
    settings: &AccessibilitySettings,
) -> ArlenTheme {
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
) -> Result<ArlenTheme, ThemeError> {
    resolve_theme_for(loader, config, None)
}

/// The same pipeline, as one toolkit's generator sees it.
///
/// **The accent has two claimants here and the more specific one wins.** The
/// toolkit override is a theme-FILE layer, so a naive chain would then let
/// `appearance.toml`'s accent - which the Appearance page writes and which is
/// applied after the file layers - paint straight over it, and a person who gave
/// GTK its own accent would watch it have no effect for the single reason that
/// they had also picked an accent at all. That is the common case, not the
/// corner. So when the toolkit's override declares an accent, the global one is
/// dropped for that toolkit and nothing else about it changes: radius intensity,
/// the font scale and the accessibility pass all still apply.
///
/// Detected by comparing the two resolves rather than by re-reading the file,
/// because the file-reading rule lives in the resolver and a second copy of it
/// here would drift - and a drifted copy does not throw, it silently ignores an
/// override somebody set.
pub fn resolve_theme_for(
    loader: &ThemeLoader,
    config: &AppearanceConfig,
    toolkit: Option<arlen_theme::Toolkit>,
) -> Result<ArlenTheme, ThemeError> {
    let theme = loader.load_for(&config.theme.active, toolkit)?;
    let mut overrides = config.overrides.clone();
    if toolkit.is_some() {
        let shared = loader.load(&config.theme.active)?;
        if toolkit_accent_wins(&shared, &theme, &overrides) {
            overrides.accent = None;
        }
    }
    let theme = apply_overrides(theme, &overrides);
    let theme = apply_accessibility(theme, &config.accessibility);
    Ok(theme)
}

/// Whether this toolkit's own accent should displace `appearance.toml`'s.
///
/// True only when there is a global accent to displace AND the toolkit's resolve
/// actually came out with a different accent than the shared one - which is the
/// evidence that its `[override.<toolkit>]` block said something about the
/// accent, read from the resolve rather than from a second reading of the file.
///
/// Pure so the precedence rule can be tested without a theme file on the
/// machine, which the loader's own path would need: it reads the real
/// `~/.config/arlen/theme.toml`.
pub fn toolkit_accent_wins(
    shared: &ArlenTheme,
    toolkit: &ArlenTheme,
    overrides: &UserOverrides,
) -> bool {
    overrides.accent.is_some() && toolkit.color.accent != shared.color.accent
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

fn rgba_to_hex_string(rgba: &arlen_theme::Rgba) -> String {
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
    use arlen_theme::ThemeVariant;

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
    fn extends_base_reads_meta_extends() {
        assert_eq!(
            extends_base("[meta]\nextends = \"light\"\n").as_deref(),
            Some("light")
        );
        assert_eq!(extends_base("[meta]\nid = \"x\"\n"), None);
        assert_eq!(extends_base("not = valid = toml ="), None);
    }

    #[test]
    fn user_theme_extending_light_uses_the_light_base() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("mylight.toml"),
            "[meta]\nid = \"mylight\"\nname = \"My Light\"\nvariant = \"light\"\nextends = \"light\"\n",
        )
        .unwrap();
        let loader = ThemeLoader::new_with_user_dir(dir.path().to_path_buf()).unwrap();
        // The user theme overrides no colors, so the base supplies every
        // token. Before the fix the base was always dark; extends=light must
        // now floor on the light bundle.
        let resolved = loader.load("mylight").unwrap();
        let light = loader.load("light").unwrap();
        let dark = loader.load("dark").unwrap();
        assert_eq!(resolved.color.bg_shell, light.color.bg_shell);
        assert_ne!(resolved.color.bg_shell, dark.color.bg_shell);
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

#[cfg(test)]
mod toolkit_precedence_tests {
    use super::*;

    fn bundled() -> ArlenTheme {
        ArlenTheme::from_bundled(arlen_theme::DARK_TOML).expect("bundled dark resolves")
    }

    /// The case the rule exists for. Somebody picks an accent on the Appearance
    /// page - the common thing to do - and then gives GTK its own. Without this
    /// the global accent is applied after the file layers and paints straight
    /// over the GTK block, so the second choice does nothing and the reason is
    /// invisible: it works for anyone who never touched the first control.
    #[test]
    fn a_toolkit_with_its_own_accent_keeps_it_over_the_global_one() {
        let shared = bundled();
        let mut gtk = bundled();
        gtk.color.accent = [0.1, 0.8, 0.3, 1.0];
        let overrides = UserOverrides {
            accent: Some("#ff0000".into()),
            ..Default::default()
        };
        assert!(toolkit_accent_wins(&shared, &gtk, &overrides));
    }

    /// And the other side of it, which is what keeps the rule from being a way
    /// to lose the accent everywhere: a toolkit that said nothing about the
    /// accent takes the global one like every other surface.
    #[test]
    fn a_toolkit_that_said_nothing_still_takes_the_global_accent() {
        let overrides = UserOverrides {
            accent: Some("#ff0000".into()),
            ..Default::default()
        };
        assert!(!toolkit_accent_wins(&bundled(), &bundled(), &overrides));
    }

    /// No global accent, nothing to displace. Stated because the function reads
    /// as "does the toolkit win", and the honest answer with no contest is no.
    #[test]
    fn with_no_global_accent_there_is_nothing_to_displace() {
        let mut gtk = bundled();
        gtk.color.accent = [0.1, 0.8, 0.3, 1.0];
        assert!(!toolkit_accent_wins(
            &bundled(),
            &gtk,
            &UserOverrides::default()
        ));
    }
}

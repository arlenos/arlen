//! Lunaris theme system.
//!
//! Loads theme tokens from `~/.config/lunaris/theme.toml`, merges with the
//! built-in Panda defaults, and provides a `ThemeWatcher` for live updates.
//! No Tauri, Iced, or cosmic dependency. Colors are stored as `[f32; 4]` RGBA
//! ready for direct use in GPU shaders and tiny_skia drawing code.

mod file;
mod watcher;

pub use file::LunarisThemeFile;
pub use watcher::ThemeWatcher;

use std::path::{Path, PathBuf};

/// Parsed hex color as RGBA with components in 0.0..=1.0.
pub type Rgba = [f32; 4];

/// Parse a CSS hex color string (#RGB, #RGBA, #RRGGBB, #RRGGBBAA) into RGBA.
/// Returns None for invalid input.
pub fn parse_hex(hex: &str) -> Option<Rgba> {
    let hex = hex.strip_prefix('#')?;
    match hex.len() {
        3 => {
            let r = u8::from_str_radix(&hex[0..1], 16).ok()? * 17;
            let g = u8::from_str_radix(&hex[1..2], 16).ok()? * 17;
            let b = u8::from_str_radix(&hex[2..3], 16).ok()? * 17;
            Some([r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0])
        }
        4 => {
            let r = u8::from_str_radix(&hex[0..1], 16).ok()? * 17;
            let g = u8::from_str_radix(&hex[1..2], 16).ok()? * 17;
            let b = u8::from_str_radix(&hex[2..3], 16).ok()? * 17;
            let a = u8::from_str_radix(&hex[3..4], 16).ok()? * 17;
            Some([r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, a as f32 / 255.0])
        }
        6 => {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            Some([r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0])
        }
        8 => {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            let a = u8::from_str_radix(&hex[6..8], 16).ok()?;
            Some([r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, a as f32 / 255.0])
        }
        _ => None,
    }
}

/// Fully resolved theme with all fields non-optional.
/// Colors are pre-parsed as `[f32; 4]` RGBA.
#[derive(Debug, Clone)]
pub struct LunarisTheme {
    // Color tokens
    pub bg_shell: Rgba,
    pub bg_app: Rgba,
    pub bg_card: Rgba,
    pub bg_overlay: Rgba,
    pub bg_input: Rgba,
    pub fg_primary: Rgba,
    pub fg_secondary: Rgba,
    pub fg_disabled: Rgba,
    pub accent: Rgba,
    pub border: Rgba,
    pub error: Rgba,
    pub warning: Rgba,
    pub success: Rgba,

    // WM tokens (used by compositor)
    pub radius_s: [f32; 4],
    pub active_hint: u32,
    pub gaps: (u32, u32),
    pub is_dark: bool,
    pub window_hint: Option<Rgba>,

    // Motion tokens
    pub duration_short: u32,
    pub duration_medium: u32,
    pub duration_long: u32,

    // Depth tokens
    pub blur_enabled: bool,

    // Typography
    pub font_sans: String,
    pub font_mono: String,
    pub font_size: f32,

    // Cursor
    pub cursor_theme: String,
    pub cursor_size: u32,
}

impl LunarisTheme {
    /// The built-in Panda theme: dark shell, light apps.
    pub fn panda() -> Self {
        Self {
            bg_shell:     parse_hex("#09090b").unwrap(),
            bg_app:       parse_hex("#ffffff").unwrap(),
            bg_card:      parse_hex("#f5f5f7").unwrap(),
            bg_overlay:   parse_hex("#00000080").unwrap(),
            bg_input:     parse_hex("#f0f0f0").unwrap(),
            fg_primary:   parse_hex("#1a1a2e").unwrap(),
            fg_secondary: parse_hex("#6b7280").unwrap(),
            fg_disabled:  parse_hex("#9ca3af").unwrap(),
            accent:       parse_hex("#09090b").unwrap(),
            border:       parse_hex("#e2e2e8").unwrap(),
            error:        parse_hex("#ef4444").unwrap(),
            warning:      parse_hex("#f59e0b").unwrap(),
            success:      parse_hex("#22c55e").unwrap(),

            radius_s:     [8.0, 8.0, 8.0, 8.0],
            active_hint:  2,
            gaps:         (4, 4),
            is_dark:      false, // Panda app surface is light
            window_hint:  None,

            duration_short:  120,
            duration_medium: 200,
            duration_long:   350,

            blur_enabled: true,

            font_sans:    "Inter".into(),
            font_mono:    "JetBrains Mono".into(),
            font_size:    14.0,

            cursor_theme: "default".into(),
            cursor_size:  24,
        }
    }

    /// Load theme from a TOML file, merging with Panda defaults.
    /// Falls back to pure Panda on any error.
    pub fn load() -> Self {
        let path = Self::default_path();
        Self::load_from(&path)
    }

    /// Load theme from a specific path, merging with Panda defaults.
    pub fn load_from(path: &Path) -> Self {
        let file = match std::fs::read_to_string(path) {
            Ok(contents) => match toml::from_str::<LunarisThemeFile>(&contents) {
                Ok(f) => f,
                Err(_) => return Self::panda(),
            },
            Err(_) => return Self::panda(),
        };
        Self::from_file(file)
    }

    /// Merge a parsed theme file with Panda defaults.
    pub fn from_file(file: LunarisThemeFile) -> Self {
        let panda = Self::panda();
        let c = |hex: &Option<String>, fallback: Rgba| -> Rgba {
            hex.as_deref()
                .and_then(parse_hex)
                .unwrap_or(fallback)
        };

        let color = file.color.unwrap_or_default();
        let bg = color.bg.unwrap_or_default();
        let fg = color.fg.unwrap_or_default();
        let motion = file.motion.unwrap_or_default();
        let depth = file.depth.unwrap_or_default();
        let typo = file.typography.unwrap_or_default();
        let cursor = file.cursor.unwrap_or_default();
        let wm = file.wm.unwrap_or_default();

        Self {
            bg_shell:     c(&bg.shell, panda.bg_shell),
            bg_app:       c(&bg.app, panda.bg_app),
            bg_card:      c(&bg.card, panda.bg_card),
            bg_overlay:   c(&bg.overlay, panda.bg_overlay),
            bg_input:     c(&bg.input, panda.bg_input),
            fg_primary:   c(&fg.primary, panda.fg_primary),
            fg_secondary: c(&fg.secondary, panda.fg_secondary),
            fg_disabled:  c(&fg.disabled, panda.fg_disabled),
            accent:       c(&color.accent, panda.accent),
            border:       c(&color.border, panda.border),
            error:        c(&color.error, panda.error),
            warning:      c(&color.warning, panda.warning),
            success:      c(&color.success, panda.success),

            radius_s: wm.radius.map(|r| [r, r, r, r]).unwrap_or(panda.radius_s),
            active_hint: wm.active_hint.unwrap_or(panda.active_hint),
            gaps: wm.gaps.map(|g| (g, g)).unwrap_or(panda.gaps),
            is_dark: wm.is_dark.unwrap_or(panda.is_dark),
            window_hint: wm.window_hint.as_deref().and_then(parse_hex),

            duration_short:  motion.duration_short.unwrap_or(panda.duration_short),
            duration_medium: motion.duration_medium.unwrap_or(panda.duration_medium),
            duration_long:   motion.duration_long.unwrap_or(panda.duration_long),

            blur_enabled: depth.blur_enabled.unwrap_or(panda.blur_enabled),

            font_sans: typo.font_sans.unwrap_or(panda.font_sans),
            font_mono: typo.font_mono.unwrap_or(panda.font_mono),
            font_size: typo.font_size.unwrap_or(panda.font_size),

            cursor_theme: cursor.theme.unwrap_or(panda.cursor_theme),
            cursor_size:  cursor.size.unwrap_or(panda.cursor_size),
        }
    }

    /// Default path: `~/.config/lunaris/theme.toml`
    pub fn default_path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join("lunaris")
            .join("theme.toml")
    }

    /// Accent color as [r, g, b] (no alpha), for shader uniforms.
    pub fn accent_rgb(&self) -> [f32; 3] {
        [self.accent[0], self.accent[1], self.accent[2]]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panda_defaults_are_valid() {
        let t = LunarisTheme::panda();
        assert!(t.bg_shell[3] > 0.0, "bg_shell alpha should be > 0");
        assert!(t.accent[3] > 0.0, "accent alpha should be > 0");
        assert_eq!(t.radius_s, [8.0, 8.0, 8.0, 8.0]);
        assert_eq!(t.font_sans, "Inter");
        assert_eq!(t.cursor_size, 24);
    }

    #[test]
    fn load_falls_back_to_panda_when_file_missing() {
        let t = LunarisTheme::load_from(Path::new("/nonexistent/theme.toml"));
        assert_eq!(t.bg_shell, LunarisTheme::panda().bg_shell);
    }

    #[test]
    fn hex_parsing_6_digit() {
        let c = parse_hex("#ff8000").unwrap();
        assert!((c[0] - 1.0).abs() < 0.01);
        assert!((c[1] - 0.502).abs() < 0.01);
        assert!((c[2] - 0.0).abs() < 0.01);
        assert!((c[3] - 1.0).abs() < 0.01);
    }

    #[test]
    fn hex_parsing_8_digit_with_alpha() {
        let c = parse_hex("#00000080").unwrap();
        assert!((c[0]).abs() < 0.01);
        assert!((c[3] - 0.502).abs() < 0.01);
    }

    #[test]
    fn hex_parsing_3_digit() {
        let c = parse_hex("#f00").unwrap();
        assert!((c[0] - 1.0).abs() < 0.01);
        assert!((c[1]).abs() < 0.01);
        assert!((c[2]).abs() < 0.01);
    }

    #[test]
    fn hex_parsing_invalid_returns_none() {
        assert!(parse_hex("not-a-color").is_none());
        assert!(parse_hex("#gg0000").is_none());
        assert!(parse_hex("#").is_none());
    }

    #[test]
    fn partial_toml_falls_through_to_panda() {
        let toml_str = r##"
[color]
accent = "#ff0000"
"##;
        let file: LunarisThemeFile = toml::from_str(toml_str).unwrap();
        let t = LunarisTheme::from_file(file);
        // Accent should be red
        assert!((t.accent[0] - 1.0).abs() < 0.01);
        // Everything else should be panda
        assert_eq!(t.bg_shell, LunarisTheme::panda().bg_shell);
        assert_eq!(t.font_sans, "Inter");
        assert_eq!(t.radius_s, [8.0, 8.0, 8.0, 8.0]);
    }

    #[test]
    fn panda_toml_matches_panda_defaults() {
        // Load the reference Panda theme.toml and verify it produces the
        // same values as LunarisTheme::panda().
        let toml_path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("themes/panda/theme.toml");
        if !toml_path.exists() {
            // Skip in CI if themes repo is not checked out alongside sdk.
            return;
        }
        let t = LunarisTheme::load_from(&toml_path);
        let p = LunarisTheme::panda();
        assert_eq!(t.bg_shell, p.bg_shell, "bg_shell mismatch");
        assert_eq!(t.bg_app, p.bg_app, "bg_app mismatch");
        assert_eq!(t.accent, p.accent, "accent mismatch");
        assert_eq!(t.border, p.border, "border mismatch");
        assert_eq!(t.fg_primary, p.fg_primary, "fg_primary mismatch");
        assert_eq!(t.radius_s, p.radius_s, "radius_s mismatch");
        assert_eq!(t.active_hint, p.active_hint, "active_hint mismatch");
        assert_eq!(t.gaps, p.gaps, "gaps mismatch");
        assert_eq!(t.is_dark, p.is_dark, "is_dark mismatch");
        assert_eq!(t.duration_short, p.duration_short, "duration_short mismatch");
        assert_eq!(t.duration_medium, p.duration_medium, "duration_medium mismatch");
        assert_eq!(t.font_sans, p.font_sans, "font_sans mismatch");
        assert_eq!(t.cursor_size, p.cursor_size, "cursor_size mismatch");
        assert_eq!(t.blur_enabled, p.blur_enabled, "blur_enabled mismatch");
    }

    #[test]
    fn all_option_fields_fall_through() {
        let file = LunarisThemeFile::default();
        let t = LunarisTheme::from_file(file);
        let p = LunarisTheme::panda();
        assert_eq!(t.bg_shell, p.bg_shell);
        assert_eq!(t.accent, p.accent);
        assert_eq!(t.duration_medium, p.duration_medium);
        assert_eq!(t.cursor_size, p.cursor_size);
    }
}

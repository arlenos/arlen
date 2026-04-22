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
///
/// The token set matches `desktop-shell/src-tauri/themes/dark.toml`
/// and `light.toml` exactly — adding a field here and forgetting
/// to update `lunaris_dark` / `lunaris_light` will silently drift
/// the compositor-rendered Lunaris window header away from the
/// shell's CSS version. The dark/light preset tests at the bottom
/// of this file are the contract that catches that drift.
#[derive(Debug, Clone)]
pub struct LunarisTheme {
    // Background layer colors
    pub bg_shell: Rgba,
    pub bg_app: Rgba,
    pub bg_card: Rgba,
    pub bg_overlay: Rgba,
    pub bg_input: Rgba,

    // Foreground colors
    pub fg_primary: Rgba,
    pub fg_secondary: Rgba,
    pub fg_disabled: Rgba,
    /// Text colour on an accent surface (white on dark, dark on
    /// light). Mirrors `--color-fg-inverse` in the shell CSS.
    pub fg_inverse: Rgba,

    // Semantic colors
    pub accent: Rgba,
    /// Hover-state accent — brighter on dark, darker on light.
    /// Matches `--color-accent-hover` in app.css.
    pub accent_hover: Rgba,
    /// Pressed-state accent — mirrors `--color-accent-pressed`.
    pub accent_pressed: Rgba,
    pub error: Rgba,
    pub warning: Rgba,
    pub success: Rgba,
    pub info: Rgba,

    // Border colors
    /// Default hairline (`--color-border-default`). Used for the
    /// window-header bottom line and control-panel separators.
    pub border: Rgba,
    /// Stronger border variant (`--color-border-strong`).
    pub border_strong: Rgba,

    // Fine-grained radii in logical pixels. `radius_s` remains a
    // 4-corner array because the compositor's window-shape radius
    // is per-corner (top can round while bottom stays square, etc).
    // The scalar radii below mirror `--radius-sm/md/lg` from the
    // shell and are the source of truth for the Lunaris window
    // header's button-radius (8px) and top-corner rounding (8px).
    pub radius_sm: f32,
    pub radius_md: f32,
    pub radius_lg: f32,

    // WM tokens (compositor-only: window shape + layout)
    /// Per-corner radius used for the full WINDOW outline shape.
    /// Driven by `appearance.toml [window] corner_radius` so the
    /// user can square the window rect without changing the
    /// header/button rounding that `radius_md` controls.
    pub radius_s: [f32; 4],
    pub active_hint: u32,
    pub gaps: (u32, u32),
    /// `true` when this is a dark-mode palette. Used by render
    /// code that wants to pick black-on-X vs white-on-X.
    pub is_dark: bool,
    pub window_hint: Option<Rgba>,

    // Motion tokens
    pub duration_short: u32,
    pub duration_medium: u32,
    pub duration_long: u32,

    // Depth tokens
    pub blur_enabled: bool,

    // Typography
    /// Font family spec. Matches the shell's `--font-sans` — a
    /// CSS-style stack string. The compositor's cosmic-text layer
    /// picks the first installable family name from it.
    pub font_sans: String,
    pub font_mono: String,
    pub font_size: f32,
    /// Font weight for the "medium" text role (headers, buttons).
    /// Matches `--font-weight-medium` in dark/light.toml. 500.
    pub font_weight_medium: u16,

    // Cursor
    pub cursor_theme: String,
    pub cursor_size: u32,
}

impl LunarisTheme {
    /// **Canonical Lunaris Dark** — values match
    /// `desktop-shell/src-tauri/themes/dark.toml` byte-for-byte.
    /// This is the preset chosen when `appearance.toml [theme] mode
    /// = "dark"` and the one the compositor-rendered window header
    /// consumes to match the CSS version pixel-for-pixel.
    pub fn lunaris_dark() -> Self {
        Self {
            bg_shell:   parse_hex("#0a0a0a").unwrap(),
            bg_app:     parse_hex("#0f0f0f").unwrap(),
            bg_card:    parse_hex("#171717").unwrap(),
            bg_overlay: parse_hex("#00000080").unwrap(),
            bg_input:   parse_hex("#1a1a1a").unwrap(),

            fg_primary:   parse_hex("#fafafa").unwrap(),
            fg_secondary: parse_hex("#a1a1aa").unwrap(),
            fg_disabled:  parse_hex("#52525b").unwrap(),
            fg_inverse:   parse_hex("#0a0a0a").unwrap(),

            accent:         parse_hex("#6366f1").unwrap(),
            accent_hover:   parse_hex("#818cf8").unwrap(),
            accent_pressed: parse_hex("#4f46e5").unwrap(),
            error:          parse_hex("#ef4444").unwrap(),
            warning:        parse_hex("#eab308").unwrap(),
            success:        parse_hex("#22c55e").unwrap(),
            info:           parse_hex("#3b82f6").unwrap(),

            border:        parse_hex("#27272a").unwrap(),
            border_strong: parse_hex("#3f3f46").unwrap(),

            radius_sm: 4.0,
            radius_md: 8.0,
            radius_lg: 12.0,

            // Window outline radius: default from
            // `appearance.toml [window] corner_radius`, not here.
            // Start at 0 so an unset user config matches the shell's
            // default windowing look.
            radius_s:    [0.0, 0.0, 0.0, 0.0],
            active_hint: 1,
            gaps:        (4, 4),
            is_dark:     true,
            window_hint: None,

            duration_short:  100,
            duration_medium: 200,
            duration_long:   400,

            blur_enabled: true,

            font_sans: "\"Inter Variable\", ui-sans-serif, system-ui, sans-serif".into(),
            font_mono: "\"JetBrains Mono\", ui-monospace, monospace".into(),
            font_size: 14.0,
            font_weight_medium: 500,

            cursor_theme: "default".into(),
            cursor_size:  24,
        }
    }

    /// **Canonical Lunaris Light** — mirrors
    /// `desktop-shell/src-tauri/themes/light.toml`. Swap-target for
    /// `[theme] mode = "light"`.
    pub fn lunaris_light() -> Self {
        Self {
            bg_shell:   parse_hex("#f5f5f7").unwrap(),
            bg_app:     parse_hex("#ffffff").unwrap(),
            bg_card:    parse_hex("#f5f5f7").unwrap(),
            bg_overlay: parse_hex("#00000040").unwrap(),
            bg_input:   parse_hex("#f0f0f0").unwrap(),

            fg_primary:   parse_hex("#18181b").unwrap(),
            fg_secondary: parse_hex("#6b7280").unwrap(),
            fg_disabled:  parse_hex("#9ca3af").unwrap(),
            fg_inverse:   parse_hex("#fafafa").unwrap(),

            accent:         parse_hex("#4f46e5").unwrap(),
            accent_hover:   parse_hex("#6366f1").unwrap(),
            accent_pressed: parse_hex("#4338ca").unwrap(),
            error:          parse_hex("#dc2626").unwrap(),
            warning:        parse_hex("#d97706").unwrap(),
            success:        parse_hex("#16a34a").unwrap(),
            info:           parse_hex("#2563eb").unwrap(),

            border:        parse_hex("#e4e4e7").unwrap(),
            border_strong: parse_hex("#d4d4d8").unwrap(),

            radius_sm: 4.0,
            radius_md: 8.0,
            radius_lg: 12.0,

            radius_s:    [0.0, 0.0, 0.0, 0.0],
            active_hint: 1,
            gaps:        (4, 4),
            is_dark:     false,
            window_hint: None,

            duration_short:  100,
            duration_medium: 200,
            duration_long:   400,

            blur_enabled: true,

            font_sans: "\"Inter Variable\", ui-sans-serif, system-ui, sans-serif".into(),
            font_mono: "\"JetBrains Mono\", ui-monospace, monospace".into(),
            font_size: 14.0,
            font_weight_medium: 500,

            cursor_theme: "default".into(),
            cursor_size:  24,
        }
    }

    /// The built-in Panda theme: dark shell, light apps.
    ///
    /// Kept for API compatibility with older code paths — NEW
    /// callers that want the Lunaris design system should call
    /// `lunaris_dark()` / `lunaris_light()` directly.
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
            fg_inverse:   parse_hex("#fafafa").unwrap(),
            accent:       parse_hex("#09090b").unwrap(),
            accent_hover: parse_hex("#1f1f23").unwrap(),
            accent_pressed: parse_hex("#000000").unwrap(),
            border:       parse_hex("#e2e2e8").unwrap(),
            border_strong: parse_hex("#d4d4d8").unwrap(),
            error:        parse_hex("#ef4444").unwrap(),
            warning:      parse_hex("#f59e0b").unwrap(),
            success:      parse_hex("#22c55e").unwrap(),
            info:         parse_hex("#3b82f6").unwrap(),

            radius_sm: 4.0,
            radius_md: 8.0,
            radius_lg: 12.0,
            radius_s:     [8.0, 8.0, 8.0, 8.0],
            active_hint:  0,
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
            font_weight_medium: 500,

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

    /// Merge a parsed theme file with the `base` preset. Any field
    /// the file omits falls through to the base. Use
    /// `lunaris_dark()` as the base for dark-mode overrides and
    /// `lunaris_light()` for light-mode, so user overrides are
    /// additive on top of the canonical Lunaris palette.
    pub fn from_file_with_base(file: LunarisThemeFile, base: Self) -> Self {
        let c = |hex: &Option<String>, fallback: Rgba| -> Rgba {
            hex.as_deref().and_then(parse_hex).unwrap_or(fallback)
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
            bg_shell:     c(&bg.shell, base.bg_shell),
            bg_app:       c(&bg.app, base.bg_app),
            bg_card:      c(&bg.card, base.bg_card),
            bg_overlay:   c(&bg.overlay, base.bg_overlay),
            bg_input:     c(&bg.input, base.bg_input),
            fg_primary:   c(&fg.primary, base.fg_primary),
            fg_secondary: c(&fg.secondary, base.fg_secondary),
            fg_disabled:  c(&fg.disabled, base.fg_disabled),
            fg_inverse:   base.fg_inverse,
            accent:       c(&color.accent, base.accent),
            accent_hover: base.accent_hover,
            accent_pressed: base.accent_pressed,
            border:       c(&color.border, base.border),
            border_strong: base.border_strong,
            error:        c(&color.error, base.error),
            warning:      c(&color.warning, base.warning),
            success:      c(&color.success, base.success),
            info:         base.info,

            radius_sm: base.radius_sm,
            radius_md: base.radius_md,
            radius_lg: base.radius_lg,

            radius_s: wm.radius.map(|r| [r, r, r, r]).unwrap_or(base.radius_s),
            active_hint: wm.active_hint.unwrap_or(base.active_hint),
            gaps: wm.gaps.map(|g| (g, g)).unwrap_or(base.gaps),
            is_dark: wm.is_dark.unwrap_or(base.is_dark),
            window_hint: wm.window_hint.as_deref().and_then(parse_hex),

            duration_short:  motion.duration_short.unwrap_or(base.duration_short),
            duration_medium: motion.duration_medium.unwrap_or(base.duration_medium),
            duration_long:   motion.duration_long.unwrap_or(base.duration_long),

            blur_enabled: depth.blur_enabled.unwrap_or(base.blur_enabled),

            font_sans: typo.font_sans.unwrap_or(base.font_sans),
            font_mono: typo.font_mono.unwrap_or(base.font_mono),
            font_size: typo.font_size.unwrap_or(base.font_size),
            font_weight_medium: base.font_weight_medium,

            cursor_theme: cursor.theme.unwrap_or(base.cursor_theme),
            cursor_size:  cursor.size.unwrap_or(base.cursor_size),
        }
    }

    /// Backward-compat wrapper: merges onto the Panda base. New
    /// callers should use `from_file_with_base(file,
    /// LunarisTheme::lunaris_dark())` and friends so the Lunaris
    /// design system drives the fallbacks.
    pub fn from_file(file: LunarisThemeFile) -> Self {
        Self::from_file_with_base(file, Self::panda())
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

    // The presets below are the CONTRACT with the shell's
    // `desktop-shell/src-tauri/themes/dark.toml` and `light.toml`.
    // If these tests fail after a theme tweak you ALSO need to bump
    // the matching TOML file — or the compositor-rendered Lunaris
    // window header drifts away from the shell's CSS version.

    #[test]
    fn lunaris_dark_matches_shell_dark_toml_colors() {
        let d = LunarisTheme::lunaris_dark();
        assert_eq!(d.bg_shell,      parse_hex("#0a0a0a").unwrap());
        assert_eq!(d.bg_app,        parse_hex("#0f0f0f").unwrap());
        assert_eq!(d.bg_card,       parse_hex("#171717").unwrap());
        assert_eq!(d.bg_input,      parse_hex("#1a1a1a").unwrap());
        assert_eq!(d.fg_primary,    parse_hex("#fafafa").unwrap());
        assert_eq!(d.fg_secondary,  parse_hex("#a1a1aa").unwrap());
        assert_eq!(d.fg_disabled,   parse_hex("#52525b").unwrap());
        assert_eq!(d.fg_inverse,    parse_hex("#0a0a0a").unwrap());
        assert_eq!(d.accent,        parse_hex("#6366f1").unwrap());
        assert_eq!(d.accent_hover,  parse_hex("#818cf8").unwrap());
        assert_eq!(d.accent_pressed,parse_hex("#4f46e5").unwrap());
        assert_eq!(d.error,         parse_hex("#ef4444").unwrap());
        assert_eq!(d.warning,       parse_hex("#eab308").unwrap());
        assert_eq!(d.success,       parse_hex("#22c55e").unwrap());
        assert_eq!(d.info,          parse_hex("#3b82f6").unwrap());
        assert_eq!(d.border,        parse_hex("#27272a").unwrap());
        assert_eq!(d.border_strong, parse_hex("#3f3f46").unwrap());
        assert_eq!(d.radius_sm, 4.0);
        assert_eq!(d.radius_md, 8.0);
        assert_eq!(d.radius_lg, 12.0);
        assert!(d.is_dark);
        assert_eq!(d.font_weight_medium, 500);
        // font_sans MUST include Inter Variable so cosmic-text on
        // the compositor picks the same family the shell's CSS
        // chooses via `--font-sans`.
        assert!(d.font_sans.contains("Inter Variable"));
    }

    #[test]
    fn lunaris_light_matches_shell_light_toml_colors() {
        let l = LunarisTheme::lunaris_light();
        assert_eq!(l.bg_shell,      parse_hex("#f5f5f7").unwrap());
        assert_eq!(l.bg_app,        parse_hex("#ffffff").unwrap());
        assert_eq!(l.fg_primary,    parse_hex("#18181b").unwrap());
        assert_eq!(l.fg_inverse,    parse_hex("#fafafa").unwrap());
        assert_eq!(l.accent,        parse_hex("#4f46e5").unwrap());
        assert_eq!(l.border,        parse_hex("#e4e4e7").unwrap());
        assert_eq!(l.radius_md, 8.0);
        assert!(!l.is_dark);
    }

    #[test]
    fn from_file_with_base_preserves_preset_when_file_is_empty() {
        // The "empty theme.toml" scenario we hit in production —
        // the zero-byte file parses to all-None, and we must still
        // land on the Lunaris Dark palette (not Panda).
        let base = LunarisTheme::lunaris_dark();
        let empty = LunarisThemeFile::default();
        let composed = LunarisTheme::from_file_with_base(empty, base.clone());
        assert_eq!(composed.bg_shell, base.bg_shell);
        assert_eq!(composed.bg_app, base.bg_app);
        assert_eq!(composed.accent, base.accent);
        assert_eq!(composed.fg_primary, base.fg_primary);
        assert_eq!(composed.is_dark, base.is_dark);
    }

    #[test]
    fn from_file_with_base_layers_overrides_on_top() {
        let base = LunarisTheme::lunaris_dark();
        let toml_str = r##"
[color]
accent = "#ff0000"
"##;
        let file: LunarisThemeFile = toml::from_str(toml_str).unwrap();
        let t = LunarisTheme::from_file_with_base(file, base.clone());
        // Accent overridden.
        assert_eq!(t.accent, parse_hex("#ff0000").unwrap());
        // Everything else still matches the dark preset.
        assert_eq!(t.bg_shell, base.bg_shell);
        assert_eq!(t.fg_primary, base.fg_primary);
        assert_eq!(t.accent_hover, base.accent_hover); // not overridable via color.accent
    }

    #[test]
    fn presets_have_non_zero_alpha_on_opaque_colors() {
        for (name, t) in [
            ("lunaris_dark", LunarisTheme::lunaris_dark()),
            ("lunaris_light", LunarisTheme::lunaris_light()),
        ] {
            assert_eq!(t.bg_shell[3], 1.0, "{name} bg_shell must be opaque");
            assert_eq!(t.bg_app[3],   1.0, "{name} bg_app must be opaque");
            assert_eq!(t.fg_primary[3], 1.0, "{name} fg_primary must be opaque");
            assert_eq!(t.accent[3],   1.0, "{name} accent must be opaque");
            assert_eq!(t.error[3],    1.0, "{name} error must be opaque");
            assert_eq!(t.border[3],   1.0, "{name} border must be opaque");
        }
    }
}

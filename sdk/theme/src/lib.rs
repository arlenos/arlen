//! Arlen theme system — single source of truth.
//!
//! Two file paths participate in theme resolution:
//!
//! 1. **Bundled theme** (`<crate>/themes/dark.toml` or `light.toml`,
//!    embedded at compile-time via `include_str!` from whichever
//!    crate consumes the schema). Provides every default value.
//! 2. **Active user theme** (`~/.local/share/arlen/themes/{id}.toml`).
//!    Optional. When `appearance.toml [theme].active` names something
//!    that isn't a built-in id, the loader reads this file and merges
//!    it on top of the matching bundled variant (resolved via the
//!    `extends` field in the theme's `[meta]` section, default `dark`).
//! 3. **User customization** (`~/.config/arlen/theme.toml`).
//!    Optional. Loose top-of-stack overrides — any field set here
//!    wins over both the active theme and the bundled defaults.
//!    This is the channel a user uses to tweak the active theme
//!    without writing a full theme.
//!
//! Plus a per-user PREFERENCE layer:
//!
//! 4. **`appearance.toml [overrides].radius_intensity`** — multiplier
//!    in `0.0..=2.0` applied to all semantic radii at *emit time*
//!    (`ArlenTheme::effective_*()`), excluding `radius.full` and
//!    `radius.window_corners` which are categorical. The base
//!    radii live in the theme; the multiplier is the user-only knob.
//!
//! Both compositor and desktop-shell read the same resolved
//! `ArlenTheme`. The schema is grouped into per-concern substructs
//! (color, radius, spacing, typography, motion, depth, wm, cursor)
//! so callers borrow the slice they care about.
//!
//! See `docs/architecture/theme-system.md` for the full architecture
//! and per-token semantic guidance.

pub mod base16;
pub mod catppuccin;
pub mod color;
mod file;
pub mod gtk;
pub mod qt;
pub mod terminal;
mod watcher;

pub use file::{
    BorderColors, ColorBgFile, ColorFgFile, ColorSection, ColorSemanticFile,
    CursorSection, DepthSection, ArlenThemeFile, IconsSection, MetaSection,
    MotionSection, RadiusSection, SoundsSection, SpacingSection,
    TerminalAnsiSection, TerminalSection, TypographySection,
    WallpaperSection, WmSection,
};
pub use watcher::ThemeWatcher;

use std::path::{Path, PathBuf};

/// The canonical bundled default themes, embedded at compile time. This crate
/// is the single source: every consumer (the shell, the compositor) reads the
/// same bytes through these constants instead of reaching into another crate's
/// tree, so the two binaries cannot drift on their defaults.
pub const DARK_TOML: &str = include_str!("../themes/dark.toml");
/// The bundled light default theme. See [`DARK_TOML`].
pub const LIGHT_TOML: &str = include_str!("../themes/light.toml");

/// Parsed hex color as RGBA with components in `0.0..=1.0`.
pub type Rgba = [f32; 4];

/// Parse a CSS hex color string (`#RGB`, `#RGBA`, `#RRGGBB`,
/// `#RRGGBBAA`) into RGBA. Returns `None` for invalid input.
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

/// Scale a colour's RGB by `factor` (alpha kept), clamped — the same shape as
/// `QColor::lighter`/`darker` with `factor = percent / 100`. Shared by the
/// outbound generators (Fusion bevel shades) and the terminal-ANSI synthesis
/// (bright variants).
pub(crate) fn scale_rgb(c: Rgba, factor: f32) -> Rgba {
    [
        (c[0] * factor).clamp(0.0, 1.0),
        (c[1] * factor).clamp(0.0, 1.0),
        (c[2] * factor).clamp(0.0, 1.0),
        c[3],
    ]
}

/// Component-wise average of two colours — the deterministic hue synthesis for
/// the ANSI slots `ArlenTheme` has no semantic source for (magenta from
/// red+blue, cyan from green+blue).
pub(crate) fn blend(a: Rgba, b: Rgba) -> Rgba {
    [
        (a[0] + b[0]) / 2.0,
        (a[1] + b[1]) / 2.0,
        (a[2] + b[2]) / 2.0,
        (a[3] + b[3]) / 2.0,
    ]
}

/// Whether a free-string theme token is inert: safe to emit verbatim into any
/// generated config without breaking out of its declaration context.
///
/// A theme is inert validated data (theming-system-plan.md): a value parses into
/// a typed colour, a number, or a *vetted* string, and nothing it carries can
/// become syntax in the file it is written into. Colours already go through
/// [`parse_hex`]; this is the analogous floor for the remaining free-string
/// fields (font families, lengths, durations, easings, shadows, the cursor theme
/// name). It is deliberately format-AGNOSTIC: the one resolved value fans out to
/// every generator (CSS custom properties, the GTK/libadwaita CSS, qt*ct INI,
/// terminal `.conf`/Xresources), so it must carry nothing that is syntax in ANY
/// of them. Rejected: control characters (newlines, the multi-line-injection
/// vector across all formats), the CSS statement/block punctuation `;{}`, the INI
/// section/comment punctuation `[]` (and `;`, already covered), the markup
/// guards `<>`, the at-rule `@`, the escape `\`, and the CSS comment sequences
/// `/*` `*/`. Every well-formed token value uses only letters, digits, spaces and
/// `,.()#%-_"'/:` — none of the rejected set — so a legitimate theme always
/// passes and an adversarial value is dropped to its safe default at resolve.
pub fn is_inert_css_token(value: &str) -> bool {
    if value.chars().any(|c| c.is_control()) {
        return false;
    }
    const FORBIDDEN: &[char] = &[';', '{', '}', '[', ']', '<', '>', '@', '\\'];
    if value.contains(FORBIDDEN) {
        return false;
    }
    if value.contains("/*") || value.contains("*/") {
        return false;
    }
    true
}

/// Resolve a free-string token: keep the author's value only if it is
/// [inert](is_inert_css_token), otherwise fall back to the built-in default. The
/// trust floor for every free-string field — an adversarial value can at worst
/// produce the default token, never break out of the generated config.
fn inert_or(value: Option<String>, default: &str) -> String {
    value
        .filter(|v| is_inert_css_token(v))
        .unwrap_or_else(|| default.to_string())
}

// ---------------------------------------------------------------------------
// Resolved theme — the post-merge struct both crates consume
// ---------------------------------------------------------------------------

/// Theme metadata.
#[derive(Debug, Clone)]
pub struct ThemeMeta {
    pub id: String,
    pub name: String,
    pub variant: ThemeVariant,
    /// Optional parent-theme id (for inheritance).
    pub extends: Option<String>,
}

/// Light or dark.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeVariant {
    Dark,
    Light,
}

/// Color tokens. Three layers: surface (bg), text (fg), semantic +
/// border. `accent_hover` and `accent_pressed` are pre-baked
/// derivatives (theme-author chooses both states; not auto-computed).
#[derive(Debug, Clone)]
pub struct ColorTokens {
    pub bg_shell:   Rgba,
    pub bg_app:     Rgba,
    pub bg_card:    Rgba,
    pub bg_overlay: Rgba,
    pub bg_input:   Rgba,

    pub fg_primary:   Rgba,
    pub fg_secondary: Rgba,
    pub fg_disabled:  Rgba,
    pub fg_inverse:   Rgba,

    pub accent:         Rgba,
    pub accent_hover:   Rgba,
    pub accent_pressed: Rgba,
    pub success: Rgba,
    pub warning: Rgba,
    pub error:   Rgba,
    pub info:    Rgba,

    pub border_default: Rgba,
    pub border_strong:  Rgba,
}

/// Border-radius tokens. **Semantic naming** (chip/button/input/
/// card/modal/full) instead of t-shirt sizes — see
/// `theme-system.md` §2 for the philosophical anchor and
/// `docs/architecture/theme-system.md` §3 for golden-formula
/// nesting guidance.
///
/// Theme authors set every field absolutely. The user's
/// `appearance.toml [overrides].radius_intensity` multiplier is
/// applied via `effective_*()` accessors, **not** stored back
/// here, so the original theme intent stays inspectable.
///
/// `full` and `window_corners` are NEVER scaled by intensity —
/// they're categorical (pill = pill, window-outline = wm-spec)
/// rather than on the design-spectrum.
#[derive(Debug, Clone)]
pub struct RadiusTokens {
    /// Inline tags, dots, badges, tiny chips. Default 4.
    pub chip: f32,
    /// Pressable button face. Default 6.
    pub button: f32,
    /// Text inputs, dropdowns, segmented controls. Default 8.
    pub input: f32,
    /// Cards, popovers, panels, dropdown menus. Default 12.
    pub card: f32,
    /// Modals, dialogs, sheets, hero containers. Default 16.
    pub modal: f32,
    /// Pills, avatars, status indicators. Default 9999. Categorical.
    pub full: f32,
    /// Per-corner radius for the COMPOSITOR-rendered window outline
    /// shape (top-left, top-right, bottom-right, bottom-left).
    /// Independent of the semantic scale because window shapes can
    /// be asymmetric (top-rounded, bottom-square for drag-attached
    /// frames). Categorical, not intensity-scaled.
    pub window_corners: [f32; 4],
    /// User-applied multiplier. `0.0..=2.0`, clamped on `effective_*`.
    /// Excluded from `full` and `window_corners`. Theme authors can
    /// default this (e.g. brutalist theme defaults to 0.5).
    pub intensity: f32,
}

/// Spacing scale. Strings because callers may want unit suffixes
/// (px / rem / em) — frontend uses these directly as CSS variables.
#[derive(Debug, Clone)]
pub struct SpacingTokens {
    pub xs: String,
    pub sm: String,
    pub md: String,
    pub lg: String,
    pub xl: String,
}

/// Typography tokens.
#[derive(Debug, Clone)]
pub struct TypographyTokens {
    pub font_sans:     String,
    pub font_mono:     String,
    pub size_base:     String,
    pub line_height:   String,
    pub weight_normal: u32,
    pub weight_medium: u32,
    pub weight_bold:   u32,
}

/// Motion / transition tokens.
#[derive(Debug, Clone)]
pub struct MotionTokens {
    pub duration_fast:   String,
    pub duration_normal: String,
    pub duration_slow:   String,
    pub easing_default:  String,
    pub easing_spring:   String,
}

/// Elevation / depth tokens.
#[derive(Debug, Clone)]
pub struct DepthTokens {
    pub shadow_sm:    String,
    pub shadow_md:    String,
    pub shadow_lg:    String,
    pub blur_enabled: bool,
}

/// Window-manager-only tokens. Compositor consumes these; shell
/// ignores them. `gaps_inner`/`gaps_outer` mirror compositor.toml's
/// `[layout]` section but are theme-author-controllable.
#[derive(Debug, Clone)]
pub struct WmTokens {
    pub active_hint: u32,
    pub gaps_inner:  u32,
    pub gaps_outer:  u32,
    /// Optional accent override for the window outline. `None` =
    /// fall through to `color.semantic.accent`.
    pub window_hint: Option<Rgba>,
}

/// Cursor configuration.
#[derive(Debug, Clone)]
pub struct CursorTokens {
    pub theme: String,
    pub size:  u32,
}

/// Resolved terminal palette: the semantic→16-ANSI projection (or the authored
/// `[terminal.ansi]` override, slot-by-slot) plus fg/bg/cursor. The terminal
/// generators serialize this; the two-path honour-vs-synthesise decision is
/// made once, at resolve.
#[derive(Debug, Clone)]
pub struct TerminalTokens {
    /// Default foreground (`fg.primary`).
    pub fg: Rgba,
    /// Default background (`bg.app`).
    pub bg: Rgba,
    /// Cursor colour (`accent`).
    pub cursor: Rgba,
    /// ANSI colours 0-15: black, red, green, yellow, blue, magenta, cyan,
    /// white, then the bright variants in the same order.
    pub ansi: [Rgba; 16],
}

/// Resolved icon-theme selection (mirrors [`CursorTokens`]'s `theme`).
#[derive(Debug, Clone)]
pub struct IconTokens {
    pub theme: String,
}

/// Fully resolved theme. Both compositor and desktop-shell consume
/// this. Construct via `ArlenTheme::resolve(...)`; do not build
/// by hand outside tests.
#[derive(Debug, Clone)]
pub struct ArlenTheme {
    pub meta:       ThemeMeta,
    pub color:      ColorTokens,
    pub radius:     RadiusTokens,
    pub spacing:    SpacingTokens,
    pub typography: TypographyTokens,
    pub motion:     MotionTokens,
    pub depth:      DepthTokens,
    pub wm:         WmTokens,
    pub cursor:     CursorTokens,
    pub terminal:   TerminalTokens,
    pub icons:      IconTokens,
}

// ---------------------------------------------------------------------------
// Resolution: parse + merge + intensity
// ---------------------------------------------------------------------------

/// Errors from theme resolution.
#[derive(Debug, thiserror::Error)]
pub enum ResolveError {
    #[error("toml parse: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("missing required field: {0}")]
    MissingField(&'static str),
    #[error("invalid color {field}: {value}")]
    InvalidColor { field: &'static str, value: String },
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

impl ArlenTheme {
    /// Resolve the active theme.
    ///
    /// `bundled` is the compile-time-embedded base theme (one of
    /// `dark.toml` / `light.toml`). `user_theme` is the optional
    /// user-installed theme file at
    /// `~/.local/share/arlen/themes/{active}.toml` — passed in
    /// already-loaded so the file-IO is the caller's responsibility.
    /// `customization` is the optional `~/.config/arlen/theme.toml`
    /// content, also caller-loaded.
    ///
    /// Merge order: `bundled` < `user_theme` (extends-resolved)
    /// < `customization`. Later layers win field-by-field.
    pub fn resolve(
        bundled: &str,
        user_theme: Option<&str>,
        customization: Option<&str>,
    ) -> Result<Self, ResolveError> {
        let bundled_file: ArlenThemeFile = toml::from_str(bundled)?;
        let user_file: Option<ArlenThemeFile> = user_theme
            .map(toml::from_str)
            .transpose()?;
        let custom_file: Option<ArlenThemeFile> = customization
            .map(toml::from_str)
            .transpose()?;

        // Merge user_file on top of bundled (field-by-field, all
        // optional in user_file so missing fields fall through).
        let merged = match user_file {
            None => bundled_file,
            Some(u) => merge_files(bundled_file, u),
        };
        let merged = match custom_file {
            None => merged,
            Some(c) => merge_files(merged, c),
        };

        // Project to the resolved struct. Required fields error if
        // missing; optional fields use sensible defaults.
        from_file(merged)
    }

    /// Convenience: parse a single bundled-theme file with no
    /// user overrides. Used in compositor startup before file-IO
    /// is set up.
    pub fn from_bundled(bundled: &str) -> Result<Self, ResolveError> {
        Self::resolve(bundled, None, None)
    }

    // ── Effective-radius accessors (intensity applied) ────────

    /// `radius.chip * intensity`, clamped to `[0, 999]` and rounded
    /// to integer pixels.
    pub fn effective_chip(&self) -> f32 {
        scale_radius(self.radius.chip, self.radius.intensity)
    }
    pub fn effective_button(&self) -> f32 {
        scale_radius(self.radius.button, self.radius.intensity)
    }
    pub fn effective_input(&self) -> f32 {
        scale_radius(self.radius.input, self.radius.intensity)
    }
    pub fn effective_card(&self) -> f32 {
        scale_radius(self.radius.card, self.radius.intensity)
    }
    pub fn effective_modal(&self) -> f32 {
        scale_radius(self.radius.modal, self.radius.intensity)
    }
    /// Pill / full radius — **never scaled by intensity** because
    /// "fully round" doesn't have a meaningful intermediate state.
    pub fn effective_full(&self) -> f32 {
        self.radius.full
    }
    /// Per-corner window outline. Scaled by `radius.intensity` so
    /// the user's single appearance-page slider drives BOTH the
    /// shell's tile/card/modal radii AND the compositor's window-
    /// corner radii in lock-step. Without this, dragging the
    /// roundness slider visually only affected shell content while
    /// the compositor kept rendering windows at the theme's
    /// hardcoded corner radius — confusing UX where "make
    /// everything sharper" left half the screen rounded.
    pub fn effective_window_corners(&self) -> [f32; 4] {
        self.radius
            .window_corners
            .map(|r| scale_radius(r, self.radius.intensity))
    }

    /// Accent color as `[r, g, b]` (no alpha), for shader uniforms.
    pub fn accent_rgb(&self) -> [f32; 3] {
        [
            self.color.accent[0],
            self.color.accent[1],
            self.color.accent[2],
        ]
    }

    /// `true` when the resolved theme is dark-variant.
    pub fn is_dark(&self) -> bool {
        self.meta.variant == ThemeVariant::Dark
    }

    /// Default config path for `~/.config/arlen/theme.toml`.
    pub fn user_customization_path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join("arlen")
            .join("theme.toml")
    }

    /// Default user-theme directory (`~/.local/share/arlen/themes/`).
    pub fn user_themes_dir() -> PathBuf {
        dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join("arlen")
            .join("themes")
    }

    /// Build the user-theme file path for a given theme id.
    pub fn user_theme_path(active_id: &str) -> PathBuf {
        Self::user_themes_dir().join(format!("{active_id}.toml"))
    }
}

/// Color-with-default helper used by `from_file`. Parses the
/// supplied hex string if present, else parses the bundled fallback.
/// Bundled fallbacks are static strings — if THEY fail to parse
/// that's a panic-worthy bug in this file, not a runtime error.
fn color_or(
    raw: Option<String>,
    field: &'static str,
    fallback_hex: &'static str,
) -> Result<Rgba, ResolveError> {
    let s = raw.unwrap_or_else(|| fallback_hex.to_string());
    parse_hex(&s).ok_or(ResolveError::InvalidColor { field, value: s })
}

/// Apply intensity to a base radius. Intensity is clamped to
/// `[0.0, 2.0]`. Result is non-negative integer pixels.
fn scale_radius(base: f32, intensity: f32) -> f32 {
    let i = intensity.clamp(0.0, 2.0);
    let raw = base * i;
    raw.max(0.0).round()
}

// ---------------------------------------------------------------------------
// File → resolved struct projection
// ---------------------------------------------------------------------------

fn from_file(f: ArlenThemeFile) -> Result<ArlenTheme, ResolveError> {
    // After merge, every required section/field falls back to a
    // sane default if absent. This lets a partial customization
    // file (e.g. user editing only the accent) resolve without
    // forcing them to repeat every other token.
    let meta = f.meta.ok_or(ResolveError::MissingField("meta"))?;
    let color_section = f.color.unwrap_or_default();
    let bg   = color_section.bg.unwrap_or_default();
    let fg   = color_section.fg.unwrap_or_default();
    let sem  = color_section.semantic.unwrap_or_default();
    let bord = color_section.border.unwrap_or_default();

    // Color resolution: each Option falls back to a hardcoded
    // default. Bundled themes are expected to set everything;
    // partial overlays can leave fields out.
    let color = ColorTokens {
        bg_shell:   color_or(bg.shell,   "bg.shell",   "#0a0a0a")?,
        bg_app:     color_or(bg.app,     "bg.app",     "#0f0f0f")?,
        bg_card:    color_or(bg.card,    "bg.card",    "#171717")?,
        bg_overlay: color_or(bg.overlay, "bg.overlay", "#00000080")?,
        bg_input:   color_or(bg.input,   "bg.input",   "#1a1a1a")?,

        fg_primary:   color_or(fg.primary,   "fg.primary",   "#fafafa")?,
        fg_secondary: color_or(fg.secondary, "fg.secondary", "#a1a1aa")?,
        fg_disabled:  color_or(fg.disabled,  "fg.disabled",  "#52525b")?,
        fg_inverse:   color_or(fg.inverse,   "fg.inverse",   "#0a0a0a")?,

        // Monochrome fallback (matches the foreground), used only if a theme
        // omits the field. Not indigo.
        accent:         color_or(sem.accent,         "semantic.accent",         "#fafafa")?,
        accent_hover:   color_or(sem.accent_hover,   "semantic.accent_hover",   "#a1a1aa")?,
        accent_pressed: color_or(sem.accent_pressed, "semantic.accent_pressed", "#d4d4d8")?,
        success:        color_or(sem.success,        "semantic.success",        "#22c55e")?,
        warning:        color_or(sem.warning,        "semantic.warning",        "#eab308")?,
        error:          color_or(sem.error,          "semantic.error",          "#ef4444")?,
        info:           color_or(sem.info,           "semantic.info",           "#3b82f6")?,

        border_default: color_or(bord.default, "border.default", "#27272a")?,
        border_strong:  color_or(bord.strong,  "border.strong",  "#3f3f46")?,
    };

    let r = f.radius.unwrap_or_default();
    let radius = RadiusTokens {
        chip:           r.chip.unwrap_or(4.0),
        button:         r.button.unwrap_or(6.0),
        input:          r.input.unwrap_or(8.0),
        card:           r.card.unwrap_or(12.0),
        modal:          r.modal.unwrap_or(16.0),
        full:           r.full.unwrap_or(9999.0),
        window_corners: r.window_corners.unwrap_or([12.0, 12.0, 12.0, 12.0]),
        intensity:      r.intensity.unwrap_or(1.0),
    };

    let s = f.spacing.unwrap_or_default();
    let spacing = SpacingTokens {
        xs: inert_or(s.xs, "4px"),
        sm: inert_or(s.sm, "8px"),
        md: inert_or(s.md, "16px"),
        lg: inert_or(s.lg, "24px"),
        xl: inert_or(s.xl, "32px"),
    };

    let t = f.typography.unwrap_or_default();
    let typography = TypographyTokens {
        font_sans:     inert_or(
            t.font_sans,
            "\"Inter Variable\", ui-sans-serif, system-ui, sans-serif",
        ),
        font_mono:     inert_or(t.font_mono, "\"JetBrains Mono\", ui-monospace, monospace"),
        size_base:     inert_or(t.size_base, "14px"),
        line_height:   inert_or(t.line_height, "1.5"),
        weight_normal: t.weight_normal.unwrap_or(400),
        weight_medium: t.weight_medium.unwrap_or(500),
        weight_bold:   t.weight_bold.unwrap_or(600),
    };

    let m = f.motion.unwrap_or_default();
    let motion = MotionTokens {
        duration_fast:   inert_or(m.duration_fast, "100ms"),
        duration_normal: inert_or(m.duration_normal, "200ms"),
        duration_slow:   inert_or(m.duration_slow, "400ms"),
        easing_default:  inert_or(m.easing_default, "cubic-bezier(0.4, 0, 0.2, 1)"),
        easing_spring:   inert_or(m.easing_spring, "cubic-bezier(0.34, 1.56, 0.64, 1)"),
    };

    let d = f.depth.unwrap_or_default();
    let depth = DepthTokens {
        shadow_sm:    inert_or(d.shadow_sm, "0 1px 2px rgba(0, 0, 0, 0.3)"),
        shadow_md:    inert_or(d.shadow_md, "0 4px 12px rgba(0, 0, 0, 0.4)"),
        shadow_lg:    inert_or(d.shadow_lg, "0 8px 24px rgba(0, 0, 0, 0.5)"),
        blur_enabled: d.blur_enabled.unwrap_or(true),
    };

    let w = f.wm.unwrap_or_default();
    let wm = WmTokens {
        active_hint: w.active_hint.unwrap_or(1),
        gaps_inner:  w.gaps_inner.unwrap_or(4),
        gaps_outer:  w.gaps_outer.unwrap_or(4),
        window_hint: w
            .window_hint
            .as_deref()
            .filter(|s| !s.is_empty())
            .and_then(parse_hex),
    };

    let c = f.cursor.unwrap_or_default();
    let cursor = CursorTokens {
        theme: inert_or(c.theme, "default"),
        size:  c.size.unwrap_or(24),
    };

    let terminal = resolve_terminal(&color, f.terminal);
    let i = f.icons.unwrap_or_default();
    let icons = IconTokens {
        theme: inert_or(i.theme, "default"),
    };

    let variant = match meta.variant.as_str() {
        "dark" => ThemeVariant::Dark,
        "light" => ThemeVariant::Light,
        other => {
            return Err(ResolveError::InvalidColor {
                field: "meta.variant",
                value: other.to_string(),
            });
        }
    };

    Ok(ArlenTheme {
        meta: ThemeMeta {
            // `meta.id` is the only theme string that becomes a *path* — the
            // desktop-shell picker stores it and `load(id)` joins
            // `themes/{id}.toml`, so a hostile `id = "../../../etc/x"` from a
            // dropped theme file is a traversal vector. Force it to a safe slug
            // (`[a-z0-9-]`, no separators, no `..`); `extends` is path-joined the
            // same way by a parent resolver, so slug it too. `name` is display
            // text that must not carry generated-config syntax, so it goes
            // through the inert floor like every other free string. A bundled
            // theme's `id`/`name` are already clean, so this is a no-op for them
            // and a containment for an imported file.
            id: base16::slugify(&meta.id),
            name: inert_or(Some(meta.name), "Imported Theme"),
            variant,
            extends: meta.extends.map(|e| base16::slugify(&e)),
        },
        color,
        radius,
        spacing,
        typography,
        motion,
        depth,
        wm,
        cursor,
        terminal,
        icons,
    })
}

/// Build the resolved terminal palette: synthesise the 16-ANSI projection from
/// the semantic tokens, then honour any authored `[terminal.ansi]` slot over the
/// synthesis (the A2 two-path generator, decided once here so the per-format
/// emitters just serialize). `ArlenTheme` honestly yields five of the eight
/// hues (error→red, success→green, warning→yellow, info→blue, accent→cursor);
/// magenta and cyan are blended from their RGB neighbours, the brights scaled up
/// from the normals, and the neutrals mapped to the fg/border ramp
/// (black←border.default, white←fg.secondary, bright-black←fg.disabled — the
/// classic muted slot — bright-white←fg.primary). An authored slot that does not
/// parse as hex keeps the synthesised colour (typed fail-to-default, like every
/// other colour field).
fn resolve_terminal(c: &ColorTokens, section: Option<TerminalSection>) -> TerminalTokens {
    let bright = |x: Rgba| scale_rgb(x, 1.25);
    let mut ansi = [
        c.border_default,          // 0  black
        c.error,                   // 1  red
        c.success,                 // 2  green
        c.warning,                 // 3  yellow
        c.info,                    // 4  blue
        blend(c.error, c.info),    // 5  magenta
        blend(c.success, c.info),  // 6  cyan
        c.fg_secondary,            // 7  white
        c.fg_disabled,             // 8  bright black
        bright(c.error),           // 9  bright red
        bright(c.success),         // 10 bright green
        bright(c.warning),         // 11 bright yellow
        bright(c.info),            // 12 bright blue
        bright(blend(c.error, c.info)),   // 13 bright magenta
        bright(blend(c.success, c.info)), // 14 bright cyan
        c.fg_primary,              // 15 bright white
    ];
    if let Some(a) = section.and_then(|t| t.ansi) {
        let slots = [
            a.black, a.red, a.green, a.yellow, a.blue, a.magenta, a.cyan, a.white,
            a.bright_black, a.bright_red, a.bright_green, a.bright_yellow,
            a.bright_blue, a.bright_magenta, a.bright_cyan, a.bright_white,
        ];
        for (i, authored) in slots.into_iter().enumerate() {
            if let Some(rgba) = authored.as_deref().and_then(parse_hex) {
                ansi[i] = rgba;
            }
        }
    }
    TerminalTokens {
        fg: c.fg_primary,
        bg: c.bg_app,
        cursor: c.accent,
        ansi,
    }
}

/// Field-by-field merge — every Optional in `over` that is `Some`
/// replaces the corresponding field in `under`. Used to layer
/// user themes / customization over the bundled defaults.
fn merge_files(under: ArlenThemeFile, over: ArlenThemeFile) -> ArlenThemeFile {
    ArlenThemeFile {
        meta: over.meta.or(under.meta),
        color: merge_color(under.color, over.color),
        radius: merge_radius(under.radius, over.radius),
        spacing: merge_spacing(under.spacing, over.spacing),
        typography: merge_typography(under.typography, over.typography),
        motion: merge_motion(under.motion, over.motion),
        depth: merge_depth(under.depth, over.depth),
        wm: merge_wm(under.wm, over.wm),
        cursor: merge_cursor(under.cursor, over.cursor),
        terminal: merge_terminal(under.terminal, over.terminal),
        icons: merge_icons(under.icons, over.icons),
        wallpaper: over.wallpaper.or(under.wallpaper),
        sounds: over.sounds.or(under.sounds),
    }
}

/// Terminal nests one level (`[terminal.ansi]`), so it composes the macro-made
/// ANSI merge the way `merge_color` composes its substructs.
fn merge_terminal(
    under: Option<TerminalSection>,
    over: Option<TerminalSection>,
) -> Option<TerminalSection> {
    match (under, over) {
        (None, x) | (x, None) => x,
        (Some(u), Some(o)) => Some(TerminalSection {
            ansi: merge_terminal_ansi(u.ansi, o.ansi),
        }),
    }
}

fn merge_color(under: Option<ColorSection>, over: Option<ColorSection>) -> Option<ColorSection> {
    match (under, over) {
        (None, x) | (x, None) => x,
        (Some(u), Some(o)) => Some(ColorSection {
            bg: merge_color_bg(u.bg, o.bg),
            fg: merge_color_fg(u.fg, o.fg),
            semantic: merge_color_semantic(u.semantic, o.semantic),
            border: merge_color_border(u.border, o.border),
        }),
    }
}

macro_rules! merge_struct_field {
    ($name:ident, $ty:ident, [$($field:ident),+ $(,)?]) => {
        fn $name(under: Option<$ty>, over: Option<$ty>) -> Option<$ty> {
            match (under, over) {
                (None, x) | (x, None) => x,
                (Some(u), Some(o)) => Some($ty {
                    $($field: o.$field.or(u.$field),)+
                }),
            }
        }
    };
}

merge_struct_field!(
    merge_color_bg,
    ColorBgFile,
    [shell, app, card, overlay, input]
);
merge_struct_field!(
    merge_color_fg,
    ColorFgFile,
    [primary, secondary, disabled, inverse]
);
merge_struct_field!(
    merge_color_semantic,
    ColorSemanticFile,
    [accent, accent_hover, accent_pressed, success, warning, error, info]
);
merge_struct_field!(merge_color_border, BorderColors, [default, strong]);
merge_struct_field!(
    merge_radius,
    RadiusSection,
    [chip, button, input, card, modal, full, window_corners, intensity]
);
merge_struct_field!(merge_spacing, SpacingSection, [xs, sm, md, lg, xl]);
merge_struct_field!(
    merge_typography,
    TypographySection,
    [
        font_sans,
        font_mono,
        size_base,
        line_height,
        weight_normal,
        weight_medium,
        weight_bold
    ]
);
merge_struct_field!(
    merge_motion,
    MotionSection,
    [
        duration_fast,
        duration_normal,
        duration_slow,
        easing_default,
        easing_spring
    ]
);
merge_struct_field!(
    merge_depth,
    DepthSection,
    [shadow_sm, shadow_md, shadow_lg, blur_enabled]
);
merge_struct_field!(
    merge_wm,
    WmSection,
    [active_hint, gaps_inner, gaps_outer, window_hint]
);
merge_struct_field!(merge_cursor, CursorSection, [theme, size]);
merge_struct_field!(
    merge_terminal_ansi,
    TerminalAnsiSection,
    [
        black,
        red,
        green,
        yellow,
        blue,
        magenta,
        cyan,
        white,
        bright_black,
        bright_red,
        bright_green,
        bright_yellow,
        bright_blue,
        bright_magenta,
        bright_cyan,
        bright_white,
    ]
);
merge_struct_field!(merge_icons, IconsSection, [theme]);

// Helper for ColorSection's substructs (they need their own
// because the grouped section nests one level deeper than the
// macro pattern handles).

// ---------------------------------------------------------------------------
// Convenience: load from disk (for callers without compile-time bytes)
// ---------------------------------------------------------------------------

/// Load and resolve a theme entirely from disk. The caller selects
/// the bundled-theme bytes; user-theme + customization are read
/// from the standard paths if they exist.
pub fn load_theme_from_disk(
    bundled: &str,
    active_id: &str,
) -> Result<ArlenTheme, ResolveError> {
    let user_path = ArlenTheme::user_theme_path(active_id);
    let user_theme = if user_path.exists() {
        Some(std::fs::read_to_string(&user_path)?)
    } else {
        None
    };
    let custom_path = ArlenTheme::user_customization_path();
    let custom = if custom_path.exists() {
        Some(std::fs::read_to_string(&custom_path)?)
    } else {
        None
    };
    ArlenTheme::resolve(bundled, user_theme.as_deref(), custom.as_deref())
}

// Path is currently unused at the public API level but might be in tests.
#[allow(dead_code)]
fn _path_marker(_p: &Path) {}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_BUNDLED: &str = r##"
[meta]
id = "dark"
name = "Arlen Dark"
variant = "dark"

[color.bg]
shell   = "#0a0a0a"
app     = "#0f0f0f"
card    = "#171717"
overlay = "#00000080"
input   = "#1a1a1a"

[color.fg]
primary   = "#fafafa"
secondary = "#a1a1aa"
disabled  = "#52525b"
inverse   = "#0a0a0a"

[color.semantic]
accent         = "#6366f1"
accent_hover   = "#818cf8"
accent_pressed = "#4f46e5"
success        = "#22c55e"
warning        = "#eab308"
error          = "#ef4444"
info           = "#3b82f6"

[color.border]
default = "#27272a"
strong  = "#3f3f46"

[radius]
chip   = 4
button = 6
input  = 8
card   = 12
modal  = 16
full   = 9999
window_corners = [12, 12, 12, 12]
intensity = 1.0

[spacing]
xs = "4px"
sm = "8px"
md = "16px"
lg = "24px"
xl = "32px"

[typography]
font_sans     = "\"Inter Variable\", ui-sans-serif"
font_mono     = "\"JetBrains Mono\", ui-monospace"
size_base     = "14px"
line_height   = "1.5"
weight_normal = 400
weight_medium = 500
weight_bold   = 600

[motion]
duration_fast   = "100ms"
duration_normal = "200ms"
duration_slow   = "400ms"
easing_default  = "cubic-bezier(0.4, 0, 0.2, 1)"
easing_spring   = "cubic-bezier(0.34, 1.56, 0.64, 1)"

[depth]
shadow_sm    = "0 1px 2px rgba(0, 0, 0, 0.3)"
shadow_md    = "0 4px 12px rgba(0, 0, 0, 0.4)"
shadow_lg    = "0 8px 24px rgba(0, 0, 0, 0.5)"
blur_enabled = true

[wm]
active_hint = 1
gaps_inner  = 4
gaps_outer  = 4

[cursor]
theme = "default"
size  = 24
"##;

    #[test]
    fn parses_bundled_dark() {
        let t = ArlenTheme::from_bundled(SAMPLE_BUNDLED).expect("resolve");
        assert_eq!(t.meta.id, "dark");
        assert_eq!(t.meta.variant, ThemeVariant::Dark);
        assert!(t.is_dark());
        assert_eq!(t.radius.chip, 4.0);
        assert_eq!(t.radius.button, 6.0);
        assert_eq!(t.radius.input, 8.0);
        assert_eq!(t.radius.card, 12.0);
        assert_eq!(t.radius.modal, 16.0);
        assert_eq!(t.radius.full, 9999.0);
        assert_eq!(t.radius.intensity, 1.0);
        assert_eq!(t.radius.window_corners, [12.0, 12.0, 12.0, 12.0]);
    }

    #[test]
    fn intensity_scales_semantic_radii() {
        let mut t = ArlenTheme::from_bundled(SAMPLE_BUNDLED).expect("resolve");
        t.radius.intensity = 2.0;
        assert_eq!(t.effective_chip(),  8.0);
        assert_eq!(t.effective_button(), 12.0);
        assert_eq!(t.effective_input(), 16.0);
        assert_eq!(t.effective_card(),  24.0);
        assert_eq!(t.effective_modal(), 32.0);
        // Full stays pill — categorical, not on the spectrum.
        assert_eq!(t.effective_full(), 9999.0);
        // Window corners NOW scale with intensity (single-knob
        // user contract: roundness slider drives shell + window
        // chrome together).
        assert_eq!(
            t.effective_window_corners(),
            [24.0, 24.0, 24.0, 24.0]
        );
    }

    #[test]
    fn intensity_zero_yields_sharp_corners() {
        let mut t = ArlenTheme::from_bundled(SAMPLE_BUNDLED).expect("resolve");
        t.radius.intensity = 0.0;
        assert_eq!(t.effective_chip(),   0.0);
        assert_eq!(t.effective_button(), 0.0);
        assert_eq!(t.effective_card(),   0.0);
        // Full stays pill — categorical, not on the spectrum.
        assert_eq!(t.effective_full(), 9999.0);
    }

    #[test]
    fn intensity_clamped_to_2_max() {
        let mut t = ArlenTheme::from_bundled(SAMPLE_BUNDLED).expect("resolve");
        t.radius.intensity = 5.0; // way over
        assert_eq!(t.effective_button(), 12.0); // == button(6) * 2 (clamped)
    }

    #[test]
    fn intensity_negative_clamped_to_zero() {
        let mut t = ArlenTheme::from_bundled(SAMPLE_BUNDLED).expect("resolve");
        t.radius.intensity = -1.0;
        assert_eq!(t.effective_button(), 0.0);
        assert_eq!(t.effective_card(),   0.0);
    }

    #[test]
    fn customization_overrides_bundled_radii() {
        let custom = r##"
[radius]
button = 20
"##;
        let t = ArlenTheme::resolve(SAMPLE_BUNDLED, None, Some(custom)).expect("resolve");
        assert_eq!(t.radius.button, 20.0);
        // Other fields fall through to bundled.
        assert_eq!(t.radius.card, 12.0);
        assert_eq!(t.radius.intensity, 1.0);
    }

    #[test]
    fn customization_overrides_accent() {
        let custom = r##"
[color.semantic]
accent = "#ff00ff"
"##;
        let t = ArlenTheme::resolve(SAMPLE_BUNDLED, None, Some(custom)).expect("resolve");
        // Magenta = 1.0, 0.0, 1.0
        assert!((t.color.accent[0] - 1.0).abs() < 0.01);
        assert!((t.color.accent[1] - 0.0).abs() < 0.01);
        assert!((t.color.accent[2] - 1.0).abs() < 0.01);
    }

    #[test]
    fn meta_only_resolves_via_defaults() {
        // Only [meta] is structurally required. Everything else
        // falls back to sane Rust-side defaults so a partial
        // customization file (e.g. user editing only the accent)
        // resolves cleanly.
        let minimal = r##"
[meta]
id = "minimal"
name = "Minimal"
variant = "dark"
"##;
        let t = ArlenTheme::from_bundled(minimal).expect("resolve from meta-only");
        assert_eq!(t.meta.id, "minimal");
        // Defaults must match the documented design system.
        assert_eq!(t.radius.chip, 4.0);
        assert_eq!(t.radius.button, 6.0);
        assert_eq!(t.radius.full, 9999.0);
    }

    #[test]
    fn missing_meta_errors() {
        let no_meta = r##"
[radius]
button = 12
"##;
        let r = ArlenTheme::from_bundled(no_meta);
        assert!(matches!(r, Err(ResolveError::MissingField("meta"))));
    }

    #[test]
    fn invalid_variant_errors() {
        let bad = SAMPLE_BUNDLED.replace(r#"variant = "dark""#, r#"variant = "purple""#);
        let r = ArlenTheme::from_bundled(&bad);
        assert!(matches!(r, Err(ResolveError::InvalidColor { .. })));
    }

    #[test]
    fn hex_parsing_6_digit() {
        let c = parse_hex("#ff8000").unwrap();
        assert!((c[0] - 1.0).abs() < 0.01);
        assert!((c[1] - 0.502).abs() < 0.01);
        assert!((c[2] - 0.0).abs() < 0.01);
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
    }

    #[test]
    fn scale_radius_rounds_to_integer() {
        // 6 * 1.5 = 9.0 (already integer)
        assert_eq!(scale_radius(6.0, 1.5), 9.0);
        // 7 * 1.5 = 10.5 -> rounds to 11
        assert_eq!(scale_radius(7.0, 1.5), 11.0);
        // 5 * 0.3 = 1.5 -> rounds to 2
        assert_eq!(scale_radius(5.0, 0.3), 2.0);
    }

    /// Every free-string field of a resolved theme (the values that fan out into
    /// generated config). The TH-0 property: all of these are always inert.
    fn free_strings(t: &ArlenTheme) -> Vec<&str> {
        vec![
            t.spacing.xs.as_str(),
            t.spacing.sm.as_str(),
            t.spacing.md.as_str(),
            t.spacing.lg.as_str(),
            t.spacing.xl.as_str(),
            t.typography.font_sans.as_str(),
            t.typography.font_mono.as_str(),
            t.typography.size_base.as_str(),
            t.typography.line_height.as_str(),
            t.motion.duration_fast.as_str(),
            t.motion.duration_normal.as_str(),
            t.motion.duration_slow.as_str(),
            t.motion.easing_default.as_str(),
            t.motion.easing_spring.as_str(),
            t.depth.shadow_sm.as_str(),
            t.depth.shadow_md.as_str(),
            t.depth.shadow_lg.as_str(),
            t.cursor.theme.as_str(),
            t.icons.theme.as_str(),
        ]
    }

    #[test]
    fn inert_gate_accepts_legitimate_values_and_rejects_break_out() {
        // Every default + a normal theme value is inert.
        for ok in [
            "4px",
            "1.5",
            "100ms",
            "cubic-bezier(0.4, 0, 0.2, 1)",
            "0 1px 2px rgba(0, 0, 0, 0.3)",
            "\"Inter Variable\", ui-sans-serif, system-ui, sans-serif",
            "default",
            "Adwaita",
            "MyFont, sans-serif",
        ] {
            assert!(is_inert_css_token(ok), "should accept inert value: {ok:?}");
        }
        // Break-out attempts across CSS + INI/conf are rejected.
        for bad in [
            "4px; color: red",            // CSS statement break-out
            "0 0 0 #000 } * { color: red", // CSS block break-out
            "x [section]",                // INI section
            "a @import url(evil)",         // at-rule
            "a\\b",                        // escape
            "a /* c */ d",                 // CSS comment
            "line1\nline2",               // multi-line injection
            "a <script> b",                // markup
        ] {
            assert!(!is_inert_css_token(bad), "should reject break-out value: {bad:?}");
        }
    }

    #[test]
    fn adversarial_free_strings_resolve_to_inert_values() {
        // The TH-0 trust-floor theorem: whatever an adversarial theme puts in the
        // free-string fields, the resolved values are all inert (a break-out value
        // is dropped to its safe default at resolve), so no generator can be made
        // to emit syntax out of theme data. Payloads avoid `'`/newline so they sit
        // in TOML literal strings; the control-char vector is covered by the unit
        // test above.
        let payloads = ["; } body", "x [section] y", "a /* */ b", "a<b>@c\\d"];
        for p in payloads {
            let user = format!(
                "[spacing]\nxs='{p}'\nsm='{p}'\nmd='{p}'\nlg='{p}'\nxl='{p}'\n\
                 [typography]\nfont_sans='{p}'\nfont_mono='{p}'\nsize_base='{p}'\nline_height='{p}'\n\
                 [motion]\nduration_fast='{p}'\nduration_normal='{p}'\nduration_slow='{p}'\neasing_default='{p}'\neasing_spring='{p}'\n\
                 [depth]\nshadow_sm='{p}'\nshadow_md='{p}'\nshadow_lg='{p}'\n\
                 [cursor]\ntheme='{p}'\n[icons]\ntheme='{p}'\n"
            );
            let t = ArlenTheme::resolve(SAMPLE_BUNDLED, Some(&user), None)
                .expect("an adversarial theme still resolves (to safe defaults)");
            for v in free_strings(&t) {
                assert!(
                    is_inert_css_token(v),
                    "payload {p:?} leaked a non-inert resolved value {v:?}"
                );
            }
        }
    }

    #[test]
    fn a_legitimate_user_free_string_survives_the_gate() {
        // The gate must not over-reject: a normal custom font passes through.
        let user = "[typography]\nfont_sans = \"My Custom Font, sans-serif\"\n";
        let t = ArlenTheme::resolve(SAMPLE_BUNDLED, Some(user), None).expect("resolve");
        assert_eq!(t.typography.font_sans, "My Custom Font, sans-serif");
    }

    #[test]
    fn terminal_ansi_is_synthesised_from_the_semantic_tokens() {
        // No [terminal] section: the two-path resolve takes the synthesis path.
        let t = ArlenTheme::from_bundled(SAMPLE_BUNDLED).expect("resolve");
        let c = &t.color;
        assert_eq!(t.terminal.ansi[1], c.error, "red <- error");
        assert_eq!(t.terminal.ansi[2], c.success, "green <- success");
        assert_eq!(t.terminal.ansi[3], c.warning, "yellow <- warning");
        assert_eq!(t.terminal.ansi[4], c.info, "blue <- info");
        assert_eq!(t.terminal.ansi[8], c.fg_disabled, "bright black is the muted slot");
        assert_eq!(t.terminal.ansi[15], c.fg_primary, "bright white <- fg.primary");
        assert_eq!(t.terminal.fg, c.fg_primary);
        assert_eq!(t.terminal.bg, c.bg_app);
        assert_eq!(t.terminal.cursor, c.accent);
        // A synthesised bright is genuinely brighter than its normal.
        assert!(t.terminal.ansi[9][0] > t.terminal.ansi[1][0], "bright red is lighter");
    }

    #[test]
    fn an_authored_ansi_slot_overrides_only_that_slot() {
        let user = "[terminal.ansi]\nred = \"#123456\"\n";
        let t = ArlenTheme::resolve(SAMPLE_BUNDLED, Some(user), None).expect("resolve");
        assert_eq!(t.terminal.ansi[1], parse_hex("#123456").unwrap(), "authored red honoured");
        // The neighbouring slots keep the synthesis: green untouched, and bright
        // red still derives from the SEMANTIC error colour (the override is
        // slot-exact, it does not re-seed the bright derivation).
        assert_eq!(t.terminal.ansi[2], t.color.success, "green still synthesised");
        assert_eq!(t.terminal.ansi[9], scale_rgb(t.color.error, 1.25), "bright red keeps the synthesis");
    }

    #[test]
    fn an_invalid_authored_ansi_slot_keeps_the_synthesis() {
        // Typed fail-to-default: a non-hex authored slot cannot reach the
        // resolved palette (it parses to None and the synthesis stands).
        let user = "[terminal.ansi]\nred = \"javascript:alert(1)\"\n";
        let t = ArlenTheme::resolve(SAMPLE_BUNDLED, Some(user), None).expect("resolve");
        assert_eq!(t.terminal.ansi[1], t.color.error, "invalid hex falls back to synthesis");
    }

    #[test]
    fn hostile_meta_id_and_name_are_contained_at_resolve() {
        // meta.id becomes a path component in the desktop-shell loader
        // (themes/{id}.toml), and meta.name is display text that must carry no
        // generated-config syntax. A dropped theme file with a traversal id and
        // an injection name must resolve to a safe slug + an inert name.
        let user = "[meta]\nid = \"../../../etc/passwd\"\nname = \"evil; } body { x: y\"\nvariant = \"dark\"\n";
        let t = ArlenTheme::resolve(SAMPLE_BUNDLED, Some(user), None).expect("resolve");
        assert_eq!(t.meta.id, "etc-passwd", "traversal id slugged to a safe component");
        assert!(!t.meta.id.contains('/') && !t.meta.id.contains(".."), "no path syntax survives");
        assert_eq!(t.meta.name, "Imported Theme", "non-inert name dropped to the default");
        // A legitimate id/name is preserved unchanged.
        let ok = "[meta]\nid = \"my-theme\"\nname = \"Solarized Dark\"\nvariant = \"dark\"\n";
        let t = ArlenTheme::resolve(SAMPLE_BUNDLED, Some(ok), None).expect("resolve");
        assert_eq!(t.meta.id, "my-theme");
        assert_eq!(t.meta.name, "Solarized Dark");
    }

    #[test]
    fn icon_theme_resolves_with_default_and_honours_an_authored_name() {
        let t = ArlenTheme::from_bundled(SAMPLE_BUNDLED).expect("resolve");
        assert_eq!(t.icons.theme, "default");
        let user = "[icons]\ntheme = \"Papirus-Dark\"\n";
        let t = ArlenTheme::resolve(SAMPLE_BUNDLED, Some(user), None).expect("resolve");
        assert_eq!(t.icons.theme, "Papirus-Dark");
    }
}

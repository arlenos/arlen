//! Lunaris theme system — single source of truth.
//!
//! Two file paths participate in theme resolution:
//!
//! 1. **Bundled theme** (`<crate>/themes/dark.toml` or `light.toml`,
//!    embedded at compile-time via `include_str!` from whichever
//!    crate consumes the schema). Provides every default value.
//! 2. **Active user theme** (`~/.local/share/lunaris/themes/{id}.toml`).
//!    Optional. When `appearance.toml [theme].active` names something
//!    that isn't a built-in id, the loader reads this file and merges
//!    it on top of the matching bundled variant (resolved via the
//!    `extends` field in the theme's `[meta]` section, default `dark`).
//! 3. **User customization** (`~/.config/lunaris/theme.toml`).
//!    Optional. Loose top-of-stack overrides — any field set here
//!    wins over both the active theme and the bundled defaults.
//!    This is the channel a user uses to tweak the active theme
//!    without writing a full theme.
//!
//! Plus a per-user PREFERENCE layer:
//!
//! 4. **`appearance.toml [overrides].radius_intensity`** — multiplier
//!    in `0.0..=2.0` applied to all semantic radii at *emit time*
//!    (`LunarisTheme::effective_*()`), excluding `radius.full` and
//!    `radius.window_corners` which are categorical. The base
//!    radii live in the theme; the multiplier is the user-only knob.
//!
//! Both compositor and desktop-shell read the same resolved
//! `LunarisTheme`. The schema is grouped into per-concern substructs
//! (color, radius, spacing, typography, motion, depth, wm, cursor)
//! so callers borrow the slice they care about.
//!
//! See `docs/architecture/theme-system.md` for the full architecture
//! and per-token semantic guidance.

mod file;
mod watcher;

pub use file::{
    BorderColors, ColorBgFile, ColorFgFile, ColorSection, ColorSemanticFile,
    CursorSection, DepthSection, LunarisThemeFile, MetaSection, MotionSection,
    RadiusSection, SoundsSection, SpacingSection, TypographySection,
    WallpaperSection, WmSection,
};
pub use watcher::ThemeWatcher;

use std::path::{Path, PathBuf};

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

/// Fully resolved theme. Both compositor and desktop-shell consume
/// this. Construct via `LunarisTheme::resolve(...)`; do not build
/// by hand outside tests.
#[derive(Debug, Clone)]
pub struct LunarisTheme {
    pub meta:       ThemeMeta,
    pub color:      ColorTokens,
    pub radius:     RadiusTokens,
    pub spacing:    SpacingTokens,
    pub typography: TypographyTokens,
    pub motion:     MotionTokens,
    pub depth:      DepthTokens,
    pub wm:         WmTokens,
    pub cursor:     CursorTokens,
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

impl LunarisTheme {
    /// Resolve the active theme.
    ///
    /// `bundled` is the compile-time-embedded base theme (one of
    /// `dark.toml` / `light.toml`). `user_theme` is the optional
    /// user-installed theme file at
    /// `~/.local/share/lunaris/themes/{active}.toml` — passed in
    /// already-loaded so the file-IO is the caller's responsibility.
    /// `customization` is the optional `~/.config/lunaris/theme.toml`
    /// content, also caller-loaded.
    ///
    /// Merge order: `bundled` < `user_theme` (extends-resolved)
    /// < `customization`. Later layers win field-by-field.
    pub fn resolve(
        bundled: &str,
        user_theme: Option<&str>,
        customization: Option<&str>,
    ) -> Result<Self, ResolveError> {
        let bundled_file: LunarisThemeFile = toml::from_str(bundled)?;
        let user_file: Option<LunarisThemeFile> = user_theme
            .map(toml::from_str)
            .transpose()?;
        let custom_file: Option<LunarisThemeFile> = customization
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

    /// Default config path for `~/.config/lunaris/theme.toml`.
    pub fn user_customization_path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join("lunaris")
            .join("theme.toml")
    }

    /// Default user-theme directory (`~/.local/share/lunaris/themes/`).
    pub fn user_themes_dir() -> PathBuf {
        dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join("lunaris")
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

fn from_file(f: LunarisThemeFile) -> Result<LunarisTheme, ResolveError> {
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

        accent:         color_or(sem.accent,         "semantic.accent",         "#6366f1")?,
        accent_hover:   color_or(sem.accent_hover,   "semantic.accent_hover",   "#818cf8")?,
        accent_pressed: color_or(sem.accent_pressed, "semantic.accent_pressed", "#4f46e5")?,
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
        xs: s.xs.unwrap_or_else(|| "4px".to_string()),
        sm: s.sm.unwrap_or_else(|| "8px".to_string()),
        md: s.md.unwrap_or_else(|| "16px".to_string()),
        lg: s.lg.unwrap_or_else(|| "24px".to_string()),
        xl: s.xl.unwrap_or_else(|| "32px".to_string()),
    };

    let t = f.typography.unwrap_or_default();
    let typography = TypographyTokens {
        font_sans:     t.font_sans.unwrap_or_else(|| {
            "\"Inter Variable\", ui-sans-serif, system-ui, sans-serif".to_string()
        }),
        font_mono:     t.font_mono.unwrap_or_else(|| {
            "\"JetBrains Mono\", ui-monospace, monospace".to_string()
        }),
        size_base:     t.size_base.unwrap_or_else(|| "14px".to_string()),
        line_height:   t.line_height.unwrap_or_else(|| "1.5".to_string()),
        weight_normal: t.weight_normal.unwrap_or(400),
        weight_medium: t.weight_medium.unwrap_or(500),
        weight_bold:   t.weight_bold.unwrap_or(600),
    };

    let m = f.motion.unwrap_or_default();
    let motion = MotionTokens {
        duration_fast:   m.duration_fast.unwrap_or_else(|| "100ms".to_string()),
        duration_normal: m.duration_normal.unwrap_or_else(|| "200ms".to_string()),
        duration_slow:   m.duration_slow.unwrap_or_else(|| "400ms".to_string()),
        easing_default:  m.easing_default.unwrap_or_else(|| {
            "cubic-bezier(0.4, 0, 0.2, 1)".to_string()
        }),
        easing_spring:   m.easing_spring.unwrap_or_else(|| {
            "cubic-bezier(0.34, 1.56, 0.64, 1)".to_string()
        }),
    };

    let d = f.depth.unwrap_or_default();
    let depth = DepthTokens {
        shadow_sm:    d.shadow_sm.unwrap_or_else(|| {
            "0 1px 2px rgba(0, 0, 0, 0.3)".to_string()
        }),
        shadow_md:    d.shadow_md.unwrap_or_else(|| {
            "0 4px 12px rgba(0, 0, 0, 0.4)".to_string()
        }),
        shadow_lg:    d.shadow_lg.unwrap_or_else(|| {
            "0 8px 24px rgba(0, 0, 0, 0.5)".to_string()
        }),
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
        theme: c.theme.unwrap_or_else(|| "default".to_string()),
        size:  c.size.unwrap_or(24),
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

    Ok(LunarisTheme {
        meta: ThemeMeta {
            id: meta.id,
            name: meta.name,
            variant,
            extends: meta.extends,
        },
        color,
        radius,
        spacing,
        typography,
        motion,
        depth,
        wm,
        cursor,
    })
}

/// Field-by-field merge — every Optional in `over` that is `Some`
/// replaces the corresponding field in `under`. Used to layer
/// user themes / customization over the bundled defaults.
fn merge_files(under: LunarisThemeFile, over: LunarisThemeFile) -> LunarisThemeFile {
    LunarisThemeFile {
        meta: over.meta.or(under.meta),
        color: merge_color(under.color, over.color),
        radius: merge_radius(under.radius, over.radius),
        spacing: merge_spacing(under.spacing, over.spacing),
        typography: merge_typography(under.typography, over.typography),
        motion: merge_motion(under.motion, over.motion),
        depth: merge_depth(under.depth, over.depth),
        wm: merge_wm(under.wm, over.wm),
        cursor: merge_cursor(under.cursor, over.cursor),
        wallpaper: over.wallpaper.or(under.wallpaper),
        sounds: over.sounds.or(under.sounds),
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
) -> Result<LunarisTheme, ResolveError> {
    let user_path = LunarisTheme::user_theme_path(active_id);
    let user_theme = if user_path.exists() {
        Some(std::fs::read_to_string(&user_path)?)
    } else {
        None
    };
    let custom_path = LunarisTheme::user_customization_path();
    let custom = if custom_path.exists() {
        Some(std::fs::read_to_string(&custom_path)?)
    } else {
        None
    };
    LunarisTheme::resolve(bundled, user_theme.as_deref(), custom.as_deref())
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
name = "Lunaris Dark"
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
        let t = LunarisTheme::from_bundled(SAMPLE_BUNDLED).expect("resolve");
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
        let mut t = LunarisTheme::from_bundled(SAMPLE_BUNDLED).expect("resolve");
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
        let mut t = LunarisTheme::from_bundled(SAMPLE_BUNDLED).expect("resolve");
        t.radius.intensity = 0.0;
        assert_eq!(t.effective_chip(),   0.0);
        assert_eq!(t.effective_button(), 0.0);
        assert_eq!(t.effective_card(),   0.0);
        // Full stays pill — categorical, not on the spectrum.
        assert_eq!(t.effective_full(), 9999.0);
    }

    #[test]
    fn intensity_clamped_to_2_max() {
        let mut t = LunarisTheme::from_bundled(SAMPLE_BUNDLED).expect("resolve");
        t.radius.intensity = 5.0; // way over
        assert_eq!(t.effective_button(), 12.0); // == button(6) * 2 (clamped)
    }

    #[test]
    fn intensity_negative_clamped_to_zero() {
        let mut t = LunarisTheme::from_bundled(SAMPLE_BUNDLED).expect("resolve");
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
        let t = LunarisTheme::resolve(SAMPLE_BUNDLED, None, Some(custom)).expect("resolve");
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
        let t = LunarisTheme::resolve(SAMPLE_BUNDLED, None, Some(custom)).expect("resolve");
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
        let t = LunarisTheme::from_bundled(minimal).expect("resolve from meta-only");
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
        let r = LunarisTheme::from_bundled(no_meta);
        assert!(matches!(r, Err(ResolveError::MissingField("meta"))));
    }

    #[test]
    fn invalid_variant_errors() {
        let bad = SAMPLE_BUNDLED.replace(r#"variant = "dark""#, r#"variant = "purple""#);
        let r = LunarisTheme::from_bundled(&bad);
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
}

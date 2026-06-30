//! TOML file schema for theme files (bundled + user-installed +
//! customization overlay). All fields are `Option` so partial files
//! merge cleanly via `merge_files()` in `lib.rs`.
//!
//! The required-vs-optional split lives in the resolver (`from_file`):
//! after the merge, certain sections must be present (color,
//! radius, spacing, typography, motion, depth) while others
//! (wm, cursor, wallpaper, sounds) are fully optional with code-
//! side defaults.

use serde::Deserialize;

/// Root structure of a theme file.
#[derive(Debug, Default, Deserialize)]
pub struct ArlenThemeFile {
    pub meta:       Option<MetaSection>,
    pub color:      Option<ColorSection>,
    pub radius:     Option<RadiusSection>,
    pub spacing:    Option<SpacingSection>,
    pub typography: Option<TypographySection>,
    pub motion:     Option<MotionSection>,
    pub depth:      Option<DepthSection>,
    pub wm:         Option<WmSection>,
    pub cursor:     Option<CursorSection>,
    pub terminal:   Option<TerminalSection>,
    pub icons:      Option<IconsSection>,
    pub wallpaper:  Option<WallpaperSection>,
    pub sounds:     Option<SoundsSection>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MetaSection {
    pub id:      String,
    pub name:    String,
    pub variant: String,
    /// Optional parent-theme id. The user-theme resolver loads the
    /// named theme as the base before applying this file.
    pub extends: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ColorSection {
    pub bg:       Option<ColorBgFile>,
    pub fg:       Option<ColorFgFile>,
    pub semantic: Option<ColorSemanticFile>,
    pub border:   Option<BorderColors>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ColorBgFile {
    pub shell:   Option<String>,
    pub app:     Option<String>,
    pub card:    Option<String>,
    pub overlay: Option<String>,
    pub input:   Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ColorFgFile {
    pub primary:   Option<String>,
    pub secondary: Option<String>,
    pub disabled:  Option<String>,
    pub inverse:   Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ColorSemanticFile {
    pub accent:         Option<String>,
    pub accent_hover:   Option<String>,
    pub accent_pressed: Option<String>,
    pub success:        Option<String>,
    pub warning:        Option<String>,
    pub error:          Option<String>,
    pub info:           Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct BorderColors {
    pub default: Option<String>,
    pub strong:  Option<String>,
}

/// Border-radius tokens. Numeric (in logical pixels) — no unit
/// suffix in the TOML. The resolver formats them with `px` for
/// CSS emission. `intensity` is the user-multiplier default
/// (theme authors can ship a non-1.0 baseline).
///
/// `window_corners` is a 4-tuple `[top-left, top-right,
/// bottom-right, bottom-left]` for the compositor-rendered window
/// outline shape.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct RadiusSection {
    pub chip:           Option<f32>,
    pub button:         Option<f32>,
    pub input:          Option<f32>,
    pub card:           Option<f32>,
    pub modal:          Option<f32>,
    pub full:           Option<f32>,
    pub window_corners: Option<[f32; 4]>,
    pub intensity:      Option<f32>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct SpacingSection {
    pub xs: Option<String>,
    pub sm: Option<String>,
    pub md: Option<String>,
    pub lg: Option<String>,
    pub xl: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct TypographySection {
    pub font_sans:     Option<String>,
    pub font_mono:     Option<String>,
    pub size_base:     Option<String>,
    pub line_height:   Option<String>,
    pub weight_normal: Option<u32>,
    pub weight_medium: Option<u32>,
    pub weight_bold:   Option<u32>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct MotionSection {
    pub duration_fast:   Option<String>,
    pub duration_normal: Option<String>,
    pub duration_slow:   Option<String>,
    pub easing_default:  Option<String>,
    pub easing_spring:   Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct DepthSection {
    pub shadow_sm:    Option<String>,
    pub shadow_md:    Option<String>,
    pub shadow_lg:    Option<String>,
    pub shadow_card:  Option<String>,
    pub blur_enabled: Option<bool>,
}

/// Window-manager tokens (compositor-only).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct WmSection {
    pub active_hint: Option<u32>,
    pub gaps_inner:  Option<u32>,
    pub gaps_outer:  Option<u32>,
    /// Optional accent override for the window outline; empty
    /// string falls through to `[color.semantic].accent`.
    pub window_hint: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct CursorSection {
    pub theme: Option<String>,
    pub size:  Option<u32>,
}

/// Terminal-specific tokens (theming-system-plan.md Fork (a) = A2). The
/// optional `[terminal.ansi]` block lets an author who cares about a tuned
/// terminal palette override the semantic→ANSI synthesis slot-by-slot; the 95%
/// of themes that leave it out get the synthesised projection.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct TerminalSection {
    pub ansi: Option<TerminalAnsiSection>,
}

/// The authored 16-colour ANSI block. Every slot is optional: a present slot
/// overrides the synthesised colour for exactly that slot (hex, same format as
/// every other colour field), absent slots keep the synthesis.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct TerminalAnsiSection {
    pub black:          Option<String>,
    pub red:            Option<String>,
    pub green:          Option<String>,
    pub yellow:         Option<String>,
    pub blue:           Option<String>,
    pub magenta:        Option<String>,
    pub cyan:           Option<String>,
    pub white:          Option<String>,
    pub bright_black:   Option<String>,
    pub bright_red:     Option<String>,
    pub bright_green:   Option<String>,
    pub bright_yellow:  Option<String>,
    pub bright_blue:    Option<String>,
    pub bright_magenta: Option<String>,
    pub bright_cyan:    Option<String>,
    pub bright_white:   Option<String>,
}

/// Icon-theme selection, mirroring `[cursor]`'s `theme` field (Fork (a) = A2:
/// the schema previously had no icon story at all).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct IconsSection {
    pub theme: Option<String>,
}

/// TRUST-FLOOR REMINDER: the wallpaper and sound fields below are free-string
/// asset paths that are NOT yet resolved into the consumed [`crate::ArlenTheme`]
/// or emitted by any generator, so they are not a break-out surface today. When
/// an asset emitter IS wired (e.g. writing a wallpaper/sound path into a
/// compositor or notification config), each path MUST be routed through the
/// inert floor (`inert_or` / `is_inert_css_token`) at resolve, exactly like the
/// other free-strings — otherwise the TH-0 inert-in/safe-out guarantee silently
/// will not cover it and a hostile theme path could carry config syntax.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct WallpaperSection {
    pub r#type:    Option<String>,
    pub file:      Option<String>,
    pub dawn:      Option<String>,
    pub morning:   Option<String>,
    pub day:       Option<String>,
    pub evening:   Option<String>,
    pub night:     Option<String>,
    pub r#loop:    Option<bool>,
    pub fps:       Option<u32>,
    pub fallback:  Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct SoundsSection {
    pub notification: Option<String>,
    pub error:        Option<String>,
    pub warning:      Option<String>,
    pub action:       Option<String>,
}

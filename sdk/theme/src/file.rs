//! TOML file schema for theme.toml.
//! All fields are Option so partial theme files merge cleanly with defaults.

use serde::Deserialize;

/// Root structure of `~/.config/lunaris/theme.toml`.
#[derive(Debug, Default, Deserialize)]
pub struct LunarisThemeFile {
    pub color: Option<ColorSection>,
    pub motion: Option<MotionSection>,
    pub depth: Option<DepthSection>,
    pub typography: Option<TypographySection>,
    pub cursor: Option<CursorSection>,
    pub wallpaper: Option<WallpaperSection>,
    pub sounds: Option<SoundsSection>,
    /// Compositor window-management tokens (radius, gaps, hints).
    pub wm: Option<WmSection>,
}

#[derive(Debug, Default, Deserialize)]
pub struct ColorSection {
    pub bg: Option<BgColors>,
    pub fg: Option<FgColors>,
    pub accent: Option<String>,
    pub border: Option<String>,
    pub error: Option<String>,
    pub warning: Option<String>,
    pub success: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct BgColors {
    pub shell: Option<String>,
    pub app: Option<String>,
    pub card: Option<String>,
    pub overlay: Option<String>,
    pub input: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct FgColors {
    pub primary: Option<String>,
    pub secondary: Option<String>,
    pub disabled: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct MotionSection {
    pub duration_short: Option<u32>,
    pub duration_medium: Option<u32>,
    pub duration_long: Option<u32>,
    pub easing_default: Option<String>,
    pub easing_enter: Option<String>,
    pub easing_exit: Option<String>,
    pub easing_spring: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct DepthSection {
    pub shadow_active: Option<String>,
    pub shadow_inactive: Option<String>,
    pub shadow_popup: Option<String>,
    pub blur_panels: Option<String>,
    pub blur_popups: Option<String>,
    pub blur_enabled: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
pub struct TypographySection {
    pub font_sans: Option<String>,
    pub font_mono: Option<String>,
    pub font_size: Option<f32>,
    pub font_weight_regular: Option<u32>,
    pub font_weight_bold: Option<u32>,
    pub font_lineheight: Option<f32>,
}

#[derive(Debug, Default, Deserialize)]
pub struct CursorSection {
    pub theme: Option<String>,
    pub size: Option<u32>,
}

#[derive(Debug, Default, Deserialize)]
pub struct WallpaperSection {
    pub r#type: Option<String>,
    pub file: Option<String>,
    pub dawn: Option<String>,
    pub morning: Option<String>,
    pub day: Option<String>,
    pub evening: Option<String>,
    pub night: Option<String>,
    pub r#loop: Option<bool>,
    pub fps: Option<u32>,
    pub fallback: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct SoundsSection {
    pub notification: Option<String>,
    pub error: Option<String>,
    pub warning: Option<String>,
    pub action: Option<String>,
}

/// Window-manager specific tokens consumed by the compositor.
#[derive(Debug, Default, Deserialize)]
pub struct WmSection {
    /// Corner radius in logical pixels (applied uniformly to all 4 corners).
    pub radius: Option<f32>,
    /// Active window hint thickness in pixels (0 to disable).
    pub active_hint: Option<u32>,
    /// Gap size in logical pixels (inner and outer).
    pub gaps: Option<u32>,
    /// Whether the shell surface is dark (true for Panda shell).
    pub is_dark: Option<bool>,
    /// Optional custom window hint color (hex string).
    pub window_hint: Option<String>,
}

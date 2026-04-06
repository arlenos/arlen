/// Full theme token schema for Lunaris OS.
///
/// Every visual property (colors, radii, spacing, typography, motion, depth)
/// is defined here. Built-in themes ship as TOML files that deserialize
/// directly into `ThemeTokens`.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Top-level
// ---------------------------------------------------------------------------

/// Complete set of design tokens for a Lunaris theme.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemeTokens {
    /// Theme metadata.
    pub meta: ThemeMeta,
    /// Color palette.
    pub colors: ColorTokens,
    /// Border radii.
    pub radius: RadiusTokens,
    /// Spacing scale.
    pub spacing: SpacingTokens,
    /// Font settings.
    pub typography: TypographyTokens,
    /// Animation timing.
    pub motion: MotionTokens,
    /// Box shadows.
    pub depth: DepthTokens,
}

/// Theme metadata stored in the `[meta]` TOML section.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemeMeta {
    /// Unique identifier (e.g. "dark", "light", "nord").
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// Dark or light variant.
    pub variant: ThemeVariant,
    /// Optional parent theme to inherit from.
    pub extends: Option<String>,
}

/// Whether the theme is dark or light.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeVariant {
    Dark,
    Light,
}

// ---------------------------------------------------------------------------
// Colors
// ---------------------------------------------------------------------------

/// All color tokens, organized by role.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColorTokens {
    /// Background colors.
    pub background: BackgroundColors,
    /// Foreground (text) colors.
    pub foreground: ForegroundColors,
    /// Semantic / accent colors.
    pub semantic: SemanticColors,
    /// Border colors.
    pub border: BorderColors,
}

/// Background color roles.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackgroundColors {
    /// Shell chrome (top bar, panels).
    pub shell: String,
    /// Application content area.
    pub app: String,
    /// Elevated cards.
    pub card: String,
    /// Modal overlays and backdrops.
    pub overlay: String,
    /// Text input fields.
    pub input: String,
}

/// Foreground (text) color roles.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForegroundColors {
    /// Primary text.
    pub primary: String,
    /// Secondary / muted text.
    pub secondary: String,
    /// Disabled text.
    pub disabled: String,
    /// Inverse text (on accent backgrounds).
    pub inverse: String,
}

/// Semantic / interactive colors.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SemanticColors {
    /// Primary accent.
    pub accent: String,
    /// Accent on hover.
    pub accent_hover: String,
    /// Accent when pressed.
    pub accent_pressed: String,
    /// Success state.
    pub success: String,
    /// Warning state.
    pub warning: String,
    /// Error / destructive state.
    pub error: String,
    /// Informational state.
    pub info: String,
}

/// Border color roles.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BorderColors {
    /// Default subtle border.
    pub default: String,
    /// Stronger, more visible border.
    pub strong: String,
}

// ---------------------------------------------------------------------------
// Non-color tokens
// ---------------------------------------------------------------------------

/// Border radius scale.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RadiusTokens {
    pub sm: String,
    pub md: String,
    pub lg: String,
    pub full: String,
}

/// Spacing scale.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpacingTokens {
    pub xs: String,
    pub sm: String,
    pub md: String,
    pub lg: String,
    pub xl: String,
}

/// Typography tokens.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypographyTokens {
    /// Sans-serif font stack.
    pub font_sans: String,
    /// Monospace font stack.
    pub font_mono: String,
    /// Base font size.
    pub size_base: String,
    /// Base line height.
    pub line_height: String,
    /// Normal font weight.
    pub weight_normal: String,
    /// Medium font weight.
    pub weight_medium: String,
    /// Bold font weight.
    pub weight_bold: String,
}

/// Animation / transition timing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MotionTokens {
    /// Fast transitions (hover, focus).
    pub duration_fast: String,
    /// Normal transitions (expand, collapse).
    pub duration_normal: String,
    /// Slow transitions (page, modal).
    pub duration_slow: String,
    /// Default easing curve.
    pub easing_default: String,
    /// Spring-like easing.
    pub easing_spring: String,
}

/// Elevation / shadow tokens.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DepthTokens {
    /// Subtle shadow (cards).
    pub shadow_sm: String,
    /// Medium shadow (dropdowns, popovers).
    pub shadow_md: String,
    /// Heavy shadow (modals).
    pub shadow_lg: String,
}

// ---------------------------------------------------------------------------
// User config
// ---------------------------------------------------------------------------

/// User overrides applied on top of any theme.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UserOverrides {
    /// Custom accent color (hex).
    pub accent: Option<String>,
    /// Font scale multiplier (1.0 = normal).
    pub font_scale: Option<f32>,
}

/// Accessibility preferences.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccessibilitySettings {
    /// Disable all animations.
    #[serde(default)]
    pub reduce_motion: bool,
}

impl Default for AccessibilitySettings {
    fn default() -> Self {
        Self { reduce_motion: false }
    }
}

/// Top-level appearance config file (`~/.config/lunaris/appearance.toml`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppearanceConfig {
    /// Which theme to use.
    pub theme: ThemeSelection,
    /// User overrides.
    #[serde(default)]
    pub overrides: UserOverrides,
    /// Accessibility settings.
    #[serde(default)]
    pub accessibility: AccessibilitySettings,
}

/// Theme selection within the appearance config.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemeSelection {
    /// Theme ID to activate.
    pub active: String,
}

impl Default for AppearanceConfig {
    fn default() -> Self {
        Self {
            theme: ThemeSelection {
                active: "dark".into(),
            },
            overrides: UserOverrides::default(),
            accessibility: AccessibilitySettings::default(),
        }
    }
}

/// Lightweight theme summary for the UI theme picker.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemeInfo {
    /// Unique identifier.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Dark or light.
    pub variant: ThemeVariant,
    /// Whether this is a built-in (non-removable) theme.
    pub is_builtin: bool,
}

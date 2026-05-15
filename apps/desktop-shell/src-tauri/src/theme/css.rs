//! CSS variable generation from a resolved `LunarisTheme`.
//!
//! Maps the canonical token hierarchy into a flat
//! `BTreeMap<String, String>` of CSS custom properties (without
//! `--` prefix), plus helpers to produce the injectable `:root`
//! string.
//!
//! Radius emission applies the user's intensity multiplier via
//! `LunarisTheme::effective_*()` so the variables observed by the
//! webview are already-scaled. `--radius-full` and the (unset
//! today) per-corner-window-radius are categorical and bypass
//! intensity.

use std::collections::BTreeMap;

use lunaris_theme::{LunarisTheme, ThemeVariant};
use serde::{Deserialize, Serialize};

use super::schema::UserOverrides;

/// Flat CSS variable set ready for the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CssVariables {
    /// Variable name (without `--`) to value.
    pub variables: BTreeMap<String, String>,
    /// Font scale multiplier (1.0 = default).
    pub font_scale: f32,
    /// `"dark"` or `"light"` (matches CSS `color-scheme`).
    pub variant: String,
}

/// Convert a resolved theme + user overrides to a flat CSS-var map.
pub fn to_css_variables(theme: &LunarisTheme, overrides: &UserOverrides) -> CssVariables {
    let mut vars = BTreeMap::new();

    // ── Colors ──
    vars.insert("color-bg-shell".into(),   rgba_to_css(&theme.color.bg_shell));
    vars.insert("color-bg-app".into(),     rgba_to_css(&theme.color.bg_app));
    vars.insert("color-bg-card".into(),    rgba_to_css(&theme.color.bg_card));
    vars.insert("color-bg-overlay".into(), rgba_to_css(&theme.color.bg_overlay));
    vars.insert("color-bg-input".into(),   rgba_to_css(&theme.color.bg_input));

    vars.insert("color-fg-primary".into(),   rgba_to_css(&theme.color.fg_primary));
    vars.insert("color-fg-secondary".into(), rgba_to_css(&theme.color.fg_secondary));
    vars.insert("color-fg-disabled".into(),  rgba_to_css(&theme.color.fg_disabled));
    vars.insert("color-fg-inverse".into(),   rgba_to_css(&theme.color.fg_inverse));

    vars.insert("color-accent".into(),         rgba_to_css(&theme.color.accent));
    vars.insert("color-accent-hover".into(),   rgba_to_css(&theme.color.accent_hover));
    vars.insert("color-accent-pressed".into(), rgba_to_css(&theme.color.accent_pressed));
    vars.insert("color-success".into(), rgba_to_css(&theme.color.success));
    vars.insert("color-warning".into(), rgba_to_css(&theme.color.warning));
    vars.insert("color-error".into(),   rgba_to_css(&theme.color.error));
    vars.insert("color-info".into(),    rgba_to_css(&theme.color.info));

    vars.insert("color-border-default".into(), rgba_to_css(&theme.color.border_default));
    vars.insert("color-border-strong".into(),  rgba_to_css(&theme.color.border_strong));

    // ── Radius (semantic; intensity applied) ──
    vars.insert("radius-chip".into(),   format!("{}px", theme.effective_chip()));
    vars.insert("radius-button".into(), format!("{}px", theme.effective_button()));
    vars.insert("radius-input".into(),  format!("{}px", theme.effective_input()));
    vars.insert("radius-card".into(),   format!("{}px", theme.effective_card()));
    vars.insert("radius-modal".into(),  format!("{}px", theme.effective_modal()));
    vars.insert("radius-full".into(),   format!("{}px", theme.effective_full()));

    // ── Spacing ──
    vars.insert("spacing-xs".into(), theme.spacing.xs.clone());
    vars.insert("spacing-sm".into(), theme.spacing.sm.clone());
    vars.insert("spacing-md".into(), theme.spacing.md.clone());
    vars.insert("spacing-lg".into(), theme.spacing.lg.clone());
    vars.insert("spacing-xl".into(), theme.spacing.xl.clone());

    // ── Typography ──
    vars.insert("font-sans".into(), theme.typography.font_sans.clone());
    vars.insert("font-mono".into(), theme.typography.font_mono.clone());
    vars.insert("font-size-base".into(), theme.typography.size_base.clone());
    vars.insert("line-height".into(),   theme.typography.line_height.clone());
    vars.insert("font-weight-normal".into(), theme.typography.weight_normal.to_string());
    vars.insert("font-weight-medium".into(), theme.typography.weight_medium.to_string());
    vars.insert("font-weight-bold".into(),   theme.typography.weight_bold.to_string());

    // ── Motion ──
    vars.insert("duration-fast".into(),   theme.motion.duration_fast.clone());
    vars.insert("duration-normal".into(), theme.motion.duration_normal.clone());
    vars.insert("duration-slow".into(),   theme.motion.duration_slow.clone());
    vars.insert("easing-default".into(),  theme.motion.easing_default.clone());
    vars.insert("easing-spring".into(),   theme.motion.easing_spring.clone());

    // ── Depth ──
    vars.insert("shadow-sm".into(), theme.depth.shadow_sm.clone());
    vars.insert("shadow-md".into(), theme.depth.shadow_md.clone());
    vars.insert("shadow-lg".into(), theme.depth.shadow_lg.clone());

    let font_scale = overrides.font_scale.unwrap_or(1.0);
    let variant = match theme.meta.variant {
        ThemeVariant::Dark => "dark",
        ThemeVariant::Light => "light",
    };

    CssVariables {
        variables: vars,
        font_scale,
        variant: variant.into(),
    }
}

/// Generate an injectable `:root { ... }` CSS string. The font-size
/// override only emits when the user picked a non-1.0 scale.
pub fn to_css_string(css_vars: &CssVariables) -> String {
    let mut lines = Vec::with_capacity(css_vars.variables.len() + 4);
    lines.push(":root {".into());

    for (name, value) in &css_vars.variables {
        lines.push(format!("  --{name}: {value};"));
    }

    if (css_vars.font_scale - 1.0).abs() > 0.001 {
        let px = (16.0 * css_vars.font_scale).round();
        lines.push(format!("  font-size: {px}px;"));
    }

    lines.push("}".into());
    lines.push(format!("html {{ color-scheme: {}; }}", css_vars.variant));

    lines.join("\n")
}

/// Format an `[r, g, b, a]` (each `0.0..=1.0`) as a CSS color
/// string. Uses `#rrggbb` for opaque, `rgba(...)` for translucent.
fn rgba_to_css(rgba: &lunaris_theme::Rgba) -> String {
    let r = (rgba[0] * 255.0).round().clamp(0.0, 255.0) as u8;
    let g = (rgba[1] * 255.0).round().clamp(0.0, 255.0) as u8;
    let b = (rgba[2] * 255.0).round().clamp(0.0, 255.0) as u8;
    if (rgba[3] - 1.0).abs() < 1e-3 {
        format!("#{r:02x}{g:02x}{b:02x}")
    } else {
        format!("rgba({r}, {g}, {b}, {:.3})", rgba[3])
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::loader::ThemeLoader;

    #[test]
    fn generates_all_expected_keys() {
        let loader = ThemeLoader::new().unwrap();
        let theme = loader.load("dark").unwrap();
        let css = to_css_variables(&theme, &UserOverrides::default());

        let expected = [
            "color-bg-shell", "color-bg-app", "color-bg-card",
            "color-bg-overlay", "color-bg-input",
            "color-fg-primary", "color-fg-secondary",
            "color-fg-disabled", "color-fg-inverse",
            "color-accent", "color-accent-hover", "color-accent-pressed",
            "color-success", "color-warning", "color-error", "color-info",
            "color-border-default", "color-border-strong",
            "radius-chip", "radius-button", "radius-input",
            "radius-card", "radius-modal", "radius-full",
            "spacing-xs", "spacing-sm", "spacing-md", "spacing-lg", "spacing-xl",
            "font-sans", "font-mono", "font-size-base", "line-height",
            "font-weight-normal", "font-weight-medium", "font-weight-bold",
            "duration-fast", "duration-normal", "duration-slow",
            "easing-default", "easing-spring",
            "shadow-sm", "shadow-md", "shadow-lg",
        ];
        for key in expected {
            assert!(css.variables.contains_key(key), "missing key: {key}");
        }
        assert_eq!(css.variables.len(), expected.len());
    }

    #[test]
    fn radius_intensity_applied_in_emission() {
        let loader = ThemeLoader::new().unwrap();
        let mut theme = loader.load("dark").unwrap();
        theme.radius.intensity = 2.0;
        let css = to_css_variables(&theme, &UserOverrides::default());
        // chip(4) * 2.0 = 8
        assert_eq!(css.variables.get("radius-chip"), Some(&"8px".into()));
        // full UNCHANGED by intensity.
        assert_eq!(css.variables.get("radius-full"), Some(&"9999px".into()));
    }

    #[test]
    fn radius_intensity_zero_yields_sharp() {
        let loader = ThemeLoader::new().unwrap();
        let mut theme = loader.load("dark").unwrap();
        theme.radius.intensity = 0.0;
        let css = to_css_variables(&theme, &UserOverrides::default());
        assert_eq!(css.variables.get("radius-chip"), Some(&"0px".into()));
        assert_eq!(css.variables.get("radius-button"), Some(&"0px".into()));
        // full STILL pill.
        assert_eq!(css.variables.get("radius-full"), Some(&"9999px".into()));
    }

    #[test]
    fn rgba_to_css_opaque_uses_hex() {
        let c = rgba_to_css(&[1.0, 0.0, 0.0, 1.0]);
        assert_eq!(c, "#ff0000");
    }

    #[test]
    fn rgba_to_css_translucent_uses_rgba() {
        let c = rgba_to_css(&[1.0, 0.0, 0.0, 0.5]);
        assert!(c.starts_with("rgba("));
        assert!(c.contains("0.500") || c.contains("0.5"));
    }

    #[test]
    fn css_string_contains_root_and_color_scheme() {
        let loader = ThemeLoader::new().unwrap();
        let theme = loader.load("dark").unwrap();
        let css = to_css_variables(&theme, &UserOverrides::default());
        let output = to_css_string(&css);
        assert!(output.contains(":root {"));
        assert!(output.contains("color-scheme: dark"));
        assert!(!output.contains("font-size:")); // scale = 1.0, no override
    }

    #[test]
    fn font_scale_injected() {
        let loader = ThemeLoader::new().unwrap();
        let theme = loader.load("light").unwrap();
        let overrides = UserOverrides {
            accent: None,
            font_scale: Some(1.25),
            radius_intensity: None,
        };
        let css = to_css_variables(&theme, &overrides);
        let output = to_css_string(&css);
        assert!(output.contains("font-size: 20px;"));
        assert_eq!(css.font_scale, 1.25);
        assert_eq!(css.variant, "light");
    }
}

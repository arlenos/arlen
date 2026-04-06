/// CSS variable generation from theme tokens.
///
/// Maps the structured `ThemeTokens` into a flat `HashMap<String, String>` of
/// CSS custom property names (without `--` prefix) to values. Also generates
/// an injectable CSS string for `:root`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::schema::{ThemeTokens, ThemeVariant, UserOverrides};

/// Flat CSS variable set ready for the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CssVariables {
    /// Variable name (without `--`) to value.
    pub variables: BTreeMap<String, String>,
    /// Font scale multiplier (1.0 = default).
    pub font_scale: f32,
    /// "dark" or "light".
    pub variant: String,
}

/// Convert theme tokens and user overrides into a flat CSS variable map.
pub fn to_css_variables(tokens: &ThemeTokens, overrides: &UserOverrides) -> CssVariables {
    let mut vars = BTreeMap::new();

    // Background
    vars.insert("color-bg-shell".into(), tokens.colors.background.shell.clone());
    vars.insert("color-bg-app".into(), tokens.colors.background.app.clone());
    vars.insert("color-bg-card".into(), tokens.colors.background.card.clone());
    vars.insert("color-bg-overlay".into(), tokens.colors.background.overlay.clone());
    vars.insert("color-bg-input".into(), tokens.colors.background.input.clone());

    // Foreground
    vars.insert("color-fg-primary".into(), tokens.colors.foreground.primary.clone());
    vars.insert("color-fg-secondary".into(), tokens.colors.foreground.secondary.clone());
    vars.insert("color-fg-disabled".into(), tokens.colors.foreground.disabled.clone());
    vars.insert("color-fg-inverse".into(), tokens.colors.foreground.inverse.clone());

    // Semantic
    vars.insert("color-accent".into(), tokens.colors.semantic.accent.clone());
    vars.insert("color-accent-hover".into(), tokens.colors.semantic.accent_hover.clone());
    vars.insert("color-accent-pressed".into(), tokens.colors.semantic.accent_pressed.clone());
    vars.insert("color-success".into(), tokens.colors.semantic.success.clone());
    vars.insert("color-warning".into(), tokens.colors.semantic.warning.clone());
    vars.insert("color-error".into(), tokens.colors.semantic.error.clone());
    vars.insert("color-info".into(), tokens.colors.semantic.info.clone());

    // Border
    vars.insert("color-border-default".into(), tokens.colors.border.default.clone());
    vars.insert("color-border-strong".into(), tokens.colors.border.strong.clone());

    // Radius
    vars.insert("radius-sm".into(), tokens.radius.sm.clone());
    vars.insert("radius-md".into(), tokens.radius.md.clone());
    vars.insert("radius-lg".into(), tokens.radius.lg.clone());
    vars.insert("radius-full".into(), tokens.radius.full.clone());

    // Spacing
    vars.insert("spacing-xs".into(), tokens.spacing.xs.clone());
    vars.insert("spacing-sm".into(), tokens.spacing.sm.clone());
    vars.insert("spacing-md".into(), tokens.spacing.md.clone());
    vars.insert("spacing-lg".into(), tokens.spacing.lg.clone());
    vars.insert("spacing-xl".into(), tokens.spacing.xl.clone());

    // Typography
    vars.insert("font-sans".into(), tokens.typography.font_sans.clone());
    vars.insert("font-mono".into(), tokens.typography.font_mono.clone());
    vars.insert("font-size-base".into(), tokens.typography.size_base.clone());
    vars.insert("line-height".into(), tokens.typography.line_height.clone());
    vars.insert("font-weight-normal".into(), tokens.typography.weight_normal.clone());
    vars.insert("font-weight-medium".into(), tokens.typography.weight_medium.clone());
    vars.insert("font-weight-bold".into(), tokens.typography.weight_bold.clone());

    // Motion
    vars.insert("duration-fast".into(), tokens.motion.duration_fast.clone());
    vars.insert("duration-normal".into(), tokens.motion.duration_normal.clone());
    vars.insert("duration-slow".into(), tokens.motion.duration_slow.clone());
    vars.insert("easing-default".into(), tokens.motion.easing_default.clone());
    vars.insert("easing-spring".into(), tokens.motion.easing_spring.clone());

    // Depth
    vars.insert("shadow-sm".into(), tokens.depth.shadow_sm.clone());
    vars.insert("shadow-md".into(), tokens.depth.shadow_md.clone());
    vars.insert("shadow-lg".into(), tokens.depth.shadow_lg.clone());

    let font_scale = overrides.font_scale.unwrap_or(1.0);
    let variant = match tokens.meta.variant {
        ThemeVariant::Dark => "dark",
        ThemeVariant::Light => "light",
    };

    CssVariables {
        variables: vars,
        font_scale,
        variant: variant.into(),
    }
}

/// Generate an injectable CSS string from the variable set.
///
/// Output format:
/// ```css
/// :root {
///   --color-bg-shell: #0a0a0a;
///   ...
///   font-size: 14px; /* only if font_scale != 1.0 */
/// }
/// html { color-scheme: dark; }
/// ```
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::loader::ThemeLoader;
    use crate::theme::schema::UserOverrides;

    #[test]
    fn generates_all_expected_keys() {
        let loader = ThemeLoader::new().unwrap();
        let tokens = loader.load("dark").unwrap();
        let css = to_css_variables(&tokens, &UserOverrides::default());

        let expected = [
            "color-bg-shell", "color-bg-app", "color-bg-card", "color-bg-overlay", "color-bg-input",
            "color-fg-primary", "color-fg-secondary", "color-fg-disabled", "color-fg-inverse",
            "color-accent", "color-accent-hover", "color-accent-pressed",
            "color-success", "color-warning", "color-error", "color-info",
            "color-border-default", "color-border-strong",
            "radius-sm", "radius-md", "radius-lg", "radius-full",
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
    fn css_string_contains_root_and_color_scheme() {
        let loader = ThemeLoader::new().unwrap();
        let tokens = loader.load("dark").unwrap();
        let css = to_css_variables(&tokens, &UserOverrides::default());
        let output = to_css_string(&css);

        assert!(output.contains(":root {"));
        assert!(output.contains("color-scheme: dark"));
        assert!(output.contains("--color-bg-shell: #0a0a0a;"));
        assert!(!output.contains("font-size:")); // scale is 1.0, no override
    }

    #[test]
    fn font_scale_injected() {
        let loader = ThemeLoader::new().unwrap();
        let tokens = loader.load("light").unwrap();
        let overrides = UserOverrides {
            accent: None,
            font_scale: Some(1.25),
        };
        let css = to_css_variables(&tokens, &overrides);
        let output = to_css_string(&css);

        assert!(output.contains("font-size: 20px;"));
        assert_eq!(css.font_scale, 1.25);
        assert_eq!(css.variant, "light");
    }

    #[test]
    fn variables_sorted_alphabetically() {
        let loader = ThemeLoader::new().unwrap();
        let tokens = loader.load("dark").unwrap();
        let css = to_css_variables(&tokens, &UserOverrides::default());

        let keys: Vec<&String> = css.variables.keys().collect();
        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(keys, sorted);
    }
}

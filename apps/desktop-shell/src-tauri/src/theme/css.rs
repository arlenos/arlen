//! CSS variable generation for the shell.
//!
//! The generator itself is the shared `arlen_theme::css` module so the
//! shell and every other Arlen app's theme consumer emit the identical
//! variable set (GAP-20). This file is the thin shell adapter: it re-exports
//! the shared types and maps the shell's [`UserOverrides`] onto the shared
//! generator's scalar `font_scale` parameter.

pub use arlen_theme::css::{to_css_string, CssVariables};

use arlen_theme::ArlenTheme;

use super::schema::UserOverrides;

/// Convert a resolved theme + the shell's user overrides to a flat CSS-var
/// map, delegating to the shared `arlen_theme::css` generator.
pub fn to_css_variables(theme: &ArlenTheme, overrides: &UserOverrides) -> CssVariables {
    arlen_theme::css::to_css_variables(theme, overrides.font_scale)
}

// ---------------------------------------------------------------------------
// Tests (shell integration: the real ThemeLoader through the shared generator)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::loader::ThemeLoader;

    #[test]
    fn loader_theme_generates_expected_keys() {
        let loader = ThemeLoader::new().unwrap();
        let theme = loader.load("dark").unwrap();
        let css = to_css_variables(&theme, &UserOverrides::default());
        for key in ["color-bg-shell", "color-accent", "radius-window", "font-sans"] {
            assert!(css.variables.contains_key(key), "missing key: {key}");
        }
        assert_eq!(css.variant, "dark");
    }

    #[test]
    fn font_scale_override_flows_through() {
        let loader = ThemeLoader::new().unwrap();
        let theme = loader.load("light").unwrap();
        let overrides = UserOverrides {
            accent: None,
            font_scale: Some(1.25),
            radius_intensity: None,
        };
        let css = to_css_variables(&theme, &overrides);
        assert_eq!(css.font_scale, 1.25);
        assert!(to_css_string(&css).contains("font-size: 20px;"));
    }
}

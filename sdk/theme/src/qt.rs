//! Qt outbound generator (theming-system-plan.md Strand 2).
//!
//! Emits the qt6ct/qt5ct `[ColorScheme]` colour `.conf` from the resolved
//! semantic tokens: high colour fidelity, zero structure (widgets stay
//! Fusion-shaped; matching shape is the gated Kvantum path). The format is the
//! one qt6ct ships and reads (verified against `/usr/share/qt6ct/colors/*.conf`):
//! a `[ColorScheme]` section with `active_colors`, `disabled_colors` and
//! `inactive_colors`, each a comma-separated list of 21 `#AARRGGBB` colours
//! (alpha FIRST, unlike CSS) in `QPalette::ColorRole` order:
//!
//! `WindowText, Button, Light, Midlight, Dark, Mid, Text, BrightText,
//! ButtonText, Base, Window, Shadow, Highlight, HighlightedText, Link,
//! LinkVisited, AlternateBase, NoRole, ToolTipBase, ToolTipText,
//! PlaceholderText`
//!
//! Like the GTK spoke this consumes the **resolved** [`ArlenTheme`] and is safe
//! by construction: every emitted value is a serialized colour, so no theme
//! free-string can reach the file.
//!
//! The Fusion bevel shades (`Light`/`Midlight`/`Dark`/`Mid`) are NOT derived by
//! multiplying the button colour, which is what Qt does for a 3D style and what
//! this did until 7 September. The house is flat: a button is a surface with a
//! hairline, so the highlight is the default border, the shadow is the deepest
//! surface, and the two middle shades are the card and the border. Looked at
//! beside an Arlen window with `dev/screenshot/shoot-toolkits.sh`.

use crate::{ArlenTheme, Rgba};

/// Serialize a resolved [`Rgba`] to Qt's `#AARRGGBB` form (alpha first).
pub fn rgba_to_qt_hex(c: Rgba) -> String {
    let to_u8 = |f: f32| (f.clamp(0.0, 1.0) * 255.0).round() as u8;
    let [r, g, b, a] = c;
    format!("#{:02x}{:02x}{:02x}{:02x}", to_u8(a), to_u8(r), to_u8(g), to_u8(b))
}

/// Relative luminance proxy, only used to pick the brighter of two colours.
fn lum(c: Rgba) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

/// The 21 active-state palette colours in `QPalette::ColorRole` order.
fn active_roles(t: &ArlenTheme) -> [Rgba; 21] {
    let c = &t.color;
    let button = c.bg_card;
    // BrightText must stay readable on dark fills whatever the variant; pick the
    // brighter of the two foregrounds.
    let bright_text = if lum(c.fg_primary) >= lum(c.fg_inverse) {
        c.fg_primary
    } else {
        c.fg_inverse
    };
    [
        c.fg_primary,             // 0  WindowText
        button,                   // 1  Button
        c.border_default,         // 2  Light: the hairline, not a bevel
        c.bg_card,                // 3  Midlight
        c.bg_shell,               // 4  Dark: the deepest surface
        c.border_default,         // 5  Mid
        c.fg_primary,             // 6  Text
        bright_text,              // 7  BrightText
        c.fg_primary,             // 8  ButtonText
        c.bg_input,               // 9  Base
        c.bg_app,                 // 10 Window
        [0.0, 0.0, 0.0, 1.0],     // 11 Shadow
        c.accent,                 // 12 Highlight
        c.fg_inverse,             // 13 HighlightedText
        c.info,                   // 14 Link
        c.accent_pressed,         // 15 LinkVisited
        c.bg_card,                // 16 AlternateBase
        c.fg_primary,             // 17 NoRole (unused by Qt; mirrors WindowText)
        c.bg_card,                // 18 ToolTipBase
        c.fg_primary,             // 19 ToolTipText
        c.fg_disabled,            // 20 PlaceholderText
    ]
}

/// The disabled-state palette: the active roles with every text role and the
/// selection muted to the disabled foreground, the convention the shipped qt6ct
/// schemes follow.
fn disabled_roles(t: &ArlenTheme) -> [Rgba; 21] {
    let c = &t.color;
    let mut roles = active_roles(t);
    for idx in [0usize, 6, 8, 13, 17, 19] {
        roles[idx] = c.fg_disabled;
    }
    roles[12] = c.border_strong; // Highlight: a flat neutral when disabled
    roles
}

/// Render one role list as the comma-separated `#AARRGGBB` line value.
fn render_roles(roles: &[Rgba; 21]) -> String {
    roles
        .iter()
        .map(|c| rgba_to_qt_hex(*c))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Generate the qt6ct/qt5ct `[ColorScheme]` `.conf` content from a resolved
/// theme. The inactive state mirrors the active one (the convention the shipped
/// schemes follow; Arlen does not dim unfocused windows).
pub fn generate_qt_conf(theme: &ArlenTheme) -> String {
    let active = render_roles(&active_roles(theme));
    let disabled = render_roles(&disabled_roles(theme));
    format!(
        "[ColorScheme]\nactive_colors={active}\ndisabled_colors={disabled}\ninactive_colors={active}\n"
    )
}

/// The qt6ct/qt5ct configuration that SELECTS the colour scheme beside it.
///
/// The scheme file has been written since this generator existed and nothing
/// pointed at it, which is the same shape the GTK3 theme was in until 7
/// September: a file on disk that no program is told to read. qt6ct chooses the
/// palette from `[Appearance] color_scheme_path` and only honours it when
/// `custom_palette` is true, so both have to be here or the scheme is decoration.
///
/// `style=Fusion` is the plan's target and not a preference: Fusion is the one
/// built-in QStyle that takes a custom palette faithfully, which is what makes
/// the colours land. Shape stays Fusion's - that is the documented ceiling of
/// this path, and Kvantum is the other one.
///
/// **No `[Fonts]`.** qt6ct writes a font size in POINTS and the theme carries
/// pixels; converting needs a dpi this crate has no business inventing, and a
/// wrong number here would resize every Qt app. Absent means Qt keeps its own,
/// which is the honest answer rather than a guessed one.
///
/// `icon_theme` is threaded through rather than read from the theme for the same
/// reason the GTK settings file does it: naming a set that is not installed
/// points an app at nothing.
pub fn generate_qt_select_conf(scheme_path: &str, icon_theme: Option<&str>) -> String {
    let mut out = String::from(
        "# arlen-generated (managed by Arlen; edits are overwritten on a theme change)\n[Appearance]\n",
    );
    out.push_str("style=Fusion\n");
    out.push_str("custom_palette=true\n");
    out.push_str(&format!("color_scheme_path={scheme_path}\n"));
    if let Some(icons) = icon_theme {
        out.push_str(&format!("icon_theme={icons}\n"));
    }
    out.push_str("standard_dialogs=default\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ArlenTheme;

    const SAMPLE: &str = include_str!("../test-fixtures/sample.toml");

    #[test]
    fn qt_hex_is_alpha_first() {
        assert_eq!(rgba_to_qt_hex([1.0, 0.0, 0.0, 1.0]), "#ffff0000");
        assert_eq!(rgba_to_qt_hex([0.0, 0.0, 0.0, 0.5019608]), "#80000000");
    }

    #[test]
    fn generates_three_lines_of_21_roles_each() {
        let t = ArlenTheme::from_bundled(SAMPLE).expect("resolve");
        let conf = generate_qt_conf(&t);
        assert!(conf.starts_with("[ColorScheme]\n"));
        for key in ["active_colors=", "disabled_colors=", "inactive_colors="] {
            let line = conf
                .lines()
                .find(|l| l.starts_with(key))
                .unwrap_or_else(|| panic!("missing {key}"));
            let colors: Vec<&str> = line[key.len()..].split(", ").collect();
            assert_eq!(colors.len(), 21, "{key} must carry 21 roles");
            for c in colors {
                assert_eq!(c.len(), 9, "each colour is #AARRGGBB, got {c:?}");
                assert!(c.starts_with('#'));
                assert!(c[1..].chars().all(|ch| ch.is_ascii_hexdigit()));
            }
        }
    }

    #[test]
    fn highlight_is_the_exact_accent_and_window_the_app_bg() {
        let t = ArlenTheme::from_bundled(SAMPLE).expect("resolve");
        let conf = generate_qt_conf(&t);
        let active = conf
            .lines()
            .find(|l| l.starts_with("active_colors="))
            .unwrap();
        let colors: Vec<&str> = active["active_colors=".len()..].split(", ").collect();
        assert_eq!(colors[12], rgba_to_qt_hex(t.color.accent), "Highlight = accent");
        assert_eq!(colors[10], rgba_to_qt_hex(t.color.bg_app), "Window = bg_app");
        assert_eq!(colors[0], rgba_to_qt_hex(t.color.fg_primary), "WindowText = fg_primary");
    }

    #[test]
    fn generated_qt_conf_is_inert_for_an_adversarial_theme() {
        // The TH-0 theorem through the Qt generator: only serialized colours are
        // emitted, so an adversarial free-string can never reach the conf.
        let payload = "x [General] y=z"; // would inject an INI section if it leaked
        let user = format!(
            "[typography]\nfont_sans='{payload}'\n[cursor]\ntheme='{payload}'\n"
        );
        let t = ArlenTheme::resolve(SAMPLE, Some(&user), None).expect("resolve");
        let conf = generate_qt_conf(&t);
        assert!(!conf.contains(payload), "adversarial free-string leaked into the Qt conf");
        // Exactly one section header, and every other line is a known key.
        assert_eq!(conf.matches('[').count(), 1, "exactly one INI section");
        for line in conf.lines().skip(1).filter(|l| !l.is_empty()) {
            assert!(
                line.starts_with("active_colors=")
                    || line.starts_with("disabled_colors=")
                    || line.starts_with("inactive_colors="),
                "unexpected Qt conf line: {line:?}"
            );
        }
    }

    #[test]
    fn the_select_conf_points_at_the_scheme_and_turns_it_on() {
        let conf = generate_qt_select_conf("/home/x/.config/qt6ct/colors/arlen.conf", Some("Adwaita"));
        assert!(conf.starts_with("# arlen-generated"), "the guard marker must lead");
        assert!(conf.contains("\n[Appearance]\n"));
        assert!(conf.contains("style=Fusion\n"));
        // Without this the path is read and ignored, which is the failure that
        // looks exactly like a wrong palette.
        assert!(conf.contains("custom_palette=true\n"));
        assert!(conf.contains("color_scheme_path=/home/x/.config/qt6ct/colors/arlen.conf\n"));
        assert!(conf.contains("icon_theme=Adwaita\n"));
        // A font size here would be points against our pixels.
        assert!(!conf.contains("[Fonts]"), "{conf}");
    }

    #[test]
    fn an_absent_icon_set_is_not_named() {
        let conf = generate_qt_select_conf("/x/arlen.conf", None);
        assert!(!conf.contains("icon_theme"), "{conf}");
        assert!(conf.contains("color_scheme_path=/x/arlen.conf\n"));
    }
}

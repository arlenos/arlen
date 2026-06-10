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
//! free-string can reach the file. The Fusion bevel shades (`Light`/`Midlight`/
//! `Dark`/`Mid`) are derived from the button colour the way Qt itself derives
//! them (`lighter(150)`/`lighter(125)`/`darker(200)`/`darker(150)`), so bevels
//! track the theme without four extra schema slots.

use crate::{ArlenTheme, Rgba};

/// Serialize a resolved [`Rgba`] to Qt's `#AARRGGBB` form (alpha first).
pub fn rgba_to_qt_hex(c: Rgba) -> String {
    let to_u8 = |f: f32| (f.clamp(0.0, 1.0) * 255.0).round() as u8;
    let [r, g, b, a] = c;
    format!("#{:02x}{:02x}{:02x}{:02x}", to_u8(a), to_u8(r), to_u8(g), to_u8(b))
}

/// Scale a colour's RGB by `factor` (alpha kept), clamped — the same shape as
/// `QColor::lighter`/`darker` with `factor = percent / 100`.
fn scale_rgb(c: Rgba, factor: f32) -> Rgba {
    [
        (c[0] * factor).clamp(0.0, 1.0),
        (c[1] * factor).clamp(0.0, 1.0),
        (c[2] * factor).clamp(0.0, 1.0),
        c[3],
    ]
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
        scale_rgb(button, 1.5),   // 2  Light
        scale_rgb(button, 1.25),  // 3  Midlight
        scale_rgb(button, 0.5),   // 4  Dark
        scale_rgb(button, 0.66),  // 5  Mid
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
}

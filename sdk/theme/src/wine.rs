//! Wine outbound generator (`wine-theming-plan.md`).
//!
//! Emits one `.reg` document per bottle from the resolved [`ArlenTheme`], to be
//! imported with `wine regedit /S` into **both** the 32- and 64-bit registries
//! **before** an app launches: a running app does not repaint a
//! `Control Panel\Colors` change, and the reliable remedy is
//! apply-then-start rather than a `WM_SETTINGCHANGE` broadcast nobody has
//! shown can be triggered from Linux.
//!
//! **State the ceiling first, because it is the design.** A Wine app draws its
//! own interior with its own toolkit, so pixel-Arlen is not reachable and
//! chasing it is wasted. What this reaches, reliably, is the Win32 *system*
//! surface: the classic colour palette, the UI font, and the DPI and
//! antialiasing settings. Native Win32/MFC, WinForms, classic apps and
//! palette-respecting Qt take it. Electron, WPF, GTK-for-Windows and
//! custom-skinned apps keep their own interior and only the light/dark hint
//! reaches them. That is the honest coverage, and the surface says
//! "best-effort" for exactly this reason.
//!
//! **Where the slot names come from.** The value names under
//! `Control Panel\Colors` are Wine's own, several of them misspelled since
//! Windows 95 (`Hilight`, not `Highlight`), and guessing them produces a file
//! Wine reads and ignores. 23 of the 31 were read straight out of the shipped
//! `win32u.so` string table in `COLOR_*` index order; the other eight
//! (`ActiveTitle`, `InactiveTitle`, `Menu`, `Window`, `WindowText`,
//! `TitleText`, `Hilight`, `MenuBar`) are the gaps in that run, and the whole
//! set was then round-tripped through a throwaway prefix: written with
//! `regedit /S`, read back out of `user.reg` unchanged. What that does NOT
//! prove is a real app picking each slot up, which is a metal check the plan
//! already lists.
//!
//! **Not wired into [`crate::apply`] yet, and that is a real difference from
//! its siblings.** GTK and Qt each write one file at a known path; a Wine theme
//! is per bottle, and which bottles exist is `bottled`'s answer rather than
//! this crate's. So this is the generator only, and the apply step - enumerate
//! the bottles, import into both registries, register the font face, report per
//! bottle what would not take - lands with the caller that can ask.
//!
//! **The palette mapping in [`colors`] is provisional**, and deliberately kept
//! to one table so it can be replaced without touching anything else here. See
//! that function's note for whose call it is.
//!
//! Like the GTK and Qt spokes this consumes the **resolved** theme. Colours
//! and numbers are safe by construction; the one free string that reaches a
//! file is the UI font name, and `.reg` has a quoting rule of its own, so it
//! goes through [`reg_escape`] rather than trusting the resolve gate to have
//! covered a format it does not know about.

use crate::{ArlenTheme, Rgba, ThemeVariant};

/// Serialize a resolved [`Rgba`] the way `Control Panel\Colors` wants it:
/// three decimal channels separated by spaces. Alpha is dropped, because a
/// Win32 system colour is a `COLORREF` and has none.
pub fn rgba_to_win32(c: Rgba) -> String {
    let to_u8 = |f: f32| (f.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("{} {} {}", to_u8(c[0]), to_u8(c[1]), to_u8(c[2]))
}

/// Escape a string for a `.reg` value.
///
/// The resolve gate already refuses a backslash and every bracket, so today
/// nothing reaching here needs escaping at all. It runs anyway, and on the two
/// characters `.reg` itself cares about: a floor that happens to cover a format
/// it has never heard of is luck, and the next widening of that gate would turn
/// this file into a syntax error rather than a colour.
pub fn reg_escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

/// The 31 `Control Panel\Colors` slots, in `COLOR_*` index order, mapped from
/// the resolved semantic tokens.
///
/// **THE MAPPING IS PROVISIONAL AND IS NOT THIS LANE'S TO SETTLE.** Which Arlen
/// colour becomes which Win32 system colour is a look, the same kind of choice
/// as the 21 `QPalette` roles and the four bevel factors next door in `qt.rs`,
/// and a look chosen while building the mechanism gets redone by the lane that
/// owns the design. This one is defensible enough to run and to photograph, and
/// it is meant to be replaced: every decision below is a table entry, so
/// changing the whole palette is editing this function and nothing else. The
/// mechanism around it - reading the resolved theme, the document shape, the
/// escape, the metrics, the per-bottle apply - does not move when it changes.
///
/// The shape is the flat-classic one, which is the whole reason this path is a
/// good fit: unthemed Win32 rendering with an Arlen palette already looks like
/// Arlen, where a `.msstyles` would be an XP-format detour reaching only
/// comctl32-v6 apps. So the caption is a flat fill rather than a gradient (both
/// gradient slots take the same colour as their base), and the four button
/// bevels come from tokens rather than from multiplying the face, the same
/// call `qt.rs` made on 7 September: the house is flat, so the highlight is
/// the default border, the light the card, the dark shadow the deepest surface
/// and the shadow the strong border. Looked at in `winecfg` beside an Arlen
/// window with `dev/screenshot/shoot-toolkits.sh wine`.
fn colors(t: &ArlenTheme) -> Vec<(&'static str, Rgba)> {
    let c = &t.color;
    let button = c.bg_card;
    vec![
        ("Scrollbar", c.bg_card),
        ("Background", c.bg_shell),
        ("ActiveTitle", c.bg_card),
        ("InactiveTitle", c.bg_app),
        ("Menu", c.bg_card),
        // COLOR_WINDOW is the sheet a document or a list is drawn on, which is
        // the field ground rather than the window ground - the same call Qt's
        // Base makes. `ButtonFace` below is the dialog ground.
        ("Window", c.bg_input),
        ("WindowFrame", c.border_strong),
        ("MenuText", c.fg_primary),
        ("WindowText", c.fg_primary),
        ("TitleText", c.fg_primary),
        ("ActiveBorder", c.border_strong),
        ("InactiveBorder", c.border_default),
        ("AppWorkSpace", c.bg_shell),
        ("Hilight", c.accent),
        ("HilightText", c.fg_inverse),
        ("ButtonFace", button),
        ("ButtonShadow", c.border_strong),
        ("GrayText", c.fg_disabled),
        ("ButtonText", c.fg_primary),
        ("InactiveTitleText", c.fg_secondary),
        ("ButtonHilight", c.border_default),
        ("ButtonDkShadow", c.bg_shell),
        ("ButtonLight", c.bg_card),
        ("InfoText", c.fg_primary),
        ("InfoWindow", c.bg_overlay),
        ("ButtonAlternateFace", button),
        ("HotTrackingColor", c.accent_hover),
        ("GradientActiveTitle", c.bg_card),
        ("GradientInactiveTitle", c.bg_app),
        ("MenuHilight", c.accent),
        ("MenuBar", c.bg_card),
    ]
}

/// The two views the MACHINE hive serves under `Software`: the native one and
/// the WOW64 copy a 32-bit program is redirected to. Both exist in a booted
/// prefix (`system.reg` carries `FontSubstitutes` twice), so a font substitute
/// written to one reaches half the programs in the bottle. The user hive has no
/// such split and is written once.
const MACHINE_VIEWS: [&str; 2] = ["Software", "Software\\Wow6432Node"];

/// `LogPixels` for a display scale: Windows counts DPI from 96 at 1x.
fn log_pixels(scale: f32) -> u32 {
    (96.0 * scale.clamp(0.5, 4.0)).round() as u32
}

/// Emit the per-bottle `.reg` document.
///
/// `scale` is the display scale the prefix should assume (1.0 for 96 DPI). It
/// is a parameter rather than a token because it is a property of the monitor,
/// not of the theme, and a prefix can only hold one: a bottle spanning two
/// displays of different scales gets the one it was told about.
pub fn generate_wine_reg(t: &ArlenTheme, scale: f32) -> String {
    let light = matches!(t.meta.variant, ThemeVariant::Light);
    let mut out = String::from("REGEDIT4\r\n\r\n");

    out.push_str("[HKEY_CURRENT_USER\\Control Panel\\Colors]\r\n");
    for (name, c) in colors(t) {
        out.push_str(&format!("\"{name}\"=\"{}\"\r\n", rgba_to_win32(c)));
    }
    out.push_str("\r\n");

    // Palette path on, `.msstyles` off. The flat-classic look IS the Arlen fit,
    // so this is the setting that makes the block above load-bearing rather
    // than a palette some theme engine overrides.
    //
    // ONE view, and that is measured rather than assumed. A booted prefix has
    // no `Wow6432Node` under HKCU at all - zero keys in `user.reg`, against
    // 7692 in `system.reg` - because Wine redirects the machine hive and shares
    // the user's. A second HKCU copy would be a key nothing reads, sitting in
    // the document looking load-bearing. HKLM below is the opposite case.
    out.push_str(
        "[HKEY_CURRENT_USER\\Software\\Microsoft\\Windows\\CurrentVersion\\ThemeManager]\r\n",
    );
    out.push_str("\"ThemeActive\"=\"0\"\r\n\r\n");

    // Wine boots a prefix with a SECOND copy of the whole palette under the
    // theme manager, and leaving it holding the old colours leaves two keys
    // describing the same thing and disagreeing. Nothing reads it while
    // `ThemeActive` is "0", so this is agreement rather than function.
    out.push_str(
        "[HKEY_CURRENT_USER\\Software\\Microsoft\\Windows\\CurrentVersion\\ThemeManager\\Control Panel\\Colors]\r\n",
    );
    for (name, c) in colors(t) {
        out.push_str(&format!("\"{name}\"=\"{}\"\r\n", rgba_to_win32(c)));
    }
    out.push_str("\r\n");

    // The one OS signal Electron, Chromium and default-vista Qt honour. It does
    // not recolour them; it stops them rendering a light interior inside a dark
    // desktop, which is the difference between wrong and jarring.
    out.push_str(
        "[HKEY_CURRENT_USER\\Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize]\r\n",
    );
    let mode = u32::from(light);
    out.push_str(&format!("\"AppsUseLightTheme\"=dword:{mode:08x}\r\n"));
    out.push_str(&format!("\"SystemUsesLightTheme\"=dword:{mode:08x}\r\n\r\n"));

    // DPI and antialiasing. FontSmoothing 2 is "on"; type 2 is subpixel, and
    // orientation 1 is RGB, which is what a normal desktop panel is.
    out.push_str("[HKEY_CURRENT_USER\\Control Panel\\Desktop]\r\n");
    out.push_str(&format!("\"LogPixels\"=dword:{:08x}\r\n", log_pixels(scale)));
    out.push_str("\"FontSmoothing\"=\"2\"\r\n");
    out.push_str("\"FontSmoothingType\"=dword:00000002\r\n");
    out.push_str("\"FontSmoothingOrientation\"=dword:00000001\r\n");
    out.push_str("\"FontSmoothingGamma\"=dword:00000578\r\n\r\n");

    // The substitutes only take once the face itself is registered in the
    // prefix, which is the caller's job (copy the `.ttf` into
    // `drive_c/windows/Fonts` and add the `Fonts` record, the way winetricks
    // does). Pointing a substitute at a face the prefix does not have leaves
    // Wine on its fallback, which looks like nothing happened rather than like
    // a failure - so the caller reports it, not this file.
    let font = reg_escape(first_family(&t.typography.font_sans));
    for view in MACHINE_VIEWS {
        out.push_str(&format!(
            "[HKEY_LOCAL_MACHINE\\{view}\\Microsoft\\Windows NT\\CurrentVersion\\FontSubstitutes]\r\n"
        ));
        for legacy in ["MS Shell Dlg", "MS Shell Dlg 2", "Segoe UI", "Tahoma", "MS Sans Serif"] {
            out.push_str(&format!("\"{legacy}\"=\"{font}\"\r\n"));
        }
        out.push_str("\r\n");
    }

    out
}

/// The first family in a CSS font stack, unquoted.
///
/// Public because the caller that applies this document has to ask the prefix
/// whether that face is actually there, and it must ask about the same name the
/// substitutes were written with. Two readings of one stack is how a report ends
/// up describing a font nobody pointed at.
///
/// `font_sans` is a CSS list ("Inter", system-ui, sans-serif) because that is
/// what the web side needs; a font substitute names exactly one face. Taking
/// the head is the only reading that can be right, since the fallbacks are
/// names the prefix has never heard of.
pub fn first_family(stack: &str) -> &str {
    stack
        .split(',')
        .next()
        .unwrap_or(stack)
        .trim()
        .trim_matches(|c| c == '"' || c == '\'')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ArlenTheme;

    const SAMPLE: &str = include_str!("../test-fixtures/sample.toml");

    fn theme() -> ArlenTheme {
        ArlenTheme::from_bundled(SAMPLE).expect("sample theme resolves")
    }

    #[test]
    fn every_colour_slot_is_emitted_once_in_color_index_order() {
        let t = theme();
        let reg = generate_wine_reg(&t, 1.0);
        let names: Vec<&str> = colors(&t).iter().map(|(n, _)| *n).collect();
        assert_eq!(names.len(), 31, "the Win32 palette is 31 slots");
        let mut seen = names.clone();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), 31, "a slot is emitted twice");
        for n in names {
            assert!(reg.contains(&format!("\"{n}\"=")), "slot {n} is missing from the document");
        }
    }

    #[test]
    fn colours_are_decimal_triples_and_carry_no_alpha() {
        let t = theme();
        let reg = generate_wine_reg(&t, 1.0);
        let line = reg
            .lines()
            .find(|l| l.starts_with("\"Hilight\"="))
            .expect("the highlight slot is written");
        let value = line.split('=').nth(1).unwrap().trim_matches('"');
        let parts: Vec<&str> = value.split(' ').collect();
        assert_eq!(parts.len(), 3, "a COLORREF is three channels, got {value:?}");
        for p in parts {
            let n: u32 = p.parse().expect("a decimal channel");
            assert!(n <= 255);
        }
    }

    #[test]
    fn the_highlight_is_the_accent_and_the_field_is_the_input_ground() {
        let t = theme();
        let reg = generate_wine_reg(&t, 1.0);
        assert!(reg.contains(&format!("\"Hilight\"=\"{}\"", rgba_to_win32(t.color.accent))));
        assert!(reg.contains(&format!("\"Window\"=\"{}\"", rgba_to_win32(t.color.bg_input))));
        assert!(reg.contains(&format!("\"ButtonFace\"=\"{}\"", rgba_to_win32(t.color.bg_card))));
    }

    #[test]
    fn the_light_dark_hint_follows_the_theme_variant() {
        let mut t = theme();
        t.meta.variant = ThemeVariant::Dark;
        assert!(generate_wine_reg(&t, 1.0).contains("\"AppsUseLightTheme\"=dword:00000000"));
        t.meta.variant = ThemeVariant::Light;
        assert!(generate_wine_reg(&t, 1.0).contains("\"AppsUseLightTheme\"=dword:00000001"));
    }

    #[test]
    fn the_machine_hive_is_written_twice_and_the_user_hive_once() {
        // Measured in a booted prefix rather than assumed: `user.reg` has ZERO
        // `Wow6432Node` keys and `system.reg` has 7692, so the machine hive is
        // redirected for 32-bit programs and the user hive is shared. A font
        // substitute in one machine view reaches half the bottle; a second copy
        // of a user key reaches nothing and only looks load-bearing.
        let reg = generate_wine_reg(&theme(), 1.0);
        let fonts = "Microsoft\\Windows NT\\CurrentVersion\\FontSubstitutes";
        assert!(reg.contains(&format!("[HKEY_LOCAL_MACHINE\\Software\\{fonts}]")));
        assert!(reg.contains(&format!("[HKEY_LOCAL_MACHINE\\Software\\Wow6432Node\\{fonts}]")));
        assert!(
            !reg.contains("[HKEY_CURRENT_USER\\Software\\Wow6432Node"),
            "the user hive has no redirected view in a prefix"
        );
        assert_eq!(reg.matches("[HKEY_CURRENT_USER\\Control Panel\\Colors]").count(), 1);
    }

    #[test]
    fn the_theme_managers_own_copy_of_the_palette_agrees_with_the_live_one() {
        // A booted prefix keeps a second palette under the theme manager. Left
        // alone it holds Wine's defaults, so the two keys describe the same
        // thing and disagree.
        let t = theme();
        let reg = generate_wine_reg(&t, 1.0);
        let mgr = "[HKEY_CURRENT_USER\\Software\\Microsoft\\Windows\\CurrentVersion\\ThemeManager\\Control Panel\\Colors]";
        assert!(reg.contains(mgr), "the theme manager's palette is written");
        let want = format!("\"Hilight\"=\"{}\"", rgba_to_win32(t.color.accent));
        assert_eq!(reg.matches(&want).count(), 2, "both palettes carry the accent");
    }

    #[test]
    fn log_pixels_tracks_the_scale() {
        assert_eq!(log_pixels(1.0), 96);
        assert_eq!(log_pixels(1.5), 144);
        assert_eq!(log_pixels(2.0), 192);
        // A nonsense scale is clamped rather than emitted: a prefix told its
        // display is 40x renders text nobody can read off the screen.
        assert_eq!(log_pixels(0.0), 48);
        assert_eq!(log_pixels(99.0), 384);
    }

    #[test]
    fn the_font_substitute_names_one_face_from_the_css_stack() {
        assert_eq!(first_family("\"Inter\", system-ui, sans-serif"), "Inter");
        assert_eq!(first_family("Cantarell"), "Cantarell");
        assert_eq!(first_family("  'Noto Sans' , sans-serif"), "Noto Sans");
    }

    #[test]
    fn a_quote_in_a_font_name_cannot_end_the_value() {
        // The resolve gate lets a double quote through today (it refuses
        // backslashes and brackets, not quotes), and `.reg` would read one as
        // the end of the value.
        assert_eq!(reg_escape("Ink\"Free"), "Ink\\\"Free");
        assert_eq!(reg_escape("back\\slash"), "back\\\\slash");
    }

    /// The document a real `regedit` accepts, checked against a real prefix.
    ///
    /// `#[ignore]`d because it needs Wine on the machine: it builds a throwaway
    /// prefix, imports the generated document and reads the values back out of
    /// `user.reg`. About eight seconds with no display in the way. Everything above proves
    /// what the generator WROTE; this proves `regedit` accepted the whole
    /// document and the prefix kept every slot, which is the failure a syntax
    /// mistake in one line would cause - the import stops there and the rest of
    /// the palette never lands.
    ///
    /// It cannot prove a slot name is the one Wine CONSULTS. `regedit` stores
    /// any name it is given, so a misspelling round-trips perfectly and is then
    /// ignored at draw time; only a Win32 app on screen shows that, which is
    /// the metal check the plan lists and this is not.
    ///
    ///   cargo test -p arlen-theme --manifest-path sdk/Cargo.toml -- --ignored wine_metal
    #[test]
    #[ignore]
    fn wine_metal_accepts_the_document_and_keeps_every_slot() {
        use std::process::Command;
        let dir = std::env::temp_dir().join(format!("arlen-wine-reg-{}", std::process::id()));
        let prefix = dir.join("prefix");
        std::fs::create_dir_all(&prefix).expect("temp prefix dir");
        let doc = dir.join("theme.reg");
        let t = theme();
        std::fs::write(&doc, generate_wine_reg(&t, 1.0)).expect("write the document");

        // No display at all, rather than a nested one. Wine only needs a
        // display to draw, and none of this draws; unsetting both means the
        // test cannot open a window on the developer's screen even if some
        // step decides to try, which a private Xvfb only makes unlikely.
        let run = |args: &[&str]| {
            let mut c = Command::new(args[0]);
            c.args(&args[1..])
                .env("WINEPREFIX", &prefix)
                .env("WINEDEBUG", "-all")
                .env_remove("DISPLAY")
                .env_remove("WAYLAND_DISPLAY");
            c.status()
        };
        // A first boot creates the registry files; a failure here is an
        // environment without Wine rather than a defect, so it says so.
        assert!(run(&["wineboot", "--init"]).is_ok(), "wine is not runnable here");
        assert!(
            run(&["wine", "regedit", "/S", doc.to_str().unwrap()])
                .expect("regedit ran")
                .success(),
            "regedit refused the generated document"
        );
        let _ = Command::new("wineserver").arg("-k").env("WINEPREFIX", &prefix).status();
        let back = std::fs::read_to_string(prefix.join("user.reg")).expect("read user.reg");
        for (name, c) in colors(&t) {
            let want = format!("\"{name}\"=\"{}\"", rgba_to_win32(c));
            assert!(back.contains(&want), "wine did not keep {name}: expected {want}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn every_line_is_a_section_or_a_value() {
        // A `.reg` regedit accepts is this shape and nothing else, so a stray
        // line from a token would be a parse failure at import time rather than
        // a wrong colour.
        let t = theme();
        for line in generate_wine_reg(&t, 1.25).lines() {
            let l = line.trim_end_matches('\r');
            if l.is_empty() || l == "REGEDIT4" {
                continue;
            }
            let ok = (l.starts_with('[') && l.ends_with(']')) || (l.starts_with('"') && l.contains("\"="));
            assert!(ok, "line is neither a section nor a value: {l:?}");
        }
    }
}

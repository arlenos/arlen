//! Catppuccin inbound adapter (theming-system-plan.md Strand 4) — the quality
//! ceiling of the base16 reference: a clean 1:1 mapping plus the accent picker
//! the spec demands (Catppuccin's fourteen accents are a user choice by
//! upstream design, with **mauve** as Arlen's default).
//!
//! The four flavor palettes are embedded verbatim from the canonical
//! `catppuccin/palette` `palette.json`, cross-checked against
//! `catppuccin.com/palette` (no disagreements). The `[terminal.ansi]` block
//! follows what the OFFICIAL terminal ports actually ship (verified identical
//! across the kitty, foot and alacritty ports): `black`←surface1,
//! `white`←subtext1, `bright_black`←surface2, `bright_white`←subtext0,
//! `magenta`←pink, `cyan`←teal, and the bright hue slots reuse the normal hex.
//! **Latte (the light flavor) inverts the four neutral slots** in those same
//! ports (`black`←subtext1, `bright_black`←subtext0, `white`←surface2,
//! `bright_white`←surface1); the palette.json `ansiColors` block disagrees
//! with the ports and is deliberately not used.
//!
//! Like every adapter, the tail is shared: Rule A derives the hover/pressed
//! accent siblings (Catppuccin authors no interaction states) and Rule B
//! clamps the WCAG floors, including `fg.inverse`-on-accent.

use crate::base16::{slugify, toml_escape};
use crate::color::{
    clamp_contrast, derive_hover_pressed, BODY_CONTRAST_FLOOR, STATUS_CONTRAST_FLOOR,
};
use crate::gtk::rgba_to_hex;
use crate::{parse_hex, Rgba};

/// The four Catppuccin flavors; `Latte` is the light one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flavor {
    Latte,
    Frappe,
    Macchiato,
    Mocha,
}

impl Flavor {
    /// The flavor's display name (the site spells Frappé with the accent; the
    /// upstream JSON key and our id use the plain form).
    pub fn name(self) -> &'static str {
        match self {
            Flavor::Latte => "Latte",
            Flavor::Frappe => "Frappe",
            Flavor::Macchiato => "Macchiato",
            Flavor::Mocha => "Mocha",
        }
    }

    fn palette(self) -> &'static Palette {
        match self {
            Flavor::Latte => &LATTE,
            Flavor::Frappe => &FRAPPE,
            Flavor::Macchiato => &MACCHIATO,
            Flavor::Mocha => &MOCHA,
        }
    }
}

/// The fourteen Catppuccin accents — a user picker by upstream spec, no
/// canonical single accent. Arlen defaults to mauve.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Accent {
    Rosewater,
    Flamingo,
    Pink,
    #[default]
    Mauve,
    Red,
    Maroon,
    Peach,
    Yellow,
    Green,
    Teal,
    Sky,
    Sapphire,
    Blue,
    Lavender,
}

/// One flavor's 26-colour palette (14 accents + 12 neutrals), hex verbatim
/// from the canonical palette.json.
struct Palette {
    rosewater: &'static str,
    flamingo: &'static str,
    pink: &'static str,
    mauve: &'static str,
    red: &'static str,
    maroon: &'static str,
    peach: &'static str,
    yellow: &'static str,
    green: &'static str,
    teal: &'static str,
    sky: &'static str,
    sapphire: &'static str,
    blue: &'static str,
    lavender: &'static str,
    text: &'static str,
    subtext1: &'static str,
    subtext0: &'static str,
    overlay1: &'static str,
    surface2: &'static str,
    surface1: &'static str,
    surface0: &'static str,
    base: &'static str,
    mantle: &'static str,
    crust: &'static str,
}

const LATTE: Palette = Palette {
    rosewater: "#dc8a78",
    flamingo: "#dd7878",
    pink: "#ea76cb",
    mauve: "#8839ef",
    red: "#d20f39",
    maroon: "#e64553",
    peach: "#fe640b",
    yellow: "#df8e1d",
    green: "#40a02b",
    teal: "#179299",
    sky: "#04a5e5",
    sapphire: "#209fb5",
    blue: "#1e66f5",
    lavender: "#7287fd",
    text: "#4c4f69",
    subtext1: "#5c5f77",
    subtext0: "#6c6f85",
    overlay1: "#8c8fa1",
    surface2: "#acb0be",
    surface1: "#bcc0cc",
    surface0: "#ccd0da",
    base: "#eff1f5",
    mantle: "#e6e9ef",
    crust: "#dce0e8",
};

const FRAPPE: Palette = Palette {
    rosewater: "#f2d5cf",
    flamingo: "#eebebe",
    pink: "#f4b8e4",
    mauve: "#ca9ee6",
    red: "#e78284",
    maroon: "#ea999c",
    peach: "#ef9f76",
    yellow: "#e5c890",
    green: "#a6d189",
    teal: "#81c8be",
    sky: "#99d1db",
    sapphire: "#85c1dc",
    blue: "#8caaee",
    lavender: "#babbf1",
    text: "#c6d0f5",
    subtext1: "#b5bfe2",
    subtext0: "#a5adce",
    overlay1: "#838ba7",
    surface2: "#626880",
    surface1: "#51576d",
    surface0: "#414559",
    base: "#303446",
    mantle: "#292c3c",
    crust: "#232634",
};

const MACCHIATO: Palette = Palette {
    rosewater: "#f4dbd6",
    flamingo: "#f0c6c6",
    pink: "#f5bde6",
    mauve: "#c6a0f6",
    red: "#ed8796",
    maroon: "#ee99a0",
    peach: "#f5a97f",
    yellow: "#eed49f",
    green: "#a6da95",
    teal: "#8bd5ca",
    sky: "#91d7e3",
    sapphire: "#7dc4e4",
    blue: "#8aadf4",
    lavender: "#b7bdf8",
    text: "#cad3f5",
    subtext1: "#b8c0e0",
    subtext0: "#a5adcb",
    overlay1: "#8087a2",
    surface2: "#5b6078",
    surface1: "#494d64",
    surface0: "#363a4f",
    base: "#24273a",
    mantle: "#1e2030",
    crust: "#181926",
};

const MOCHA: Palette = Palette {
    rosewater: "#f5e0dc",
    flamingo: "#f2cdcd",
    pink: "#f5c2e7",
    mauve: "#cba6f7",
    red: "#f38ba8",
    maroon: "#eba0ac",
    peach: "#fab387",
    yellow: "#f9e2af",
    green: "#a6e3a1",
    teal: "#94e2d5",
    sky: "#89dceb",
    sapphire: "#74c7ec",
    blue: "#89b4fa",
    lavender: "#b4befe",
    text: "#cdd6f4",
    subtext1: "#bac2de",
    subtext0: "#a6adc8",
    overlay1: "#7f849c",
    surface2: "#585b70",
    surface1: "#45475a",
    surface0: "#313244",
    base: "#1e1e2e",
    mantle: "#181825",
    crust: "#11111b",
};

fn accent_hex(p: &Palette, accent: Accent) -> &'static str {
    match accent {
        Accent::Rosewater => p.rosewater,
        Accent::Flamingo => p.flamingo,
        Accent::Pink => p.pink,
        Accent::Mauve => p.mauve,
        Accent::Red => p.red,
        Accent::Maroon => p.maroon,
        Accent::Peach => p.peach,
        Accent::Yellow => p.yellow,
        Accent::Green => p.green,
        Accent::Teal => p.teal,
        Accent::Sky => p.sky,
        Accent::Sapphire => p.sapphire,
        Accent::Blue => p.blue,
        Accent::Lavender => p.lavender,
    }
}

/// The embedded palette hex is compile-time-known and valid; parse it.
fn px(hex: &str) -> Rgba {
    parse_hex(hex).expect("embedded catppuccin hex is valid")
}

/// Adapt a Catppuccin flavor + accent choice into a complete Arlen theme TOML.
/// Slot assignment: `base`→bg.app, `mantle`→bg.shell, `surface0`→bg.card,
/// `surface1`→bg.input + border.default, `surface2`→border.strong,
/// `text`→fg.primary, `subtext1`→fg.secondary, `overlay1`→fg.disabled,
/// `crust`→fg.inverse (the extreme tone, dark-on-pastel for the dark flavors
/// and light-on-vivid for latte) — then the shared Rule A + Rule B tail.
pub fn adapt_catppuccin(flavor: Flavor, accent: Accent) -> String {
    let p = flavor.palette();
    let dark = flavor != Flavor::Latte;

    let bg_app = px(p.base);
    let bg_shell = px(p.mantle);
    let bg_card = px(p.surface0);
    let bg_input = px(p.surface1);
    let crust = px(p.crust);
    let bg_overlay = [crust[0], crust[1], crust[2], 0.5];

    let accent_rgba = px(accent_hex(p, accent));
    let fg_primary = clamp_contrast(px(p.text), bg_app, BODY_CONTRAST_FLOOR);
    let fg_secondary = clamp_contrast(px(p.subtext1), bg_app, BODY_CONTRAST_FLOOR);
    let fg_disabled = px(p.overlay1);
    let fg_inverse = clamp_contrast(crust, accent_rgba, BODY_CONTRAST_FLOOR);
    let success = clamp_contrast(px(p.green), bg_app, STATUS_CONTRAST_FLOOR);
    let warning = clamp_contrast(px(p.yellow), bg_app, STATUS_CONTRAST_FLOOR);
    let error = clamp_contrast(px(p.red), bg_app, STATUS_CONTRAST_FLOOR);
    let info = clamp_contrast(px(p.blue), bg_app, STATUS_CONTRAST_FLOOR);

    let (accent_hover, accent_pressed) = derive_hover_pressed(accent_rgba, dark);

    // The official ports' ANSI neutrals; latte inverts them.
    let (ansi_black, ansi_white, ansi_bright_black, ansi_bright_white) = if dark {
        (p.surface1, p.subtext1, p.surface2, p.subtext0)
    } else {
        (p.subtext1, p.surface2, p.subtext0, p.surface1)
    };

    let h = rgba_to_hex;
    let name = format!("Catppuccin {}", flavor.name());
    let id = slugify(&name);
    let name = toml_escape(&name);
    let variant = if dark { "dark" } else { "light" };

    format!(
        r##"[meta]
id = "{id}"
name = "{name}"
variant = "{variant}"

[color.bg]
shell   = "{shell}"
app     = "{app}"
card    = "{card}"
overlay = "{overlay}"
input   = "{input}"

[color.fg]
primary   = "{fg_primary}"
secondary = "{fg_secondary}"
disabled  = "{fg_disabled}"
inverse   = "{fg_inverse}"

[color.semantic]
accent         = "{accent}"
accent_hover   = "{hover}"
accent_pressed = "{pressed}"
success        = "{success}"
warning        = "{warning}"
error          = "{error}"
info           = "{info}"

[color.border]
default = "{border_default}"
strong  = "{border_strong}"

[terminal.ansi]
black          = "{a_black}"
red            = "{red}"
green          = "{green}"
yellow         = "{yellow}"
blue           = "{blue}"
magenta        = "{magenta}"
cyan           = "{cyan}"
white          = "{a_white}"
bright_black   = "{a_bblack}"
bright_red     = "{red}"
bright_green   = "{green}"
bright_yellow  = "{yellow}"
bright_blue    = "{blue}"
bright_magenta = "{magenta}"
bright_cyan    = "{cyan}"
bright_white   = "{a_bwhite}"
"##,
        shell = h(bg_shell),
        app = h(bg_app),
        card = h(bg_card),
        overlay = h(bg_overlay),
        input = h(bg_input),
        fg_primary = h(fg_primary),
        fg_secondary = h(fg_secondary),
        fg_disabled = h(fg_disabled),
        fg_inverse = h(fg_inverse),
        accent = h(accent_rgba),
        hover = h(accent_hover),
        pressed = h(accent_pressed),
        success = h(success),
        warning = h(warning),
        error = h(error),
        info = h(info),
        border_default = h(bg_input),
        border_strong = h(px(p.surface2)),
        a_black = ansi_black,
        red = p.red,
        green = p.green,
        yellow = p.yellow,
        blue = p.blue,
        magenta = p.pink,
        cyan = p.teal,
        a_white = ansi_white,
        a_bblack = ansi_bright_black,
        a_bwhite = ansi_bright_white,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::{contrast_ratio, srgb_to_oklch};
    use crate::ArlenTheme;

    #[test]
    fn mocha_with_the_default_accent_maps_the_palette() {
        let toml_text = adapt_catppuccin(Flavor::Mocha, Accent::default());
        let t = ArlenTheme::from_bundled(&toml_text).expect("adapted mocha resolves");
        assert_eq!(t.meta.id, "catppuccin-mocha");
        assert!(t.is_dark());
        assert_eq!(t.color.accent, parse_hex("#cba6f7").unwrap(), "default accent is mauve");
        assert_eq!(t.color.bg_app, parse_hex("#1e1e2e").unwrap(), "bg.app <- base");
        assert_eq!(t.color.bg_shell, parse_hex("#181825").unwrap(), "bg.shell <- mantle");
        assert_eq!(t.color.fg_primary, parse_hex("#cdd6f4").unwrap(), "fg <- text");
    }

    #[test]
    fn the_accent_picker_selects_any_of_the_fourteen() {
        let toml_text = adapt_catppuccin(Flavor::Mocha, Accent::Blue);
        let t = ArlenTheme::from_bundled(&toml_text).unwrap();
        assert_eq!(t.color.accent, parse_hex("#89b4fa").unwrap());
    }

    #[test]
    fn latte_is_light_and_inverts_the_ansi_neutrals() {
        let toml_text = adapt_catppuccin(Flavor::Latte, Accent::default());
        let t = ArlenTheme::from_bundled(&toml_text).expect("latte resolves");
        assert!(!t.is_dark());
        // The official latte ports: black <- subtext1, bright white <- surface1.
        assert_eq!(t.terminal.ansi[0], parse_hex("#5c5f77").unwrap());
        assert_eq!(t.terminal.ansi[15], parse_hex("#bcc0cc").unwrap());
    }

    #[test]
    fn dark_flavors_use_the_official_port_neutrals_and_alias_brights() {
        let toml_text = adapt_catppuccin(Flavor::Mocha, Accent::default());
        let t = ArlenTheme::from_bundled(&toml_text).unwrap();
        assert_eq!(t.terminal.ansi[0], parse_hex("#45475a").unwrap(), "black <- surface1");
        assert_eq!(t.terminal.ansi[7], parse_hex("#bac2de").unwrap(), "white <- subtext1");
        assert_eq!(t.terminal.ansi[8], parse_hex("#585b70").unwrap(), "bright black <- surface2");
        assert_eq!(t.terminal.ansi[15], parse_hex("#a6adc8").unwrap(), "bright white <- subtext0");
        assert_eq!(t.terminal.ansi[5], parse_hex("#f5c2e7").unwrap(), "magenta <- pink");
        assert_eq!(t.terminal.ansi[6], parse_hex("#94e2d5").unwrap(), "cyan <- teal");
        assert_eq!(t.terminal.ansi[1], t.terminal.ansi[9], "bright red aliases red");
    }

    #[test]
    fn every_flavor_resolves_and_clears_the_wcag_floors() {
        for flavor in [Flavor::Latte, Flavor::Frappe, Flavor::Macchiato, Flavor::Mocha] {
            let t = ArlenTheme::from_bundled(&adapt_catppuccin(flavor, Accent::default()))
                .unwrap_or_else(|e| panic!("{flavor:?} failed to resolve: {e:?}"));
            assert!(
                contrast_ratio(t.color.fg_primary, t.color.bg_app) >= 4.5,
                "{flavor:?}: fg.primary readable"
            );
            assert!(
                contrast_ratio(t.color.fg_inverse, t.color.accent) >= 4.5,
                "{flavor:?}: inverse-on-accent readable"
            );
        }
    }

    #[test]
    fn rule_a_siblings_are_hue_fixed_for_an_imported_green_accent() {
        // The plan's named bug: an imported green accent must not bleed indigo.
        let t = ArlenTheme::from_bundled(&adapt_catppuccin(Flavor::Mocha, Accent::Green)).unwrap();
        let a = srgb_to_oklch(t.color.accent);
        let hov = srgb_to_oklch(t.color.accent_hover);
        let prs = srgb_to_oklch(t.color.accent_pressed);
        assert!((hov.h - a.h).abs() < 2.0, "hover hue fixed");
        assert!((prs.h - a.h).abs() < 2.0, "pressed hue fixed");
        assert!(hov.l > a.l && prs.l < a.l, "dark flavor: hover up, pressed down");
    }
}

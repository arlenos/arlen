//! base16 inbound adapter (theming-system-plan.md Strand 4) — the reference
//! adapter the others are measured against.
//!
//! Parses a base16 scheme file (both the legacy flat YAML — `scheme:` /
//! `base00: "181818"`, hex without `#` — and the current tinted-theming shape —
//! `system:`/`name:`/`variant:` with a nested `palette:` of `"#181818"` values)
//! and emits a complete Arlen theme TOML whose values are the mapped palette.
//! The output parses as an [`crate::ArlenThemeFile`] and layers over the
//! bundled base via the normal resolve merge, so an imported scheme costs
//! nothing it does not author.
//!
//! Slot assignment (Fork (b)): `base00`→bg.app, `base01`→bg.card,
//! `base02`→bg.input + border.default, `base03`→fg.disabled + border.strong,
//! `base04`→fg.secondary, `base05`→fg.primary, **`base0D`→accent** (blue, the
//! UI-primary convention — red stays red), `base08`→error, `base0A`→warning,
//! `base0B`→success, `base0C`→info. The shell background (darker than the app
//! surface, which base16 cannot express) is reconstructed by an OKLCH
//! lightness nudge of `base00` toward the variant's pole.
//!
//! The shared adapter tail: **Rule A** derives `accent_hover`/`accent_pressed`
//! (base16 brights are canonically aliases, so no authored sibling exists) and
//! **Rule B** clamps fg-on-bg, the status hues and `fg.inverse`-on-accent to
//! the WCAG floors. The `[terminal.ansi]` block is authored explicitly to the
//! base16-shell canon (brights == normals, `base03`→bright black,
//! `base07`→bright white) rather than left to the resolve-time synthesis,
//! which would brighten the hues against that canon.

use crate::color::{
    clamp_contrast, derive_hover_pressed, srgb_to_oklch, oklch_to_srgb,
    BODY_CONTRAST_FLOOR, STATUS_CONTRAST_FLOOR,
};
use crate::gtk::rgba_to_hex;
use crate::{parse_hex, Rgba};

/// A parsed base16 scheme: the display name, author, optional declared variant
/// and the sixteen palette slots `base00..base0F` in order.
#[derive(Debug, Clone)]
pub struct Base16Scheme {
    pub name: String,
    pub author: String,
    /// `"dark"` / `"light"` when the file declares it (the current format
    /// does); inferred from `base00`'s lightness otherwise.
    pub variant: Option<String>,
    pub palette: [Rgba; 16],
}

/// Why a scheme file could not be parsed.
#[derive(Debug, PartialEq, Eq)]
pub enum Base16Error {
    /// A required key (`scheme`/`name`, or a `baseXX` slot) is missing.
    MissingKey(&'static str),
    /// A palette value did not parse as a hex colour.
    BadHex(String),
}

impl std::fmt::Display for Base16Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Base16Error::MissingKey(k) => write!(f, "missing key: {k}"),
            Base16Error::BadHex(v) => write!(f, "invalid hex value: {v}"),
        }
    }
}

impl std::error::Error for Base16Error {}

/// Strip an optional matching quote pair (double or single) and surrounding
/// whitespace from a YAML scalar.
fn unquote(v: &str) -> &str {
    let v = v.trim();
    if (v.starts_with('"') && v.ends_with('"') && v.len() >= 2)
        || (v.starts_with('\'') && v.ends_with('\'') && v.len() >= 2)
    {
        &v[1..v.len() - 1]
    } else {
        v
    }
}

/// Parse a base16 scheme from either supported YAML shape. This is a
/// deliberate flat-subset parser, not a YAML engine: both canonical formats
/// are `key: value` lines (the current one nests the palette exactly one
/// level, which indentation-insensitive key matching handles), and a real
/// YAML dependency would buy nothing but surface. Values may be
/// double-quoted, single-quoted or bare; hex with or without the `#` prefix.
pub fn parse_scheme(text: &str) -> Result<Base16Scheme, Base16Error> {
    let mut name = None;
    let mut author = None;
    let mut variant = None;
    let mut palette: [Option<Rgba>; 16] = [None; 16];

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some((key, value)) = trimmed.split_once(':') else {
            continue;
        };
        let key = key.trim();
        let value = unquote(value);
        if value.is_empty() {
            continue; // the `palette:` section header
        }
        match key {
            // Legacy uses `scheme:`, the current format `name:`; first wins.
            "scheme" | "name" => {
                if name.is_none() {
                    name = Some(value.to_string());
                }
            }
            "author" => author = Some(value.to_string()),
            "variant" => variant = Some(value.to_string()),
            _ => {
                if let Some(hex_slot) = key.strip_prefix("base") {
                    if hex_slot.len() == 2 {
                        if let Ok(idx) = usize::from_str_radix(hex_slot, 16) {
                            if idx < 16 {
                                let with_hash;
                                let hex = if value.starts_with('#') {
                                    value
                                } else {
                                    with_hash = format!("#{value}");
                                    &with_hash
                                };
                                palette[idx] = Some(
                                    parse_hex(hex)
                                        .ok_or_else(|| Base16Error::BadHex(value.to_string()))?,
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    let name = name.ok_or(Base16Error::MissingKey("scheme/name"))?;
    let mut slots = [[0.0f32; 4]; 16];
    for (i, slot) in palette.into_iter().enumerate() {
        slots[i] = slot.ok_or(Base16Error::MissingKey("base00..base0F"))?;
    }
    Ok(Base16Scheme {
        name,
        author: author.unwrap_or_default(),
        variant,
        palette: slots,
    })
}

/// Lowercase the name into a stable theme id (`[a-z0-9-]`, runs collapsed).
pub(crate) fn slugify(name: &str) -> String {
    let mut slug = String::with_capacity(name.len());
    let mut last_dash = true; // suppress a leading dash
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash {
            slug.push('-');
            last_dash = true;
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    if slug.is_empty() {
        slug.push_str("imported");
    }
    slug
}

/// Escape a free string into a TOML basic-string literal (quotes, backslashes
/// and control characters), so an arbitrary scheme name/author cannot break
/// the emitted document.
pub(crate) fn toml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if c.is_control() => out.push_str(&format!("\\u{:04X}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// Nudge a background's OKLCH lightness toward the variant's pole (darker on
/// dark, lighter on light) — the surface reconstruction for the one Arlen
/// surface base16 cannot express.
fn nudge_l(c: Rgba, delta: f32) -> Rgba {
    let mut ok = srgb_to_oklch(c);
    ok.l = (ok.l + delta).clamp(0.0, 1.0);
    oklch_to_srgb(ok, c[3])
}

/// Adapt a parsed base16 scheme into a complete Arlen theme TOML (the text of
/// a `~/.local/share/arlen/themes/{id}.toml`). The emitted values are hex
/// serializations of the mapped palette after the Rule A + Rule B tail, plus
/// the TOML-escaped name — nothing else from the source reaches the output.
pub fn adapt_base16(scheme: &Base16Scheme) -> String {
    let p = &scheme.palette;
    let dark = match scheme.variant.as_deref() {
        Some("light") => false,
        Some("dark") => true,
        // Infer from the background's perceptual lightness.
        _ => srgb_to_oklch(p[0x0]).l < 0.5,
    };

    let bg_app = p[0x0];
    let bg_shell = nudge_l(bg_app, if dark { -0.03 } else { 0.03 });
    let bg_card = p[0x1];
    let bg_input = p[0x2];
    let bg_overlay = [bg_app[0], bg_app[1], bg_app[2], 0.5];

    // Rule B: text legible on the app surface, status hues at the large floor,
    // and the classic import failure, inverse-on-accent.
    let fg_primary = clamp_contrast(p[0x5], bg_app, BODY_CONTRAST_FLOOR);
    let fg_secondary = clamp_contrast(p[0x4], bg_app, BODY_CONTRAST_FLOOR);
    let fg_disabled = p[0x3]; // deliberately muted; WCAG exempts disabled text
    let accent = p[0xD];
    let fg_inverse = clamp_contrast(bg_app, accent, BODY_CONTRAST_FLOOR);
    let success = clamp_contrast(p[0xB], bg_app, STATUS_CONTRAST_FLOOR);
    let warning = clamp_contrast(p[0xA], bg_app, STATUS_CONTRAST_FLOOR);
    let error = clamp_contrast(p[0x8], bg_app, STATUS_CONTRAST_FLOOR);
    let info = clamp_contrast(p[0xC], bg_app, STATUS_CONTRAST_FLOOR);

    // Rule A: base16 brights are canonically aliases of the normals, so the
    // source offers no authored brighter sibling — synthesise the pair.
    let (accent_hover, accent_pressed) = derive_hover_pressed(accent, dark);

    let border_default = p[0x2];
    let border_strong = p[0x3];

    let h = rgba_to_hex;
    let id = slugify(&scheme.name);
    let name = toml_escape(&scheme.name);
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
black          = "{a0}"
red            = "{a1}"
green          = "{a2}"
yellow         = "{a3}"
blue           = "{a4}"
magenta        = "{a5}"
cyan           = "{a6}"
white          = "{a7}"
bright_black   = "{a8}"
bright_red     = "{a1}"
bright_green   = "{a2}"
bright_yellow  = "{a3}"
bright_blue    = "{a4}"
bright_magenta = "{a5}"
bright_cyan    = "{a6}"
bright_white   = "{a15}"
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
        accent = h(accent),
        hover = h(accent_hover),
        pressed = h(accent_pressed),
        success = h(success),
        warning = h(warning),
        error = h(error),
        info = h(info),
        border_default = h(border_default),
        border_strong = h(border_strong),
        a0 = h(p[0x0]),
        a1 = h(p[0x8]),
        a2 = h(p[0xB]),
        a3 = h(p[0xA]),
        a4 = h(p[0xD]),
        a5 = h(p[0xE]),
        a6 = h(p[0xC]),
        a7 = h(p[0x5]),
        a8 = h(p[0x3]),
        a15 = h(p[0x7]),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::contrast_ratio;
    use crate::ArlenTheme;

    /// The canonical legacy example, verbatim from
    /// chriskempson/base16-default-schemes/default-dark.yaml.
    const LEGACY: &str = r##"
scheme: "Default Dark"
author: "Chris Kempson (http://chriskempson.com)"
base00: "181818"
base01: "282828"
base02: "383838"
base03: "585858"
base04: "b8b8b8"
base05: "d8d8d8"
base06: "e8e8e8"
base07: "f8f8f8"
base08: "ab4642"
base09: "dc9656"
base0A: "f7ca88"
base0B: "a1b56c"
base0C: "86c1b9"
base0D: "7cafc2"
base0E: "ba8baf"
base0F: "a16946"
"##;

    /// The same scheme in the current tinted-theming shape (nested palette,
    /// `#`-prefixed values, declared variant).
    const CURRENT: &str = r##"
system: "base16"
name: "Default Dark"
author: "Chris Kempson (http://chriskempson.com)"
variant: "dark"
palette:
  base00: "#181818"
  base01: "#282828"
  base02: "#383838"
  base03: "#585858"
  base04: "#b8b8b8"
  base05: "#d8d8d8"
  base06: "#e8e8e8"
  base07: "#f8f8f8"
  base08: "#ab4642"
  base09: "#dc9656"
  base0A: "#f7ca88"
  base0B: "#a1b56c"
  base0C: "#86c1b9"
  base0D: "#7cafc2"
  base0E: "#ba8baf"
  base0F: "#a16946"
"##;

    #[test]
    fn parses_the_legacy_format() {
        let s = parse_scheme(LEGACY).expect("legacy parses");
        assert_eq!(s.name, "Default Dark");
        assert_eq!(s.variant, None);
        assert_eq!(s.palette[0x0], parse_hex("#181818").unwrap());
        assert_eq!(s.palette[0xF], parse_hex("#a16946").unwrap());
    }

    #[test]
    fn parses_the_current_format() {
        let s = parse_scheme(CURRENT).expect("current parses");
        assert_eq!(s.name, "Default Dark");
        assert_eq!(s.variant.as_deref(), Some("dark"));
        assert_eq!(s.palette[0xD], parse_hex("#7cafc2").unwrap());
    }

    #[test]
    fn parses_single_quoted_and_bare_values() {
        let mixed = LEGACY
            .replace("\"181818\"", "'181818'")
            .replace("\"282828\"", "282828");
        let s = parse_scheme(&mixed).expect("mixed quoting parses");
        assert_eq!(s.palette[0x0], parse_hex("#181818").unwrap());
        assert_eq!(s.palette[0x1], parse_hex("#282828").unwrap());
    }

    #[test]
    fn missing_slots_and_bad_hex_fail_closed() {
        assert_eq!(
            parse_scheme("scheme: \"X\"\nbase00: \"181818\"\n").unwrap_err(),
            Base16Error::MissingKey("base00..base0F")
        );
        assert!(matches!(
            parse_scheme(&LEGACY.replace("\"ab4642\"", "\"not-hex\"")).unwrap_err(),
            Base16Error::BadHex(_)
        ));
    }

    #[test]
    fn adapts_the_plan_slot_assignment() {
        let s = parse_scheme(LEGACY).unwrap();
        let toml_text = adapt_base16(&s);
        // The output is a complete, parseable Arlen theme.
        let t = ArlenTheme::from_bundled(&toml_text).expect("adapted theme resolves standalone");
        assert_eq!(t.meta.id, "default-dark");
        assert!(t.is_dark(), "variant inferred dark from base00");
        assert_eq!(t.color.bg_app, parse_hex("#181818").unwrap(), "bg.app <- base00");
        assert_eq!(t.color.accent, parse_hex("#7cafc2").unwrap(), "accent <- base0D, not base08");
        assert_eq!(t.color.error, parse_hex("#ab4642").unwrap(), "error keeps red");
        assert_eq!(t.color.fg_primary, parse_hex("#d8d8d8").unwrap(), "fg <- base05");
        // The ANSI canon: brights alias the normals, base07 is bright white.
        assert_eq!(t.terminal.ansi[1], t.terminal.ansi[9], "bright red == red");
        assert_eq!(t.terminal.ansi[15], parse_hex("#f8f8f8").unwrap());
        assert_eq!(t.terminal.ansi[8], parse_hex("#585858").unwrap(), "bright black <- base03");
    }

    #[test]
    fn rule_a_derived_states_are_hue_fixed_siblings() {
        let s = parse_scheme(LEGACY).unwrap();
        let t = ArlenTheme::from_bundled(&adapt_base16(&s)).unwrap();
        let a = crate::color::srgb_to_oklch(t.color.accent);
        let hov = crate::color::srgb_to_oklch(t.color.accent_hover);
        assert!(hov.l > a.l, "hover brightens on a dark scheme");
        assert!((hov.h - a.h).abs() < 2.0, "no hue bleed: {} vs {}", hov.h, a.h);
        assert_ne!(t.color.accent_hover, t.color.accent);
    }

    #[test]
    fn rule_b_clamps_an_illegible_scheme_legible() {
        // A brutalist low-contrast scheme: mid-grey text on a mid-grey bg.
        let low = LEGACY
            .replace("\"d8d8d8\"", "\"5a5a5a\"") // base05 fg barely above bg
            .replace("\"b8b8b8\"", "\"4a4a4a\""); // base04 secondary worse
        let s = parse_scheme(&low).unwrap();
        let t = ArlenTheme::from_bundled(&adapt_base16(&s)).unwrap();
        assert!(
            contrast_ratio(t.color.fg_primary, t.color.bg_app) >= 4.5,
            "fg.primary clamped to the body floor"
        );
        assert!(
            contrast_ratio(t.color.fg_secondary, t.color.bg_app) >= 4.5,
            "fg.secondary clamped to the body floor"
        );
        assert!(
            contrast_ratio(t.color.fg_inverse, t.color.accent) >= 4.5,
            "the classic failure, inverse-on-accent, is clamped"
        );
    }

    #[test]
    fn an_adversarial_scheme_name_cannot_break_the_emitted_toml() {
        let hostile = LEGACY.replace(
            "\"Default Dark\"",
            "\"Evil\\\" \\nname = [broken]\"",
        );
        let s = parse_scheme(&hostile).unwrap();
        let toml_text = adapt_base16(&s);
        // Two layers hold: toml_escape keeps the emitted document a single valid
        // theme (it parses), and resolve's inert floor then drops the non-inert
        // name (it carries `[`/`\`) to the safe default — defence in depth.
        let t = ArlenTheme::from_bundled(&toml_text).expect("escaped name keeps the TOML valid");
        assert!(
            crate::is_inert_css_token(&t.meta.name),
            "the resolved theme name is inert, got {:?}",
            t.meta.name
        );
    }

    #[test]
    fn slugify_collapses_to_a_stable_id() {
        assert_eq!(slugify("Default Dark"), "default-dark");
        assert_eq!(slugify("  Gruvbox (Medium)!"), "gruvbox-medium");
        assert_eq!(slugify("---"), "imported");
    }
}

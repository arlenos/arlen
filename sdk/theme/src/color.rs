//! Colour math for the inbound adapters (theming-system-plan.md Fork (b)):
//! sRGB ↔ OKLCH conversion, WCAG contrast, and the two rules every adapter's
//! shared tail applies —
//!
//! * **Rule A** ([`derive_hover_pressed`]): the runtime never computes
//!   `accent_hover`/`accent_pressed` (theme-author-chosen by contract), so an
//!   adapter must derive them for an imported accent — in OKLCH lightness,
//!   hue-fixed (hover +7% L on dark / −7% on light; pressed the other way plus
//!   a touch of desaturation). Without this the bundled indigo hover bleeds
//!   under an imported green accent.
//! * **Rule B** ([`clamp_contrast`]): the WCAG guardrail — push a foreground's
//!   OKLCH lightness away from its background, the smallest move that clears
//!   the floor (4.5:1 body text, 3:1 status/large), hue and chroma fixed.
//!
//! The OKLab transform uses Björn Ottosson's published matrices (the OKLCH
//! definition). WCAG relative luminance and contrast ratio follow WCAG 2.x
//! exactly. Out-of-gamut results of a lightness nudge are component-clamped
//! back into sRGB.

use crate::{ColorTokens, Rgba};

/// A colour in OKLCH: perceptual lightness `l` (`0..=1`), chroma `c`
/// (`0..~0.4`), hue `h` in degrees (`0..360`, meaningless when `c` is 0).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Oklch {
    pub l: f32,
    pub c: f32,
    pub h: f32,
}

fn srgb_to_linear(u: f32) -> f32 {
    if u <= 0.04045 {
        u / 12.92
    } else {
        ((u + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(u: f32) -> f32 {
    if u <= 0.0031308 {
        u * 12.92
    } else {
        1.055 * u.powf(1.0 / 2.4) - 0.055
    }
}

/// Convert sRGB (alpha ignored) to OKLCH.
pub fn srgb_to_oklch(rgba: Rgba) -> Oklch {
    let r = srgb_to_linear(rgba[0]);
    let g = srgb_to_linear(rgba[1]);
    let b = srgb_to_linear(rgba[2]);

    // Linear sRGB -> LMS (Ottosson M1), then the cube-root nonlinearity.
    let l = 0.412_221_47 * r + 0.536_332_54 * g + 0.051_445_995 * b;
    let m = 0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b;
    let s = 0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b;
    let l_ = l.cbrt();
    let m_ = m.cbrt();
    let s_ = s.cbrt();

    // LMS' -> OKLab (Ottosson M2).
    let lab_l = 0.210_454_26 * l_ + 0.793_617_8 * m_ - 0.004_072_047 * s_;
    let lab_a = 1.977_998_5 * l_ - 2.428_592_2 * m_ + 0.450_593_7 * s_;
    let lab_b = 0.025_904_037 * l_ + 0.782_771_77 * m_ - 0.808_675_77 * s_;

    let c = (lab_a * lab_a + lab_b * lab_b).sqrt();
    let h = lab_b.atan2(lab_a).to_degrees().rem_euclid(360.0);
    Oklch { l: lab_l, c, h }
}

/// Convert OKLCH back to sRGB with the given alpha, component-clamping any
/// out-of-gamut result into `0..=1` (the smallest-surprise gamut strategy for
/// the ±7% nudges this module performs).
pub fn oklch_to_srgb(ok: Oklch, alpha: f32) -> Rgba {
    let h = ok.h.to_radians();
    let lab_a = ok.c * h.cos();
    let lab_b = ok.c * h.sin();

    // OKLab -> LMS' (inverse M2), cube, then LMS -> linear sRGB (inverse M1).
    let l_ = ok.l + 0.396_337_78 * lab_a + 0.215_803_76 * lab_b;
    let m_ = ok.l - 0.105_561_346 * lab_a - 0.063_854_17 * lab_b;
    let s_ = ok.l - 0.089_484_18 * lab_a - 1.291_485_5 * lab_b;
    let l = l_ * l_ * l_;
    let m = m_ * m_ * m_;
    let s = s_ * s_ * s_;

    let r = 4.076_741_7 * l - 3.307_711_6 * m + 0.230_969_94 * s;
    let g = -1.268_438 * l + 2.609_757_4 * m - 0.341_319_38 * s;
    let b = -0.004_196_086_3 * l - 0.703_418_6 * m + 1.707_614_7 * s;

    [
        linear_to_srgb(r).clamp(0.0, 1.0),
        linear_to_srgb(g).clamp(0.0, 1.0),
        linear_to_srgb(b).clamp(0.0, 1.0),
        alpha.clamp(0.0, 1.0),
    ]
}

/// WCAG 2.x relative luminance of an sRGB colour (alpha ignored).
pub fn relative_luminance(rgba: Rgba) -> f32 {
    let lin = |u: f32| {
        if u <= 0.03928 {
            u / 12.92
        } else {
            ((u + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * lin(rgba[0]) + 0.7152 * lin(rgba[1]) + 0.0722 * lin(rgba[2])
}

/// WCAG 2.x contrast ratio between two colours, `1.0..=21.0` (order-free).
pub fn contrast_ratio(a: Rgba, b: Rgba) -> f32 {
    let la = relative_luminance(a);
    let lb = relative_luminance(b);
    let (hi, lo) = if la >= lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

/// APCA (Accessible Perceptual Contrast Algorithm, W3 0.1.9) lightness contrast
/// `Lc` of `text` over `bg` (alpha ignored) - the perceptual contrast measure the
/// WCAG 2.2 / EN 301 549 era surfaces alongside the legacy [`contrast_ratio`]. `Lc`
/// is SIGNED: positive for dark text on a light background, negative for light text
/// on a dark one; its magnitude runs roughly `0..=108`. Rough use thresholds (APCA
/// "bronze"): `|Lc| >= 75` for body text, `>= 60` for larger or secondary text,
/// `>= 45` for large or non-text; below the low-contrast clip the pair is treated as
/// having no usable contrast and returns `0`.
///
/// This is NOT the WCAG 2.x ratio and the two scales are not interconvertible: APCA
/// uses a straight 2.4-power sRGB-to-luminance (no piecewise segment), a near-black
/// soft-clamp, and asymmetric power curves per polarity. Surfacing both is the goal -
/// WCAG 2.x stays the legal floor, APCA the perceptual read.
pub fn apca_lc(text: Rgba, bg: Rgba) -> f32 {
    // APCA-W3 0.1.9 constants (the "0.0.98G-4g" set).
    const TRC: f32 = 2.4;
    const RCO: f32 = 0.2126729;
    const GCO: f32 = 0.7151522;
    const BCO: f32 = 0.0721750;
    const BLK_THRS: f32 = 0.022;
    const BLK_CLMP: f32 = 1.414;
    const DELTA_Y_MIN: f32 = 0.0005;
    const SCALE: f32 = 1.14;
    const LO_CLIP: f32 = 0.1;
    const LO_OFFSET: f32 = 0.027;
    const NORM_BG: f32 = 0.56;
    const NORM_TXT: f32 = 0.57;
    const REV_TXT: f32 = 0.62;
    const REV_BG: f32 = 0.65;

    let luminance = |c: Rgba| {
        RCO * c[0].max(0.0).powf(TRC) + GCO * c[1].max(0.0).powf(TRC) + BCO * c[2].max(0.0).powf(TRC)
    };
    // Soft-clamp near-black luminance so very dark pairs do not over-report.
    let soft_clamp = |y: f32| {
        if y < BLK_THRS {
            y + (BLK_THRS - y).powf(BLK_CLMP)
        } else {
            y
        }
    };

    let txt_y = soft_clamp(luminance(text));
    let bg_y = soft_clamp(luminance(bg));

    // Indistinguishable luminances carry no contrast.
    if (bg_y - txt_y).abs() < DELTA_Y_MIN {
        return 0.0;
    }

    let sapc = if bg_y > txt_y {
        // Dark text on a light background (normal polarity, positive Lc).
        (bg_y.powf(NORM_BG) - txt_y.powf(NORM_TXT)) * SCALE
    } else {
        // Light text on a dark background (reverse polarity, negative Lc).
        (bg_y.powf(REV_BG) - txt_y.powf(REV_TXT)) * SCALE
    };

    // Clip a near-zero result to no-contrast, else trim the low-contrast offset.
    let out = if sapc.abs() < LO_CLIP {
        0.0
    } else if sapc > 0.0 {
        sapc - LO_OFFSET
    } else {
        sapc + LO_OFFSET
    };
    out * 100.0
}

/// Which contrast floor a colour pair is held to: body text (strict) or large
/// text / icons / non-text UI (relaxed).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContrastUse {
    /// Body text: WCAG 2.x AA 4.5, APCA bronze `|Lc|` 60.
    Body,
    /// Large text, icons or non-text UI (WCAG 2.2 1.4.11): AA 3.0, APCA `|Lc|` 45.
    Large,
}

impl ContrastUse {
    /// The WCAG 2.x AA ratio floor for this use.
    pub fn wcag_floor(self) -> f32 {
        match self {
            ContrastUse::Body => 4.5,
            ContrastUse::Large => 3.0,
        }
    }
    /// The APCA `|Lc|` floor for this use (bronze tier).
    pub fn apca_floor(self) -> f32 {
        match self {
            ContrastUse::Body => 60.0,
            ContrastUse::Large => 45.0,
        }
    }
}

/// One foreground-over-background pair audited for contrast: a human label, both
/// measures (WCAG 2.x ratio + signed APCA `Lc`), the floor it is held to, and
/// whether it clears each. `apca_pass` tests `|apca|` (the sign is only polarity).
#[derive(Debug, Clone)]
pub struct ContrastFinding {
    /// Human label, e.g. `"fg.primary on bg.app"`.
    pub pair: &'static str,
    /// The WCAG 2.x contrast ratio (`1.0..=21.0`).
    pub wcag: f32,
    /// The signed APCA `Lc`.
    pub apca: f32,
    /// The floor this pair is held to.
    pub usage: ContrastUse,
    /// Whether the WCAG ratio clears its floor.
    pub wcag_pass: bool,
    /// Whether `|apca|` clears its floor.
    pub apca_pass: bool,
}

/// Audit a resolved theme's key foreground-over-background pairs, reporting both
/// the WCAG 2.x ratio and the APCA `Lc` against the AA / bronze floor for each
/// pair's use. This is the compute half of the A11Y contrast surfacing: the UI
/// renders the findings, nothing here renders. Body-text pairs (primary/secondary
/// text on each surface, the inverse label on accent) take the strict floor; the
/// status hues on the app surface and the strong border, used as icons / non-text
/// UI, take the relaxed floor. `fg.disabled` is omitted - WCAG exempts disabled text.
pub fn contrast_report(c: &ColorTokens) -> Vec<ContrastFinding> {
    let pairs: &[(&'static str, Rgba, Rgba, ContrastUse)] = &[
        ("fg.primary on bg.app", c.fg_primary, c.bg_app, ContrastUse::Body),
        ("fg.secondary on bg.app", c.fg_secondary, c.bg_app, ContrastUse::Body),
        ("fg.primary on bg.card", c.fg_primary, c.bg_card, ContrastUse::Body),
        ("fg.primary on bg.input", c.fg_primary, c.bg_input, ContrastUse::Body),
        ("fg.inverse on accent", c.fg_inverse, c.accent, ContrastUse::Body),
        ("success on bg.app", c.success, c.bg_app, ContrastUse::Large),
        ("warning on bg.app", c.warning, c.bg_app, ContrastUse::Large),
        ("error on bg.app", c.error, c.bg_app, ContrastUse::Large),
        ("info on bg.app", c.info, c.bg_app, ContrastUse::Large),
        ("border.strong on bg.app", c.border_strong, c.bg_app, ContrastUse::Large),
    ];
    pairs
        .iter()
        .map(|&(pair, fg, bg, usage)| {
            let wcag = contrast_ratio(fg, bg);
            let apca = apca_lc(fg, bg);
            ContrastFinding {
                pair,
                wcag,
                apca,
                usage,
                wcag_pass: wcag >= usage.wcag_floor(),
                apca_pass: apca.abs() >= usage.apca_floor(),
            }
        })
        .collect()
}

/// Rule B's WCAG floor for body text (and for `fg.inverse` on `accent`).
pub const BODY_CONTRAST_FLOOR: f32 = 4.5;
/// Rule B's WCAG floor for status colours / large text.
pub const STATUS_CONTRAST_FLOOR: f32 = 3.0;

/// How far Rule A nudges the accent's OKLCH lightness (the plan's 6–8% band).
const HOVER_L_NUDGE: f32 = 0.07;
/// Rule A's "touch of desaturation" on the pressed state.
const PRESSED_CHROMA_FACTOR: f32 = 0.9;

/// Rule A: derive `(accent_hover, accent_pressed)` from an imported accent —
/// OKLCH lightness nudges, hue-fixed. On a dark theme hover brightens and
/// pressed darkens (the other way on light); pressed also desaturates slightly.
/// Use only when the source did not author the siblings itself (an authored
/// brighter sibling is preferred over this synthetic nudge).
pub fn derive_hover_pressed(accent: Rgba, dark: bool) -> (Rgba, Rgba) {
    let ok = srgb_to_oklch(accent);
    let dir = if dark { 1.0 } else { -1.0 };
    let hover = Oklch {
        l: (ok.l + dir * HOVER_L_NUDGE).clamp(0.0, 1.0),
        ..ok
    };
    let pressed = Oklch {
        l: (ok.l - dir * HOVER_L_NUDGE).clamp(0.0, 1.0),
        c: ok.c * PRESSED_CHROMA_FACTOR,
        ..ok
    };
    (oklch_to_srgb(hover, accent[3]), oklch_to_srgb(pressed, accent[3]))
}

/// Rule B: clamp `fg` against `bg` to at least `floor` contrast (4.5 for body
/// text, 3.0 for status/large). Returns `fg` unchanged when the pair already
/// clears. Otherwise walks the foreground's OKLCH lightness toward BOTH poles
/// (hue and chroma fixed) and returns the colour from whichever direction clears
/// the floor in the fewest steps — the genuinely smallest move (an equal-steps
/// tie keeps the lighter pole, deterministically). Walking only one
/// direction is wrong: when the foreground is between the background and a pole
/// that tops out below the floor, the opposite pole may clear comfortably (e.g.
/// a mid accent on a light background — white-on-accent caps low, black clears),
/// and a one-way walk would ship the sub-floor extreme. If NEITHER pole reaches
/// the floor, returns the best ratio found across both — the best available,
/// never worse than the input. A non-finite input (NaN lightness) returns `fg`
/// unchanged rather than looping.
pub fn clamp_contrast(fg: Rgba, bg: Rgba, floor: f32) -> Rgba {
    let start = contrast_ratio(fg, bg);
    if start >= floor {
        return fg;
    }
    let fg_ok = srgb_to_oklch(fg);
    if !fg_ok.l.is_finite() {
        return fg;
    }
    const STEP: f32 = 0.01;
    let mut best = fg;
    let mut best_ratio = start;
    // The fewest-steps clear across both directions. The `steps < s` test is
    // strict, so on an equal-steps tie the first-tried (lighter, dir=+1) pole is
    // kept — a deterministic tie-break, not a smaller move (the moves are equal).
    let mut cleared: Option<(u32, Rgba)> = None;
    for dir in [1.0f32, -1.0] {
        let mut l = fg_ok.l;
        let mut steps = 0u32;
        loop {
            l += dir * STEP;
            steps += 1;
            let clamped_l = l.clamp(0.0, 1.0);
            let candidate = oklch_to_srgb(Oklch { l: clamped_l, ..fg_ok }, fg[3]);
            let ratio = contrast_ratio(candidate, bg);
            if ratio >= floor {
                if cleared.is_none_or(|(s, _)| steps < s) {
                    cleared = Some((steps, candidate));
                }
                break;
            }
            if ratio > best_ratio {
                best_ratio = ratio;
                best = candidate;
            }
            if clamped_l <= 0.0 || clamped_l >= 1.0 {
                break;
            }
        }
    }
    cleared.map(|(_, c)| c).unwrap_or(best)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_hex;

    fn close(a: f32, b: f32, eps: f32) -> bool {
        (a - b).abs() <= eps
    }

    #[test]
    fn oklch_round_trips_a_grid_of_srgb_colours() {
        // The conversion property: srgb -> oklch -> srgb is identity to within
        // rounding for in-gamut colours.
        for r in [0.0f32, 0.13, 0.4, 0.62, 0.85, 1.0] {
            for g in [0.0f32, 0.21, 0.5, 0.77, 1.0] {
                for b in [0.0f32, 0.33, 0.69, 1.0] {
                    let back = oklch_to_srgb(srgb_to_oklch([r, g, b, 1.0]), 1.0);
                    for (i, want) in [r, g, b].iter().enumerate() {
                        assert!(
                            close(back[i], *want, 2e-3),
                            "round-trip drift at rgb({r},{g},{b}) component {i}: {} vs {want}",
                            back[i]
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn oklch_matches_the_published_reference_values() {
        // Ottosson's published OKLab values: white = (1, 0, 0); sRGB red
        // #ff0000 = L 0.6279..., a 0.2249, b 0.1258 (chroma ~0.2576).
        let white = srgb_to_oklch([1.0, 1.0, 1.0, 1.0]);
        assert!(close(white.l, 1.0, 5e-3), "white L = {}", white.l);
        assert!(white.c < 5e-3, "white chroma = {}", white.c);

        let red = srgb_to_oklch([1.0, 0.0, 0.0, 1.0]);
        assert!(close(red.l, 0.6279, 5e-3), "red L = {}", red.l);
        assert!(close(red.c, 0.2576, 5e-3), "red C = {}", red.c);
    }

    #[test]
    fn apca_matches_the_published_sanity_values() {
        // The canonical APCA-W3 0.1.9 sanity pairs (text over bg):
        //   #888 on #fff -> Lc  63.06 (dark text on light, positive)
        //   #000 on #aaa -> Lc  58.15
        //   #fff on #888 -> Lc -68.54 (light text on dark, negative)
        //   #aaa on #000 -> Lc -56.24
        let lc = |t: &str, b: &str| apca_lc(parse_hex(t).unwrap(), parse_hex(b).unwrap());
        assert!(close(lc("#888888", "#ffffff"), 63.06, 0.6), "{}", lc("#888888", "#ffffff"));
        assert!(close(lc("#000000", "#aaaaaa"), 58.15, 0.6), "{}", lc("#000000", "#aaaaaa"));
        assert!(close(lc("#ffffff", "#888888"), -68.54, 0.6), "{}", lc("#ffffff", "#888888"));
        assert!(close(lc("#aaaaaa", "#000000"), -56.24, 0.6), "{}", lc("#aaaaaa", "#000000"));
    }

    #[test]
    fn contrast_report_audits_the_bundled_dark_theme() {
        let t = crate::ArlenTheme::from_bundled(include_str!("../themes/dark.toml"))
            .expect("bundled dark resolves");
        let report = contrast_report(&t.color);
        assert_eq!(report.len(), 10);
        // The body text pair is near-white on near-black: it must clear AA on both
        // scales (other tests assert the >= 4.5 WCAG floor for this pair).
        let body = report.iter().find(|f| f.pair == "fg.primary on bg.app").unwrap();
        assert!(body.wcag_pass && body.wcag >= 4.5, "body WCAG {}", body.wcag);
        assert!(body.apca_pass && body.apca.abs() >= 60.0, "body APCA {}", body.apca);
        // Every pair is present and its pass flags agree with the floor.
        for f in &report {
            assert_eq!(f.wcag_pass, f.wcag >= f.usage.wcag_floor(), "{}", f.pair);
            assert_eq!(f.apca_pass, f.apca.abs() >= f.usage.apca_floor(), "{}", f.pair);
        }
    }

    #[test]
    fn contrast_use_floors() {
        assert_eq!(ContrastUse::Body.wcag_floor(), 4.5);
        assert_eq!(ContrastUse::Body.apca_floor(), 60.0);
        assert_eq!(ContrastUse::Large.wcag_floor(), 3.0);
        assert_eq!(ContrastUse::Large.apca_floor(), 45.0);
    }

    #[test]
    fn apca_polarity_and_no_contrast() {
        // Sign encodes polarity; an identical pair has no usable contrast.
        let white = [1.0, 1.0, 1.0, 1.0];
        let black = [0.0, 0.0, 0.0, 1.0];
        assert!(apca_lc(black, white) > 0.0, "dark-on-light is positive");
        assert!(apca_lc(white, black) < 0.0, "light-on-dark is negative");
        assert_eq!(apca_lc(white, white), 0.0, "same colour -> no contrast");
    }

    #[test]
    fn wcag_contrast_matches_the_canonical_anchors() {
        let black = [0.0, 0.0, 0.0, 1.0];
        let white = [1.0, 1.0, 1.0, 1.0];
        assert!(close(contrast_ratio(black, white), 21.0, 0.01));
        assert!(close(contrast_ratio(white, white), 1.0, 1e-6));
        // #767676 on white is the canonical just-passes 4.5:1 grey; #888888
        // fails comfortably (~3.5:1).
        let just_passes = parse_hex("#767676").unwrap();
        let fails = parse_hex("#888888").unwrap();
        assert!(contrast_ratio(just_passes, white) >= 4.5);
        assert!(contrast_ratio(fails, white) < 4.5);
    }

    #[test]
    fn rule_a_brightens_hover_and_desaturates_pressed_on_dark() {
        let accent = parse_hex("#22c55e").unwrap(); // an imported green
        let (hover, pressed) = derive_hover_pressed(accent, true);
        let a = srgb_to_oklch(accent);
        let h = srgb_to_oklch(hover);
        let p = srgb_to_oklch(pressed);
        assert!(h.l > a.l, "hover is lighter on dark ({} vs {})", h.l, a.l);
        assert!(p.l < a.l, "pressed is darker on dark ({} vs {})", p.l, a.l);
        assert!(p.c < a.c, "pressed desaturates ({} vs {})", p.c, a.c);
        // Hue-fixed: the green stays a green (no indigo bleed).
        assert!(close(h.h, a.h, 2.0), "hover hue drifted: {} vs {}", h.h, a.h);
        assert!(close(p.h, a.h, 2.0), "pressed hue drifted: {} vs {}", p.h, a.h);
    }

    #[test]
    fn rule_a_inverts_direction_on_light() {
        let accent = parse_hex("#6366f1").unwrap();
        let (hover, pressed) = derive_hover_pressed(accent, false);
        let a = srgb_to_oklch(accent);
        assert!(srgb_to_oklch(hover).l < a.l, "hover darkens on light");
        assert!(srgb_to_oklch(pressed).l > a.l, "pressed lightens on light");
    }

    #[test]
    fn rule_b_returns_a_passing_pair_unchanged() {
        let fg = parse_hex("#fafafa").unwrap();
        let bg = parse_hex("#0f0f0f").unwrap();
        assert_eq!(clamp_contrast(fg, bg, 4.5), fg);
    }

    #[test]
    fn rule_b_pushes_a_failing_pair_to_the_floor_minimally() {
        // A mid-grey on a dark background: fails 4.5, must be pushed lighter.
        let fg = parse_hex("#555555").unwrap();
        let bg = parse_hex("#222222").unwrap();
        let before = contrast_ratio(fg, bg);
        assert!(before < 4.5, "fixture should fail ({before})");
        let clamped = clamp_contrast(fg, bg, 4.5);
        let after = contrast_ratio(clamped, bg);
        assert!(after >= 4.5, "clamped pair clears the floor ({after})");
        assert!(
            after < 6.0,
            "the move is the smallest that clears, not a jump to the pole ({after})"
        );
        // Pushed AWAY from the dark bg: lighter, hue/chroma intact.
        assert!(srgb_to_oklch(clamped).l > srgb_to_oklch(fg).l);
    }

    #[test]
    fn rule_b_yields_the_best_extreme_when_the_floor_is_unreachable() {
        // 21:1 over mid-grey is unreachable; the clamp must terminate and hand
        // back the best in-gamut value rather than loop or overshoot. Over
        // mid-grey the black pole (~5.3:1) beats the white pole (~3.9:1), and
        // the both-direction walk must return the better one.
        let fg = parse_hex("#808080").unwrap();
        let bg = parse_hex("#808080").unwrap();
        let clamped = clamp_contrast(fg, bg, 21.0);
        let got = contrast_ratio(clamped, bg);
        assert!(got >= 5.0, "got the best available extreme, the black pole ({got})");
    }

    #[test]
    fn rule_b_takes_the_opposite_pole_when_the_near_pole_cannot_clear() {
        // The bug the review caught: fg slightly lighter than bg, but the white
        // pole tops out below the floor while the dark pole clears. A one-way
        // (away-from-bg => lighter) walk would ship ~3.9:1; both-direction must
        // return the black pole that clears 4.5.
        let fg = parse_hex("#8c8c8c").unwrap();
        let bg = parse_hex("#808080").unwrap();
        assert!(contrast_ratio(fg, bg) < 4.5, "fixture should fail");
        let clamped = clamp_contrast(fg, bg, 4.5);
        assert!(
            contrast_ratio(clamped, bg) >= 4.5,
            "the opposite (dark) pole clears the floor ({})",
            contrast_ratio(clamped, bg)
        );
        assert!(
            srgb_to_oklch(clamped).l < srgb_to_oklch(fg).l,
            "clamped darker — the clearing direction, not the away-from-bg one"
        );
    }

    #[test]
    fn rule_b_picks_the_smaller_move_across_both_directions() {
        // Both poles clear: the result must be the nearer one (fewest steps).
        let fg = parse_hex("#9a9a9a").unwrap();
        let bg = parse_hex("#3a3a3a").unwrap();
        if contrast_ratio(fg, bg) >= 4.5 {
            return; // fixture already passes; nothing to assert
        }
        let clamped = clamp_contrast(fg, bg, 4.5);
        let moved = (srgb_to_oklch(clamped).l - srgb_to_oklch(fg).l).abs();
        assert!(contrast_ratio(clamped, bg) >= 4.5);
        assert!(moved < 0.5, "the smaller lightness move was chosen ({moved})");
    }
}

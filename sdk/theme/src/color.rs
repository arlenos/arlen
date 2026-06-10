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

use crate::Rgba;

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
/// the floor in the fewest steps — the genuinely smallest move. Walking only one
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
    // The fewest-steps clear across both directions (smaller move wins ties).
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

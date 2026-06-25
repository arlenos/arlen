//! I18N-R3: the born-RTL lint. Physical directional CSS (`margin-left`,
//! `text-align: right`, `float: left`) does not mirror under `dir="rtl"`; the
//! logical equivalents (`margin-inline-start`, `text-align: start`,
//! `float: inline-start`) do, for free. This is the pure scanner that flags the
//! physical forms so an RTL layout is correct from the start rather than retrofit.
//!
//! Scope of this first cut: unambiguous physical CSS PROPERTIES + the
//! direction-valued `text-align` / `float`. The Tailwind physical-utility layer
//! (`ml-`/`pl-`/`text-left` -> `ms-`/`ps-`/`text-start`) and bare positional
//! `left:`/`right:` (legitimately physical for some overlays) are follow-ups; this
//! gate stays low-false-positive so it can be a hard CI check. Format-agnostic:
//! the caller feeds `.css` and `.svelte` source alike.

/// A flagged physical-directional CSS usage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RtlFinding {
    /// 1-based line number.
    pub line: usize,
    /// The physical property or value found.
    pub found: String,
    /// The logical replacement to use instead.
    pub suggestion: String,
}

/// Physical longhand properties and their logical replacements.
const PHYSICAL_PROPS: &[(&str, &str)] = &[
    ("margin-left", "margin-inline-start"),
    ("margin-right", "margin-inline-end"),
    ("padding-left", "padding-inline-start"),
    ("padding-right", "padding-inline-end"),
    ("border-left", "border-inline-start"),
    ("border-right", "border-inline-end"),
];

/// Whether `prop` appears in `line` as a CSS property: the match is not part of a
/// longer identifier on its left (so a search for `margin-left` is not satisfied by
/// some `x-margin-left`) and is followed, after optional whitespace, by `:`.
fn declares(line: &str, prop: &str) -> bool {
    let mut from = 0;
    while let Some(rel) = line[from..].find(prop) {
        let at = from + rel;
        let left_ok = line[..at]
            .chars()
            .next_back()
            .is_none_or(|c| !c.is_alphanumeric() && c != '-' && c != '_');
        if left_ok && line[at + prop.len()..].trim_start().starts_with(':') {
            return true;
        }
        from = at + prop.len();
    }
    false
}

/// Whether `line` sets `prop` to a value beginning with `val` (e.g.
/// `text-align: right`). The property must not be part of a longer identifier.
fn value_is(line: &str, prop: &str, val: &str) -> bool {
    let mut from = 0;
    while let Some(rel) = line[from..].find(prop) {
        let at = from + rel;
        let left_ok = line[..at]
            .chars()
            .next_back()
            .is_none_or(|c| !c.is_alphanumeric() && c != '-' && c != '_');
        if left_ok {
            if let Some(rest) = line[at + prop.len()..].trim_start().strip_prefix(':') {
                if rest.trim_start().starts_with(val) {
                    return true;
                }
            }
        }
        from = at + prop.len();
    }
    false
}

/// Physical Tailwind v4 spacing utilities (a value follows the prefix) and their
/// logical replacements. Tailwind ships `ms-`/`me-`/`ps-`/`pe-` for these.
const TW_SPACING: &[(&str, &str)] = &[("ml-", "ms-"), ("mr-", "me-"), ("pl-", "ps-"), ("pr-", "pe-")];

/// Physical Tailwind border / radius side utilities (bare, or a value follows after
/// a `-`). Tailwind ships `border-s`/`border-e` and `rounded-s`/`rounded-e`.
const TW_SIDE: &[(&str, &str)] =
    &[("border-l", "border-s"), ("border-r", "border-e"), ("rounded-l", "rounded-s"), ("rounded-r", "rounded-e")];

/// Whether the char before index `at` can begin a class token: a separator inside
/// `class="..."` / `class:` / a `clsx(...)` arg / a variant prefix (`md:ml-2`), or
/// the line start. Notably NOT alphanumeric / `-` / `_` / `.`, so a kebab
/// identifier that merely ends in a prefix (`control-ml-x`, `scroll-left`) is not a
/// class-token start.
fn class_boundary_before(line: &str, at: usize) -> bool {
    line[..at]
        .chars()
        .next_back()
        .is_none_or(|c| matches!(c, ' ' | '\t' | '"' | '\'' | '`' | ':' | '{' | '(' | '>'))
}

/// Whether `rest` (the text right after a spacing prefix) is a Tailwind value: a
/// number / fraction (`4`, `1/2`), an arbitrary value (`[2px]`, `(--v)`), or the
/// `auto` / `px` / `full` keywords. A bare word (`pl-panel`) is not.
fn tailwind_value_after(rest: &str) -> bool {
    rest.starts_with(|c: char| c.is_ascii_digit() || c == '[' || c == '(')
        || rest.starts_with("auto")
        || rest.starts_with("px")
        || rest.starts_with("full")
}

/// Flag physical Tailwind utility tokens on a line into `findings`.
fn scan_tailwind(line: &str, line_no: usize, findings: &mut Vec<RtlFinding>) {
    let push = |findings: &mut Vec<RtlFinding>, found: &str, suggestion: &str| {
        findings.push(RtlFinding {
            line: line_no,
            found: found.to_string(),
            suggestion: suggestion.to_string(),
        });
    };
    // text-left / text-right: exact tokens (a boundary, not a letter, must follow).
    for (phys, logical) in [("text-left", "text-start"), ("text-right", "text-end")] {
        let mut from = 0;
        while let Some(rel) = line[from..].find(phys) {
            let at = from + rel;
            let after = &line[at + phys.len()..];
            let tail_ok = after.chars().next().is_none_or(|c| !c.is_alphanumeric() && c != '-');
            if class_boundary_before(line, at) && tail_ok {
                push(findings, phys, logical);
                break;
            }
            from = at + phys.len();
        }
    }
    // Spacing utilities: a value must follow the prefix.
    for (phys, logical) in TW_SPACING {
        let mut from = 0;
        while let Some(rel) = line[from..].find(phys) {
            let at = from + rel;
            if class_boundary_before(line, at) && tailwind_value_after(&line[at + phys.len()..]) {
                push(findings, phys, logical);
                break;
            }
            from = at + phys.len();
        }
    }
    // Border / radius sides: bare, or `-<value>`, or a digit; never a letter
    // (so `border-l` is flagged but `border-light` is not).
    for (phys, logical) in TW_SIDE {
        let mut from = 0;
        while let Some(rel) = line[from..].find(phys) {
            let at = from + rel;
            let next = line[at + phys.len()..].chars().next();
            let tail_ok = next.is_none_or(|c| !c.is_alphabetic());
            if class_boundary_before(line, at) && tail_ok {
                push(findings, phys, logical);
                break;
            }
            from = at + phys.len();
        }
    }
}

/// Scan CSS / Svelte source for physical directional usages, returning a finding
/// (line, the physical form, the logical replacement) per flagged line. Covers CSS
/// properties + direction-valued `text-align`/`float`, and physical Tailwind
/// utilities (`ml-`/`pl-`/`text-left`/`border-l` -> the `ms-`/`ps-`/`text-start`/
/// `border-s` logical forms). Empty when the source is already direction-neutral.
pub fn scan_rtl(source: &str) -> Vec<RtlFinding> {
    let mut findings = Vec::new();
    for (n, line) in source.lines().enumerate() {
        let line_no = n + 1;
        for (phys, logical) in PHYSICAL_PROPS {
            if declares(line, phys) {
                findings.push(RtlFinding {
                    line: line_no,
                    found: (*phys).to_string(),
                    suggestion: (*logical).to_string(),
                });
            }
        }
        // Direction-valued properties: text-align / float take left|right.
        for &(val, text_align, float) in &[
            ("left", "text-align: start", "float: inline-start"),
            ("right", "text-align: end", "float: inline-end"),
        ] {
            if value_is(line, "text-align", val) {
                findings.push(RtlFinding {
                    line: line_no,
                    found: format!("text-align: {val}"),
                    suggestion: text_align.to_string(),
                });
            }
            if value_is(line, "float", val) {
                findings.push(RtlFinding {
                    line: line_no,
                    found: format!("float: {val}"),
                    suggestion: float.to_string(),
                });
            }
        }
        scan_tailwind(line, line_no, &mut findings);
    }
    findings
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_physical_longhand_properties() {
        let f = scan_rtl("  margin-left: 8px;\n  padding-right: 4px;");
        assert_eq!(f.len(), 2);
        assert_eq!(f[0].found, "margin-left");
        assert_eq!(f[0].suggestion, "margin-inline-start");
        assert_eq!(f[1].line, 2);
        assert_eq!(f[1].suggestion, "padding-inline-end");
    }

    #[test]
    fn flags_directional_text_align_and_float() {
        let f = scan_rtl("text-align: right;\nfloat: left;");
        assert_eq!(f.len(), 2);
        assert_eq!(f[0].found, "text-align: right");
        assert_eq!(f[0].suggestion, "text-align: end");
        assert_eq!(f[1].found, "float: left");
        assert_eq!(f[1].suggestion, "float: inline-start");
    }

    #[test]
    fn accepts_logical_and_neutral_css() {
        // Logical longhands, centered text, and an unrelated `left` word must not
        // trip the gate.
        let src = "margin-inline-start: 8px;\ntext-align: center;\nborder-inline-end: 1px;\n// the left pane";
        assert!(scan_rtl(src).is_empty(), "{:?}", scan_rtl(src));
    }

    #[test]
    fn does_not_false_match_a_longer_identifier() {
        // A hyphenated/underscored compound that merely ends in the property name
        // is not a declaration of it.
        assert!(scan_rtl("--my-margin-left-token: 1;").is_empty());
        assert!(scan_rtl("custom_padding-left_var: 1;").is_empty());
    }

    #[test]
    fn float_right_and_text_align_left_map_correctly() {
        let f = scan_rtl("float: right;\ntext-align: left;");
        assert_eq!(f[0].suggestion, "float: inline-end");
        assert_eq!(f[1].suggestion, "text-align: start");
    }

    #[test]
    fn flags_physical_tailwind_spacing_and_text_align() {
        let f = scan_rtl(r#"<div class="ml-4 pr-2 text-right md:pl-auto">"#);
        let found: Vec<&str> = f.iter().map(|x| x.found.as_str()).collect();
        assert!(found.contains(&"ml-"), "{found:?}");
        assert!(found.contains(&"pr-"), "{found:?}");
        assert!(found.contains(&"text-right"), "{found:?}");
        assert!(found.contains(&"pl-"), "md:-variant boundary: {found:?}");
        let ml = f.iter().find(|x| x.found == "ml-").unwrap();
        assert_eq!(ml.suggestion, "ms-");
    }

    #[test]
    fn flags_tailwind_border_and_radius_sides() {
        let f = scan_rtl(r#"class="border-l rounded-r-lg""#);
        let found: Vec<&str> = f.iter().map(|x| x.found.as_str()).collect();
        assert!(found.contains(&"border-l"), "{found:?}");
        assert!(found.contains(&"rounded-r"), "{found:?}");
    }

    #[test]
    fn tailwind_low_false_positive() {
        // rounded-lg (size, not a side), a kebab identifier ending in a prefix, a
        // spacing prefix with a non-value word, and a bare CSS word must NOT flag.
        for src in [
            r#"class="rounded-lg""#,
            r#"class="border-light-thing""#,
            r#"class="control-ml-2 scroll-left""#,
            r#"class="pl-panel""#,
            "the left margin",
        ] {
            assert!(scan_rtl(src).is_empty(), "false positive on {src:?}: {:?}", scan_rtl(src));
        }
    }
}

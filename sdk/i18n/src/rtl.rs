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

/// Scan CSS / Svelte source for physical directional usages, returning a finding
/// (line, the physical form, the logical replacement) per flagged line. Empty when
/// the source is already direction-neutral.
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
}

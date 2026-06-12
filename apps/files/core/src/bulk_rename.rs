//! FM-R11 bulk-rename PLANNING: a pure transform from a set of current names and
//! a [`RenameRule`] to a preview of new names, with conflict detection
//! (file-manager-plan.md, the FM-R11 line). The live-preview UI renders this; the
//! actual rename of each row goes through the existing `ops::rename`, so this
//! module touches no filesystem and is fully testable in isolation.
//!
//! The rule is applied per name as a fixed, documented pipeline -
//! find/replace -> case -> numbering - so the preview is deterministic and the
//! UI can show exactly what will happen before anything is renamed.

use serde::{Deserialize, Serialize};

/// A letter-case transform applied to the whole name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CaseTransform {
    /// Lowercase the whole name.
    Lower,
    /// Uppercase the whole name.
    Upper,
    /// Uppercase the first character of each whitespace-separated word, leaving
    /// the rest of each word unchanged.
    Title,
}

/// Sequential numbering applied via a template. `template` may contain the tokens
/// `{n}` (the zero-padded sequence number) and `{name}` (the name produced by the
/// find/replace + case stages). A template with no `{n}` has the number appended,
/// so numbering always makes the rows distinct.
#[derive(Debug, Clone, Deserialize)]
pub struct Numbering {
    /// The output template, e.g. `"{name}-{n}"` or `"photo-{n}"`.
    pub template: String,
    /// The first sequence number (assigned to the first name).
    pub start: u64,
    /// Added to the sequence number for each subsequent name.
    pub step: u64,
    /// Minimum width of the number, left-padded with zeros (`3` -> `001`).
    pub pad: usize,
}

/// A bulk-rename rule. Empty/absent fields are no-ops, so a default rule leaves
/// every name unchanged.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct RenameRule {
    /// Literal substring to replace (every occurrence). `None`/empty: no
    /// find/replace stage.
    pub find: Option<String>,
    /// The replacement for `find`.
    #[serde(default)]
    pub replace: String,
    /// Match `find` case-insensitively (the replacement is inserted verbatim).
    #[serde(default)]
    pub find_case_insensitive: bool,
    /// A case transform applied after find/replace.
    pub case: Option<CaseTransform>,
    /// Sequential numbering applied last.
    pub numbering: Option<Numbering>,
}

/// Why a planned name cannot be applied as-is. Precedence (highest first):
/// `Invalid` > `Duplicate` > `Unchanged` > `None`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ConflictKind {
    /// The new name is applicable and distinct.
    None,
    /// The new name equals the old name (nothing to do).
    Unchanged,
    /// The new name is not a usable filename (empty, `.`/`..`, or contains `/`
    /// or a NUL byte).
    Invalid,
    /// The new name collides with another row's final name.
    Duplicate,
}

/// One planned rename: the original name, the proposed new name and any conflict.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RenamePreview {
    /// The current filename.
    pub old: String,
    /// The proposed new filename after the rule is applied.
    pub new: String,
    /// Whether `new` is applicable, and if not, why.
    pub conflict: ConflictKind,
}

/// If `hay` case-insensitively starts with `lower_find` (already lowercased),
/// the number of bytes of `hay` that span the match, else `None`. Scans `hay`
/// char by char, lowercasing each, so it never indexes across the byte-length
/// difference between a string and its lowercase (which Unicode case-folding can
/// introduce, e.g. `İ` -> `i̇`).
fn ci_match_len(hay: &str, lower_find: &str) -> Option<usize> {
    if lower_find.is_empty() {
        return None;
    }
    let mut lowered = String::new();
    let mut consumed = 0;
    for ch in hay.chars() {
        lowered.extend(ch.to_lowercase());
        consumed += ch.len_utf8();
        if lowered.len() >= lower_find.len() {
            return lowered.starts_with(lower_find).then_some(consumed);
        }
    }
    // `hay` ran out before covering `lower_find`.
    lowered.starts_with(lower_find).then_some(consumed)
}

/// Case-insensitive literal replace of every occurrence of `find` in `haystack`.
/// UTF-8 safe across case-folding length changes (it never byte-indexes the
/// lowercased form against the original).
fn replace_ci(haystack: &str, find: &str, replace: &str) -> String {
    if find.is_empty() {
        return haystack.to_string();
    }
    let lower_find = find.to_lowercase();
    let mut out = String::with_capacity(haystack.len());
    let mut rest = haystack;
    while !rest.is_empty() {
        if let Some(matched) = ci_match_len(rest, &lower_find) {
            out.push_str(replace);
            rest = &rest[matched..];
        } else {
            let ch = rest.chars().next().unwrap();
            out.push(ch);
            rest = &rest[ch.len_utf8()..];
        }
    }
    out
}

/// Apply a case transform to a whole name.
fn apply_case(name: &str, case: CaseTransform) -> String {
    match case {
        CaseTransform::Lower => name.to_lowercase(),
        CaseTransform::Upper => name.to_uppercase(),
        CaseTransform::Title => name
            .split(' ')
            .map(|word| {
                let mut chars = word.chars();
                match chars.next() {
                    Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                    None => String::new(),
                }
            })
            .collect::<Vec<_>>()
            .join(" "),
    }
}

/// Apply numbering to a name using its sequence number.
fn apply_numbering(name: &str, num: &Numbering, seq: u64) -> String {
    let number = format!("{seq:0width$}", width = num.pad);
    if num.template.contains("{n}") {
        num.template
            .replace("{name}", name)
            .replace("{n}", &number)
    } else {
        // No `{n}` token: append the number so the rows stay distinct.
        format!("{}{}", num.template.replace("{name}", name), number)
    }
}

/// Whether a produced name is a usable filename.
fn is_valid_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains('/')
        && !name.contains('\0')
}

/// Plan a bulk rename: apply `rule` to each name in `names` (in order, the order
/// also drives the numbering sequence) and return a per-row preview with conflict
/// detection. Pure: no filesystem access.
pub fn plan_rename(names: &[String], rule: &RenameRule) -> Vec<RenamePreview> {
    // Stage 1-3: produce the proposed new name for each row.
    let news: Vec<String> = names
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let mut s = name.clone();
            if let Some(find) = rule.find.as_deref().filter(|f| !f.is_empty()) {
                s = if rule.find_case_insensitive {
                    replace_ci(&s, find, &rule.replace)
                } else {
                    s.replace(find, &rule.replace)
                };
            }
            if let Some(case) = rule.case {
                s = apply_case(&s, case);
            }
            if let Some(num) = &rule.numbering {
                let seq = num.start + (i as u64) * num.step;
                s = apply_numbering(&s, num, seq);
            }
            s
        })
        .collect();

    // Conflict pass: a final name appearing more than once is a duplicate (an
    // unchanged row's final name is its old name, so a rename colliding with a
    // file that stays put is caught too).
    let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for n in &news {
        *counts.entry(n.as_str()).or_insert(0) += 1;
    }

    names
        .iter()
        .zip(news.iter())
        .map(|(old, new)| {
            let conflict = if !is_valid_name(new) {
                ConflictKind::Invalid
            } else if counts.get(new.as_str()).copied().unwrap_or(0) > 1 {
                ConflictKind::Duplicate
            } else if new == old {
                ConflictKind::Unchanged
            } else {
                ConflictKind::None
            };
            RenamePreview {
                old: old.clone(),
                new: new.clone(),
                conflict,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn find_replace_is_literal_and_per_occurrence() {
        let rule = RenameRule {
            find: Some("IMG".to_string()),
            replace: "photo".to_string(),
            ..Default::default()
        };
        let out = plan_rename(&names(&["IMG_1.jpg", "IMG_IMG.png"]), &rule);
        assert_eq!(out[0].new, "photo_1.jpg");
        assert_eq!(out[1].new, "photo_photo.png");
        assert_eq!(out[0].conflict, ConflictKind::None);
    }

    #[test]
    fn case_insensitive_find_preserves_replacement_casing() {
        let rule = RenameRule {
            find: Some("img".to_string()),
            replace: "Photo".to_string(),
            find_case_insensitive: true,
            ..Default::default()
        };
        let out = plan_rename(&names(&["IMG_1.JPG"]), &rule);
        assert_eq!(out[0].new, "Photo_1.JPG");
    }

    #[test]
    fn case_insensitive_replace_is_unicode_safe() {
        // `İ` (U+0130) lowercases to a LONGER string than itself; the previous
        // byte-indexed matcher panicked on that misalignment. This must just work.
        let rule = RenameRule {
            find: Some("i".to_string()),
            replace: "X".to_string(),
            find_case_insensitive: true,
            ..Default::default()
        };
        let out = plan_rename(&names(&["\u{0130}MG.jpg"]), &rule);
        assert!(out[0].new.starts_with('X'), "matched case-insensitively, got {}", out[0].new);
    }

    #[test]
    fn case_transforms_apply_to_the_whole_name() {
        let lower = RenameRule {
            case: Some(CaseTransform::Lower),
            ..Default::default()
        };
        assert_eq!(plan_rename(&names(&["My File.TXT"]), &lower)[0].new, "my file.txt");
        let title = RenameRule {
            case: Some(CaseTransform::Title),
            ..Default::default()
        };
        assert_eq!(plan_rename(&names(&["my report.pdf"]), &title)[0].new, "My Report.pdf");
    }

    #[test]
    fn numbering_substitutes_name_and_padded_sequence() {
        let rule = RenameRule {
            numbering: Some(Numbering {
                template: "{name}-{n}".to_string(),
                start: 1,
                step: 1,
                pad: 3,
            }),
            ..Default::default()
        };
        let out = plan_rename(&names(&["a", "b", "c"]), &rule);
        assert_eq!(out[0].new, "a-001");
        assert_eq!(out[1].new, "b-002");
        assert_eq!(out[2].new, "c-003");
    }

    #[test]
    fn numbering_without_n_token_appends_the_number() {
        let rule = RenameRule {
            numbering: Some(Numbering {
                template: "shot".to_string(),
                start: 5,
                step: 5,
                pad: 2,
            }),
            ..Default::default()
        };
        let out = plan_rename(&names(&["x", "y"]), &rule);
        // No {name} either: a constant template + appended number stays distinct.
        assert_eq!(out[0].new, "shot05");
        assert_eq!(out[1].new, "shot10");
    }

    #[test]
    fn a_collision_between_two_rows_is_flagged_duplicate() {
        // Replacing the differing part with the same string collides.
        let rule = RenameRule {
            find: Some("1".to_string()),
            replace: "X".to_string(),
            ..Default::default()
        };
        // "a1" -> "aX", "a1" -> "aX" : both duplicate.
        let out = plan_rename(&names(&["a1", "a1"]), &rule);
        assert_eq!(out[0].conflict, ConflictKind::Duplicate);
        assert_eq!(out[1].conflict, ConflictKind::Duplicate);
    }

    #[test]
    fn a_rename_onto_an_unchanged_file_is_a_duplicate() {
        // Row 0 renames to "b"; row 1 ("b") is unchanged and stays "b": collision.
        let rule = RenameRule {
            find: Some("a".to_string()),
            replace: "b".to_string(),
            ..Default::default()
        };
        let out = plan_rename(&names(&["a", "b"]), &rule);
        assert_eq!(out[0].new, "b");
        assert_eq!(out[0].conflict, ConflictKind::Duplicate);
        assert_eq!(out[1].conflict, ConflictKind::Duplicate);
    }

    #[test]
    fn an_unchanged_row_is_flagged_unchanged() {
        let out = plan_rename(&names(&["keep.txt"]), &RenameRule::default());
        assert_eq!(out[0].conflict, ConflictKind::Unchanged);
    }

    #[test]
    fn a_name_turning_into_a_path_is_invalid() {
        let rule = RenameRule {
            find: Some("_".to_string()),
            replace: "/".to_string(),
            ..Default::default()
        };
        let out = plan_rename(&names(&["a_b"]), &rule);
        assert_eq!(out[0].new, "a/b");
        assert_eq!(out[0].conflict, ConflictKind::Invalid);
    }
}

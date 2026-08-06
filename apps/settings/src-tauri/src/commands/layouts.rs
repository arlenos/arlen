// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: GPL-3.0-only

//! The keyboard layouts the system knows, named in the user's language.
//!
//! The settings page carried nineteen layouts with hand-written English labels,
//! and a comment refusing to translate them: the descriptions belong to
//! `xkeyboard-config`, which ships translations of every one, and writing German
//! names for our nineteen would invent data with a canonical source while leaving
//! the other five hundred unavailable.
//!
//! So both halves come from the system. `evdev.lst` lists the layouts and their
//! variants with the English descriptions, and those descriptions are the message
//! ids of the `xkeyboard-config` gettext catalog, so translating one is a lookup
//! rather than a guess. A description with no translation comes back in English,
//! which is still its correct name.

use std::path::{Path, PathBuf};

use super::mo::load_catalog;

/// One selectable layout.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Layout {
    /// The xkb layout string, `de` or `de(nodeadkeys)`.
    pub value: String,
    /// The description in the requested language.
    pub label: String,
    /// The base layout a variant belongs to, so the page can group or filter.
    /// `None` for a base layout.
    pub parent: Option<String>,
}

/// A layout and its untranslated description, as the rules file states them.
#[derive(Debug, Clone, PartialEq)]
pub struct RulesEntry {
    pub value: String,
    pub description: String,
    pub parent: Option<String>,
}

/// Parse the layout and variant sections of an xkb rules list.
///
/// The format is sections introduced by `! name` and indented `key<space>value`
/// rows. A variant row's value is `parent: Description`, which is how a variant
/// carries the layout it belongs to.
///
/// Sections other than these two are skipped rather than parsed: `! model` and
/// `! option` use the same row shape, and taking them would put keyboard models
/// in a layout list.
pub fn parse_rules_list(text: &str) -> Vec<RulesEntry> {
    let mut out = Vec::new();
    let mut section = "";
    for line in text.lines() {
        if let Some(name) = line.strip_prefix('!') {
            section = name.trim();
            continue;
        }
        if section != "layout" && section != "variant" {
            continue;
        }
        let row = line.trim();
        if row.is_empty() {
            continue;
        }
        let Some((key, rest)) = row.split_once(char::is_whitespace) else { continue };
        let rest = rest.trim();
        if key.is_empty() || rest.is_empty() {
            continue;
        }
        if section == "layout" {
            out.push(RulesEntry {
                value: key.to_owned(),
                description: rest.to_owned(),
                parent: None,
            });
        } else if let Some((parent, description)) = rest.split_once(':') {
            let (parent, description) = (parent.trim(), description.trim());
            if parent.is_empty() || description.is_empty() {
                continue;
            }
            out.push(RulesEntry {
                value: format!("{parent}({key})"),
                description: description.to_owned(),
                parent: Some(parent.to_owned()),
            });
        }
    }
    out
}

/// Translate a parsed list. Separate from the parse so the join is testable
/// without a rules file, and from the read so it is testable without a system.
pub fn localise(entries: Vec<RulesEntry>, catalog: &super::mo::MoCatalog) -> Vec<Layout> {
    entries
        .into_iter()
        .map(|e| Layout {
            label: catalog.get(&e.description).to_owned(),
            value: e.value,
            parent: e.parent,
        })
        .collect()
}

/// Where the rules list lives, and where message catalogs live. Both are
/// arguments in the core so a test needs neither.
const RULES_PATHS: &[&str] =
    &["/usr/share/X11/xkb/rules/evdev.lst", "/usr/local/share/X11/xkb/rules/evdev.lst"];
const LOCALE_ROOT: &str = "/usr/share/locale";

fn read_rules(paths: &[PathBuf]) -> Option<String> {
    paths.iter().find_map(|p| std::fs::read_to_string(p).ok())
}

/// Every layout the system offers, named in `locale`.
///
/// An empty list is the honest answer on a system with no xkb rules installed,
/// and the page keeps whatever it had rather than showing an empty picker.
#[tauri::command]
pub async fn settings_keyboard_layouts(locale: String) -> Result<Vec<Layout>, String> {
    let paths: Vec<PathBuf> = RULES_PATHS.iter().map(PathBuf::from).collect();
    let Some(text) = read_rules(&paths) else { return Ok(Vec::new()) };
    let catalog = load_catalog(Path::new(LOCALE_ROOT), "xkeyboard-config", &locale);
    Ok(localise(parse_rules_list(&text), &catalog))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
! model
  pc105           Generic 105-key PC
! layout
  us              English (US)
  de              German
! variant
  nodeadkeys      de: German (no dead keys)
  dvorak          us: English (Dvorak)
! option
  grp:alt_shift_toggle  Alt+Shift
";

    #[test]
    fn layouts_and_variants_are_read_and_other_sections_are_not() {
        let entries = parse_rules_list(SAMPLE);
        let values: Vec<&str> = entries.iter().map(|e| e.value.as_str()).collect();
        assert_eq!(values, vec!["us", "de", "de(nodeadkeys)", "us(dvorak)"]);
        // A model and an option use the same row shape; taking them would put
        // "Generic 105-key PC" in a layout picker.
        assert!(entries.iter().all(|e| e.description != "Generic 105-key PC"));
        assert!(entries.iter().all(|e| !e.description.starts_with("Alt+")));
    }

    #[test]
    fn a_variant_carries_the_layout_it_belongs_to() {
        let entries = parse_rules_list(SAMPLE);
        let v = entries.iter().find(|e| e.value == "de(nodeadkeys)").unwrap();
        assert_eq!(v.parent.as_deref(), Some("de"));
        assert_eq!(v.description, "German (no dead keys)");
        let base = entries.iter().find(|e| e.value == "de").unwrap();
        assert_eq!(base.parent, None);
    }

    #[test]
    fn a_description_with_no_translation_keeps_its_own_name() {
        let entries = parse_rules_list(SAMPLE);
        let empty = super::super::mo::MoCatalog::default();
        let out = localise(entries, &empty);
        assert_eq!(out.iter().find(|l| l.value == "de").unwrap().label, "German");
    }

    #[test]
    fn a_malformed_row_is_dropped_without_taking_the_section_with_it() {
        let text = "! layout\n  us              English (US)\n  broken\n  de              German\n";
        let entries = parse_rules_list(text);
        assert_eq!(entries.len(), 2, "the row with no description is skipped");
        assert!(entries.iter().any(|e| e.value == "de"));
    }

    #[test]
    fn a_variant_row_without_a_parent_is_skipped_rather_than_guessed() {
        let text = "! variant\n  orphan          no colon here\n";
        assert!(parse_rules_list(text).is_empty());
    }
}

#[cfg(test)]
mod system_tests {
    use super::*;

    /// Against the real files, because the point of this module is that it reads
    /// the system rather than a table we wrote.
    #[test]
    fn the_real_rules_list_localises_into_german() {
        let paths: Vec<PathBuf> = RULES_PATHS.iter().map(PathBuf::from).collect();
        let Some(text) = read_rules(&paths) else {
            eprintln!("no xkb rules installed; skipping");
            return;
        };
        let entries = parse_rules_list(&text);
        assert!(entries.len() > 100, "the system knows far more than the nineteen we listed");

        let catalog = super::load_catalog(Path::new(LOCALE_ROOT), "xkeyboard-config", "de");
        if catalog.len() == 0 {
            eprintln!("no German xkeyboard-config catalog; skipping the translation half");
            return;
        }
        let out = localise(entries, &catalog);
        let de = out.iter().find(|l| l.value == "de").expect("German is a layout");
        assert_eq!(de.label, "Deutsch");
        let ndk = out.iter().find(|l| l.value == "de(nodeadkeys)").expect("a German variant");
        assert_eq!(ndk.label, "Deutsch (ohne Akzenttasten)");
    }
}

#[cfg(test)]
mod shape_tests {
    use super::*;

    /// Print what the page would receive, so the volume and the wording are a fact
    /// rather than an assumption when the picker is wired to it.
    #[test]
    fn report_the_shape_the_page_receives() {
        let paths: Vec<PathBuf> = RULES_PATHS.iter().map(PathBuf::from).collect();
        let Some(text) = read_rules(&paths) else { return };
        let catalog = super::load_catalog(Path::new(LOCALE_ROOT), "xkeyboard-config", "de");
        let entries = parse_rules_list(&text);
        // Compare against the ENGLISH description, not the label: looking the
        // translated label up again asks whether "Deutsch" is a message id, which
        // it is not, so every row came back "untranslated" the first time I ran
        // this. The count is the reason the probe exists, so it has to be right.
        let untranslated =
            entries.iter().filter(|e| catalog.get(&e.description) == e.description).count();
        let out = localise(entries, &catalog);
        let bases = out.iter().filter(|l| l.parent.is_none()).count();
        eprintln!("layouts: {} total, {bases} base, {} variants", out.len(), out.len() - bases);
        eprintln!("still English after lookup: {untranslated}");
        for v in ["us", "de", "de(nodeadkeys)", "ch", "cz", "jp"] {
            if let Some(l) = out.iter().find(|l| l.value == v) {
                eprintln!("  {v:16} {}", l.label);
            }
        }
    }
}

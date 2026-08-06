// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: GPL-3.0-only

//! Read a gettext `.mo` catalog.
//!
//! Some of the strings the settings app displays are not ours to translate. The
//! keyboard layout list is the clear case: "German (no dead keys)" belongs to
//! `xkeyboard-config`, which ships its own translations of every layout
//! description, and hand-writing a German name for each in our catalog would be
//! inventing data that already has a canonical source - and inventing it for the
//! nineteen layouts we happened to list, out of the hundreds the system knows.
//!
//! The obvious route is `libintl`, and it is the wrong one here. `dgettext`
//! resolves against the PROCESS locale, set once by `setlocale`, while the locale
//! this app renders in is a store the user can change without restarting. Reading
//! the catalog for a named language keeps the two independent, and a `.mo` file is
//! a simple enough format to read directly: a header, two tables of
//! (length, offset) pairs, and the strings.
//!
//! Only what the lookup needs is implemented. Plural forms and message contexts
//! are parsed far enough to be ignored correctly rather than mistaken for
//! ordinary entries.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Both byte orders a `.mo` file may declare, as they appear in the first word.
const MAGIC_LE: u32 = 0x9504_12de;
const MAGIC_BE: u32 = 0xde12_0495;

/// A parsed catalog: original string to translation.
#[derive(Debug, Default, Clone)]
pub struct MoCatalog {
    entries: HashMap<String, String>,
}

impl MoCatalog {
    /// The translation of `msgid`, or `msgid` itself.
    ///
    /// Falling back to the original is what gettext does and what the caller
    /// wants: an untranslated layout description is still the right label, just
    /// in English.
    pub fn get<'a>(&'a self, msgid: &'a str) -> &'a str {
        self.entries.get(msgid).map(String::as_str).unwrap_or(msgid)
    }

    /// How many entries were read, for tests and for deciding a file was worth
    /// reading at all.
    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

/// Read four bytes at `at` in the file's declared order.
fn word(bytes: &[u8], at: usize, big_endian: bool) -> Option<u32> {
    let raw: [u8; 4] = bytes.get(at..at + 4)?.try_into().ok()?;
    Some(if big_endian { u32::from_be_bytes(raw) } else { u32::from_le_bytes(raw) })
}

/// One (length, offset) pair from a string table, as a slice of the file.
fn string_at(bytes: &[u8], table: usize, index: usize, big_endian: bool) -> Option<&[u8]> {
    let entry = table.checked_add(index.checked_mul(8)?)?;
    let len = word(bytes, entry, big_endian)? as usize;
    let off = word(bytes, entry + 4, big_endian)? as usize;
    bytes.get(off..off.checked_add(len)?)
}

/// Parse a `.mo` file's bytes.
///
/// Returns `None` for anything that is not a `.mo` file. A malformed one is not
/// worth reporting: the caller's fallback is the original English string, which
/// is a correct label, so failing quietly degrades exactly as far as it should.
pub fn parse_mo(bytes: &[u8]) -> Option<MoCatalog> {
    let magic = word(bytes, 0, false)?;
    let big_endian = match magic {
        MAGIC_LE => false,
        MAGIC_BE => true,
        _ => return None,
    };

    let count = word(bytes, 8, big_endian)? as usize;
    let originals = word(bytes, 12, big_endian)? as usize;
    let translations = word(bytes, 16, big_endian)? as usize;

    let mut entries = HashMap::with_capacity(count);
    for i in 0..count {
        let Some(id) = string_at(bytes, originals, i, big_endian) else { continue };
        let Some(text) = string_at(bytes, translations, i, big_endian) else { continue };

        // A context-qualified id is `context\u{4}msgid` and a plural id is
        // `singular\0plural`. Neither is a plain lookup, and taking the bytes
        // before the separator would silently answer a different question, so
        // both are skipped. The empty id is the header, not a message.
        if id.is_empty() || id.contains(&0x04) || id.contains(&0) {
            continue;
        }
        let (Ok(id), Ok(text)) = (std::str::from_utf8(id), std::str::from_utf8(text)) else {
            continue;
        };
        if !text.is_empty() {
            entries.insert(id.to_owned(), text.to_owned());
        }
    }
    Some(MoCatalog { entries })
}

/// Where a domain's catalog lives for a language, most specific first.
///
/// `de-AT` should try Austrian German before German, matching the fallback chain
/// the message catalogs use, so a user does not get English from a regional tag
/// whose base language is installed.
pub fn catalog_paths(root: &Path, domain: &str, locale: &str) -> Vec<PathBuf> {
    let mut langs = Vec::new();
    let normalised = locale.replace('-', "_");
    langs.push(normalised.clone());
    if let Some((base, _)) = normalised.split_once('_') {
        langs.push(base.to_owned());
    }
    langs
        .into_iter()
        .map(|lang| root.join(lang).join("LC_MESSAGES").join(format!("{domain}.mo")))
        .collect()
}

/// Load a domain's catalog for a locale, or an empty one.
pub fn load_catalog(root: &Path, domain: &str, locale: &str) -> MoCatalog {
    catalog_paths(root, domain, locale)
        .into_iter()
        .find_map(|p| std::fs::read(p).ok().and_then(|b| parse_mo(&b)))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a little-endian `.mo` in memory from id/text pairs.
    fn build_mo(pairs: &[(&[u8], &[u8])]) -> Vec<u8> {
        let count = pairs.len();
        let header = 28usize;
        let orig_table = header;
        let trans_table = orig_table + count * 8;
        let mut strings = Vec::new();
        let mut orig = Vec::new();
        let mut trans = Vec::new();
        let base = trans_table + count * 8;
        for (id, text) in pairs {
            orig.push((id.len() as u32, (base + strings.len()) as u32));
            strings.extend_from_slice(id);
            strings.push(0);
        }
        for (_, text) in pairs {
            trans.push((text.len() as u32, (base + strings.len()) as u32));
            strings.extend_from_slice(text);
            strings.push(0);
        }
        let mut out = Vec::new();
        out.extend_from_slice(&MAGIC_LE.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes()); // revision
        out.extend_from_slice(&(count as u32).to_le_bytes());
        out.extend_from_slice(&(orig_table as u32).to_le_bytes());
        out.extend_from_slice(&(trans_table as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes()); // hash size
        out.extend_from_slice(&0u32.to_le_bytes()); // hash offset
        for (len, off) in orig.into_iter().chain(trans) {
            out.extend_from_slice(&len.to_le_bytes());
            out.extend_from_slice(&off.to_le_bytes());
        }
        out.extend_from_slice(&strings);
        out
    }

    #[test]
    fn a_translation_is_found_and_a_missing_one_falls_back() {
        let mo = build_mo(&[(b"German", "Deutsch".as_bytes())]);
        let cat = parse_mo(&mo).expect("well-formed");
        assert_eq!(cat.get("German"), "Deutsch");
        assert_eq!(cat.get("Klingon"), "Klingon", "an untranslated label is still correct");
    }

    #[test]
    fn the_header_and_the_shapes_that_are_not_plain_lookups_are_skipped() {
        let mo = build_mo(&[
            (b"", b"Content-Type: text/plain\n"),
            (b"keyboard\x04German", "Deutsch (Tastatur)".as_bytes()),
            (b"one file\0many files", "eine Datei\0viele Dateien".as_bytes()),
            (b"German", "Deutsch".as_bytes()),
        ]);
        let cat = parse_mo(&mo).expect("well-formed");
        assert_eq!(cat.len(), 1, "only the plain entry counts");
        assert_eq!(cat.get("German"), "Deutsch");
        // The context-qualified entry must not answer for the bare id.
        assert_eq!(cat.get("keyboard\u{4}German"), "keyboard\u{4}German");
    }

    #[test]
    fn a_file_that_is_not_a_catalog_is_refused_rather_than_guessed_at() {
        assert!(parse_mo(b"").is_none());
        assert!(parse_mo(b"not a mo file at all").is_none());
        // Truncated after a valid magic: the tables cannot be read, so nothing is
        // returned rather than a partial catalog of whatever happened to parse.
        let mut short = MAGIC_LE.to_le_bytes().to_vec();
        short.extend_from_slice(&[0u8; 4]);
        assert!(parse_mo(&short).is_none());
    }

    #[test]
    fn an_offset_past_the_end_drops_that_entry_and_keeps_the_rest() {
        let mut mo = build_mo(&[(b"German", "Deutsch".as_bytes()), (b"French", "Franzosisch".as_bytes())]);
        // Point the first original at an impossible offset.
        let orig_table = 28usize;
        mo[orig_table + 4..orig_table + 8].copy_from_slice(&u32::MAX.to_le_bytes());
        let cat = parse_mo(&mo).expect("the file is still a catalog");
        assert_eq!(cat.get("French"), "Franzosisch");
        assert_eq!(cat.get("German"), "German", "the broken entry is simply absent");
    }

    #[test]
    fn a_regional_locale_tries_its_own_catalog_before_the_base_language() {
        let paths = catalog_paths(Path::new("/usr/share/locale"), "xkeyboard-config", "de-AT");
        assert_eq!(paths.len(), 2);
        assert!(paths[0].ends_with("de_AT/LC_MESSAGES/xkeyboard-config.mo"));
        assert!(paths[1].ends_with("de/LC_MESSAGES/xkeyboard-config.mo"));
    }
}

#[cfg(test)]
mod system_tests {
    use super::*;

    /// The synthetic tests prove the parser against files this module wrote. This
    /// one proves it against a file it did not: the real `xkeyboard-config`
    /// catalog, whose layout descriptions are the reason any of this exists.
    ///
    /// Skipped rather than failed where the catalog is not installed, because a
    /// build host without German locale data is not a defect in the parser.
    #[test]
    fn the_real_xkeyboard_catalog_reads() {
        let root = Path::new("/usr/share/locale");
        let cat = load_catalog(root, "xkeyboard-config", "de");
        if cat.len() == 0 {
            eprintln!("no German xkeyboard-config catalog installed; skipping");
            return;
        }
        assert_eq!(cat.get("German"), "Deutsch");
        assert_eq!(cat.get("German (no dead keys)"), "Deutsch (ohne Akzenttasten)");
        // A description the catalog does not translate still comes back usable.
        assert!(!cat.get("Swiss").is_empty());
    }
}

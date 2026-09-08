//! Resolving a module's own message keys, in the user's locale.
//!
//! A module's results were untranslatable by construction: the wire record
//! carried `title` and `description` as literals and nothing else, so a module
//! shipped one language and every user read it. The shell's own `SearchResult`
//! has carried `title_key`/`description_key` for exactly this reason.
//!
//! **The host resolves, never the guest.** A module ships a catalogue beside its
//! manifest and names a key; which locale the person reads is not something the
//! module learns, asks for, or decides. That is why there is no `locale()` host
//! call and why this module takes the chosen locale from the same place every
//! other Arlen surface does.
//!
//! **The literal stays required.** A module with no catalogue, a catalogue with
//! no entry for this locale, or a key nobody wrote all fall back to the string
//! the module already had to provide. A single-language module stays a legal
//! module, which is the difference between a capability and an obligation.

use std::path::Path;

use arlen_i18n::{Args, Catalog, Locale, Localizer};

/// A module's catalogue chain, or nothing if it ships none.
///
/// Files live at `<module dir>/i18n/<locale>.json`, the same shape the apps use,
/// so `Catalog::from_json` reads them without a second format existing.
pub fn load(module_dir: &Path, chosen: &str) -> Option<Localizer> {
    let dir = module_dir.join("i18n");
    let mut chain: Vec<(Locale, Catalog)> = Vec::new();
    let mut seen: Vec<&str> = Vec::new();
    // The chosen locale first, then the source locale as the fallback rung. A
    // module that ships only `en` still answers for a German reader, with
    // English, which is what falling back means.
    for tag in [chosen, arlen_i18n::SOURCE_LOCALE] {
        // The chosen locale may BE the source one, and a chain listing it twice
        // would read the same catalogue twice for nothing.
        if seen.contains(&tag) {
            continue;
        }
        seen.push(tag);
        let Ok(text) = std::fs::read_to_string(dir.join(format!("{tag}.json"))) else {
            continue;
        };
        // Parse errors are per-entry: a catalogue with one broken message still
        // answers for the rest rather than being discarded whole.
        if let Ok((catalog, broken)) = Catalog::from_json(&text) {
            for (key, err) in broken {
                tracing::warn!(locale = tag, key = %key, "module catalogue entry did not parse: {err}");
            }
            let Ok(locale) = tag.parse::<Locale>() else { continue };
            chain.push((locale, catalog));
        }
    }
    (!chain.is_empty()).then(|| Localizer::new(chain))
}

/// The text to show: the key resolved, or the literal the module supplied.
pub fn resolve(
    localizer: Option<&Localizer>,
    key: Option<&str>,
    args_json: Option<&str>,
    literal: &str,
) -> String {
    let (Some(localizer), Some(key)) = (localizer, key) else {
        return literal.to_string();
    };
    if !localizer.has(key) {
        return literal.to_string();
    }
    localizer.localize(key, &args(args_json))
}

/// The JSON object a module sent, as formatter arguments.
///
/// Anything that is not an object of scalars is ignored rather than refused: the
/// worst case for a malformed `args` is a message that renders its placeholder,
/// and refusing the whole result would lose the module's literal too.
fn args(json: Option<&str>) -> Args {
    let mut out = Args::new();
    let Some(text) = json else { return out };
    let Ok(serde_json::Value::Object(map)) = serde_json::from_str::<serde_json::Value>(text) else {
        return out;
    };
    for (k, v) in map {
        let value = match v {
            serde_json::Value::String(s) => arlen_i18n::ArgValue::Text(s),
            // Integer first, because it is the plural operand and 3 formatted as
            // a float is "3.0" in a sentence that meant "3".
            serde_json::Value::Number(n) => match (n.as_i64(), n.as_f64()) {
                (Some(i), _) => arlen_i18n::ArgValue::Integer(i),
                (None, Some(f)) => arlen_i18n::ArgValue::Float(f),
                _ => continue,
            },
            _ => continue,
        };
        out.insert(k, value);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn module_with(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("i18n")).unwrap();
        for (name, body) in files {
            std::fs::write(dir.path().join("i18n").join(name), body).unwrap();
        }
        dir
    }

    #[test]
    fn a_key_resolves_in_the_chosen_locale() {
        let dir = module_with(&[
            ("en.json", r#"{"m.hit": "Heart"}"#),
            ("de.json", r#"{"m.hit": "Herz"}"#),
        ]);
        let loc = load(dir.path(), "de").expect("a catalogue");
        assert_eq!(resolve(Some(&loc), Some("m.hit"), None, "Heart"), "Herz");
    }

    #[test]
    fn a_locale_the_module_does_not_ship_falls_back_to_the_source() {
        let dir = module_with(&[("en.json", r#"{"m.hit": "Heart"}"#)]);
        let loc = load(dir.path(), "de").expect("the source catalogue is still a chain");
        assert_eq!(resolve(Some(&loc), Some("m.hit"), None, "literal"), "Heart");
    }

    #[test]
    fn a_module_with_no_catalogue_keeps_its_literal() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load(dir.path(), "de").is_none());
        assert_eq!(resolve(None, Some("m.hit"), None, "Heart"), "Heart");
    }

    #[test]
    fn a_key_nobody_wrote_keeps_the_literal_rather_than_showing_the_key() {
        let dir = module_with(&[("en.json", r#"{"m.other": "Something"}"#)]);
        let loc = load(dir.path(), "en").expect("a catalogue");
        assert_eq!(resolve(Some(&loc), Some("m.hit"), None, "Heart"), "Heart");
    }

    #[test]
    fn arguments_reach_the_message() {
        let dir = module_with(&[("en.json", r#"{"m.count": "{$n} hits"}"#)]);
        let loc = load(dir.path(), "en").expect("a catalogue");
        assert_eq!(
            resolve(Some(&loc), Some("m.count"), Some(r#"{"n": 3}"#), "literal"),
            "3 hits"
        );
    }

    #[test]
    fn malformed_arguments_do_not_lose_the_result() {
        let dir = module_with(&[("en.json", r#"{"m.hit": "Heart"}"#)]);
        let loc = load(dir.path(), "en").expect("a catalogue");
        assert_eq!(resolve(Some(&loc), Some("m.hit"), Some("not json"), "literal"), "Heart");
    }
}

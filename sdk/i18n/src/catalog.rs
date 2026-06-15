//! The MF2 catalog + locale fallback (i18n-plan.md I18N-R1).
//!
//! A [`Catalog`] is one locale's messages: a map from key to parsed [`Message`].
//! A [`Localizer`] holds catalogs along a fallback chain (e.g. `de-AT → de → en`)
//! and resolves a key to a rendered string: the first catalog in the chain that
//! has the key formats it; if no catalog has it, the key itself is returned (a
//! visible, debuggable fallback, never a panic).
//!
//! The catalog CORE is format-agnostic - it takes `key -> MF2-source` pairs, so
//! the on-disk serialization (a JSON map here, the [`Catalog::from_json`]
//! convenience) stays decoupled from the Svelte side's loader; both stacks read
//! the same MF2 messages, which are the durable asset.

use std::collections::BTreeMap;

use icu_locale_core::Locale;

use crate::format::{format, Args};
use crate::model::Message;
use crate::parse::{parse_message, ParseError};

/// One locale's parsed messages.
#[derive(Debug, Clone, Default)]
pub struct Catalog {
    messages: BTreeMap<String, Message>,
}

impl Catalog {
    /// Build a catalog from `key -> MF2-source` entries. Returns the catalog
    /// with every message that parsed, plus the per-key parse errors for the
    /// ones that did not (so one malformed message degrades to a key fallback
    /// rather than failing the whole catalog).
    pub fn from_entries(
        entries: impl IntoIterator<Item = (String, String)>,
    ) -> (Self, Vec<(String, ParseError)>) {
        let mut messages = BTreeMap::new();
        let mut errors = Vec::new();
        for (key, source) in entries {
            match parse_message(&source) {
                Ok(m) => {
                    messages.insert(key, m);
                }
                Err(e) => errors.push((key, e)),
            }
        }
        (Self { messages }, errors)
    }

    /// Build a catalog from a JSON object of `{"key": "MF2 source", ...}`.
    /// Returns the catalog + per-key parse errors. A non-object JSON, or a
    /// non-string value, is a [`CatalogError`].
    pub fn from_json(json: &str) -> Result<(Self, Vec<(String, ParseError)>), CatalogError> {
        let value: serde_json::Value =
            serde_json::from_str(json).map_err(|e| CatalogError(format!("invalid JSON: {e}")))?;
        let obj = value
            .as_object()
            .ok_or_else(|| CatalogError("catalog JSON must be an object".into()))?;
        let mut entries = Vec::with_capacity(obj.len());
        for (k, v) in obj {
            let s = v
                .as_str()
                .ok_or_else(|| CatalogError(format!("message `{k}` must be a string")))?;
            entries.push((k.clone(), s.to_string()));
        }
        Ok(Self::from_entries(entries))
    }

    /// The parsed message for `key`, if present.
    pub fn get(&self, key: &str) -> Option<&Message> {
        self.messages.get(key)
    }

    /// The number of messages in the catalog.
    pub fn len(&self) -> usize {
        self.messages.len()
    }

    /// Whether the catalog is empty.
    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }
}

/// A catalog that failed to load (bad serialization). Distinct from a per-message
/// parse error, which degrades gracefully rather than failing the catalog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogError(pub String);

impl std::fmt::Display for CatalogError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "catalog error: {}", self.0)
    }
}

impl std::error::Error for CatalogError {}

/// A localizer: an ordered fallback chain of `(locale, catalog)`. `localize`
/// resolves a key against the chain and formats it in that catalog's locale.
pub struct Localizer {
    chain: Vec<(Locale, Catalog)>,
}

impl Localizer {
    /// Build a localizer from a fallback chain, most-specific locale first
    /// (e.g. `[(de-AT, …), (de, …), (en, …)]`). The last entry should be the
    /// source locale so every key resolves there at worst.
    pub fn new(chain: Vec<(Locale, Catalog)>) -> Self {
        Self { chain }
    }

    /// Localize `key` with `args`: the first catalog in the chain that has the
    /// key formats it in that catalog's locale; if none has it, the key itself
    /// is returned (a visible fallback - a missing translation shows the key,
    /// never an empty string or a panic).
    pub fn localize(&self, key: &str, args: &Args) -> String {
        for (locale, catalog) in &self.chain {
            if let Some(message) = catalog.get(key) {
                return format(message, locale, args);
            }
        }
        key.to_string()
    }

    /// Whether some catalog in the chain has `key`.
    pub fn has(&self, key: &str) -> bool {
        self.chain.iter().any(|(_, c)| c.get(key).is_some())
    }
}

/// The locale fallback chain from `requested` down to `source`.
///
/// Strips subtags most-specific first: `de-AT → de → en` (the source). The
/// source locale is always the final entry (so a key authored in the source
/// always resolves), and duplicates are removed while preserving order. Example:
/// `fallback_chain("de-AT", "en") == ["de-AT", "de", "en"]`.
pub fn fallback_chain(requested: &Locale, source: &Locale) -> Vec<Locale> {
    let mut chain: Vec<Locale> = Vec::new();
    // The requested locale as given.
    chain.push(requested.clone());
    // The language-only form (the subtag before the first `-`), if the request
    // carried a region/script/variant. Done on the string form to avoid coupling
    // to the locale struct's subtag fields.
    let req_str = requested.to_string();
    if let Some(lang) = req_str.split('-').next() {
        if lang != req_str {
            if let Ok(lang_only) = lang.parse::<Locale>() {
                if !chain.contains(&lang_only) {
                    chain.push(lang_only);
                }
            }
        }
    }
    // The source locale, last, as the guaranteed floor.
    if !chain.contains(source) {
        chain.push(source.clone());
    }
    chain
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::ArgValue;

    fn loc(s: &str) -> Locale {
        s.parse().unwrap()
    }

    fn args(pairs: &[(&str, ArgValue)]) -> Args {
        pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
    }

    #[test]
    fn from_json_parses_messages_and_reports_bad_ones() {
        let json = r#"{"greeting": "Hello, {$name}!", "broken": "{$unclosed", "plain": "hi"}"#;
        let (cat, errors) = Catalog::from_json(json).unwrap();
        assert!(cat.get("greeting").is_some());
        assert!(cat.get("plain").is_some());
        assert!(cat.get("broken").is_none(), "the malformed message is dropped");
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].0, "broken");
    }

    #[test]
    fn non_object_json_is_a_catalog_error() {
        assert!(Catalog::from_json("[1,2,3]").is_err());
        assert!(Catalog::from_json("not json").is_err());
        assert!(Catalog::from_json(r#"{"k": 5}"#).is_err(), "non-string value");
    }

    #[test]
    fn fallback_chain_strips_to_language_then_source() {
        assert_eq!(
            fallback_chain(&loc("de-AT"), &loc("en")),
            vec![loc("de-AT"), loc("de"), loc("en")]
        );
        // Already language-only: just itself + source.
        assert_eq!(fallback_chain(&loc("fr"), &loc("en")), vec![loc("fr"), loc("en")]);
        // Requesting the source: no duplicate.
        assert_eq!(fallback_chain(&loc("en"), &loc("en")), vec![loc("en")]);
    }

    #[test]
    fn localize_resolves_through_the_fallback_chain() {
        let (de, _) = Catalog::from_json(r#"{"greeting": "Hallo, {$name}!"}"#).unwrap();
        let (en, _) = Catalog::from_json(
            r#"{"greeting": "Hello, {$name}!", "bye": "Goodbye!"}"#,
        )
        .unwrap();
        let l = Localizer::new(vec![(loc("de"), de), (loc("en"), en)]);
        // `greeting` resolves in de (the more specific catalog).
        assert_eq!(l.localize("greeting", &args(&[("name", ArgValue::Text("Tim".into()))])), "Hallo, Tim!");
        // `bye` is missing in de, falls back to en.
        assert_eq!(l.localize("bye", &args(&[])), "Goodbye!");
        // An unknown key falls back to the key itself (visible, debuggable).
        assert_eq!(l.localize("nope", &args(&[])), "nope");
    }

    #[test]
    fn localize_formats_plurals_in_the_resolved_locale() {
        let src = ".input {$count :number}\n.match $count\none {{{$count} item}}\n* {{{$count} items}}";
        let json = format!("{{\"items\": {}}}", serde_json::to_string(src).unwrap());
        let (en, errs) = Catalog::from_json(&json).unwrap();
        assert!(errs.is_empty());
        let l = Localizer::new(vec![(loc("en"), en)]);
        assert_eq!(l.localize("items", &args(&[("count", ArgValue::Integer(1))])), "1 item");
        assert_eq!(l.localize("items", &args(&[("count", ArgValue::Integer(7))])), "7 items");
    }
}

//! The format-agnostic value model the [`FormatHandler`](crate::FormatHandler)
//! trait speaks: a config addressed by a single dotted key-path
//! (`"browser.startup.homepage"`), each leaf a typed scalar.
//!
//! The adapter schema (foundation integration-packages plan) addresses every
//! setting by one dotted `key` string and types it string/bool/int/float/enum,
//! all of which reduce to the scalars here. Structured and array values are out
//! of scope for v1; a handler that meets a non-scalar existing value at a
//! modelled path reports it as [`ConfigValue::Opaque`] so an edit refuses rather
//! than flattening it. Each handler maps the one dotted path onto its format's
//! native nesting (TOML/JSON tables, INI `section.key`, a bare `prefs.js` key),
//! so the trait surface stays format-independent.

use serde::{Deserialize, Serialize};

/// A scalar config value. Every adapter `type` (string/bool/int/float/enum)
/// reduces to one of these; an `enum` is carried as its [`ConfigValue::String`]
/// member.
///
/// [`ConfigValue::Opaque`] is never produced by a caller setting a value: it is
/// only what a handler reports when an *existing* value at a modelled path is a
/// structure or array (not a scalar). The self-check and edit paths treat an
/// Opaque target as unsettable, so a structured value can never be silently
/// flattened into a scalar.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum ConfigValue {
    /// A text value (also how an `enum` member is carried).
    String(String),
    /// A boolean.
    Bool(bool),
    /// A signed integer.
    Int(i64),
    /// A floating-point number.
    Float(f64),
    /// An existing value that is not a scalar (an object/array), or a scalar a
    /// handler could not faithfully classify. Unsettable: an edit against an
    /// Opaque target is refused rather than flattening it.
    Opaque,
}

impl ConfigValue {
    /// Whether two values are equal for the purpose of the self-check's
    /// "the edit took" assertion. Floats are compared bitwise so a re-parsed
    /// `Float` neither spuriously fails (a raw `f64 ==` would mis-handle the
    /// `-0.0`/`NaN` corners) nor spuriously passes. Every other variant is a
    /// straightforward structural compare.
    ///
    /// This is deliberately distinct from `PartialEq`: the trait's `PartialEq`
    /// follows `f64`'s IEEE semantics (so two `NaN`s differ), while the
    /// self-check wants "is this the same value I wrote", which is a bit-pattern
    /// question. A handler serializes a `Float` to text and the re-parse must
    /// reproduce the same bits.
    pub fn same_value(&self, other: &ConfigValue) -> bool {
        match (self, other) {
            (ConfigValue::Float(a), ConfigValue::Float(b)) => a.to_bits() == b.to_bits(),
            _ => self == other,
        }
    }
}

/// A dotted key-path identifying one setting (`"browser.startup.homepage"`).
/// Kept as the verbatim string the adapter schema carries; a handler splits it
/// on `.` only where its format nests (TOML/JSON), and treats it as one opaque
/// key where the format's own keys already contain dots (Firefox `prefs.js`).
pub type KeyPath = String;

/// A read snapshot of a config document: every modelled key-path paired with its
/// value, in document order.
///
/// Ordering is preserved so the self-check can compare two snapshots positionally
/// where useful and so a caller can display settings in file order. `read`
/// produces this; the edit functions ([`FormatHandler::set`](crate::FormatHandler::set)
/// and [`remove`](crate::FormatHandler::remove)) operate on *text*, not on this
/// model, because format-preservation needs the original bytes (a model -> text
/// serialize would drop comments and whitespace).
#[derive(Debug, Clone, PartialEq)]
pub struct ConfigModel {
    /// The modelled (key-path, value) pairs in document order.
    entries: Vec<(KeyPath, ConfigValue)>,
}

impl ConfigModel {
    /// Build a model from an ordered list of (key-path, value) pairs.
    pub fn from_entries(entries: Vec<(KeyPath, ConfigValue)>) -> Self {
        ConfigModel { entries }
    }

    /// The modelled pairs in document order.
    pub fn entries(&self) -> &[(KeyPath, ConfigValue)] {
        &self.entries
    }

    /// The value at `key`, or `None` if the key is not modelled.
    ///
    /// On the (malformed) chance a document carries the same key-path twice, the
    /// first occurrence wins, matching document order.
    pub fn get(&self, key: &str) -> Option<&ConfigValue> {
        self.entries
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v)
    }

    /// The number of modelled key-paths.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the model has no modelled key-paths.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_value_floats_compare_bitwise_not_by_ieee() {
        // The self-check asks "is this the same bits I wrote", which differs from
        // `f64`'s IEEE `==` at exactly two corners (the documented reason
        // `same_value` exists): two NaNs are the same value (PartialEq says they
        // differ), and +0.0 / -0.0 are different values (PartialEq says equal).
        let nan = ConfigValue::Float(f64::NAN);
        assert!(nan.same_value(&ConfigValue::Float(f64::NAN)), "NaN is the same value as NaN");
        assert_ne!(nan, ConfigValue::Float(f64::NAN), "PartialEq still follows IEEE (NaN != NaN)");

        let pos0 = ConfigValue::Float(0.0);
        let neg0 = ConfigValue::Float(-0.0);
        assert!(!pos0.same_value(&neg0), "+0.0 and -0.0 are different bit patterns");
        assert_eq!(pos0, neg0, "PartialEq still follows IEEE (+0.0 == -0.0)");

        assert!(ConfigValue::Float(1.5).same_value(&ConfigValue::Float(1.5)));
    }

    #[test]
    fn same_value_non_float_variants_compare_structurally() {
        assert!(ConfigValue::String("x".into()).same_value(&ConfigValue::String("x".into())));
        assert!(!ConfigValue::String("x".into()).same_value(&ConfigValue::String("y".into())));
        assert!(ConfigValue::Bool(true).same_value(&ConfigValue::Bool(true)));
        assert!(ConfigValue::Int(7).same_value(&ConfigValue::Int(7)));
        // Across variants is never the same value.
        assert!(!ConfigValue::Int(1).same_value(&ConfigValue::String("1".into())));
        assert!(!ConfigValue::Bool(true).same_value(&ConfigValue::Int(1)));
        // Opaque (an unsettable structured target) is the same only as Opaque.
        assert!(ConfigValue::Opaque.same_value(&ConfigValue::Opaque));
        assert!(!ConfigValue::Opaque.same_value(&ConfigValue::Int(0)));
    }

    fn model() -> ConfigModel {
        ConfigModel::from_entries(vec![
            ("a.one".into(), ConfigValue::Int(1)),
            ("a.two".into(), ConfigValue::Bool(false)),
            ("a.one".into(), ConfigValue::Int(2)), // a duplicate key-path
        ])
    }

    #[test]
    fn get_returns_the_first_occurrence_and_none_for_missing() {
        let m = model();
        assert_eq!(m.get("a.two"), Some(&ConfigValue::Bool(false)));
        // First occurrence wins for a (malformed) duplicate key-path.
        assert_eq!(m.get("a.one"), Some(&ConfigValue::Int(1)));
        assert_eq!(m.get("absent"), None);
    }

    #[test]
    fn entries_preserve_document_order_and_count() {
        let m = model();
        assert_eq!(m.len(), 3, "all entries kept, duplicates included");
        assert!(!m.is_empty());
        let keys: Vec<&str> = m.entries().iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, ["a.one", "a.two", "a.one"], "order is preserved verbatim");
    }

    #[test]
    fn an_empty_model_is_empty() {
        let m = ConfigModel::from_entries(vec![]);
        assert!(m.is_empty());
        assert_eq!(m.len(), 0);
        assert_eq!(m.get("anything"), None);
    }
}

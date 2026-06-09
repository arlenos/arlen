//! The injection-safe primitives for the structured typed read op (RS-R2).
//!
//! RS-R2 is the only bypass-proof read path for sensitive labels: the daemon owns
//! the entire query shape (the caller never supplies query text), so a value can
//! never smuggle clause structure and a field can never launder past a name-keyed
//! filter. These are the minimal building blocks - a typed value, its Cypher-literal
//! encoder, and the identifier validator - reimplemented daemon-side so the graph
//! daemon stays dependency-free (it must not pull the AI layer in). The shape
//! mirrors the typed graph-query DSL the AI layer already uses; this is the
//! daemon-local, owner-enforcing variant.
//!
//! Wired into the `0x08` read branch by a later RS-R2 commit; the tests exercise
//! these primitives now.
#![allow(dead_code)]

use serde::Deserialize;

/// A typed scalar a caller may compare a field against. Untagged so the wire form
/// is the bare JSON value (`true`, `42`, `"text"`); the daemon never accepts query
/// text, only these typed values.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum TypedValue {
    /// A boolean.
    Bool(bool),
    /// An integer (timestamps are integers).
    Int(i64),
    /// A text value.
    Text(String),
}

/// Encode a typed value as a Cypher literal. Text becomes a single-quoted string
/// with backslash, quote and tab/newline/carriage-return escaped; any other control
/// character is REJECTED (`None`) rather than emitted, so a value cannot smuggle
/// structure into the query. Bool/Int render directly.
pub fn encode_literal(value: &TypedValue) -> Option<String> {
    match value {
        TypedValue::Bool(b) => Some(if *b { "true".into() } else { "false".into() }),
        TypedValue::Int(n) => Some(n.to_string()),
        TypedValue::Text(s) => {
            let mut out = String::with_capacity(s.len() + 2);
            out.push('\'');
            for ch in s.chars() {
                match ch {
                    '\\' => out.push_str("\\\\"),
                    '\'' => out.push_str("\\'"),
                    '\n' => out.push_str("\\n"),
                    '\r' => out.push_str("\\r"),
                    '\t' => out.push_str("\\t"),
                    c if (c as u32) < 0x20 => return None,
                    c => out.push(c),
                }
            }
            out.push('\'');
            Some(out)
        }
    }
}

/// Whether `s` is a safe Cypher identifier: non-empty, at most 64 chars, an ASCII
/// letter or `_` first, then ASCII alphanumerics or `_`. A label or field name that
/// fails this is refused (not escaped), so only known-shape identifiers are
/// interpolated into the built query.
pub fn is_valid_identifier(s: &str) -> bool {
    if s.is_empty() || s.len() > 64 {
        return false;
    }
    let mut chars = s.chars();
    let first = chars.next().expect("non-empty");
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_scalars() {
        assert_eq!(encode_literal(&TypedValue::Bool(true)).unwrap(), "true");
        assert_eq!(encode_literal(&TypedValue::Bool(false)).unwrap(), "false");
        assert_eq!(encode_literal(&TypedValue::Int(-7)).unwrap(), "-7");
        assert_eq!(encode_literal(&TypedValue::Text("x".into())).unwrap(), "'x'");
    }

    #[test]
    fn escapes_quote_backslash_and_whitespace_controls() {
        assert_eq!(
            encode_literal(&TypedValue::Text("a'b".into())).unwrap(),
            "'a\\'b'"
        );
        assert_eq!(
            encode_literal(&TypedValue::Text("a\\b".into())).unwrap(),
            "'a\\\\b'"
        );
        assert_eq!(
            encode_literal(&TypedValue::Text("a\tb\nc\rd".into())).unwrap(),
            "'a\\tb\\nc\\rd'"
        );
    }

    #[test]
    fn rejects_other_control_chars() {
        // A NUL or a bell cannot be emitted - it would smuggle structure.
        assert!(encode_literal(&TypedValue::Text("a\u{0}b".into())).is_none());
        assert!(encode_literal(&TypedValue::Text("a\u{7}b".into())).is_none());
    }

    #[test]
    fn an_injection_value_is_escaped_not_executed() {
        // The classic break-out attempt becomes an inert quoted literal.
        let v = TypedValue::Text("' OR 1=1 RETURN n --".into());
        let lit = encode_literal(&v).unwrap();
        assert_eq!(lit, "'\\' OR 1=1 RETURN n --'");
        assert!(lit.starts_with('\'') && lit.ends_with('\''));
    }

    #[test]
    fn identifier_validation() {
        assert!(is_valid_identifier("path"));
        assert!(is_valid_identifier("_id"));
        assert!(is_valid_identifier("last_accessed"));
        assert!(!is_valid_identifier(""));
        assert!(!is_valid_identifier("9lives")); // leading digit
        assert!(!is_valid_identifier("n.email")); // a dot is not an identifier
        assert!(!is_valid_identifier("properties(n)")); // the B4 laundering attempt
        assert!(!is_valid_identifier("a b")); // space
        assert!(!is_valid_identifier(&"x".repeat(65))); // over-long
    }
}

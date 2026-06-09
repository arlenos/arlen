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

/// The result cap for a typed read, and the default when the request omits one.
pub const MAX_TYPED_READ_LIMIT: i64 = 100;

/// The default `limit` when a request omits it.
pub fn default_typed_read_limit() -> i64 {
    20
}

/// One equality filter: a field compared to a typed value. The value is encoded by
/// [`encode_literal`], never interpolated raw.
#[derive(Debug, Clone, Deserialize)]
pub struct TypedFilter {
    /// The field name (identifier-checked).
    pub field: String,
    /// The value to match.
    pub value: TypedValue,
}

/// A structured read over a single sensitive label. The body the `0x08` op accepts;
/// the daemon builds all Cypher, the caller never supplies query text. `deny_unknown_fields`
/// so a typo cannot smuggle an unvalidated key.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TypedReadRequest {
    /// The system label to read (unprefixed, e.g. `"CommandHistory"`), checked
    /// against the caller's readable-label allowlist.
    pub label: String,
    /// Equality filters anchoring the read. At least one is required (the owner-axis
    /// v1 approximation): an unanchored read of a sensitive label is the
    /// wholesale-harvest shape this op exists to prevent.
    #[serde(default)]
    pub filters: Vec<TypedFilter>,
    /// The fields to project; each is identifier-checked, empty is rejected.
    pub select: Vec<String>,
    /// Result cap, clamped to `[1, MAX_TYPED_READ_LIMIT]`.
    #[serde(default = "default_typed_read_limit")]
    pub limit: i64,
}

/// A validated read: every label/field is a safe identifier, the label is in the
/// caller's readable set, at least one anchoring filter is present, and the limit is
/// clamped. The Cypher builder consumes this; nothing here is caller-controlled text.
pub struct ValidatedRead {
    /// The validated label (a safe identifier in the readable set).
    pub label: String,
    /// The anchoring equality filters (field is a safe identifier).
    pub filters: Vec<(String, TypedValue)>,
    /// The projected fields (each a safe identifier).
    pub select: Vec<String>,
    /// The clamped result cap.
    pub limit: i64,
}

/// Validate a typed read against the caller's readable labels. Enforces the three
/// axes: the LABEL must be in `readable_labels` (and a safe identifier); every
/// filter and select FIELD must be a safe identifier (no `properties(n)`, no alias,
/// no `*` - the caller supplies no query text); and at least one anchoring filter
/// is required (the OWNER axis v1 approximation, pending an `_owner` column). Errors
/// carry an internal reason for logging; the handler maps any error to the single
/// uniform denial (no existence oracle).
pub fn validate_typed_read(
    req: TypedReadRequest,
    readable_labels: &[String],
) -> Result<ValidatedRead, &'static str> {
    if !readable_labels.iter().any(|l| l.eq_ignore_ascii_case(&req.label)) {
        return Err("label outside the caller's read scope");
    }
    if !is_valid_identifier(&req.label) {
        return Err("label is not a safe identifier");
    }
    if req.filters.is_empty() {
        return Err("a sensitive read must be anchored by at least one filter");
    }
    for f in &req.filters {
        if !is_valid_identifier(&f.field) {
            return Err("filter field is not a safe identifier");
        }
    }
    if req.select.is_empty() {
        return Err("select is empty");
    }
    for s in &req.select {
        if !is_valid_identifier(s) {
            return Err("select field is not a safe identifier");
        }
    }
    let limit = req.limit.clamp(1, MAX_TYPED_READ_LIMIT);
    Ok(ValidatedRead {
        label: req.label,
        filters: req.filters.into_iter().map(|f| (f.field, f.value)).collect(),
        select: req.select,
        limit,
    })
}

/// Build the deterministic, injection-safe Cypher for a validated read:
/// `MATCH (n:Label) WHERE n.field = <literal> AND … RETURN n.s0, n.s1, … LIMIT N`.
/// The label and every field are validated identifiers (safe to interpolate) and
/// every value goes through [`encode_literal`]; a value that cannot be encoded (a
/// control char) yields `None`, which the handler maps to the uniform denial. The
/// anchoring `WHERE` is always present (validation requires a filter) and the
/// `LIMIT` is always final, so the read is bounded and caller-anchored by
/// construction.
pub fn build_cypher(read: &ValidatedRead) -> Option<String> {
    let mut conditions = Vec::with_capacity(read.filters.len());
    for (field, value) in &read.filters {
        conditions.push(format!("n.{field} = {}", encode_literal(value)?));
    }
    let where_clause = conditions.join(" AND ");
    let projection = read
        .select
        .iter()
        .map(|s| format!("n.{s}"))
        .collect::<Vec<_>>()
        .join(", ");
    Some(format!(
        "MATCH (n:{}) WHERE {} RETURN {} LIMIT {}",
        read.label, where_clause, projection, read.limit
    ))
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

    fn req(label: &str, filters: Vec<TypedFilter>, select: &[&str]) -> TypedReadRequest {
        TypedReadRequest {
            label: label.into(),
            filters,
            select: select.iter().map(|s| s.to_string()).collect(),
            limit: 20,
        }
    }

    fn filter(field: &str, value: TypedValue) -> TypedFilter {
        TypedFilter { field: field.into(), value }
    }

    #[test]
    fn validate_accepts_an_in_scope_anchored_read() {
        let r = req(
            "CommandHistory",
            vec![filter("session_id", TypedValue::Text("s1".into()))],
            &["command", "ran_at"],
        );
        let v = validate_typed_read(r, &["CommandHistory".into()]).unwrap();
        assert_eq!(v.label, "CommandHistory");
        assert_eq!(v.filters.len(), 1);
        assert_eq!(v.select, ["command", "ran_at"]);
    }

    #[test]
    fn validate_denies_out_of_scope_label() {
        let r = req("Secrets", vec![filter("id", TypedValue::Text("x".into()))], &["v"]);
        assert!(validate_typed_read(r, &["CommandHistory".into()]).is_err());
    }

    #[test]
    fn validate_denies_unanchored_read() {
        let r = req("CommandHistory", vec![], &["command"]);
        assert!(validate_typed_read(r, &["CommandHistory".into()]).is_err());
    }

    #[test]
    fn validate_denies_bad_field_identifiers() {
        // A laundering attempt in select.
        let r = req(
            "CommandHistory",
            vec![filter("session_id", TypedValue::Text("s".into()))],
            &["properties(n)"],
        );
        assert!(validate_typed_read(r, &["CommandHistory".into()]).is_err());
        // A bad filter field.
        let r2 = req(
            "CommandHistory",
            vec![filter("n.email", TypedValue::Text("s".into()))],
            &["command"],
        );
        assert!(validate_typed_read(r2, &["CommandHistory".into()]).is_err());
    }

    #[test]
    fn validate_denies_empty_select() {
        let r = req("CommandHistory", vec![filter("id", TypedValue::Int(1))], &[]);
        assert!(validate_typed_read(r, &["CommandHistory".into()]).is_err());
    }

    #[test]
    fn validate_clamps_the_limit() {
        let mut r = req("CommandHistory", vec![filter("id", TypedValue::Int(1))], &["v"]);
        r.limit = 10_000;
        assert_eq!(validate_typed_read(r, &["CommandHistory".into()]).unwrap().limit, MAX_TYPED_READ_LIMIT);
    }

    #[test]
    fn build_cypher_is_anchored_bounded_and_injection_safe() {
        let r = req(
            "CommandHistory",
            vec![filter("session_id", TypedValue::Text("s1".into()))],
            &["command", "ran_at"],
        );
        let v = validate_typed_read(r, &["CommandHistory".into()]).unwrap();
        let cypher = build_cypher(&v).unwrap();
        assert_eq!(
            cypher,
            "MATCH (n:CommandHistory) WHERE n.session_id = 's1' RETURN n.command, n.ran_at LIMIT 20"
        );
        // The LIMIT is final (a trailing clause cannot append rows past the cap).
        assert!(cypher.trim_end().ends_with("LIMIT 20"));

        // An injection value is escaped into an inert literal, not executed.
        let r2 = req(
            "CommandHistory",
            vec![filter("session_id", TypedValue::Text("' OR 1=1 RETURN n --".into()))],
            &["command"],
        );
        let v2 = validate_typed_read(r2, &["CommandHistory".into()]).unwrap();
        let c2 = build_cypher(&v2).unwrap();
        assert!(c2.contains("n.session_id = '\\' OR 1=1 RETURN n --'"));
    }

    #[test]
    fn build_cypher_rejects_an_unencodable_value() {
        let r = req(
            "CommandHistory",
            vec![filter("session_id", TypedValue::Text("a\u{0}b".into()))],
            &["command"],
        );
        let v = validate_typed_read(r, &["CommandHistory".into()]).unwrap();
        assert!(build_cypher(&v).is_none());
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

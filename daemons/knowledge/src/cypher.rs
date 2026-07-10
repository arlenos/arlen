//! A small Cypher query-builder: typed constructors for the common statement
//! shapes, centralizing the dialect-specific node-pattern syntax and the
//! single-quote string escaping (the injection boundary, [`escape_cypher`]) that
//! today is hand-rolled at ~430 raw `format!` sites across the daemon.
//!
//! The point (kg-engine-decision.md, the Cypher-coupling hedge): the lock-in is
//! not the graph engine (we are on the healthy LadybugDB successor) but the raw
//! construction sprawl. Routing construction through this module turns a future
//! engine swap from rewriting every site into re-implementing these builders, and
//! keeps the escaping in one audited place. Migrated INCREMENTALLY; the node-by-id
//! `MERGE`/`MATCH` is the dominant pattern (30+ sites) and lands first.
//!
//! `label`/`var` are TRUSTED schema identifiers (the daemon supplies them from
//! `graph_schema`, never from a caller), so they are interpolated verbatim; the
//! `id` is caller-derived and always escaped.

use crate::utils::escape_cypher;

/// The `(<var>:<Label> {id: '<escaped id>'})` node-by-id pattern shared by the
/// `MATCH`/`MERGE` builders. `var`/`label` are trusted schema identifiers
/// (interpolated verbatim); the `id` is caller-derived and always escaped.
fn node_pattern(var: &str, label: &str, id: &str) -> String {
    format!("({var}:{label} {{id: '{}'}})", escape_cypher(id))
}

/// `MERGE (<var>:<Label> {id: '<escaped id>'})` - the idempotent node-by-id
/// upsert. Byte-for-byte identical to the hand-rolled
/// `format!("MERGE ({var}:{label} {{id: '{}'}})", escape_cypher(id))` it replaces.
pub fn merge_node(var: &str, label: &str, id: &str) -> String {
    format!("MERGE {}", node_pattern(var, label, id))
}

/// `MATCH (<var>:<Label> {id: '<escaped id>'})` - the node-by-id lookup.
pub fn match_node(var: &str, label: &str, id: &str) -> String {
    format!("MATCH {}", node_pattern(var, label, id))
}

/// `MATCH (<va>:<La> {id: '<esc a>'}), (<vb>:<Lb> {id: '<esc b>'})` - the
/// two-node-by-id lookup that prefixes an edge create/merge between them (the
/// dominant edge-writing shape). Both ids are escaped.
pub fn match_two_nodes(
    va: &str,
    la: &str,
    ida: &str,
    vb: &str,
    lb: &str,
    idb: &str,
) -> String {
    format!(
        "MATCH {}, {}",
        node_pattern(va, la, ida),
        node_pattern(vb, lb, idb)
    )
}

/// The `(<var>:<Label> {<field>: '<escaped value>'})` node-by-field pattern - the
/// node-by-id shape generalized to any single trusted-identifier field (e.g.
/// `app_id`). `var`/`label`/`field` are trusted schema identifiers (interpolated
/// verbatim); `value` is caller-derived and always escaped.
fn node_pattern_by_field(var: &str, label: &str, field: &str, value: &str) -> String {
    format!("({var}:{label} {{{field}: '{}'}})", escape_cypher(value))
}

/// `MATCH (<var>:<Label> {<field>: '<escaped value>'})` - the node-by-field
/// lookup that prefixes a `WHERE`/`SET`/`DETACH DELETE`/`RETURN` (e.g. `Grant`
/// anchored by `app_id`). Byte-for-byte identical to the hand-rolled
/// `MATCH ({var}:{label} {{{field}: '{}'}})` with `escape_cypher(value)`.
pub fn match_node_by_field(var: &str, label: &str, field: &str, value: &str) -> String {
    format!("MATCH {}", node_pattern_by_field(var, label, field, value))
}

/// A property value in a `SET` clause. The daemon supplies the type (never the
/// caller), so text is escaped-and-quoted (the injection boundary) while
/// numerics and bools are bare, exactly as the hand-rolled `format!` sites do.
pub enum SetValue<'a> {
    /// A `STRING` column: escaped and single-quoted.
    Text(&'a str),
    /// An `INT64` column (also timestamps): the decimal digits, unquoted.
    Int(i64),
    /// A `BOOL` column: `true` / `false`, unquoted. Lands with the Grant/Project
    /// lifecycle-flag SET sites; exercised by the builder tests meanwhile.
    #[allow(dead_code)]
    Bool(bool),
}

impl SetValue<'_> {
    /// Render the value as it appears on the right of a `SET var.field = _`.
    fn render(&self) -> String {
        match self {
            SetValue::Text(s) => format!("'{}'", escape_cypher(s)),
            SetValue::Int(n) => n.to_string(),
            SetValue::Bool(b) => b.to_string(),
        }
    }
}

/// `MERGE (<var>:<Label> {id: '<esc>'}) SET <var>.<f0> = <v0>, ...` - the
/// node-by-id upsert that also assigns properties. The id and every `Text`
/// value are escaped in one place; `field`/`label`/`var` are trusted schema
/// identifiers, interpolated verbatim. Byte-for-byte identical to the
/// hand-rolled form (single space before `SET`, `, ` between assignments).
pub fn merge_node_set(var: &str, label: &str, id: &str, sets: &[(&str, SetValue)]) -> String {
    let assigns = sets
        .iter()
        .map(|(field, value)| format!("{var}.{field} = {}", value.render()))
        .collect::<Vec<_>>()
        .join(", ");
    format!("{} SET {assigns}", merge_node(var, label, id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_builders_match_the_hand_rolled_form() {
        assert_eq!(merge_node("a", "App", "org.arlen.files"), "MERGE (a:App {id: 'org.arlen.files'})");
        assert_eq!(match_node("g", "Grant", "grant-1"), "MATCH (g:Grant {id: 'grant-1'})");
    }

    #[test]
    fn match_node_by_field_matches_the_hand_rolled_form() {
        // Byte-identical to the hand-rolled `MATCH (g:Grant {{app_id: '{esc}'}})`
        // the Grant-by-app_id sites use (space after the field colon).
        assert_eq!(
            match_node_by_field("g", "Grant", "app_id", "org.arlen.files"),
            "MATCH (g:Grant {app_id: 'org.arlen.files'})"
        );
    }

    #[test]
    fn merge_node_set_matches_the_hand_rolled_form() {
        // One text assignment: escaped id, escaped quoted value.
        assert_eq!(
            merge_node_set("t", "EntityType", "system.File", &[("label", SetValue::Text("File"))]),
            "MERGE (t:EntityType {id: 'system.File'}) SET t.label = 'File'",
        );
        // Mixed types: text escaped+quoted, int/bool bare, joined with ", ".
        assert_eq!(
            merge_node_set(
                "f",
                "File",
                "/x/o'brien",
                &[
                    ("path", SetValue::Text("/x/o'brien")),
                    ("last_accessed", SetValue::Int(42)),
                    ("pinned", SetValue::Bool(true)),
                ],
            ),
            "MERGE (f:File {id: '/x/o\\'brien'}) SET f.path = '/x/o\\'brien', f.last_accessed = 42, f.pinned = true",
        );
    }

    #[test]
    fn two_node_match_matches_the_hand_rolled_form() {
        // The prefix that a hand-rolled edge write concatenates a MERGE/CREATE
        // onto, byte-for-byte identical (comma-space between the two patterns).
        assert_eq!(
            match_two_nodes("g", "Grant", "grant-1", "a", "App", "org.arlen.files"),
            "MATCH (g:Grant {id: 'grant-1'}), (a:App {id: 'org.arlen.files'})",
        );
        // Both ids are escaped independently.
        let raw = format!(
            "MATCH (s:File {{id: '{}'}}), (t:File {{id: '{}'}})",
            escape_cypher("a'b"),
            escape_cypher("c\\d"),
        );
        assert_eq!(match_two_nodes("s", "File", "a'b", "t", "File", "c\\d"), raw);
    }

    #[test]
    fn the_id_is_escaped_but_the_label_and_var_are_verbatim() {
        // A quote/backslash in the id cannot break out of the literal (the
        // injection boundary is centralized here).
        assert_eq!(
            merge_node("f", "File", "/x/o'brien\\a"),
            "MERGE (f:File {id: '/x/o\\'brien\\\\a'})",
        );
        // The builder escapes identically to the hand-rolled call it replaces.
        let raw = format!("MATCH (f:File {{id: '{}'}})", escape_cypher("a'b"));
        assert_eq!(match_node("f", "File", "a'b"), raw);
    }
}

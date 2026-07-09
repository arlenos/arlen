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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_builders_match_the_hand_rolled_form() {
        assert_eq!(merge_node("a", "App", "org.arlen.files"), "MERGE (a:App {id: 'org.arlen.files'})");
        assert_eq!(match_node("g", "Grant", "grant-1"), "MATCH (g:Grant {id: 'grant-1'})");
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

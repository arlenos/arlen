//! BR-6: removing everything one source wrote.
//!
//! Revoking a bridge takes away its future authority; this is the other half,
//! the data it already wrote. The requirement is that revoking a bridge means
//! its data is gone, efficiently.
//!
//! **No scan and no owner index is needed, because the data is already
//! partitioned.** Entity storage is table-per-type, rel storage is table-per
//! `(edge, from, to)` triple, and a bridge writes only inside its own
//! namespace - so every table holding its rows is identifiable from the
//! namespace alone, and the purge is a set of table drops rather than a search
//! for rows with a matching column.
//!
//! Two facts from the engine probes in `graph.rs` shape this:
//!
//! 1. lbug permits `DROP TABLE` for node and rel tables, and dropping one
//!    source's tables leaves the rest of the graph intact.
//! 2. **The order is mandatory.** A node table cannot be dropped while a rel
//!    table still references it, so edges go first. A purge that dropped node
//!    tables in registry order would fail partway and leave the source half
//!    removed, which is worse than not having started.

/// The tables a purge must drop, in the order it must drop them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PurgePlan {
    /// Rel tables, dropped first.
    pub rel_tables: Vec<String>,
    /// Node tables, dropped after every referencing rel table is gone.
    pub node_tables: Vec<String>,
}

impl PurgePlan {
    /// The statements to run, already ordered.
    pub fn statements(&self) -> Vec<String> {
        self.rel_tables
            .iter()
            .chain(self.node_tables.iter())
            .map(|t| format!("DROP TABLE {t}"))
            .collect()
    }

    /// Whether this namespace has anything stored.
    pub fn is_empty(&self) -> bool {
        self.rel_tables.is_empty() && self.node_tables.is_empty()
    }
}

/// Whether `qualified_type` belongs to `namespace`.
///
/// The dotted boundary is the whole point: `md.obsidian` owns
/// `md.obsidian.Note` and does NOT own `md.obsidianvault.Note`. Getting this
/// wrong deletes a neighbour's data, which is why it is a strict segment check
/// and not a `starts_with`.
fn is_under(qualified_type: &str, namespace: &str) -> bool {
    qualified_type
        .strip_prefix(namespace)
        .and_then(|rest| rest.strip_prefix('.'))
        .is_some_and(|tail| !tail.is_empty())
}

/// Plan the purge of everything written under `namespace`.
///
/// `all_types` is the registry's list of qualified entity types and
/// `known_rel_tables` the rel tables that currently exist, each with the
/// endpoint types it was created for.
///
/// **Types come from the registry, never from table-name prefixes.** Table
/// names sanitise non-alphanumerics to `_`, so `e_md_obsidian_` is a prefix of
/// both `md.obsidian.Note` and `md.obsidian-x.Note` - matching on it would
/// delete a neighbouring namespace. The registry holds the qualified types
/// where that ambiguity does not exist.
///
/// A rel table is included when EITHER endpoint is in the namespace. A bridge
/// can only write its own types, so an edge it created always has at least one
/// endpoint here; and an edge pointing INTO its data has to go too, or dropping
/// the node table it references would be refused.
pub fn plan_purge(
    namespace: &str,
    all_types: &[String],
    known_rel_tables: &[(String, String, String)],
) -> PurgePlan {
    let owned: Vec<&String> = all_types
        .iter()
        .filter(|t| is_under(t, namespace))
        .collect();

    let mut rel_tables: Vec<String> = known_rel_tables
        .iter()
        .filter(|(_, from, to)| is_under(from, namespace) || is_under(to, namespace))
        .map(|(edge, from, to)| super::entity_rel_table_name(edge, from, to))
        .collect();
    rel_tables.sort();
    rel_tables.dedup();

    let mut node_tables: Vec<String> = owned
        .iter()
        .map(|t| super::entity_table_name(t))
        .collect();
    node_tables.sort();
    node_tables.dedup();

    PurgePlan {
        rel_tables,
        node_tables,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn types() -> Vec<String> {
        [
            "md.obsidian.Note",
            "md.obsidian.Tag",
            "md.obsidianvault.Note",
            "system.File",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect()
    }

    /// The failure that would matter most: taking a neighbour's data with it.
    #[test]
    fn a_neighbouring_namespace_is_not_purged() {
        let plan = plan_purge("md.obsidian", &types(), &[]);
        assert_eq!(plan.node_tables.len(), 2, "{plan:?}");
        let vault = super::super::entity_table_name("md.obsidianvault.Note");
        assert!(!plan.node_tables.contains(&vault), "{plan:?}");
        let file = super::super::entity_table_name("system.File");
        assert!(!plan.node_tables.contains(&file), "{plan:?}");
    }

    /// Edges go first or the node drops are refused - the engine enforces it.
    #[test]
    fn the_statements_drop_every_edge_before_any_node() {
        let rels = vec![(
            "LINKS_TO".to_string(),
            "md.obsidian.Note".to_string(),
            "system.File".to_string(),
        )];
        let plan = plan_purge("md.obsidian", &types(), &rels);
        let stmts = plan.statements();
        let last_rel = stmts
            .iter()
            .position(|s| s.contains(&plan.rel_tables[0]))
            .unwrap();
        let first_node = stmts
            .iter()
            .position(|s| s.contains(&plan.node_tables[0]))
            .unwrap();
        assert!(last_rel < first_node, "{stmts:?}");
    }

    /// An edge pointing INTO the namespace has to go too: leaving it would make
    /// the node-table drop fail.
    #[test]
    fn an_inbound_edge_is_included() {
        let rels = vec![(
            "MENTIONS".to_string(),
            "system.File".to_string(),
            "md.obsidian.Note".to_string(),
        )];
        let plan = plan_purge("md.obsidian", &types(), &rels);
        assert_eq!(plan.rel_tables.len(), 1, "{plan:?}");
    }

    /// An edge between two other namespaces is none of this purge's business.
    #[test]
    fn an_unrelated_edge_is_left_alone() {
        let rels = vec![(
            "LINKS_TO".to_string(),
            "system.File".to_string(),
            "md.obsidianvault.Note".to_string(),
        )];
        let plan = plan_purge("md.obsidian", &types(), &rels);
        assert!(plan.rel_tables.is_empty(), "{plan:?}");
    }

    /// A namespace that never wrote anything yields nothing to run.
    #[test]
    fn a_namespace_with_no_types_has_an_empty_plan() {
        let plan = plan_purge("com.nothing", &types(), &[]);
        assert!(plan.is_empty());
        assert!(plan.statements().is_empty());
    }
}

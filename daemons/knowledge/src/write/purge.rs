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
/// `all_types` is the registry's list of qualified entity types; `catalog` is
/// what the engine reports for its rel tables, each as
/// `(rel_table, source_node_table, destination_node_table)`.
///
/// The rel side comes from the ENGINE, not the registry, because a rel table's
/// name is a hash of its `(edge, from, to)` triple and the triple cannot be
/// recovered from it. `CALL show_tables()` and `CALL show_connection(name)`
/// answer this exactly, which beats enumerating every possible triple and
/// checking which exist.
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
    catalog: &[(String, String, String)],
) -> PurgePlan {
    let mut node_tables: Vec<String> = all_types
        .iter()
        .filter(|t| is_under(t, namespace))
        .map(|t| super::entity_table_name(t))
        .collect();
    node_tables.sort();
    node_tables.dedup();

    let mut rel_tables: Vec<String> = catalog
        .iter()
        .filter(|(_, src, dst)| node_tables.contains(src) || node_tables.contains(dst))
        .map(|(rel, _, _)| rel.clone())
        .collect();
    rel_tables.sort();
    rel_tables.dedup();

    PurgePlan {
        rel_tables,
        node_tables,
    }
}

/// What a purge actually removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Purged {
    /// The tables that are now gone.
    pub dropped: Vec<String>,
    /// Tables the plan named that could not be dropped, with the reason.
    ///
    /// Reported rather than aborting: a table already gone is the state the
    /// caller wanted, and one that refuses is worth naming rather than hiding
    /// behind a failed whole.
    pub failed: Vec<(String, String)>,
}

/// Read the engine's rel tables and their endpoints.
///
/// `show_tables` classifies, `show_connection` resolves. Both are read
/// positionally: the catalog's column names contain spaces, so quoting them in
/// a `RETURN` reads as a string literal rather than an identifier.
pub async fn read_rel_catalog(
    graph: &crate::graph::GraphHandle,
) -> anyhow::Result<Vec<(String, String, String)>> {
    let listed = graph
        .query_rows("CALL show_tables() RETURN name, type".to_string())
        .await?;

    let mut out = Vec::new();
    for row in &listed.rows {
        let (Some(crate::graph::CellValue::String(name)), Some(crate::graph::CellValue::String(kind))) =
            (row.first(), row.get(1))
        else {
            continue;
        };
        if kind != "REL" {
            continue;
        }
        // A table name from the catalog, quoted into the call. It came from the
        // engine rather than from a caller, but it is still interpolated, so a
        // name carrying a quote is skipped instead of trusted.
        if name.contains('\'') || name.contains('"') {
            continue;
        }
        let endpoints = graph
            .query_rows(format!("CALL show_connection('{name}') RETURN *"))
            .await?;
        if let Some(row) = endpoints.rows.first() {
            if let (Some(crate::graph::CellValue::String(src)), Some(crate::graph::CellValue::String(dst))) =
                (row.first(), row.get(1))
            {
                out.push((name.clone(), src.clone(), dst.clone()));
            }
        }
    }
    Ok(out)
}

/// Remove everything stored under `namespace`.
///
/// Runs the plan's statements in order - edges before nodes, which the engine
/// requires - and reports per table rather than stopping at the first refusal.
/// Re-running is safe: a table already dropped simply lands in `failed` with
/// the engine's own message, and the end state is the one that was asked for.
pub async fn purge_namespace(
    graph: &crate::graph::GraphHandle,
    namespace: &str,
    all_types: &[String],
) -> anyhow::Result<Purged> {
    let catalog = read_rel_catalog(graph).await?;
    let plan = plan_purge(namespace, all_types, &catalog);

    let mut purged = Purged {
        dropped: Vec::new(),
        failed: Vec::new(),
    };
    for statement in plan.statements() {
        let table = statement.trim_start_matches("DROP TABLE ").to_string();
        match graph.write(statement).await {
            Ok(_) => purged.dropped.push(table),
            Err(e) => purged.failed.push((table, e.to_string())),
        }
    }
    Ok(purged)
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
            "r_LINKS_TO_1".to_string(),
            super::super::entity_table_name("md.obsidian.Note"),
            super::super::entity_table_name("system.File"),
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
            "r_MENTIONS_1".to_string(),
            super::super::entity_table_name("system.File"),
            super::super::entity_table_name("md.obsidian.Note"),
        )];
        let plan = plan_purge("md.obsidian", &types(), &rels);
        assert_eq!(plan.rel_tables.len(), 1, "{plan:?}");
    }

    /// An edge between two other namespaces is none of this purge's business.
    #[test]
    fn an_unrelated_edge_is_left_alone() {
        let rels = vec![(
            "r_LINKS_TO_2".to_string(),
            super::super::entity_table_name("system.File"),
            super::super::entity_table_name("md.obsidianvault.Note"),
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
    /// End to end against a real graph: a source's nodes and edges go, its
    /// neighbour's stay. This is the property the whole feature exists for.
    #[tokio::test]
    async fn purging_a_namespace_removes_its_data_and_spares_the_rest() {
        let tmp = tempfile::TempDir::new().unwrap();
        let graph = crate::graph::spawn(tmp.path().join("g").to_str().unwrap()).unwrap();

        let note = super::super::entity_table_name("md.obsidian.Note");
        let file = super::super::entity_table_name("system.File");
        graph
            .write(format!("CREATE NODE TABLE {note}(id STRING, PRIMARY KEY(id))"))
            .await
            .unwrap();
        graph
            .write(format!("CREATE NODE TABLE {file}(id STRING, PRIMARY KEY(id))"))
            .await
            .unwrap();
        graph
            .write(format!("CREATE REL TABLE r_LINKS_TO_1(FROM {note} TO {file})"))
            .await
            .unwrap();
        graph.write(format!("CREATE (:{note} {{id:'n1'}})")).await.unwrap();
        graph.write(format!("CREATE (:{file} {{id:'f1'}})")).await.unwrap();

        let types = vec!["md.obsidian.Note".to_string(), "system.File".to_string()];
        let purged = purge_namespace(&graph, "md.obsidian", &types).await.unwrap();
        assert!(purged.failed.is_empty(), "{purged:?}");
        assert_eq!(purged.dropped.len(), 2, "the edge table and the node table");

        // The neighbour is untouched, which is the half that would be a disaster
        // to get wrong.
        let rest = graph
            .query_rows(format!("MATCH (f:{file}) RETURN f.id"))
            .await
            .unwrap();
        assert_eq!(rest.rows.len(), 1, "the neighbour's row survived");

        // And the source's table is genuinely gone, not merely emptied.
        assert!(
            graph.query_rows(format!("MATCH (n:{note}) RETURN n.id")).await.is_err(),
            "the purged table still answers queries"
        );
    }

    /// Re-running must be safe: the second pass finds nothing left and says so
    /// rather than failing, because the end state is the one that was asked for.
    #[tokio::test]
    async fn purging_twice_is_not_an_error_the_second_time() {
        let tmp = tempfile::TempDir::new().unwrap();
        let graph = crate::graph::spawn(tmp.path().join("g").to_str().unwrap()).unwrap();
        let note = super::super::entity_table_name("md.obsidian.Note");
        graph
            .write(format!("CREATE NODE TABLE {note}(id STRING, PRIMARY KEY(id))"))
            .await
            .unwrap();

        let types = vec!["md.obsidian.Note".to_string()];
        assert_eq!(
            purge_namespace(&graph, "md.obsidian", &types).await.unwrap().dropped.len(),
            1
        );
        let again = purge_namespace(&graph, "md.obsidian", &types).await.unwrap();
        assert!(again.dropped.is_empty());
        assert_eq!(again.failed.len(), 1, "the second pass names what was already gone");
    }

}

// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

//! The Library read: bridged content, browsed by source.
//!
//! The Knowledge app's Library shows what the bridges have put in the graph,
//! grouped by where it came from. Everything it needs to render a type it has
//! never seen is DECLARED BY THE TYPE (`bridge-architecture.md`, "A bridged type
//! declares how it is displayed"): a display name, a class from a closed set, and
//! which field is the title. The daemon reads the declaration out of the schema
//! registry and the rows out of the dynamic entity table, so a new connector is
//! one repository and no change to the desktop.
//!
//! **The two fallbacks are the load-bearing part.** A type that declared no
//! display name is shown by its qualified identifier, and an entry whose declared
//! title field is empty is shown by its external key - the note's own path, the
//! message's own id. Never the first string field that happens to be there.
//!
//! **An empty source is not a section.** A registered type with no rows (or with
//! no table yet, because nothing has ever written one) is left out entirely,
//! rather than rendered as a heading over nothing. A machine with no bridges gets
//! an empty list and the surface says so once.

use serde::Serialize;

use crate::graph::{CellValue, GraphHandle};
use crate::schema::{EntityDefinition, SchemaRegistry};
use crate::write::entity_table_name;

/// The most sections one read returns. A section is one declared type from one
/// source, so this is generous for any real machine and stops a registry full of
/// declared-but-tiny types turning one read into hundreds of graph queries.
const MAX_LIBRARY_SECTIONS: usize = 64;

/// The most entries one section carries. The Library is a browse surface, not an
/// export: it shows the recent end of each source and the rest is the search op's
/// job.
const MAX_LIBRARY_ENTRIES: usize = 50;

/// The namespaces the Library never shows. `system` is this machine's own
/// observation data (files, apps, sessions) and has its own surfaces; `shared` is
/// the cross-app people/places/tags types, which are the subject of other
/// entries rather than library content themselves.
const NOT_LIBRARY_NAMESPACES: &[&str] = &["system", "shared"];

/// One item in a section.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LibraryEntry {
    /// The graph node id, which is what a later read or a revoke names.
    pub id: String,
    /// The declared title field's value, or - when the type declared no title
    /// field, or the row's is empty - the entry's own external key.
    pub title: String,
    /// The declared sub-line, absent when the type declared none or the row's is
    /// empty. Absent rather than an empty string, so the surface can leave the
    /// line out instead of rendering a gap.
    pub sub: Option<String>,
    /// When THIS MACHINE last wrote the row, in seconds since the epoch.
    ///
    /// **This is not when the item was made.** A bridge does not tell the graph
    /// when a note was written or a message was sent - no declared field carries
    /// that - so the only moment here is the sync that learned it. A surface
    /// showing this must say "added", never "published" or "received".
    pub added: Option<i64>,
}

/// One section: one declared type, from one source.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LibrarySection {
    /// The namespace the type belongs to - the origin tag, and what a per-source
    /// revoke severs.
    pub source: String,
    /// The fully qualified type, which is also the fallback label.
    #[serde(rename = "type")]
    pub qualified_type: String,
    /// What to call the type in front of a person.
    pub label: String,
    /// How to lay the section out: one of the closed set.
    pub class: &'static str,
    /// The entries, most recently written first.
    pub entries: Vec<LibraryEntry>,
}

/// The declared types the Library would show, in a stable order.
///
/// Pure: it reads the registry and nothing else, so the selection rule (which
/// namespaces are library content, and the cap) is testable without a graph.
pub fn library_types(registry: &SchemaRegistry) -> Vec<String> {
    registry
        .all_entity_types()
        .into_iter()
        .filter(|t| {
            let ns = t.rsplit_once('.').map(|(ns, _)| ns).unwrap_or(t);
            // `system.File` splits to `system`; `md.obsidian.Note` to `md.obsidian`.
            !NOT_LIBRARY_NAMESPACES.contains(&ns) && !ns.is_empty()
        })
        .take(MAX_LIBRARY_SECTIONS)
        .collect()
}

/// The namespace half of a qualified type.
fn namespace_of(qualified_type: &str) -> &str {
    qualified_type
        .rsplit_once('.')
        .map(|(ns, _)| ns)
        .unwrap_or(qualified_type)
}

/// A display field name that is safe to interpolate into the read.
///
/// The schema validator already refuses a field name that is not snake_case and
/// one that names nothing the type declares, so this is the second wall rather
/// than the only one: a declaration that reached the registry by some other route
/// still cannot put text into a query.
fn safe_display_field<'a>(def: &'a EntityDefinition, declared: &'a Option<String>) -> Option<&'a str> {
    let name = declared.as_deref()?;
    if !crate::typed_read::is_valid_identifier(name) || !def.fields.contains_key(name) {
        return None;
    }
    Some(name)
}

/// The string in a cell, when it holds a non-empty one.
///
/// A null, a blank string or a non-string cell all read as absent - a title field
/// that is present but empty must fall back to the identifier, not render as a
/// blank row.
fn non_empty_text(cell: Option<&CellValue>) -> Option<String> {
    match cell {
        Some(CellValue::String(s)) if !s.trim().is_empty() => Some(s.clone()),
        _ => None,
    }
}

/// Seconds since the epoch for an RFC 3339 stamp, or `None` when the cell holds
/// something else. The write path writes `Utc::now().to_rfc3339()`, so this parses
/// what that produced and answers `None` rather than a guessed time for anything
/// it does not recognise.
fn epoch_seconds(cell: Option<&CellValue>) -> Option<i64> {
    let CellValue::String(s) = cell? else {
        return None;
    };
    chrono::DateTime::parse_from_rfc3339(s).ok().map(|t| t.timestamp())
}

/// Read one type's section, or `None` when it has no rows.
///
/// A missing table is the same answer as no rows: a declared type nobody has
/// written has no table, and the graph errors on a `MATCH` against one. That is
/// not a failure of the read, so it is absorbed here - otherwise a single
/// declared-and-unused type would sink the whole library.
async fn read_section(
    graph: &GraphHandle,
    qualified_type: &str,
    def: &EntityDefinition,
) -> Option<LibrarySection> {
    let table = entity_table_name(qualified_type);
    let title = safe_display_field(def, &def.title_field);
    let sub = safe_display_field(def, &def.subtitle_field);

    // Columns in a fixed order, so the row indices below are not a guess.
    let mut cols = vec![
        "n.id".to_string(),
        "n._external_key".to_string(),
        "n._modified_at".to_string(),
    ];
    cols.push(title.map_or_else(|| "null".to_string(), |f| format!("n.{f}")));
    cols.push(sub.map_or_else(|| "null".to_string(), |f| format!("n.{f}")));

    // `_modified_at` is an RFC 3339 stamp the write path always renders in UTC, so
    // it sorts lexicographically in the order it sorts chronologically. Ordering on
    // the string avoids a second column and a cast.
    let cypher = format!(
        "MATCH (n:{table}) WHERE n._deleted = false RETURN {} \
         ORDER BY n._modified_at DESC LIMIT {MAX_LIBRARY_ENTRIES}",
        cols.join(", ")
    );
    let rs = graph.query_rows(cypher).await.ok()?;
    if rs.rows.is_empty() {
        return None;
    }

    let entries: Vec<LibraryEntry> = rs
        .rows
        .iter()
        .filter_map(|row| {
            let id = non_empty_text(row.first())?;
            let external_key = non_empty_text(row.get(1));
            // The declared title, else the external key, else the node id. Never a
            // field the type did not name.
            let title = non_empty_text(row.get(3))
                .or(external_key)
                .unwrap_or_else(|| id.clone());
            Some(LibraryEntry {
                id,
                title,
                sub: non_empty_text(row.get(4)),
                added: epoch_seconds(row.get(2)),
            })
        })
        .collect();
    if entries.is_empty() {
        return None;
    }

    Some(LibrarySection {
        source: namespace_of(qualified_type).to_string(),
        qualified_type: qualified_type.to_string(),
        label: def.display_label(qualified_type).to_string(),
        class: def.display_class.as_str(),
        entries,
    })
}

/// Every non-empty section, in registry order.
pub async fn read_library(
    graph: &GraphHandle,
    registry: &SchemaRegistry,
) -> Vec<LibrarySection> {
    let mut sections = Vec::new();
    for qualified_type in library_types(registry) {
        let Some(def) = registry.get_entity(&qualified_type) else {
            continue;
        };
        if let Some(section) = read_section(graph, &qualified_type, def).await {
            sections.push(section);
        }
    }
    sections
}

/// The denial for an unreachable graph. Distinct from an empty library on
/// purpose: "you have bridged nothing" and "nobody could ask" are different
/// sentences and the surface says a different one for each.
pub const LIBRARY_UNAVAILABLE: &str = "ERROR: the library could not be read";

/// The wire form: the sections as JSON.
///
/// **A probe runs first, and it is the point of this function.** Every per-type
/// read absorbs its own error, because a declared type nobody has written has no
/// table and the graph errors on a `MATCH` against one - which is not a failure.
/// Absorb that without asking anything else and a graph that is down reads as a
/// library with nothing in it, which is the exact shape this tree keeps removing:
/// a surface stating something nobody measured. So one question the graph can
/// always answer goes first, and its failure is a failure.
pub async fn handle_library(graph: &GraphHandle, registry: &SchemaRegistry) -> String {
    if graph.query_rows("RETURN 1".to_string()).await.is_err() {
        return LIBRARY_UNAVAILABLE.to_string();
    }
    let sections = read_library(graph, registry).await;
    serde_json::to_string(&sections).unwrap_or_else(|_| LIBRARY_UNAVAILABLE.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::SchemaRegistry;

    /// A registry holding one bridged type that declares its display identity.
    fn registry_with_obsidian() -> SchemaRegistry {
        let mut reg = SchemaRegistry::new(vec![]);
        reg.load_from_str(
            r#"
[meta]
namespace = "md.obsidian"

[entities.Note]
display_name = "Notes"
display_class = "notes"
title_field = "title"
[entities.Note.fields.title]
type = "text"
[entities.Note.fields.folder]
type = "string"
"#,
        )
        .unwrap();
        reg
    }

    #[test]
    fn the_library_is_bridged_content_not_the_machine_or_the_shared_types() {
        let types = library_types(&registry_with_obsidian());
        assert!(types.contains(&"md.obsidian.Note".to_string()));
        // `system.*` is this machine watching itself and has its own surfaces;
        // `shared.*` is people and places, which are the subject of other rows.
        // Both are seeded into every registry, so their absence here is the rule
        // doing work rather than an empty registry.
        assert!(!types.iter().any(|t| t.starts_with("system.")));
        assert!(!types.iter().any(|t| t.starts_with("shared.")));
    }

    /// Stand up the entity table the write path would have created, and put two
    /// rows in it: one with a title, one without.
    async fn seeded_graph(dir: &std::path::Path) -> crate::graph::GraphHandle {
        let graph = crate::graph::spawn(dir.join("graph").to_str().unwrap()).unwrap();
        let table = entity_table_name("md.obsidian.Note");
        graph
            .write(format!(
                "CREATE NODE TABLE IF NOT EXISTS {table}(id STRING, _type STRING, \
                 _external_key STRING, _owner STRING, _version INT64, _created_at STRING, \
                 _modified_at STRING, _deleted BOOL, _run_id STRING, _origin_ref STRING, \
                 folder STRING, title STRING, PRIMARY KEY(id))"
            ))
            .await
            .unwrap();
        for (key, title, folder, at) in [
            ("notes/deep-work.md", "Deep work", "Reading", "2026-09-01T10:00:00+00:00"),
            ("notes/untitled.md", "", "", "2026-09-02T10:00:00+00:00"),
        ] {
            graph
                .write(format!(
                    "CREATE (n:{table} {{id: 'md.obsidian.Note:{key}', _external_key: '{key}', \
                     _modified_at: '{at}', _deleted: false, title: '{title}', folder: '{folder}'}})"
                ))
                .await
                .unwrap();
        }
        graph
    }

    #[tokio::test]
    async fn a_section_carries_the_display_identity_the_type_declared() {
        let tmp = tempfile::TempDir::new().unwrap();
        let graph = seeded_graph(tmp.path()).await;
        let sections = read_library(&graph, &registry_with_obsidian()).await;
        assert_eq!(sections.len(), 1);
        let s = &sections[0];
        assert_eq!(s.source, "md.obsidian");
        assert_eq!(s.label, "Notes");
        assert_eq!(s.class, "notes");
        assert_eq!(s.entries.len(), 2);
        // Newest first, so the untitled row leads.
        assert_eq!(s.entries[0].title, "notes/untitled.md");
        assert_eq!(s.entries[1].title, "Deep work");
    }

    #[tokio::test]
    async fn an_entry_with_no_title_shows_its_key_and_never_another_field() {
        let tmp = tempfile::TempDir::new().unwrap();
        let graph = seeded_graph(tmp.path()).await;
        let sections = read_library(&graph, &registry_with_obsidian()).await;
        let untitled = &sections[0].entries[0];
        // The row HAS a `folder` field. A library that reaches for it because the
        // title is empty is the guess this contract exists to forbid.
        assert_eq!(untitled.title, "notes/untitled.md");
        assert_eq!(untitled.sub, None);
        // 2026-09-02T10:00:00Z, the stamp the row carries - the moment the graph
        // learned it, which is the only moment anything here knows.
        assert_eq!(untitled.added, Some(1_788_343_200));
    }

    #[tokio::test]
    async fn a_declared_type_with_no_rows_is_not_a_section() {
        // No table was ever created for it, which is what a declared-but-unwritten
        // type looks like on a real machine. It must not appear as a heading over
        // nothing, and it must not sink the read either.
        let tmp = tempfile::TempDir::new().unwrap();
        let graph = crate::graph::spawn(tmp.path().join("graph").to_str().unwrap()).unwrap();
        assert!(read_library(&graph, &registry_with_obsidian()).await.is_empty());
        // And the op still answers a list rather than the unavailable shape: the
        // graph is up, there is simply nothing bridged.
        assert_eq!(handle_library(&graph, &registry_with_obsidian()).await, "[]");
    }

    #[tokio::test]
    async fn a_soft_deleted_row_is_gone_from_the_library() {
        let tmp = tempfile::TempDir::new().unwrap();
        let graph = seeded_graph(tmp.path()).await;
        let table = entity_table_name("md.obsidian.Note");
        graph
            .write(format!("MATCH (n:{table}) SET n._deleted = true"))
            .await
            .unwrap();
        assert!(read_library(&graph, &registry_with_obsidian()).await.is_empty());
    }

    #[test]
    fn a_display_field_the_type_does_not_declare_is_not_interpolated() {
        // The validator refuses this at registration; the read is the second wall,
        // because a declaration that arrived some other way must still not put
        // text into a query.
        let mut def = EntityDefinition::default();
        def.fields.insert("title".into(), Default::default());
        def.title_field = Some("title".into());
        assert_eq!(safe_display_field(&def, &def.title_field), Some("title"));
        for hostile in ["absent", "title, n.x", "1title", ""] {
            let declared = Some(hostile.to_string());
            assert_eq!(safe_display_field(&def, &declared), None, "{hostile}");
        }
    }
}

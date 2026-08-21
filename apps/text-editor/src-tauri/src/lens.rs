// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

//! The editor's own lens reads.
//!
//! The lens panel invoked `provenance_of`, `related_of` and `project_of` - names
//! defined in the FILES app's binary. A Tauri command lives in one app's process,
//! so those calls were rejected at runtime and the panel fell to its fixture on
//! every load. Nobody noticed because the fixture is labelled and the failure is
//! silent, and until this app had a host at all the scope gate filed it under "no
//! backend" rather than "calling the wrong app".
//!
//! `provenance_of` is answered here, from this app's own read of the graph. The
//! other two are not - but NOT because they are refused, which is what this said
//! and what the Knowledge app's modules said too. The gate authorises a traversal
//! by its endpoints (`raw_read_label_gate`, daemon.rs:4394) and its restricted
//! list is empty, so a `FILE_PART_OF` query from this caller is answered; measured
//! 16 August, with rows.
//!
//! Backlinks and project membership are therefore unbuilt reads, not forbidden
//! ones, and this app already has the permission it needs. Building them means
//! writing the two queries and keeping each join optional, so a file with no
//! project still renders its other sections.

use std::collections::HashMap;

use serde::Serialize;
use serde_json::Value;

/// One lineage step, in exactly the shape the lens renders.
///
/// Named `Lens…` rather than `ProvenanceStep` because the Files app already has a
/// struct by that name, and `check-invoke-shape.py` refuses to compare a type
/// whose name is defined twice - it cannot know which one a call means. It said
/// so, by file and line, in a section of its output I had not read: the pair went
/// uncompared, which is how the field-name mismatch survived. A unique name is
/// the whole fix.
///
/// The field names are the frontend's, and that is not a detail: the first cut
/// of this returned `verb`/`subject`/`when`, which type-checked on both sides
/// and would have rendered "undefined undefined" in the panel. A command whose
/// answer does not fit its caller is a silent blank, not an error.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LensProvenanceStep {
    /// The relation as a MESSAGE ID, never the word: the panel resolves it
    /// through the catalogue, so an English string here would ship untranslated.
    pub relation: String,
    /// Who or what acted, at the fidelity the graph actually has.
    pub actor: String,
    /// `user` | `graph` | `external` | `model` | `agent` - tints the dot only,
    /// never adds specificity the record does not carry.
    pub origin: &'static str,
    /// When, already phrased for display. The panel prints this verbatim, so an
    /// empty string is the honest form for "the graph did not record a time".
    pub when_ms: i64,
    /// `resolved` | `pid` | `proxy`: how confidently the actor is known.
    pub fidelity: &'static str,
}

/// What the project section can say, including the state it could not say before.
///
/// An empty as-of answer is AMBIGUOUS and the two readings are opposite: the file
/// was in no project then, or the graph was not recording membership yet. The
/// second is true of every instant before 16 August, because promotion only
/// started stamping intervals then. Rendering both as "no project" would put a
/// confident claim about the past on screen where the truth is "nobody knows".
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LensProjectAnswer {
    pub project: Option<LensProject>,
    /// The graph holds membership for this file, but all of it begins AFTER the
    /// instant asked about - so the answer is absence of record, not absence of
    /// membership.
    pub unrecorded: bool,
}

/// A document that references the open file: the backlink the panel lists.
///
/// `snippet` is the plan's inline context (#3) and is EMPTY here: `LINKS_TO`
/// records the link structure and deliberately never stores document content, so
/// the graph knows that `index.md` references this file but not the sentence it
/// did it in. Rendering an invented sentence is the exact failure this panel
/// keeps being rebuilt to stop, so the field is carried and left blank rather
/// than filled with something plausible.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LensBacklink {
    /// What to show: the referencing document's basename.
    pub file: String,
    /// What to open, and the list key.
    pub r#ref: String,
    pub snippet: String,
}

/// The file's project and the siblings that share it, in the shape the panel
/// renders. Named `Lens…` for the same reason as the step above.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LensProject {
    pub name: String,
    pub members: Vec<LensMember>,
}

/// A sibling: what to show and what to open, which are not the same string.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LensMember {
    pub path: String,
    pub name: String,
}

/// Where the open file came from, as far as the graph can say.
///
/// An unknown file answers with an empty list rather than a guess; an unreachable
/// daemon is an error, so the panel can tell "nothing recorded" from "could not
/// ask".
#[tauri::command]
pub async fn provenance_of(r#ref: String) -> Result<Vec<LensProvenanceStep>, String> {
    let socket = os_sdk::runtime::socket_path("ARLEN_KNOWLEDGE_SOCKET", "knowledge.sock");
    let client = os_sdk::graph::UnixGraphClient::new(socket.to_string_lossy().into_owned());
    let rows = client
        .query_rows(&file_query(&r#ref))
        .await
        .map_err(|e| e.to_string())?;
    Ok(steps_from_rows(&rows))
}

/// The project the open file belongs to, or `None` when the graph knows of none.
///
/// This is the section the lens has been serving a fixture for, and it was left
/// unbuilt on a belief that turned out to be false: the modules here and in the
/// Knowledge app all said the read gate refuses a query naming a relationship
/// type. It does not - `raw_read_label_gate` (daemon.rs:4394) authorises a
/// traversal by its ENDPOINTS, and its restricted list is empty. Measured against
/// a live daemon on 16 August, with rows.
///
/// LIVE-ONLY on both stamps, so an archived project or a closed membership does
/// not surface as the file's current home; the bitemporal edge carries both.
#[tauri::command]
pub async fn project_of(
    r#ref: String,
    as_of: Option<i64>,
) -> Result<LensProjectAnswer, String> {
    let socket = os_sdk::runtime::socket_path("ARLEN_KNOWLEDGE_SOCKET", "knowledge.sock");
    let client = os_sdk::graph::UnixGraphClient::new(socket.to_string_lossy().into_owned());
    let rows = client
        .query_rows(&project_query(&r#ref, as_of))
        .await
        .map_err(|e| e.to_string())?;
    let project = project_from_rows(&rows);
    if project.is_some() {
        return Ok(LensProjectAnswer { project, unrecorded: false });
    }
    // Only on the empty answer, and only when a past instant was asked for: the
    // common path keeps its single read.
    let unrecorded = match as_of {
        None => false,
        Some(t) => {
            let rows = client
                .query_rows(&began_after_query(&r#ref, t))
                .await
                .map_err(|e| e.to_string())?;
            !rows.is_empty()
        }
    };
    Ok(LensProjectAnswer { project: None, unrecorded })
}

/// Does the graph hold a membership for this file that BEGAN after `t`?
///
/// Answering yes turns an empty as-of result from "in no project then" into "not
/// recorded that far back". An unstamped edge is excluded on purpose: with no
/// known start it is read as always-having-been, so it would have matched the
/// as-of query already and we would not be here.
fn began_after_query(node: &str, t: i64) -> String {
    let safe = node.replace('\\', "\\\\").replace('\'', "\\'");
    let matches_file = if node.starts_with('/') {
        format!("f.path = '{safe}'")
    } else {
        format!("f.path ENDS WITH '/{safe}'")
    };
    format!(
        "MATCH (f:File)-[r:FILE_PART_OF]->(p:Project) \
         WHERE {matches_file} AND r.valid_at > {t} \
         RETURN p.name AS name LIMIT 1"
    )
}

/// The documents that reference the open file (plan #3, cross-content backlinks).
///
/// This section has shown a labelled sample since the editor had a host, on the
/// belief that "backlinks" had nothing to traverse. That was true of file-to-file
/// edges in general and false for this one: promotion parses a markdown
/// document's `[text](path)` and `[[wikilink]]` references and records a
/// `LINKS_TO` edge per target it has already observed (`promotion.rs::
/// link_markdown_document`), storing the link STRUCTURE and never the content.
/// Measured 16 August: a document with two links produced exactly two edges.
///
/// Backlinks run against the arrow - the interesting question is who points HERE
/// - so the pattern is `(other)-[:LINKS_TO]->(this)`. Both nodes name their label
/// because the read gate refuses an unlabelled one, bare back-reference included.
#[tauri::command]
pub async fn related_of(r#ref: String) -> Result<Vec<LensBacklink>, String> {
    let socket = os_sdk::runtime::socket_path("ARLEN_KNOWLEDGE_SOCKET", "knowledge.sock");
    let client = os_sdk::graph::UnixGraphClient::new(socket.to_string_lossy().into_owned());
    let rows = client
        .query_rows(&backlinks_query(&r#ref))
        .await
        .map_err(|e| e.to_string())?;
    Ok(backlinks_from_rows(&rows))
}

/// The same caller-shape rule as `project_query`: an absolute path is matched
/// exactly, a bare name as a trailing segment.
fn backlinks_query(node: &str) -> String {
    let safe = node.replace('\\', "\\\\").replace('\'', "\\'");
    let matches_file = if node.starts_with('/') {
        format!("f.path = '{safe}'")
    } else {
        format!("f.path ENDS WITH '/{safe}'")
    };
    format!(
        "MATCH (other:File)-[:LINKS_TO]->(f:File) \
         WHERE {matches_file} AND other.path <> f.path \
         RETURN other.path AS path \
         ORDER BY other.last_accessed DESC LIMIT {MAX_BACKLINKS}"
    )
}

/// Enough to orient without turning the lens into a list; the same bound and the
/// same reason as the project members above.
const MAX_BACKLINKS: usize = 12;

/// Pure, so the shape is tested without a daemon. Deduped on path, because two
/// links from one document to this file are one backlink.
fn backlinks_from_rows(rows: &[HashMap<String, Value>]) -> Vec<LensBacklink> {
    let mut seen = std::collections::BTreeSet::new();
    let mut out = Vec::new();
    for row in rows {
        let Some(path) = row.get("path").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) else {
            continue;
        };
        if !seen.insert(path.to_string()) {
            continue;
        }
        out.push(LensBacklink {
            file: path.rsplit('/').next().unwrap_or(path).to_string(),
            r#ref: path.to_string(),
            snippet: String::new(),
        });
    }
    out
}

/// The open end of a bitemporal interval, as a literal the gate accepts.
///
/// `i64::MAX`, so "not closed" compares as "closes later than any question".
const OPEN_END: i64 = i64::MAX;

/// The membership predicate for one edge alias, live or as of an instant.
///
/// LIVE (`None`) is the form every read in this app has used: an edge with no
/// close stamp is the open interval. AS OF an instant it becomes a containment
/// test, and the two `coalesce` calls are what make it correct rather than merely
/// plausible:
///
/// * `coalesce(valid_at, 0)` reads a MISSING start as "no known beginning". Every
///   membership promotion wrote before 16 August carries NULL there, and treating
///   NULL as "started at the epoch" is the honest reading - the alternative,
///   `valid_at <= t` on a NULL, is false, which would report that a file belonged
///   to no project at any past instant simply because nobody recorded when it
///   began. That is a confident wrong answer about real data.
/// * `coalesce(invalid_at, OPEN_END)` is the same convention on the close side,
///   and matches what `IS NULL` means in the live form.
///
/// No parenthesised group, because the read gate reads `(` after WHERE as an
/// unlabelled node and refuses the whole query.
fn interval(alias: &str, as_of: Option<i64>) -> String {
    match as_of {
        None => format!("{alias}.invalid_at IS NULL AND {alias}.expired_at IS NULL"),
        Some(t) => format!(
            "coalesce({alias}.valid_at, 0) <= {t} \
             AND coalesce({alias}.invalid_at, {OPEN_END}) > {t} \
             AND coalesce({alias}.expired_at, {OPEN_END}) > {t}"
        ),
    }
}

/// A node's liveness on the same axis: live now, or not yet expired at `t`.
fn node_live(alias: &str, as_of: Option<i64>) -> String {
    match as_of {
        None => format!("{alias}.expired_at IS NULL"),
        Some(t) => format!("coalesce({alias}.expired_at, {OPEN_END}) > {t}"),
    }
}

/// ONE predicate, chosen here rather than an `OR` group, and the reason is the
/// daemon's read gate rather than taste.
///
/// `file_query` above can write `WHERE path = x OR path ENDS WITH y` because with
/// no `AND` beside it the clause needs no parentheses. This query has to add the
/// liveness stamps, which forces `WHERE (a OR b) AND c` - and the gate's pattern
/// scanner reads ANY `(` that is not preceded by an identifier as the start of a
/// node (daemon.rs:4234), so a parenthesised WHERE group is an unlabelled node to
/// it and the whole read is denied. Measured 16 August: "every node in the pattern
/// must name a label", from a query whose two nodes are both labelled.
///
/// So the caller's shape decides the predicate: an absolute path is matched
/// exactly, anything else as a trailing path segment. That is stricter than the
/// OR as well - a caller who hands over a full path no longer also matches some
/// other file whose name happens to end that way.
fn project_query(node: &str, as_of: Option<i64>) -> String {
    let safe = node.replace('\\', "\\\\").replace('\'', "\\'");
    let matches_file = if node.starts_with('/') {
        format!("f.path = '{safe}'")
    } else {
        format!("f.path ENDS WITH '/{safe}'")
    };
    // One query, two hops: the file's project and, back down the same edge, the
    // project's OTHER members - plan #4, "pulling the project's other members
    // into the backlink panel". Both hops carry their own liveness stamps,
    // because a sibling whose membership was closed left the project and saying
    // otherwise is the kind of quiet lie this panel keeps being rebuilt to stop.
    //
    // The membership hop is optional so a file that belongs to a project ALONE
    // still answers with its project rather than nothing - the section would
    // otherwise vanish for the first file in a new project, which is exactly
    // when someone looks at it.
    //
    // `p` is already bound, and Cypher would take a bare `(p)` on the second hop.
    // The gate would not: it demands every node in a pattern NAME a label, and a
    // bare back-reference names none, so the whole read is refused - the same
    // unlabelled-node rule as the parenthesised group, reached a different way.
    // Measured against a live daemon: `(p)` refused, `(p:Project)` answered.
    let member_edge = interval("r", as_of);
    let sibling_edge = interval("r2", as_of);
    let project_live = node_live("p", as_of);
    format!(
        "MATCH (f:File)-[r:FILE_PART_OF]->(p:Project) \
         WHERE {matches_file} \
           AND {member_edge} AND {project_live} \
         OPTIONAL MATCH (p:Project)<-[r2:FILE_PART_OF]-(sib:File) \
         WHERE {sibling_edge} AND sib.path <> f.path \
         RETURN p.name AS name, sib.path AS member, sib.last_accessed AS at \
         ORDER BY sib.last_accessed DESC LIMIT {MAX_MEMBERS}"
    )
}

/// Enough to orient, not enough to become a file list. The panel is a lens, not
/// a browser: a project with four hundred files should not push the provenance
/// section off the screen.
const MAX_MEMBERS: usize = 12;

/// Pure, so the shape is tested without a daemon. An empty name is None rather
/// than an empty chip: a project with no readable name is not a project the panel
/// can say anything true about.
///
/// The rows are one per sibling (the project name repeats), so the name comes
/// from the first and the members are collected across all of them. A row whose
/// `member` is null is the no-siblings case the OPTIONAL hop produces.
fn project_from_rows(rows: &[HashMap<String, Value>]) -> Option<LensProject> {
    let name = rows
        .first()?
        .get("name")?
        .as_str()
        .filter(|s| !s.is_empty())?
        .to_string();
    let mut members = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for row in rows {
        let Some(path) = row.get("member").and_then(|v| v.as_str()).filter(|s| !s.is_empty())
        else {
            continue;
        };
        if !seen.insert(path.to_string()) {
            continue;
        }
        members.push(LensMember {
            path: path.to_string(),
            // The chip shows the basename because a lens beside the text has no
            // room for a full path, but the click opens `path` - the name alone
            // is not openable, and two projects can hold the same basename.
            name: path.rsplit('/').next().unwrap_or(path).to_string(),
        });
    }
    Some(LensProject { name, members })
}

/// Match by exact path or basename, since the lens is given whichever the
/// surface has. The name is escaped: it arrives from the frontend, and a quote in
/// a filename would otherwise end the literal and leave the rest as Cypher.
fn file_query(node: &str) -> String {
    let safe = node.replace('\\', "\\\\").replace('\'', "\\'");
    format!(
        "MATCH (f:File) WHERE f.path = '{safe}' OR f.path ENDS WITH '/{safe}' \
         RETURN f.path AS path, f.app_id AS app_id, f.last_accessed AS at LIMIT 1"
    )
}

/// Pure, so the shape is tested without a daemon.
fn steps_from_rows(rows: &[HashMap<String, Value>]) -> Vec<LensProvenanceStep> {
    let Some(row) = rows.first() else {
        return Vec::new();
    };
    let mut steps = Vec::new();
    if let Some(app) = row.get("app_id").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
        steps.push(LensProvenanceStep {
            relation: "te.pv.verb.openedIn".into(),
            actor: app.to_string(),
            // The promotion pipeline wrote this from an observed file open, so
            // the origin is the graph's own observation, not a claim by a user
            // or a model.
            origin: "graph",
            // THE INSTANT, not a rendering of it. The comment here used to argue
            // that an ISO day is "the one form that is unambiguous without
            // knowing the reader's locale", and that the phrasing is the app's
            // to make - which is right, and the app was then printing the ISO
            // day verbatim, so the panel said `2026-08-14` to a person. Sending
            // the instant lets it make the phrasing it was always meant to.
            //
            // Epoch MILLIseconds, matching the file manager's provenance step,
            // and `0` means the graph had no timestamp rather than 1970.
            when_ms: row
                .get("at")
                .and_then(|v| v.as_i64())
                .map(|micros| micros / 1_000)
                .unwrap_or_default(),
            // The graph records the app id it saw, so the actor is resolved -
            // not a pid we guessed a name for.
            fidelity: "resolved",
        });
    }
    steps
}


#[cfg(test)]
mod project_tests {
    use super::*;

    fn row(name: &str, member: Option<&str>) -> HashMap<String, Value> {
        let mut r = HashMap::new();
        r.insert("name".to_string(), serde_json::json!(name));
        r.insert(
            "member".to_string(),
            member.map_or(Value::Null, |m| serde_json::json!(m)),
        );
        r
    }

    #[test]
    fn a_named_live_project_is_returned_and_a_blank_one_is_not() {
        let p = project_from_rows(&[row("Arlen", None)]).expect("named project");
        assert_eq!(p.name, "Arlen");
        // The OPTIONAL hop answers one row with a null member when the file is
        // the project's only member. That is a project, not an absence.
        assert!(p.members.is_empty());
        // No rows: the file is in no project, which the panel must show as absent
        // rather than as an empty chip.
        assert_eq!(project_from_rows(&[]), None);
        assert_eq!(project_from_rows(&[row("", None)]), None);
    }

    #[test]
    fn siblings_are_collected_across_rows_and_shown_by_basename() {
        let p = project_from_rows(&[
            row("Arlen", Some("/w/arlen/notes.md")),
            row("Arlen", Some("/w/arlen/docs/plan.md")),
            // The same sibling twice cannot become two chips.
            row("Arlen", Some("/w/arlen/notes.md")),
        ])
        .expect("project");
        assert_eq!(p.name, "Arlen");
        let shown: Vec<&str> = p.members.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(shown, ["notes.md", "plan.md"]);
        // The chip opens the PATH: a basename is not openable, and the panel
        // wires the click to this field.
        assert_eq!(p.members[1].path, "/w/arlen/docs/plan.md");
    }

    #[test]
    fn as_of_reads_a_missing_start_as_no_known_beginning() {
        // The reason this needs `coalesce` rather than a plain comparison: every
        // membership written before 16 August carries NULL `valid_at`, and
        // `NULL <= t` is false, so the honest-looking query would report that
        // those files belonged to no project at any past instant. Confidently
        // wrong about real data is worse than the fixture this replaced.
        let q = project_query("/w/a/README.md", Some(1_000));
        assert!(q.contains("coalesce(r.valid_at, 0) <= 1000"), "{q}");
        assert!(q.contains("coalesce(r.invalid_at,"), "{q}");
        // The sibling hop moves with it, or the members listed beside a past
        // project would be today's.
        assert!(q.contains("coalesce(r2.valid_at, 0) <= 1000"), "{q}");
        // And the project itself: one archived since `t` was still live then.
        assert!(q.contains("coalesce(p.expired_at,"), "{q}");
    }

    #[test]
    fn the_live_form_is_untouched_when_no_instant_is_asked_for() {
        // `None` must be byte-for-byte the query this app has always sent, so
        // wiring as-of cannot regress the default view.
        let q = project_query("/w/a/README.md", None);
        assert!(q.contains("r.invalid_at IS NULL AND r.expired_at IS NULL"), "{q}");
        assert!(q.contains("p.expired_at IS NULL"), "{q}");
        assert!(!q.contains("coalesce"), "no interval arithmetic in the live read: {q}");
    }

    #[test]
    fn neither_form_carries_a_parenthesised_predicate() {
        // The gate reads `(` after WHERE as an unlabelled node. `coalesce(` is a
        // CALL, not a group - it is preceded by an identifier - but the predicate
        // must still never open a bare group, so check both forms the same way.
        for q in [
            project_query("README.md", None),
            project_query("README.md", Some(42)),
        ] {
            let mut rest = q.as_str();
            while let Some((_, after)) = rest.split_once("WHERE ") {
                let end = ["OPTIONAL MATCH", "RETURN", "ORDER BY"]
                    .iter()
                    .filter_map(|k| after.find(k))
                    .min()
                    .unwrap_or(after.len());
                assert!(
                    !after[..end].contains(" ("),
                    "no bare group in the predicate: {}",
                    &after[..end]
                );
                rest = after;
            }
        }
    }

    #[test]
    fn the_where_clause_carries_no_parenthesised_group() {
        // The gate reads a `(` that no identifier precedes as a NODE, so a
        // parenthesised WHERE group is an unlabelled node to it and the read is
        // refused - measured, with both real nodes labelled.
        for q in [project_query("README.md", None), project_query("/home/t/a/README.md", None)] {
            // Per PREDICATE, not per query: the second hop is a MATCH whose own
            // nodes are parenthesised and must be. What may never carry a `(` is
            // the text between a WHERE and the clause that ends it.
            let mut rest = q.as_str();
            let mut checked = 0;
            while let Some((_, after)) = rest.split_once("WHERE ") {
                let end = ["OPTIONAL MATCH", "RETURN", "ORDER BY"]
                    .iter()
                    .filter_map(|k| after.find(k))
                    .min()
                    .unwrap_or(after.len());
                let predicate = &after[..end];
                assert!(!predicate.contains('('), "no group in the predicate: {predicate}");
                checked += 1;
                rest = after;
            }
            assert_eq!(checked, 2, "both WHERE clauses were checked: {q}");
        }
        // Every node names a label, including the already-bound `p` on the second
        // hop - a bare back-reference is an unlabelled node to the gate.
        for q in [project_query("README.md", None), project_query("/x/README.md", None)] {
            assert!(!q.contains("(p)<-"), "the bound node repeats its label: {q}");
            assert!(q.contains("(p:Project)<-"), "second hop labels its node: {q}");
        }
        // An absolute path matches exactly; a bare name matches a trailing segment.
        assert!(project_query("/home/t/a/README.md", None).contains("f.path = '/home/t/a/README.md'"));
        assert!(project_query("README.md", None).contains("ENDS WITH '/README.md'"));
    }

    #[test]
    fn a_quote_in_the_name_cannot_end_the_literal() {
        let q = project_query("Tim's notes.md", None);
        assert!(q.contains("Tim\\'s notes.md"), "the quote is escaped: {q}");
        // Both stamps and the project's own liveness are required, so an archived
        // project cannot surface as the file's current home.
        assert!(q.contains("r.invalid_at IS NULL"));
        assert!(q.contains("p.expired_at IS NULL"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn row(pairs: &[(&str, Value)]) -> HashMap<String, Value> {
        pairs.iter().map(|(k, v)| ((*k).to_string(), v.clone())).collect()
    }

    #[test]
    fn a_quote_in_a_filename_cannot_end_the_literal() {
        let q = file_query("it's a file.md");
        assert!(q.contains("it\\'s a file.md"), "{q}");
    }

    #[test]
    fn the_shape_carries_every_field_the_panel_reads() {
        let json = serde_json::to_string(&LensProvenanceStep {
            relation: "x".into(),
            actor: "y".into(),
            origin: "graph",
            when_ms: 0,
            fidelity: "resolved",
        })
        .unwrap();
        // The panel reads all five; a renamed field renders as a blank rather
        // than an error, so the names are asserted rather than trusted.
        for field in ["relation", "actor", "origin", "when_ms", "fidelity"] {
            assert!(json.contains(&format!("\"{field}\"")), "{field} missing from {json}");
        }
    }

    #[test]
    fn an_unknown_file_gets_no_steps_rather_than_a_guess() {
        assert!(steps_from_rows(&[]).is_empty());
    }

    #[test]
    fn the_opening_app_becomes_one_true_step() {
        let r = row(&[
            ("app_id", json!("text-editor")),
            ("at", json!(1_786_000_000_000_000i64)),
        ]);
        let steps = steps_from_rows(&[r]);
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].relation, "te.pv.verb.openedIn", "a message id, never a word");
        assert_eq!(steps[0].actor, "text-editor");
        // The instant, in milliseconds: the words are the panel's to write.
        assert_eq!(steps[0].when_ms, 1_786_000_000_000i64);
        assert_eq!(steps[0].origin, "graph");
        assert_eq!(steps[0].fidelity, "resolved");
    }
}

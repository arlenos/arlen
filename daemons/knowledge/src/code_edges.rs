//! Replacing one file's part of the code graph, and binding references to
//! definitions by name.
//!
//! The code indexer sees one file at a time, so a `code.indexed` event carries
//! the file's definitions and the NAMES it refers to. Which definition a name
//! binds to needs every file of the project, and this daemon is the one place
//! that holds them, so the binding is done here, when the file is promoted.
//!
//! The rule is the one `code-graph-layer.md` §5 sets: a name the project
//! defines once binds to that definition as `inferred`; a name it defines
//! several times binds to every one of them as `ambiguous`; a name it does not
//! define is someone else's code and gets no edge. Syntax alone cannot do
//! better, and the label says so on every edge.
//!
//! The names a file refers to are kept as `CodeRef` nodes, because a binding is
//! not settled when it is written: the definition a call points at may move,
//! gain a namesake or go away when ANOTHER file is saved. Each promotion
//! therefore binds again every reference, in any file, whose name this file
//! defines now or defined before.
//!
//! All of it is a projection. Nothing here is in the fact journal: the source
//! files are the record, and indexing them again rebuilds every node and edge.

use std::collections::{BTreeSet, HashMap, HashSet};

use anyhow::Result;
use tracing::debug;

use crate::cypher::{id_list, match_two_nodes, merge_node};
use crate::graph::GraphHandle;
use crate::proto::CodeFileIndexPayload;
use crate::utils::escape_cypher;

/// The most definitions one name may bind to. Past this the name says nothing
/// about which code is meant (`new`, `get`, `from` in a large project), and
/// writing every pairing would cost a number of edges that grows with the
/// square of the project, so such a name gets no edge at all.
pub const MAX_CANDIDATES: usize = 8;

/// The three kinds of reference and the edge each one becomes.
const REL_OF_KIND: [(&str, &str); 3] = [
    ("calls", "CALLS"),
    ("imports", "IMPORTS"),
    ("references", "REFERENCES"),
];

/// The edge label a reference kind is written as, or `None` for a kind this
/// daemon does not know (a newer indexer, or a crafted event).
pub fn rel_label(kind: &str) -> Option<&'static str> {
    REL_OF_KIND.iter().find(|(k, _)| *k == kind).map(|(_, rel)| *rel)
}

/// A definition a name can bind to.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Definition {
    /// The `CodeSymbol` id.
    pub id: String,
    /// What kind of definition it is (`function`, `struct`, `file`, ...).
    pub kind: String,
}

/// Whether a reference of `ref_kind` can mean a definition of `def_kind` at
/// all. This is knowledge the syntax does carry: a call does not call a type
/// alias, and a type in a field is not a function.
fn can_bind(ref_kind: &str, def_kind: &str) -> bool {
    match ref_kind {
        // A struct and a class are callable: a tuple struct's constructor, `new C()`.
        "calls" => matches!(def_kind, "function" | "method" | "macro" | "struct" | "class"),
        "references" => {
            matches!(def_kind, "struct" | "enum" | "trait" | "type_alias" | "class" | "interface")
        }
        "imports" => true,
        _ => false,
    }
}

/// The definitions a reference of `ref_kind` binds to among everything the
/// project defines under its name, each with the confidence of the binding.
///
/// A file stands in for its module only when nothing declares that module:
/// `mod graph;` and `graph.rs` are one thing named twice, and counting both
/// would make every module import ambiguous.
pub fn bind<'a>(ref_kind: &str, named: &'a [Definition]) -> Vec<(&'a str, &'static str)> {
    let fitting: Vec<&Definition> = named.iter().filter(|d| can_bind(ref_kind, &d.kind)).collect();
    let declared: Vec<&Definition> =
        fitting.iter().copied().filter(|d| d.kind != "file").collect();
    let targets = if declared.is_empty() { fitting } else { declared };
    let confidence = match targets.len() {
        0 => return Vec::new(),
        1 => "inferred",
        n if n <= MAX_CANDIDATES => "ambiguous",
        _ => return Vec::new(),
    };
    targets.into_iter().map(|d| (d.id.as_str(), confidence)).collect()
}

/// A reference waiting to be bound: the symbol it is made in, its kind and the
/// name it refers to.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Pending {
    from: String,
    kind: String,
    name: String,
}

/// The condition that keeps a node inside the scope references are bound in:
/// the outermost live project around the file, or the file alone when it is in
/// no project. Outermost, so that a project nested in another binds in the
/// same scope from both sides.
async fn scope_condition(graph: &GraphHandle, source_file: &str, var: &str) -> Result<String> {
    let rows = graph
        .query_rows(
            "MATCH (p:Project) WHERE p.expired_at IS NULL AND p.root_path IS NOT NULL \
             RETURN p.root_path"
                .to_string(),
        )
        .await?;
    let root = rows
        .rows
        .iter()
        .filter_map(|row| row.first())
        .map(|cell| cell.as_str().trim_end_matches('/'))
        .filter(|root| {
            !root.is_empty()
                && source_file.starts_with(root)
                && source_file.as_bytes().get(root.len()) == Some(&b'/')
        })
        .min_by_key(|root| root.len());
    Ok(match root {
        Some(root) => format!("{var}.source_file STARTS WITH '{}/'", escape_cypher(root)),
        None => format!("{var}.source_file = '{}'", escape_cypher(source_file)),
    })
}

/// Replace everything the code graph holds about one file with the freshly
/// parsed index, in one transaction.
///
/// The file's `CodeSymbol`s and `CodeRef`s are replaced whole (a re-parse
/// supersedes the previous set, and a crash in the middle leaves the old one).
/// Then the references are bound: this file's own, and every other file's
/// whose name this file defines now or defined until now, since those are the
/// bindings the change can have moved. All interpolated values are escaped, so
/// file content cannot inject Cypher.
pub async fn replace_file(graph: &GraphHandle, p: &CodeFileIndexPayload) -> Result<()> {
    if p.source_file.is_empty() {
        return Ok(());
    }
    let file = p.source_file.as_str();
    let file_esc = escape_cypher(file);
    let lang_esc = escape_cypher(&p.language);

    // The definitions as they will stand. A duplicate id would make two
    // `CREATE`s collide on the primary key, failing the whole transaction and
    // stalling ALL promotion; the indexer already drops them, but a crafted bus
    // message must not be able to wedge the pipeline, so a repeat is skipped
    // here too. `by_position` keeps the payload's numbering for the references.
    let mut seen_ids = HashSet::new();
    let by_position: Vec<Option<&str>> = p
        .symbols
        .iter()
        .map(|s| {
            (!s.id.is_empty() && !s.name.is_empty() && seen_ids.insert(s.id.as_str()))
                .then_some(s.id.as_str())
        })
        .collect();
    let symbols: Vec<_> = p
        .symbols
        .iter()
        .zip(&by_position)
        .filter_map(|(s, kept)| kept.map(|_| s))
        .collect();

    // This file's references, each from a definition that exists.
    let mut own: Vec<Pending> = Vec::new();
    let mut own_seen = HashSet::new();
    for r in &p.references {
        let Some(Some(from)) = by_position.get(r.from as usize) else {
            continue;
        };
        if r.name.is_empty() || rel_label(&r.kind).is_none() {
            continue;
        }
        let pending = Pending {
            from: (*from).to_string(),
            kind: r.kind.clone(),
            name: r.name.clone(),
        };
        if own_seen.insert(pending.clone()) {
            own.push(pending);
        }
    }

    // The names whose set of definitions this promotion changes: what the file
    // defined until now and what it defines from now on.
    let before = graph
        .query_rows(format!(
            "MATCH (s:CodeSymbol {{source_file: '{file_esc}'}}) RETURN s.name"
        ))
        .await?;
    let mut changed: BTreeSet<String> = before
        .rows
        .iter()
        .filter_map(|row| row.first())
        .map(|cell| cell.as_str().to_string())
        .filter(|name| !name.is_empty())
        .collect();
    changed.extend(symbols.iter().map(|s| s.name.clone()));

    let scope_s = scope_condition(graph, file, "s").await?;
    let scope_r = scope_s.replacen("s.", "r.", 1);
    let scope_b = scope_s.replacen("s.", "b.", 1);

    // Every definition, in the other files of the scope, of a name that is
    // about to be bound: the changed ones and the ones this file refers to.
    let mut wanted: BTreeSet<String> = changed.clone();
    wanted.extend(own.iter().map(|r| r.name.clone()));
    let mut named: HashMap<String, Vec<Definition>> = HashMap::new();
    if !wanted.is_empty() {
        let rows = graph
            .query_rows(format!(
                "MATCH (s:CodeSymbol) WHERE s.name IN [{}] AND {scope_s} \
                 AND s.source_file <> '{file_esc}' RETURN s.name, s.id, s.kind",
                id_list(wanted.iter())
            ))
            .await?;
        for row in &rows.rows {
            if let [name, id, kind, ..] = row.as_slice() {
                named.entry(name.as_str().to_string()).or_default().push(Definition {
                    id: id.as_str().to_string(),
                    kind: kind.as_str().to_string(),
                });
            }
        }
    }
    for s in &symbols {
        named.entry(s.name.clone()).or_default().push(Definition {
            id: s.id.clone(),
            kind: s.kind.clone(),
        });
    }

    // The other files' references to a changed name, to be bound again.
    let mut others: Vec<Pending> = Vec::new();
    if !changed.is_empty() {
        let rows = graph
            .query_rows(format!(
                "MATCH (r:CodeRef) WHERE r.name IN [{}] AND {scope_r} \
                 AND r.source_file <> '{file_esc}' RETURN r.from_symbol, r.kind, r.name",
                id_list(changed.iter())
            ))
            .await?;
        for row in &rows.rows {
            if let [from, kind, name, ..] = row.as_slice() {
                others.push(Pending {
                    from: from.as_str().to_string(),
                    kind: kind.as_str().to_string(),
                    name: name.as_str().to_string(),
                });
            }
        }
    }

    let mut stmts = Vec::new();
    // Per-file replace: the file's prior symbols go, and with them every edge
    // from or to them.
    stmts.push(format!(
        "MATCH (s:CodeSymbol {{source_file: '{file_esc}'}}) DETACH DELETE s"
    ));
    stmts.push(format!(
        "MATCH (r:CodeRef {{source_file: '{file_esc}'}}) DELETE r"
    ));
    // The other files' edges to a changed name go too: each is written again
    // below against the definitions as they now stand.
    if !changed.is_empty() {
        let names = id_list(changed.iter());
        for (_, rel) in REL_OF_KIND {
            stmts.push(format!(
                "MATCH (:CodeSymbol)-[e:{rel}]->(b:CodeSymbol) \
                 WHERE b.name IN [{names}] AND {scope_b} DELETE e"
            ));
        }
    }
    // The fusion anchor: the File node the symbols hang off (the activity graph
    // already carries its provenance/project/timeline).
    stmts.push(merge_node("f", "File", file));
    for sym in &symbols {
        let id_esc = escape_cypher(&sym.id);
        let name_esc = escape_cypher(&sym.name);
        let loc_esc = escape_cypher(&sym.source_location);
        let kind_esc = escape_cypher(&sym.kind);
        stmts.push(format!(
            "CREATE (:CodeSymbol {{id: '{id_esc}', name: '{name_esc}', \
             source_file: '{file_esc}', source_location: '{loc_esc}', \
             language: '{lang_esc}', kind: '{kind_esc}'}})"
        ));
        stmts.push(format!(
            "{} CREATE (f)-[:DEFINES {{confidence: 'extracted'}}]->(s)",
            match_two_nodes("f", "File", file, "s", "CodeSymbol", &sym.id)
        ));
    }
    for r in &own {
        stmts.push(format!(
            "CREATE (:CodeRef {{id: '{}', from_symbol: '{}', source_file: '{file_esc}', \
             name: '{}', kind: '{}'}})",
            escape_cypher(&format!("{}|{}|{}", r.from, r.kind, r.name)),
            escape_cypher(&r.from),
            escape_cypher(&r.name),
            escape_cypher(&r.kind),
        ));
    }
    let mut edges = 0usize;
    for r in own.iter().chain(&others) {
        let (Some(rel), Some(defs)) = (rel_label(&r.kind), named.get(&r.name)) else {
            continue;
        };
        for (target, confidence) in bind(&r.kind, defs) {
            stmts.push(format!(
                "{} CREATE (a)-[:{rel} {{confidence: '{confidence}'}}]->(b)",
                match_two_nodes("a", "CodeSymbol", &r.from, "b", "CodeSymbol", target)
            ));
            edges += 1;
        }
    }

    graph.transaction(stmts).await?;
    debug!(
        source_file = %p.source_file,
        symbols = symbols.len(),
        references = own.len(),
        rebound = others.len(),
        edges,
        "promoted code.indexed"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def(id: &str, kind: &str) -> Definition {
        Definition { id: id.to_string(), kind: kind.to_string() }
    }

    /// Parse `source` as the indexer would and promote it under `path`.
    async fn index(graph: &GraphHandle, path: &str, source: &str) {
        use arlen_code_indexer::extract::extract;
        use arlen_code_indexer::index::{build_payload, language_for_path};
        use prost::Message as _;
        let language = language_for_path(path).expect("a source file");
        let wire = build_payload(path, language, &extract(language, source)).encode_to_vec();
        let payload = CodeFileIndexPayload::decode(wire.as_slice()).expect("the two proto copies agree");
        replace_file(graph, &payload).await.expect("the file promotes");
    }

    /// Every edge of one label as `(from name, from file, to name, to file, confidence)`.
    async fn edges(graph: &GraphHandle, rel: &str) -> Vec<(String, String, String, String, String)> {
        let rows = graph
            .query_rows(format!(
                "MATCH (a:CodeSymbol)-[e:{rel}]->(b:CodeSymbol) \
                 RETURN a.name, a.source_file, b.name, b.source_file, e.confidence"
            ))
            .await
            .unwrap();
        let file = |cell: &crate::graph::CellValue| {
            cell.as_str().rsplit('/').next().unwrap_or_default().to_string()
        };
        rows.rows
            .iter()
            .map(|r| {
                (r[0].as_str().to_string(), file(&r[1]), r[2].as_str().to_string(), file(&r[3]), r[4].as_str().to_string())
            })
            .collect()
    }

    fn has(edges: &[(String, String, String, String, String)], from: (&str, &str), to: (&str, &str), confidence: &str) -> bool {
        edges.iter().any(|e| {
            (e.0.as_str(), e.1.as_str()) == from && (e.2.as_str(), e.3.as_str()) == to && e.4 == confidence
        })
    }

    /// The code indexer's own source, read from disk and promoted file by file:
    /// a real crate with real cross-file calls, and small enough to know by heart.
    #[tokio::test]
    async fn a_real_crate_gets_its_calls_imports_and_references() {
        let tmp = tempfile::TempDir::new().unwrap();
        let graph = crate::graph::spawn(tmp.path().join("graph").to_str().unwrap()).unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../code-indexer")
            .canonicalize()
            .unwrap();
        let root_str = root.to_str().unwrap();
        graph
            .write(format!(
                "CREATE (:Project {{id: 'p', name: 'code-indexer', root_path: '{}'}})",
                escape_cypher(root_str)
            ))
            .await
            .unwrap();
        let files = ["src/extract.rs", "src/index.rs", "src/lib.rs", "src/main.rs"];
        for file in files {
            let path = root.join(file);
            let source = std::fs::read_to_string(&path).unwrap();
            index(&graph, path.to_str().unwrap(), &source).await;
        }

        let calls = edges(&graph, "CALLS").await;
        // Inside one file.
        assert!(has(&calls, ("build_payload", "index.rs"), ("symbol_id", "index.rs"), "inferred"), "{calls:?}");
        assert!(has(&calls, ("build_payload", "index.rs"), ("enclosing", "index.rs"), "inferred"));
        // Across files, and promoted before the file that defines the callee
        // would have had to be: main.rs comes last here, index.rs second.
        assert!(has(&calls, ("handle_event", "main.rs"), ("build_payload", "index.rs"), "inferred"));
        assert!(has(&calls, ("handle_event", "main.rs"), ("extract", "extract.rs"), "inferred"));
        assert!(has(&calls, ("extract", "extract.rs"), ("symbol_kind", "extract.rs"), "inferred"));
        // `as_key` is a method of four types here: every candidate is kept.
        let as_key: Vec<_> = calls
            .iter()
            .filter(|e| e.0 == "build_payload" && e.2 == "as_key")
            .collect();
        assert!(as_key.len() > 1, "several candidates: {as_key:?}");
        assert!(as_key.iter().all(|e| e.4 == "ambiguous"), "and none is passed off as the one");
        // Nothing the crate does not define: `format`, `push`, `collect`.
        assert!(calls.iter().all(|e| e.2 != "push" && e.2 != "collect"), "no edge to someone else's code");

        let imports = edges(&graph, "IMPORTS").await;
        // A top-level `use` is the file's.
        assert!(has(&imports, ("main", "main.rs"), ("build_payload", "index.rs"), "inferred"), "{imports:?}");
        assert!(has(&imports, ("index", "index.rs"), ("FileIndex", "extract.rs"), "inferred"));

        let references = edges(&graph, "REFERENCES").await;
        assert!(has(&references, ("build_payload", "index.rs"), ("FileIndex", "extract.rs"), "inferred"), "{references:?}");
        assert!(has(&references, ("Symbol", "extract.rs"), ("SymbolKind", "extract.rs"), "inferred"), "a field's type");

        // The analysis op reads this graph, and finds what a reader of the
        // crate would call its centre.
        let analysis = crate::code_analysis::analyze_code_graph(&graph, 3).await.unwrap();
        assert!(
            analysis.god_symbols.iter().any(|g| g.id.contains("#function:build_payload@")),
            "build_payload is a hub: {:?}",
            analysis.god_symbols
        );

        // A save of index.rs that moves every definition down a line gives each
        // a new id. The calls into it from main.rs, a file that is NOT indexed
        // again, must follow.
        let index_path = root.join("src/index.rs");
        let moved = format!("\n{}", std::fs::read_to_string(&index_path).unwrap());
        index(&graph, index_path.to_str().unwrap(), &moved).await;
        let after = edges(&graph, "CALLS").await;
        assert!(
            has(&after, ("handle_event", "main.rs"), ("build_payload", "index.rs"), "inferred"),
            "the caller in another file is bound to the moved definition"
        );
        assert_eq!(after.len(), calls.len(), "nothing lost and nothing doubled by the save");
        assert_eq!(edges(&graph, "IMPORTS").await.len(), imports.len());
        assert_eq!(edges(&graph, "REFERENCES").await.len(), references.len());

        // A second definition of a name turns the binding ambiguous, in the
        // file that calls it and was not touched.
        let twin = root.join("src/twin.rs");
        index(&graph, twin.to_str().unwrap(), "pub fn build_payload() {}\n").await;
        let with_twin = edges(&graph, "CALLS").await;
        assert!(has(&with_twin, ("handle_event", "main.rs"), ("build_payload", "index.rs"), "ambiguous"));
        assert!(has(&with_twin, ("handle_event", "main.rs"), ("build_payload", "twin.rs"), "ambiguous"));
        // And taking it away again settles it.
        index(&graph, twin.to_str().unwrap(), "").await;
        let settled = edges(&graph, "CALLS").await;
        assert!(has(&settled, ("handle_event", "main.rs"), ("build_payload", "index.rs"), "inferred"));
        assert_eq!(settled.len(), calls.len());
    }

    /// A file outside every project binds within itself and reaches no other.
    #[tokio::test]
    async fn a_file_in_no_project_binds_only_within_itself() {
        let tmp = tempfile::TempDir::new().unwrap();
        let graph = crate::graph::spawn(tmp.path().join("graph").to_str().unwrap()).unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        index(&graph, "/a/one.rs", "fn helper() {}\nfn run() { helper(); }\n").await;
        index(&graph, "/b/two.rs", "fn go() { helper(); }\n").await;
        let calls = edges(&graph, "CALLS").await;
        assert_eq!(calls.len(), 1, "{calls:?}");
        assert!(has(&calls, ("run", "one.rs"), ("helper", "one.rs"), "inferred"));
    }

    #[test]
    fn one_definition_is_inferred_and_several_are_all_kept_as_ambiguous() {
        let one = [def("a#helper", "function")];
        assert_eq!(bind("calls", &one), vec![("a#helper", "inferred")]);

        let two = [def("a#run", "function"), def("b#run", "method")];
        assert_eq!(
            bind("calls", &two),
            vec![("a#run", "ambiguous"), ("b#run", "ambiguous")],
            "never collapsed to one winner"
        );

        assert!(bind("calls", &[]).is_empty(), "an undefined name is someone else's code");
    }

    #[test]
    fn a_reference_only_binds_to_a_kind_it_can_mean() {
        let named = [def("a#Size", "struct"), def("b#Size", "const")];
        assert_eq!(
            bind("references", &named),
            vec![("a#Size", "inferred")],
            "a type in a field is not a constant, so one candidate is left"
        );
        assert!(bind("calls", &[def("a#Alias", "type_alias")]).is_empty());
        assert_eq!(bind("imports", &named).len(), 2, "an import can name anything");
        assert!(bind("inherits", &named).is_empty(), "an unknown kind binds to nothing");
    }

    #[test]
    fn a_file_stands_in_for_its_module_only_when_nothing_declares_it() {
        let both = [def("lib.rs#module:graph", "module"), def("graph.rs#file", "file")];
        assert_eq!(bind("imports", &both), vec![("lib.rs#module:graph", "inferred")]);

        let file_only = [def("utils.ts#file", "file")];
        assert_eq!(bind("imports", &file_only), vec![("utils.ts#file", "inferred")]);
        assert!(bind("calls", &file_only).is_empty(), "a file is not called");
    }

    #[test]
    fn a_name_defined_everywhere_binds_to_nothing() {
        let many: Vec<Definition> =
            (0..=MAX_CANDIDATES).map(|i| def(&format!("f{i}#new"), "method")).collect();
        assert!(bind("calls", &many).is_empty(), "past the cap the name says nothing");
        assert_eq!(bind("calls", &many[..MAX_CANDIDATES]).len(), MAX_CANDIDATES);
    }
}

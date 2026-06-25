//! CG-R2: cross-file resolution at query time. The per-file extractor (CG-R1,
//! [`crate::extract`]) records reference NAMES only - which `CodeSymbol` a `bar()`
//! call or a `use a::b::c` import binds to is deferred to here. This module is the
//! pure stitching core: build a by-name index of every definition the project
//! declares, then resolve each reference against it.
//!
//! Confidence follows code-graph-layer.md exactly: a single candidate is
//! [`Confidence::Inferred`] (a reasonable deduction); several candidates are
//! [`Confidence::Ambiguous`] and EVERY candidate is kept - the resolution is
//! never collapsed to one heuristic winner; a name the project does not define
//! resolves to nothing (an external / std symbol) and yields no edge.
//!
//! Pure: the caller supplies the project's per-file [`FileIndex`]es; no graph, no
//! IO, so it is unit-tested directly. The query API + the CALLS/IMPORTS edge
//! materialization that consume these resolutions are the next CG-R2 slice.

use std::collections::HashMap;

use crate::extract::{Confidence, FileIndex, Reference, Symbol};

/// A defining symbol located in the project: the file that declares it and the
/// symbol itself (name, kind, line, column).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Definition {
    /// The file declaring the symbol.
    pub file: String,
    /// The declared symbol.
    pub symbol: Symbol,
}

/// A reference bound to its target definition(s) at query time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedReference {
    /// The file making the reference.
    pub from_file: String,
    /// The reference being resolved (name, kind, line) - verbatim from the index.
    pub reference: Reference,
    /// The candidate definitions the name binds to. More than one => the binding
    /// is [`Confidence::Ambiguous`] and all are kept (never collapsed).
    pub targets: Vec<Definition>,
    /// [`Confidence::Inferred`] for a single candidate, [`Confidence::Ambiguous`]
    /// for several.
    pub confidence: Confidence,
}

/// A by-name index of every definition the project declares, for resolution.
#[derive(Debug, Default)]
pub struct SymbolTable {
    by_name: HashMap<String, Vec<Definition>>,
}

impl SymbolTable {
    /// Build the table from the project's per-file indexes (each its path + the
    /// [`FileIndex`] extracted from it). Definitions keep insertion order within
    /// a name, so resolution output is deterministic for a fixed input order.
    pub fn build<'a, I>(files: I) -> Self
    where
        I: IntoIterator<Item = (&'a str, &'a FileIndex)>,
    {
        let mut by_name: HashMap<String, Vec<Definition>> = HashMap::new();
        for (path, index) in files {
            for sym in &index.symbols {
                by_name
                    .entry(sym.name.clone())
                    .or_default()
                    .push(Definition { file: path.to_string(), symbol: sym.clone() });
            }
        }
        SymbolTable { by_name }
    }

    /// The definitions a name binds to, in insertion order; empty when the
    /// project declares none (the name is external / std).
    pub fn definitions(&self, name: &str) -> &[Definition] {
        self.by_name.get(name).map(Vec::as_slice).unwrap_or(&[])
    }

    /// The number of distinct names the project defines (telemetry / tests).
    pub fn len(&self) -> usize {
        self.by_name.len()
    }

    /// Whether the project defines no symbols.
    pub fn is_empty(&self) -> bool {
        self.by_name.is_empty()
    }
}

/// Resolve one file's references against the project [`SymbolTable`]. A reference
/// to a name the project does not define is dropped (no edge - it is external);
/// a single candidate yields an [`Confidence::Inferred`] binding, several an
/// [`Confidence::Ambiguous`] one that keeps every candidate.
pub fn resolve_file(
    from_file: &str,
    index: &FileIndex,
    table: &SymbolTable,
) -> Vec<ResolvedReference> {
    let mut out = Vec::new();
    for reference in &index.references {
        let candidates = table.definitions(&reference.name);
        if candidates.is_empty() {
            continue;
        }
        let confidence = if candidates.len() == 1 {
            Confidence::Inferred
        } else {
            Confidence::Ambiguous
        };
        out.push(ResolvedReference {
            from_file: from_file.to_string(),
            reference: reference.clone(),
            targets: candidates.to_vec(),
            confidence,
        });
    }
    out
}

/// Resolve every file's references across the whole project: build the table
/// once, then resolve each file against it. The result is every edge the code
/// graph can bind at query time, in file-then-reference order.
pub fn resolve_project<'a, I>(files: I) -> Vec<ResolvedReference>
where
    I: IntoIterator<Item = (&'a str, &'a FileIndex)> + Clone,
{
    let table = SymbolTable::build(files.clone());
    let mut out = Vec::new();
    for (path, index) in files {
        out.extend(resolve_file(path, index, &table));
    }
    out
}

/// Find the project's references that bind to `name` - the "who calls / imports
/// this?" reverse of [`resolve_file`]'s jump-to-definition, the other half of code
/// navigation. Returns each [`ResolvedReference`] (with its `from_file`, the call/
/// import kind + line, and the resolved targets) whose name is `name`. Built over
/// the same project index, so the `Ambiguous`/`Inferred` confidence on each binding
/// is identical to the forward direction.
pub fn callers_of<'a, I>(name: &str, files: I) -> Vec<ResolvedReference>
where
    I: IntoIterator<Item = (&'a str, &'a FileIndex)> + Clone,
{
    resolve_project(files)
        .into_iter()
        .filter(|r| r.reference.name == name)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extract::{RefKind, SymbolKind};

    fn sym(name: &str, line: usize) -> Symbol {
        Symbol { name: name.to_string(), kind: SymbolKind::Function, line, column: 1 }
    }
    fn call(name: &str, line: usize) -> Reference {
        Reference { name: name.to_string(), kind: RefKind::Call, line }
    }
    fn file(symbols: Vec<Symbol>, references: Vec<Reference>) -> FileIndex {
        FileIndex { symbols, references }
    }

    #[test]
    fn a_single_definition_resolves_as_inferred() {
        let defs = file(vec![sym("foo", 1)], vec![]);
        let caller = file(vec![], vec![call("foo", 3)]);
        let table = SymbolTable::build([("lib.rs", &defs), ("main.rs", &caller)]);
        let resolved = resolve_file("main.rs", &caller, &table);
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].confidence, Confidence::Inferred);
        assert_eq!(resolved[0].targets.len(), 1);
        assert_eq!(resolved[0].targets[0].file, "lib.rs");
        assert_eq!(resolved[0].targets[0].symbol.name, "foo");
    }

    #[test]
    fn several_definitions_are_ambiguous_and_keep_every_candidate() {
        // Two files each define `helper`; a call to it cannot pick one, so the
        // binding is Ambiguous and BOTH targets are kept.
        let a = file(vec![sym("helper", 1)], vec![]);
        let b = file(vec![sym("helper", 9)], vec![]);
        let caller = file(vec![], vec![call("helper", 2)]);
        let table = SymbolTable::build([("a.rs", &a), ("b.rs", &b), ("c.rs", &caller)]);
        let resolved = resolve_file("c.rs", &caller, &table);
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].confidence, Confidence::Ambiguous);
        let files: Vec<&str> = resolved[0].targets.iter().map(|d| d.file.as_str()).collect();
        assert_eq!(files, ["a.rs", "b.rs"], "every candidate kept, in input order");
    }

    #[test]
    fn an_undefined_name_resolves_to_nothing() {
        // A call to a name the project never defines (a std / external symbol) is
        // dropped: no edge, rather than a fabricated target.
        let caller = file(vec![], vec![call("println", 1), call("local", 2)]);
        let defs = file(vec![sym("local", 5)], vec![]);
        let table = SymbolTable::build([("main.rs", &caller), ("lib.rs", &defs)]);
        let resolved = resolve_file("main.rs", &caller, &table);
        assert_eq!(resolved.len(), 1, "only `local` resolves; `println` is external");
        assert_eq!(resolved[0].reference.name, "local");
    }

    #[test]
    fn callers_of_finds_every_reference_to_a_symbol() {
        // `foo` is defined in lib and called from two other files; `callers_of`
        // returns both call sites (the find-callers reverse of jump-to-def).
        let lib = file(vec![sym("foo", 1)], vec![]);
        let a = file(vec![], vec![call("foo", 4)]);
        let b = file(vec![], vec![call("foo", 7), call("other", 8)]);
        let callers = callers_of("foo", [("lib.rs", &lib), ("a.rs", &a), ("b.rs", &b)]);
        assert_eq!(callers.len(), 2);
        let from: Vec<&str> = callers.iter().map(|c| c.from_file.as_str()).collect();
        assert_eq!(from, ["a.rs", "b.rs"]);
        assert!(callers.iter().all(|c| c.reference.name == "foo"));
        assert!(callers.iter().all(|c| c.targets.iter().any(|d| d.file == "lib.rs")));
    }

    #[test]
    fn callers_of_an_undefined_symbol_is_empty() {
        // Nothing defines `ghost`, so even a reference to it resolves to no edge
        // and the reverse query is empty (not a fabricated caller).
        let a = file(vec![], vec![call("ghost", 1)]);
        assert!(callers_of("ghost", [("a.rs", &a)]).is_empty());
    }

    #[test]
    fn resolve_project_binds_each_files_references() {
        let lib = file(vec![sym("foo", 1), sym("bar", 2)], vec![call("foo", 2)]);
        let main = file(vec![], vec![call("bar", 1), call("nope", 2)]);
        let edges = resolve_project([("lib.rs", &lib), ("main.rs", &main)]);
        // lib's `foo` self-call + main's `bar`; `nope` is external (dropped).
        assert_eq!(edges.len(), 2);
        assert!(edges.iter().all(|e| e.confidence == Confidence::Inferred));
        assert!(edges.iter().any(|e| e.from_file == "lib.rs" && e.reference.name == "foo"));
        assert!(edges.iter().any(|e| e.from_file == "main.rs" && e.reference.name == "bar"));
    }
}

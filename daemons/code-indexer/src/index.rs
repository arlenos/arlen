//! Pure helpers that turn a [`crate::extract::FileIndex`] into the
//! `code.indexed` event payload, plus the file/project filters the daemon
//! applies. Kept pure so the id scheme, the language filter and the
//! project-scope check are unit-tested without a running bus or graph.

use crate::extract::{FileIndex, SymbolKind};
use os_sdk::proto::{CodeFileIndexPayload, CodeSymbolPayload};

/// The language tag the Rust extractor produces.
pub const RUST_LANGUAGE: &str = "rust";

/// The stable id of a symbol: `<source_file>#<kind>:<name>@<line>`.
///
/// Includes the line so two same-kind same-name definitions in one file (rare,
/// e.g. `#[cfg]` duplicates) stay distinct. The whole file's symbols are replaced
/// on a re-parse (per-file isolation), so a line shift on edit re-creates the id
/// rather than orphaning - correct because the replace is atomic.
pub fn symbol_id(source_file: &str, kind: SymbolKind, name: &str, line: usize) -> String {
    format!("{source_file}#{}:{name}@{line}", kind.as_key())
}

/// Whether a path is a Rust source file (CG-R1 is Rust-first).
pub fn is_rust_file(path: &str) -> bool {
    path.ends_with(".rs")
}

/// Whether `path` lies under any of the project `roots` - the project-scope
/// guardrail (code-graph-layer.md §6: only files in a project, never the whole
/// disk). A root matches if `path` equals it or sits below it at a path
/// boundary, so `/p/src/lib.rs` is under `/p` but `/p-other/x` is not under
/// `/p`.
pub fn path_under_any(path: &str, roots: &[String]) -> bool {
    roots.iter().any(|root| {
        let root = root.trim_end_matches('/');
        if root.is_empty() {
            return false;
        }
        path == root
            || (path.starts_with(root)
                && path.as_bytes().get(root.len()) == Some(&b'/'))
    })
}

/// Build the `code.indexed` payload for a file from its extracted index.
///
/// Each symbol becomes a [`CodeSymbolPayload`] with a derived stable id and its
/// 1-based line as `source_location`. The references (calls/imports) the extractor
/// found are NOT carried here: cross-file resolution is CG-R2, so CG-R1 promotes
/// only the definitions and their File fusion.
pub fn build_payload(source_file: &str, index: &FileIndex) -> CodeFileIndexPayload {
    let symbols = index
        .symbols
        .iter()
        .map(|s| CodeSymbolPayload {
            id: symbol_id(source_file, s.kind, &s.name, s.line),
            name: s.name.clone(),
            source_location: s.line.to_string(),
            kind: s.kind.as_key().to_string(),
        })
        .collect();
    CodeFileIndexPayload {
        source_file: source_file.to_string(),
        language: RUST_LANGUAGE.to_string(),
        symbols,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extract::Symbol;

    #[test]
    fn symbol_id_is_stable_and_unique_per_definition() {
        assert_eq!(
            symbol_id("/p/lib.rs", SymbolKind::Function, "foo", 12),
            "/p/lib.rs#function:foo@12"
        );
        // Same name, different line -> distinct id.
        assert_ne!(
            symbol_id("/p/lib.rs", SymbolKind::Function, "foo", 12),
            symbol_id("/p/lib.rs", SymbolKind::Function, "foo", 99)
        );
        // Same name, different kind -> distinct id.
        assert_ne!(
            symbol_id("/p/lib.rs", SymbolKind::Struct, "Foo", 1),
            symbol_id("/p/lib.rs", SymbolKind::Function, "Foo", 1)
        );
    }

    #[test]
    fn rust_filter_matches_only_dot_rs() {
        assert!(is_rust_file("/p/src/lib.rs"));
        assert!(!is_rust_file("/p/README.md"));
        assert!(!is_rust_file("/p/build.rs.bak"));
    }

    #[test]
    fn project_scope_respects_path_boundaries() {
        let roots = vec!["/home/tim/arlen".to_string(), "/home/tim/proj/".to_string()];
        assert!(path_under_any("/home/tim/arlen/src/lib.rs", &roots), "below a root");
        assert!(path_under_any("/home/tim/arlen", &roots), "the root itself");
        assert!(path_under_any("/home/tim/proj/x.rs", &roots), "trailing-slash root normalised");
        assert!(!path_under_any("/home/tim/arlen-backup/x.rs", &roots), "prefix but not a boundary");
        assert!(!path_under_any("/usr/lib/foo.rs", &roots), "outside every root");
        assert!(!path_under_any("/anything", &[]), "no roots -> never in scope");
    }

    #[test]
    fn build_payload_derives_ids_and_drops_references() {
        let index = FileIndex {
            symbols: vec![
                Symbol { name: "helper".into(), kind: SymbolKind::Function, line: 1 },
                Symbol { name: "Widget".into(), kind: SymbolKind::Struct, line: 5 },
            ],
            references: vec![],
        };
        let payload = build_payload("/p/lib.rs", &index);
        assert_eq!(payload.source_file, "/p/lib.rs");
        assert_eq!(payload.language, "rust");
        assert_eq!(payload.symbols.len(), 2);
        assert_eq!(payload.symbols[0].id, "/p/lib.rs#function:helper@1");
        assert_eq!(payload.symbols[0].source_location, "1");
        assert_eq!(payload.symbols[1].kind, "struct");
    }
}

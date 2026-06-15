//! Pure helpers that turn a [`crate::extract::FileIndex`] into the
//! `code.indexed` event payload, plus the file/project filters the daemon
//! applies. Kept pure so the id scheme, the language filter and the
//! project-scope check are unit-tested without a running bus or graph.

use crate::extract::{FileIndex, SymbolKind};
use os_sdk::proto::{CodeFileIndexPayload, CodeSymbolPayload};

/// The language tag the Rust extractor produces.
pub const RUST_LANGUAGE: &str = "rust";

/// The most symbols promoted from one file. A generated / minified `.rs` can
/// declare tens of thousands of symbols; the §6 anti-Nepomuk budget caps the
/// promoted set so one pathological file cannot build a multi-megabyte payload
/// (the event bus drops a frame over 1 MB) or a giant single graph transaction.
/// A file past the cap is indexed up to it (truncation is logged by the daemon).
pub const MAX_SYMBOLS_PER_FILE: usize = 4000;

/// The stable id of a symbol: `<source_file>#<kind>:<name>@<line>:<column>`.
///
/// Includes BOTH line and column so two same-kind same-name definitions on one
/// line (e.g. two `impl` blocks' `fn x` on a single line) stay distinct - without
/// the column they collide on the KG primary key and a duplicate `CREATE` fails
/// the whole promotion transaction. The whole file's symbols are replaced on a
/// re-parse (per-file isolation), so a position shift on edit re-creates the id
/// rather than orphaning - correct because the replace is atomic.
pub fn symbol_id(source_file: &str, kind: SymbolKind, name: &str, line: usize, column: usize) -> String {
    format!("{source_file}#{}:{name}@{line}:{column}", kind.as_key())
}

/// The largest file the indexer reads (anti-Nepomuk §6): a multi-gigabyte or
/// generated `.rs` is skipped rather than read whole into RAM and parsed. 2 MiB
/// comfortably covers hand-written source; minified / generated blobs above it
/// are skipped (and logged), never read.
pub const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;

/// Whether a path is a Rust source file (CG-R1 is Rust-first).
pub fn is_rust_file(path: &str) -> bool {
    path.ends_with(".rs")
}

/// Directory names whose contents are never indexed even when they sit under a
/// project root: build outputs, dependency caches and VCS metadata.
///
/// A generated `.rs` under `target/` (a build script's `OUT_DIR` output, or a
/// macro-expansion artifact) is not the user's authored code. Parsing it wastes
/// the per-file budget on every build (the unbounded-cost trap that gets a
/// semantic indexer disabled - prior-art-lessons.md §3 guardrail 1) and pollutes
/// the code graph with non-authored symbols. Only the cargo build dir and the
/// universal VCS / package-cache dirs are listed - deliberately NOT generic
/// names like `build`/`dist`, which a project may legitimately use for source.
const IGNORED_DIR_COMPONENTS: &[&str] = &[
    "target",       // cargo build output (incl. generated OUT_DIR/*.rs)
    "node_modules", // npm / pnpm
    "vendor",       // vendored dependencies
    ".git",
    ".hg",
    ".svn",
    ".jj",
    ".cache",
    "__pycache__",
];

/// Whether any path component is an ignored build/cache/VCS directory.
///
/// Component-exact (not substring), so a file named `target.rs` or a dir
/// `my-targets/` is unaffected; only a `target/` (etc.) path segment matches.
pub fn path_in_ignored_dir(path: &str) -> bool {
    path.split('/').any(|c| IGNORED_DIR_COMPONENTS.contains(&c))
}

/// Whether a path contains a `..` component - a directory traversal that could
/// escape the project root even while textually prefixed by it (e.g.
/// `/proj/../../etc/x.rs`). [`path_under_any`] is a textual prefix check, so a
/// traversal must be rejected separately. The daemon skips such a path rather
/// than canonicalising it: canonicalisation would diverge the `source_file` from
/// the activity layer's `File` node id (the raw event path) and break the fusion,
/// so a traversal is simply not indexed.
pub fn path_has_traversal(path: &str) -> bool {
    path.split('/').any(|c| c == "..")
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
    let mut seen = std::collections::HashSet::new();
    let symbols = index
        .symbols
        .iter()
        .take(MAX_SYMBOLS_PER_FILE)
        .filter_map(|s| {
            let id = symbol_id(source_file, s.kind, &s.name, s.line, s.column);
            // Defence in depth: even with line:column ids a malformed payload (or a
            // future grammar quirk) must never ship two symbols sharing an id - a
            // duplicate primary key fails the promotion transaction and stalls ALL
            // KG promotion. Drop a colliding id rather than emit it.
            if !seen.insert(id.clone()) {
                return None;
            }
            Some(CodeSymbolPayload {
                id,
                name: s.name.clone(),
                source_location: format!("{}:{}", s.line, s.column),
                kind: s.kind.as_key().to_string(),
            })
        })
        .collect();
    CodeFileIndexPayload {
        source_file: source_file.to_string(),
        language: RUST_LANGUAGE.to_string(),
        symbols,
    }
}

/// Whether the file declared more symbols than the [`MAX_SYMBOLS_PER_FILE`] cap,
/// so the daemon can log the truncation honestly (never silently drop coverage).
pub fn was_truncated(index: &FileIndex) -> bool {
    index.symbols.len() > MAX_SYMBOLS_PER_FILE
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extract::Symbol;

    #[test]
    fn symbol_id_is_stable_and_unique_per_definition() {
        assert_eq!(
            symbol_id("/p/lib.rs", SymbolKind::Function, "foo", 12, 4),
            "/p/lib.rs#function:foo@12:4"
        );
        // Same name, different line -> distinct id.
        assert_ne!(
            symbol_id("/p/lib.rs", SymbolKind::Function, "foo", 12, 4),
            symbol_id("/p/lib.rs", SymbolKind::Function, "foo", 99, 4)
        );
        // Same name + line, different COLUMN -> distinct id (the poison-pill fix:
        // two `fn x` on one line must not collide on the primary key).
        assert_ne!(
            symbol_id("/p/lib.rs", SymbolKind::Method, "x", 1, 8),
            symbol_id("/p/lib.rs", SymbolKind::Method, "x", 1, 30)
        );
        // Same name, different kind -> distinct id.
        assert_ne!(
            symbol_id("/p/lib.rs", SymbolKind::Struct, "Foo", 1, 1),
            symbol_id("/p/lib.rs", SymbolKind::Function, "Foo", 1, 1)
        );
    }

    #[test]
    fn rust_filter_matches_only_dot_rs() {
        assert!(is_rust_file("/p/src/lib.rs"));
        assert!(!is_rust_file("/p/README.md"));
        assert!(!is_rust_file("/p/build.rs.bak"));
    }

    #[test]
    fn build_cache_and_vcs_dirs_are_ignored_by_component() {
        // Generated / dependency / VCS content under a project is never indexed.
        assert!(path_in_ignored_dir("/home/tim/proj/target/debug/build/x-123/out/gen.rs"));
        assert!(path_in_ignored_dir("/home/tim/proj/node_modules/pkg/index.rs"));
        assert!(path_in_ignored_dir("/home/tim/proj/.git/hooks/x.rs"));
        assert!(path_in_ignored_dir("/home/tim/proj/vendor/dep/lib.rs"));
        // Authored source is NOT ignored; the match is component-exact, so a
        // file or dir whose NAME merely contains a keyword is unaffected.
        assert!(!path_in_ignored_dir("/home/tim/proj/src/lib.rs"));
        assert!(!path_in_ignored_dir("/home/tim/proj/src/target.rs"), "file named target.rs is fine");
        assert!(!path_in_ignored_dir("/home/tim/proj/my-targets/build.rs"), "dir name containing 'target' is fine");
        assert!(!path_in_ignored_dir("/home/tim/proj/build.rs"), "a top-level build.rs IS authored source");
    }

    #[test]
    fn traversal_paths_are_detected() {
        assert!(path_has_traversal("/home/tim/proj/../../../etc/shadow.rs"), "escapes the root");
        assert!(path_has_traversal("../x.rs"));
        assert!(!path_has_traversal("/home/tim/proj/src/lib.rs"), "clean absolute path");
        assert!(!path_has_traversal("/home/tim/my..dir/lib.rs"), "dotdot inside a name is not a component");
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
                Symbol { name: "helper".into(), kind: SymbolKind::Function, line: 1, column: 4 },
                Symbol { name: "Widget".into(), kind: SymbolKind::Struct, line: 5, column: 8 },
            ],
            references: vec![],
        };
        let payload = build_payload("/p/lib.rs", &index);
        assert_eq!(payload.source_file, "/p/lib.rs");
        assert_eq!(payload.language, "rust");
        assert_eq!(payload.symbols.len(), 2);
        assert_eq!(payload.symbols[0].id, "/p/lib.rs#function:helper@1:4");
        assert_eq!(payload.symbols[0].source_location, "1:4");
        assert_eq!(payload.symbols[1].kind, "struct");
    }

    #[test]
    fn build_payload_drops_a_colliding_id_and_caps_symbols() {
        // Two symbols that derive the SAME id (same name/kind/line/column) must not
        // both ship - a duplicate primary key would stall promotion. Only one is kept.
        let dup = Symbol { name: "x".into(), kind: SymbolKind::Method, line: 1, column: 8 };
        let index = FileIndex { symbols: vec![dup.clone(), dup], references: vec![] };
        assert_eq!(build_payload("/p/lib.rs", &index).symbols.len(), 1, "colliding id dropped");

        // A file past the cap is truncated to MAX_SYMBOLS_PER_FILE (each id distinct
        // by column here), and was_truncated reports it.
        let many: Vec<Symbol> = (0..MAX_SYMBOLS_PER_FILE + 50)
            .map(|i| Symbol { name: "f".into(), kind: SymbolKind::Function, line: 1, column: i + 1 })
            .collect();
        let big = FileIndex { symbols: many, references: vec![] };
        assert!(was_truncated(&big), "over-cap file is flagged");
        assert_eq!(build_payload("/p/lib.rs", &big).symbols.len(), MAX_SYMBOLS_PER_FILE, "capped");
    }
}

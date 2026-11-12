//! Pure helpers that turn a [`crate::extract::FileIndex`] into the
//! `code.indexed` event payload, plus the file/project filters the daemon
//! applies. Kept pure so the id scheme, the language filter and the
//! project-scope check are unit-tested without a running bus or graph.

use crate::extract::{FileIndex, Language, Symbol, SymbolKind};
use os_sdk::proto::{CodeFileIndexPayload, CodeReferencePayload, CodeSymbolPayload};

/// The source language for a path, by extension, or `None` if the file is not a
/// supported source file (the daemon then skips it without spawning a parse).
/// CG-R3: Rust, Python and TypeScript/TSX.
pub fn language_for_path(path: &str) -> Option<Language> {
    let ext = path.rsplit('.').next().filter(|_| path.contains('.'))?;
    Some(match ext {
        "rs" => Language::Rust,
        "py" | "pyi" => Language::Python,
        "ts" | "mts" | "cts" => Language::TypeScript,
        "tsx" => Language::Tsx,
        "svelte" => Language::Svelte,
        _ => return None,
    })
}

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

/// The most references carried from one file. A reference is small on the wire
/// (an index, a name, a kind and a line), and one definition naming the same
/// thing twice ships once, so hand-written source stays far below this; the cap
/// is for the generated file that would otherwise fill the 1 MB bus frame.
pub const MAX_REFERENCES_PER_FILE: usize = 8000;

/// The id of the symbol that stands for the file itself: `<source_file>#file`.
pub fn file_symbol_id(source_file: &str) -> String {
    format!("{source_file}#file")
}

/// The name a file goes by as a module: its stem, or the directory's name for
/// the files a language uses to stand for their directory (`mod.rs`,
/// `index.ts`, `__init__.py`), since `use a::b` and `import b` name the
/// directory there and never the file.
pub fn module_name(source_file: &str) -> String {
    let mut parts = source_file.rsplit('/');
    let file = parts.next().unwrap_or(source_file);
    let stem = file.split('.').next().unwrap_or(file);
    // Each language has its own such file, and the name means nothing in the
    // others: `index.rs` is a module called `index`.
    let stands_for_directory = match file.rsplit('.').next() {
        Some("rs") => stem == "mod",
        Some("py" | "pyi") => stem == "__init__",
        Some(_) => stem == "index",
        None => false,
    };
    if stands_for_directory {
        if let Some(dir) = parts.next().filter(|d| !d.is_empty()) {
            return dir.to_string();
        }
    }
    stem.to_string()
}

/// The position in `symbols` of the innermost definition whose lines contain
/// `line`, or `None` when the line is outside every definition.
///
/// Innermost is the shortest span, so a call in a method inside an inline
/// module is the method's and not the module's. A symbol whose end is unknown
/// counts as its own line only.
fn enclosing(symbols: &[&Symbol], line: usize) -> Option<usize> {
    symbols
        .iter()
        .enumerate()
        .filter(|(_, s)| s.line <= line && line <= s.end_line.max(s.line))
        .min_by_key(|(_, s)| s.end_line.max(s.line) - s.line)
        .map(|(i, _)| i)
}

/// Build the `code.indexed` payload for a file from its extracted index.
///
/// The first symbol is always the file itself (kind `file`), so a reference
/// made outside every definition still has a symbol to start from. Each
/// extracted symbol follows as a [`CodeSymbolPayload`] with a derived stable id
/// and its 1-based line as `source_location`.
///
/// Each reference names the symbol it was made in by that symbol's position in
/// the list, and the NAME it refers to. Which definition that name binds to is
/// not decided here: this file is all the indexer has seen, and the binding
/// needs the whole project.
pub fn build_payload(source_file: &str, language: Language, index: &FileIndex) -> CodeFileIndexPayload {
    let mut seen = std::collections::HashSet::new();
    let mut kept: Vec<&Symbol> = Vec::new();
    let mut symbols = vec![CodeSymbolPayload {
        id: file_symbol_id(source_file),
        name: module_name(source_file),
        source_location: "1:1".to_string(),
        kind: SymbolKind::File.as_key().to_string(),
    }];
    for s in index.symbols.iter().take(MAX_SYMBOLS_PER_FILE) {
        let id = symbol_id(source_file, s.kind, &s.name, s.line, s.column);
        // Defence in depth: even with line:column ids a malformed payload (or a
        // future grammar quirk) must never ship two symbols sharing an id - a
        // duplicate primary key fails the promotion transaction and stalls ALL
        // KG promotion. Drop a colliding id rather than emit it.
        if !seen.insert(id.clone()) {
            continue;
        }
        kept.push(s);
        symbols.push(CodeSymbolPayload {
            id,
            name: s.name.clone(),
            source_location: format!("{}:{}", s.line, s.column),
            kind: s.kind.as_key().to_string(),
        });
    }

    let mut said = std::collections::HashSet::new();
    let mut references = Vec::new();
    for r in &index.references {
        if references.len() == MAX_REFERENCES_PER_FILE {
            break;
        }
        // `kept[i]` is `symbols[i + 1]`: the file symbol holds position 0.
        let from = enclosing(&kept, r.line).map_or(0, |i| i + 1);
        // One definition naming the same thing twice is one edge.
        if !said.insert((from, r.name.as_str(), r.kind.as_key())) {
            continue;
        }
        references.push(CodeReferencePayload {
            from: from as u32,
            name: r.name.clone(),
            kind: r.kind.as_key().to_string(),
            line: r.line as u32,
        });
    }

    CodeFileIndexPayload {
        source_file: source_file.to_string(),
        language: language.as_key().to_string(),
        symbols,
        references,
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
    fn language_is_detected_by_extension() {
        assert_eq!(language_for_path("/p/src/lib.rs"), Some(Language::Rust));
        assert_eq!(language_for_path("/p/app.py"), Some(Language::Python));
        assert_eq!(language_for_path("/p/types.pyi"), Some(Language::Python));
        assert_eq!(language_for_path("/p/index.ts"), Some(Language::TypeScript));
        assert_eq!(language_for_path("/p/mod.mts"), Some(Language::TypeScript));
        assert_eq!(language_for_path("/p/App.tsx"), Some(Language::Tsx));
        assert_eq!(language_for_path("/p/Card.svelte"), Some(Language::Svelte));
        assert_eq!(language_for_path("/p/README.md"), None);
        assert_eq!(language_for_path("/p/Makefile"), None, "no extension");
        assert_eq!(language_for_path("/p/.gitignore"), None, "dotfile, not a .rs/.py/.ts");
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

    fn sym(name: &str, kind: SymbolKind, line: usize, column: usize, end_line: usize) -> Symbol {
        Symbol { name: name.into(), kind, line, column, end_line }
    }

    #[test]
    fn build_payload_leads_with_the_file_and_derives_ids() {
        let index = FileIndex {
            symbols: vec![
                sym("helper", SymbolKind::Function, 1, 4, 3),
                sym("Widget", SymbolKind::Struct, 5, 8, 7),
            ],
            references: vec![],
        };
        let payload = build_payload("/p/lib.rs", Language::Rust, &index);
        assert_eq!(payload.source_file, "/p/lib.rs");
        assert_eq!(payload.language, "rust");
        assert_eq!(payload.symbols.len(), 3);
        assert_eq!(payload.symbols[0].id, "/p/lib.rs#file");
        assert_eq!(payload.symbols[0].kind, "file");
        assert_eq!(payload.symbols[0].name, "lib");
        assert_eq!(payload.symbols[1].id, "/p/lib.rs#function:helper@1:4");
        assert_eq!(payload.symbols[1].source_location, "1:4");
        assert_eq!(payload.symbols[2].kind, "struct");
        assert!(payload.references.is_empty());
    }

    #[test]
    fn a_reference_starts_from_the_innermost_definition_around_it() {
        use crate::extract::{RefKind, Reference};
        let r = |name: &str, kind, line| Reference { name: name.into(), kind, line };
        let index = FileIndex {
            symbols: vec![
                sym("outer", SymbolKind::Module, 3, 5, 20),
                sym("run", SymbolKind::Function, 5, 8, 9),
                sym("Widget", SymbolKind::Struct, 12, 12, 14),
            ],
            references: vec![
                r("HashMap", RefKind::Import, 1),  // above every definition
                r("helper", RefKind::Call, 6),     // inside run, inside outer
                r("helper", RefKind::Call, 8),     // the same call again
                r("Size", RefKind::TypeUse, 13),   // a field of Widget
                r("init", RefKind::Call, 18),      // in outer, in no function
            ],
        };
        let payload = build_payload("/p/src/lib.rs", Language::Rust, &index);
        let got: Vec<(&str, &str, &str)> = payload
            .references
            .iter()
            .map(|r| (payload.symbols[r.from as usize].name.as_str(), r.name.as_str(), r.kind.as_str()))
            .collect();
        assert_eq!(
            got,
            vec![
                ("lib", "HashMap", "imports"),
                ("run", "helper", "calls"),
                ("Widget", "Size", "references"),
                ("outer", "init", "calls"),
            ],
            "the innermost span wins, the file takes what no definition holds, a repeat ships once"
        );
    }

    #[test]
    fn a_directory_file_goes_by_its_directory() {
        assert_eq!(module_name("/p/src/graph/mod.rs"), "graph");
        assert_eq!(module_name("/p/src/lib/index.ts"), "lib");
        assert_eq!(module_name("/p/pkg/__init__.py"), "pkg");
        assert_eq!(module_name("/p/src/promotion.rs"), "promotion");
        assert_eq!(module_name("/p/src/index.rs"), "index", "only a directory file in its own language");
        assert_eq!(module_name("/p/types.d.ts"), "types");
    }

    #[test]
    fn build_payload_drops_a_colliding_id_and_caps_symbols() {
        // Two symbols that derive the SAME id (same name/kind/line/column) must not
        // both ship - a duplicate primary key would stall promotion. Only one is kept.
        let dup = sym("x", SymbolKind::Method, 1, 8, 1);
        let index = FileIndex { symbols: vec![dup.clone(), dup], references: vec![] };
        assert_eq!(build_payload("/p/lib.rs", Language::Rust, &index).symbols.len(), 2, "colliding id dropped");

        // A file past the cap is truncated to MAX_SYMBOLS_PER_FILE (each id distinct
        // by column here), and was_truncated reports it. The file itself is one more.
        let many: Vec<Symbol> = (0..MAX_SYMBOLS_PER_FILE + 50)
            .map(|i| sym("f", SymbolKind::Function, 1, i + 1, 1))
            .collect();
        let big = FileIndex { symbols: many, references: vec![] };
        assert!(was_truncated(&big), "over-cap file is flagged");
        assert_eq!(
            build_payload("/p/lib.rs", Language::Rust, &big).symbols.len(),
            MAX_SYMBOLS_PER_FILE + 1,
            "capped"
        );
    }
}

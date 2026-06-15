//! The per-file extraction core (code-graph-layer.md CG-R1 + CG-R3).
//!
//! Parses ONE source file in complete isolation (tree-sitter, purely syntactic,
//! no build, no cross-file visibility) and returns its [`FileIndex`]: the
//! `CodeSymbol` definitions it declares, and the outgoing references (calls,
//! imports) it makes. This is the load-bearing "per-file isolation at index"
//! design - a file-save re-runs only this, never a whole-project reindex.
//!
//! Multi-language ([`Language`]): Rust, Python and TypeScript share this one
//! extractor. Each language brings its own tree-sitter grammar and capture query
//! ([`Language::query`]); the classification, dedup and reference handling are
//! shared. Adding a language is a grammar + a query, nothing more (CG-R3).
//!
//! What it does NOT do: resolve a reference to its target definition. A call to
//! `bar()` records the NAME `bar`; which `CodeSymbol` it binds to is cross-file
//! and is resolved at QUERY time (CG-R2, stack-graph path-finding). So the
//! definitions here are [`Confidence::Extracted`] (syntactically present), and
//! the eventual CALLS/IMPORTS edges carry their resolution confidence when CG-R2
//! materialises them. Honest by construction: this layer never claims to know a
//! binding it cannot see.

/// The confidence label on an extracted fact (code-graph-layer.md §5). Mandatory:
/// syntactic-only analysis loses overrides/shadowing/aliases/types, so a fact is
/// never presented as ground truth above what the method can actually know.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confidence {
    /// Explicit in the source (a definition, an `import`/`use`, a direct call
    /// site). The fact's EXISTENCE is certain.
    Extracted,
    /// A reasonable deduction (a cross-file resolution with a single candidate).
    /// Produced by CG-R2 resolution, not by this per-file extractor.
    Inferred,
    /// Uncertain - multiple candidate targets. Every candidate is kept; the
    /// resolution is never collapsed to one. Produced by CG-R2.
    Ambiguous,
}

impl Confidence {
    /// The stable lowercase key stored on the KG edge.
    pub fn as_key(self) -> &'static str {
        match self {
            Confidence::Extracted => "extracted",
            Confidence::Inferred => "inferred",
            Confidence::Ambiguous => "ambiguous",
        }
    }
}

/// What kind of code definition a [`Symbol`] is. A small shared core
/// (code-graph-layer.md §4); per-language extras would attach as attributes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolKind {
    /// A free function (`fn foo`).
    Function,
    /// A method (`fn foo` inside an `impl`).
    Method,
    /// A `struct`.
    Struct,
    /// An `enum`.
    Enum,
    /// A `trait`.
    Trait,
    /// A `mod`.
    Module,
    /// A `type` alias.
    TypeAlias,
    /// A `const`.
    Const,
    /// A `static`.
    Static,
    /// A `macro_rules!` definition.
    Macro,
    /// A class (Python `class`, TypeScript `class`). Rust has no class; this is
    /// a shared-core kind for the object-oriented languages.
    Class,
    /// An interface (TypeScript `interface`). Distinct from a Rust `trait`
    /// (which keeps [`SymbolKind::Trait`]) so the language's own vocabulary shows.
    Interface,
}

impl SymbolKind {
    /// The stable lowercase key stored on the `CodeSymbol` node.
    pub fn as_key(self) -> &'static str {
        match self {
            SymbolKind::Function => "function",
            SymbolKind::Method => "method",
            SymbolKind::Struct => "struct",
            SymbolKind::Enum => "enum",
            SymbolKind::Trait => "trait",
            SymbolKind::Module => "module",
            SymbolKind::TypeAlias => "type_alias",
            SymbolKind::Const => "const",
            SymbolKind::Static => "static",
            SymbolKind::Macro => "macro",
            SymbolKind::Class => "class",
            SymbolKind::Interface => "interface",
        }
    }
}

/// A code definition extracted from a file: a function, type, module, etc. The
/// `line` is 1-based (the row the name appears on), the unit the UI and the
/// `source_location` use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    /// The declared name (e.g. `foo`, `MyStruct`).
    pub name: String,
    /// The kind of definition.
    pub kind: SymbolKind,
    /// 1-based line of the name.
    pub line: usize,
    /// 1-based column of the name. Carried so two same-name same-line definitions
    /// (e.g. two `impl` blocks' `fn x` on one line) get distinct symbol ids -
    /// without it they collide on the KG primary key and stall promotion.
    pub column: usize,
}

/// What kind of outgoing reference a [`Reference`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefKind {
    /// A call site (`bar()`, `x.method()`, `path::bar()`).
    Call,
    /// An import (`use a::b::c;`).
    Import,
}

impl RefKind {
    /// The stable lowercase key (the eventual edge label, lowercased).
    pub fn as_key(self) -> &'static str {
        match self {
            RefKind::Call => "calls",
            RefKind::Import => "imports",
        }
    }
}

/// An outgoing reference the file makes - the NAME referenced, not yet resolved
/// to a target [`Symbol`] (cross-file resolution is CG-R2). The `name` is the
/// callee identifier or the imported path's final segment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reference {
    /// The referenced name (callee, imported path tail).
    pub name: String,
    /// Whether it is a call or an import.
    pub kind: RefKind,
    /// 1-based line of the reference.
    pub line: usize,
}

/// The complete per-file index: the definitions the file declares and the
/// references it makes. Built in isolation from a single file's source.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileIndex {
    /// The `CodeSymbol` definitions in this file (all [`Confidence::Extracted`]).
    pub symbols: Vec<Symbol>,
    /// The outgoing call/import references (names only; targets resolved at
    /// query time, CG-R2).
    pub references: Vec<Reference>,
}

/// A source language the extractor supports. Adding one is a grammar
/// ([`Language::grammar`]) + a capture query ([`Language::query`]); the rest of
/// the pipeline is language-agnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    /// Rust (`.rs`).
    Rust,
    /// Python (`.py`, `.pyi`).
    Python,
    /// TypeScript (`.ts`, `.mts`, `.cts`) and TSX (`.tsx`).
    TypeScript,
    /// TSX (`.tsx`) - TypeScript with JSX; a distinct grammar.
    Tsx,
    /// Svelte (`.svelte`) - the code lives in `<script>` block(s), extracted via
    /// the TypeScript grammar (no separate svelte grammar; the markup declares
    /// no symbols).
    Svelte,
}

impl Language {
    /// The stable lowercase tag stored on the `code.indexed` payload + the graph.
    pub fn as_key(self) -> &'static str {
        match self {
            // TSX is TypeScript-with-JSX; it tags as `typescript` in the graph
            // (same language, the grammar split is an internal parsing detail).
            Language::Rust => "rust",
            Language::Python => "python",
            Language::TypeScript | Language::Tsx => "typescript",
            Language::Svelte => "svelte",
        }
    }

    /// The tree-sitter grammar for this language. Svelte has no single grammar
    /// (it delegates to TypeScript per `<script>` block in [`extract_svelte`],
    /// which short-circuits before this is called); the TS grammar is the inert
    /// fallback so the match stays total.
    fn grammar(self) -> tree_sitter::Language {
        match self {
            Language::Rust => tree_sitter_rust::LANGUAGE.into(),
            Language::Python => tree_sitter_python::LANGUAGE.into(),
            Language::TypeScript | Language::Svelte => {
                tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()
            }
            Language::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
        }
    }

    /// The capture query that tags this language's definitions + references.
    fn query(self) -> &'static str {
        match self {
            Language::Rust => RUST_QUERY,
            Language::Python => PYTHON_QUERY,
            Language::TypeScript | Language::Tsx | Language::Svelte => TYPESCRIPT_QUERY,
        }
    }
}

/// The tree-sitter query that captures Rust definitions + references in one pass.
/// Each pattern tags its capture so [`extract`] can classify it. Methods
/// (a `function_item` under an `impl_item`) are distinguished from free
/// functions by matching the impl-nested shape separately.
const RUST_QUERY: &str = r#"
; --- definitions ---
(function_item name: (identifier) @function)
(impl_item body: (declaration_list (function_item name: (identifier) @method)))
(trait_item body: (declaration_list (function_item name: (identifier) @method)))
(trait_item body: (declaration_list (function_signature_item name: (identifier) @method)))
(struct_item name: (type_identifier) @struct)
(enum_item name: (type_identifier) @enum)
(trait_item name: (type_identifier) @trait)
(mod_item name: (identifier) @module)
(type_item name: (type_identifier) @type_alias)
(const_item name: (identifier) @const)
(static_item name: (identifier) @static)
(macro_definition name: (identifier) @macro)

; --- references ---
(call_expression function: (identifier) @call)
(call_expression function: (scoped_identifier name: (identifier) @call))
(call_expression function: (field_expression field: (field_identifier) @call))
(use_declaration) @use
"#;

/// Python definitions + references. A `function_definition` inside a class body
/// is captured `@method`; the broad `@function` also matches it and the shared
/// (name, line) dedup drops the duplicate (the [`SymbolKind::Method`] wins).
/// Imported names are captured directly (the `@import` node IS the name), so no
/// text-parse is needed (unlike Rust's `use`).
const PYTHON_QUERY: &str = r#"
; --- definitions ---
(function_definition name: (identifier) @function)
(class_definition body: (block (function_definition name: (identifier) @method)))
(class_definition name: (identifier) @class)

; --- references ---
(call function: (identifier) @call)
(call function: (attribute attribute: (identifier) @call))
(import_statement name: (dotted_name (identifier) @import))
(import_from_statement name: (dotted_name (identifier) @import))
(aliased_import alias: (identifier) @import)
"#;

/// TypeScript (and TSX) definitions + references. `function_declaration` and
/// `method_definition` are distinct node types, so methods need no dedup. Const
/// arrow functions (`const f = () => {}`) are intentionally not captured here -
/// they are ambiguous between a value and a function, a later refinement.
const TYPESCRIPT_QUERY: &str = r#"
; --- definitions ---
(function_declaration name: (identifier) @function)
(method_definition name: (property_identifier) @method)
(class_declaration name: (type_identifier) @class)
(interface_declaration name: (type_identifier) @interface)
(enum_declaration name: (identifier) @enum)
(type_alias_declaration name: (type_identifier) @type_alias)

; --- references ---
(call_expression function: (identifier) @call)
(call_expression function: (member_expression property: (property_identifier) @call))
(import_specifier name: (identifier) @import)
(import_clause (identifier) @import)
(namespace_import (identifier) @import)
"#;

/// Extract the [`FileIndex`] from one Rust source file, in isolation.
///
/// Purely syntactic: no build, no other-file visibility. A malformed file yields
/// whatever tree-sitter can recover (error-tolerant parse) plus the symbols it
/// did recognise, never a panic. Definitions are deduplicated only by
/// (name, kind, line); a genuinely duplicated name on different lines is kept
/// (it is a real second definition the resolver must disambiguate).
pub fn extract(language: Language, source: &str) -> FileIndex {
    // Svelte is not a single grammar: its symbols live in `<script>` block(s),
    // extracted via the TypeScript grammar with the block's line offset applied.
    if language == Language::Svelte {
        return extract_svelte(source);
    }
    let mut parser = tree_sitter::Parser::new();
    let grammar = language.grammar();
    if parser.set_language(&grammar).is_err() {
        return FileIndex::default();
    }
    let Some(tree) = parser.parse(source, None) else {
        return FileIndex::default();
    };
    let Ok(query) = tree_sitter::Query::new(&grammar, language.query()) else {
        return FileIndex::default();
    };

    let bytes = source.as_bytes();
    let mut symbols = Vec::new();
    let mut references = Vec::new();
    let mut cursor = tree_sitter::QueryCursor::new();
    let capture_names = query.capture_names();

    use tree_sitter::StreamingIterator as _;
    let mut matches = cursor.matches(&query, tree.root_node(), bytes);
    while let Some(m) = matches.next() {
        for cap in m.captures {
            let cap_name = capture_names[cap.index as usize];
            let node = cap.node;
            let line = node.start_position().row + 1;
            let column = node.start_position().column + 1;
            // The captured text is the name node's bytes; lossless utf8 only.
            let Ok(text) = node.utf8_text(bytes) else {
                continue;
            };
            match cap_name {
                "use" => {
                    // `use a::b::c;` / `use a::b::{c, d};` - record the imported
                    // tail name(s). The site is EXTRACTED; resolution is CG-R2.
                    for name in import_names(text) {
                        references.push(Reference {
                            name,
                            kind: RefKind::Import,
                            line,
                        });
                    }
                }
                "import" => {
                    // Python / TypeScript: the captured node IS the imported
                    // name (no text-parse), recorded as an import reference.
                    if is_ident(text) {
                        references.push(Reference {
                            name: text.to_string(),
                            kind: RefKind::Import,
                            line,
                        });
                    }
                }
                "call" => references.push(Reference {
                    name: text.to_string(),
                    kind: RefKind::Call,
                    line,
                }),
                other => {
                    if let Some(kind) = symbol_kind(other) {
                        symbols.push(Symbol {
                            name: text.to_string(),
                            kind,
                            line,
                            column,
                        });
                    }
                }
            }
        }
    }

    // A method (an impl- or trait-nested `function_item`) is also matched by the
    // broad `@function` pattern, so it lands twice. The `@method` capture wins:
    // drop the duplicate `Function` at the same (name, line).
    let method_keys: std::collections::HashSet<(&str, usize)> = symbols
        .iter()
        .filter(|s| s.kind == SymbolKind::Method)
        .map(|s| (s.name.as_str(), s.line))
        .collect();
    let dup: Vec<(String, usize)> = symbols
        .iter()
        .filter(|s| s.kind == SymbolKind::Function && method_keys.contains(&(s.name.as_str(), s.line)))
        .map(|s| (s.name.clone(), s.line))
        .collect();
    symbols.retain(|s| {
        s.kind != SymbolKind::Function || !dup.iter().any(|(n, l)| n == &s.name && *l == s.line)
    });

    FileIndex {
        symbols,
        references,
    }
}

/// Extract a Rust file (the CG-R1 entry point, kept as the Rust-specific wrapper
/// over the generalised [`extract`]).
pub fn extract_rust(source: &str) -> FileIndex {
    extract(Language::Rust, source)
}

/// Extract a Svelte component. Its code lives in `<script>` block(s) (TS or JS;
/// the TS grammar parses both); the markup declares no symbols. Each block's
/// inner content is run through the TypeScript extractor and its 1-based line
/// numbers are offset back to the block's position in the `.svelte` file, so a
/// symbol's `source_location` points at the real line. A module-context block
/// and the instance block are both handled.
fn extract_svelte(source: &str) -> FileIndex {
    let mut out = FileIndex::default();
    for (inner, start_line) in svelte_script_blocks(source) {
        let block = extract(Language::TypeScript, inner);
        // The block's first inner line sits at file line `start_line`, so an
        // inner line N maps to file line `start_line + (N - 1)`.
        let offset = start_line.saturating_sub(1);
        for mut s in block.symbols {
            s.line += offset;
            out.symbols.push(s);
        }
        for mut r in block.references {
            r.line += offset;
            out.references.push(r);
        }
    }
    out
}

/// Find each `<script>...</script>` block in a Svelte file: returns
/// `(inner_text, 1-based file line of the inner content's start)`. Case-
/// insensitive on the tag and tolerant of attributes (`lang="ts"`,
/// `context="module"`). Fail-safe: a malformed / unterminated tag ends the scan
/// rather than panicking. The lowercased copy is only used to LOCATE ASCII tag
/// bytes (ASCII lowercasing is length-preserving, so byte offsets align with
/// `source`); the returned slices come from `source`.
fn svelte_script_blocks(source: &str) -> Vec<(&str, usize)> {
    let lower = source.to_ascii_lowercase();
    let mut blocks = Vec::new();
    let mut pos = 0;
    while let Some(rel) = lower[pos..].find("<script") {
        let tag_start = pos + rel;
        let Some(gt_rel) = lower[tag_start..].find('>') else {
            break;
        };
        let inner_start = tag_start + gt_rel + 1;
        let Some(close_rel) = lower[inner_start..].find("</script") else {
            break;
        };
        let inner_end = inner_start + close_rel;
        let inner = &source[inner_start..inner_end];
        let start_line = source[..inner_start].bytes().filter(|&b| b == b'\n').count() + 1;
        blocks.push((inner, start_line));
        pos = inner_end;
    }
    blocks
}

/// Map a definition capture name to its [`SymbolKind`].
fn symbol_kind(capture: &str) -> Option<SymbolKind> {
    Some(match capture {
        "function" => SymbolKind::Function,
        "method" => SymbolKind::Method,
        "struct" => SymbolKind::Struct,
        "enum" => SymbolKind::Enum,
        "trait" => SymbolKind::Trait,
        "module" => SymbolKind::Module,
        "type_alias" => SymbolKind::TypeAlias,
        "const" => SymbolKind::Const,
        "static" => SymbolKind::Static,
        "macro" => SymbolKind::Macro,
        "class" => SymbolKind::Class,
        "interface" => SymbolKind::Interface,
        _ => return None,
    })
}

/// Extract the imported tail name(s) from a `use_declaration`'s source text.
///
/// Handles the common shapes syntactically: `use a::b::c;` -> `c`;
/// `use a::b::{c, d};` -> `c`, `d`; `use a::b as e;` -> `e` (the alias is what
/// the local scope binds). A glob `use a::*;` contributes no specific name (the
/// targets are unknowable per-file; CG-R2's resolution covers globs). Best-effort
/// + syntactic; the resolution to a real symbol is CG-R2.
fn import_names(use_text: &str) -> Vec<String> {
    // Strip the leading `use`, the trailing `;`, and any `pub`/whitespace.
    let body = use_text
        .trim()
        .trim_start_matches("pub")
        .trim()
        .trim_start_matches("use")
        .trim()
        .trim_end_matches(';')
        .trim();

    // Aliased single import: `a::b as e` -> the alias `e`.
    if let Some((_, alias)) = body.rsplit_once(" as ") {
        let alias = alias.trim();
        if is_ident(alias) {
            return vec![alias.to_string()];
        }
    }

    // Braced group: `a::b::{c, d as e, f}` -> each tail. The `open < close` guard
    // is load-bearing: tree-sitter error-recovers a malformed `use a::}{;` into a
    // use_declaration whose text has the braces reversed, and slicing `open+1..close`
    // with open > close would panic (the no-panic contract this function promises).
    if let (Some(open), Some(close)) = (body.find('{'), body.rfind('}')) {
        if open < close {
            let inner = &body[open + 1..close];
            return inner
                .split(',')
                .filter_map(|item| {
                    let item = item.trim();
                    let tail = item.rsplit(" as ").next().unwrap_or(item).trim();
                    let tail = tail.rsplit("::").next().unwrap_or(tail).trim();
                    (is_ident(tail)).then(|| tail.to_string())
                })
                .collect();
        }
    }

    // Plain path: `a::b::c` -> `c`. A glob tail `*` is dropped.
    let tail = body.rsplit("::").next().unwrap_or(body).trim();
    if is_ident(tail) {
        vec![tail.to_string()]
    } else {
        Vec::new()
    }
}

/// Whether a string is a plausible Rust identifier (so a glob `*`, a stray brace
/// fragment, or empty is not recorded as an imported name).
fn is_ident(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !s.chars().next().is_some_and(|c| c.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names_of(idx: &FileIndex, kind: SymbolKind) -> Vec<&str> {
        idx.symbols
            .iter()
            .filter(|s| s.kind == kind)
            .map(|s| s.name.as_str())
            .collect()
    }

    fn refs_of(idx: &FileIndex, kind: RefKind) -> Vec<&str> {
        idx.references
            .iter()
            .filter(|r| r.kind == kind)
            .map(|r| r.name.as_str())
            .collect()
    }

    #[test]
    fn extracts_functions_and_their_call_site() {
        let src = r#"
            fn helper() -> i32 { 42 }
            fn main() {
                let _ = helper();
            }
        "#;
        let idx = extract_rust(src);
        let fns = names_of(&idx, SymbolKind::Function);
        assert!(fns.contains(&"helper"), "helper is a function def");
        assert!(fns.contains(&"main"), "main is a function def");
        assert!(refs_of(&idx, RefKind::Call).contains(&"helper"), "the call to helper is recorded");
    }

    #[test]
    fn methods_are_distinguished_from_free_functions() {
        let src = r#"
            struct Widget;
            impl Widget {
                fn render(&self) {}
            }
            fn free() {}
        "#;
        let idx = extract_rust(src);
        assert_eq!(names_of(&idx, SymbolKind::Method), vec!["render"], "render is a method");
        assert_eq!(names_of(&idx, SymbolKind::Function), vec!["free"], "free is a function");
        assert_eq!(names_of(&idx, SymbolKind::Struct), vec!["Widget"]);
    }

    #[test]
    fn extracts_types_traits_modules_and_consts() {
        let src = r#"
            mod inner {}
            enum Color { Red, Green }
            trait Draw { fn draw(&self); }
            type Id = u64;
            const MAX: usize = 10;
            static NAME: &str = "x";
            macro_rules! mac { () => {}; }
        "#;
        let idx = extract_rust(src);
        assert_eq!(names_of(&idx, SymbolKind::Module), vec!["inner"]);
        assert_eq!(names_of(&idx, SymbolKind::Enum), vec!["Color"]);
        assert_eq!(names_of(&idx, SymbolKind::Trait), vec!["Draw"]);
        assert_eq!(names_of(&idx, SymbolKind::TypeAlias), vec!["Id"]);
        assert_eq!(names_of(&idx, SymbolKind::Const), vec!["MAX"]);
        assert_eq!(names_of(&idx, SymbolKind::Static), vec!["NAME"]);
        assert_eq!(names_of(&idx, SymbolKind::Macro), vec!["mac"]);
        // A trait method declaration is a method, not a free function.
        assert!(names_of(&idx, SymbolKind::Method).contains(&"draw"), "trait method is a method");
        assert!(!names_of(&idx, SymbolKind::Function).contains(&"draw"), "not double-counted as a function");
    }

    #[test]
    fn extracts_imports_in_all_shapes() {
        let src = r#"
            use std::collections::HashMap;
            use std::io::{Read, Write};
            use std::fmt::Debug as Dbg;
            use crate::foo::*;
        "#;
        let idx = extract_rust(src);
        let imports = refs_of(&idx, RefKind::Import);
        assert!(imports.contains(&"HashMap"), "plain path tail");
        assert!(imports.contains(&"Read") && imports.contains(&"Write"), "braced group");
        assert!(imports.contains(&"Dbg"), "alias binds the alias name");
        assert!(!imports.contains(&"*"), "a glob contributes no specific name");
    }

    #[test]
    fn method_and_path_calls_are_recorded() {
        let src = r#"
            fn run() {
                thing.method();
                std::process::exit(0);
                plain();
            }
        "#;
        let idx = extract_rust(src);
        let calls = refs_of(&idx, RefKind::Call);
        assert!(calls.contains(&"method"), "method call");
        assert!(calls.contains(&"exit"), "path call tail");
        assert!(calls.contains(&"plain"), "plain call");
    }

    #[test]
    fn malformed_source_does_not_panic() {
        // Error-tolerant: a broken file still yields what tree-sitter recovers.
        let idx = extract_rust("fn ok() {} fn broken( { let ");
        assert!(names_of(&idx, SymbolKind::Function).contains(&"ok"));
        // A reversed-brace use is recovered by tree-sitter into a use_declaration
        // whose text has `}` before `{`; the slice guard must not panic.
        let _ = extract_rust("use a::}{;\nfn f() {}");
        let _ = extract_rust("use ::{;");
    }

    #[test]
    fn same_name_definitions_on_one_line_get_distinct_columns() {
        // Two methods named `x` on a single valid line: without a distinct column
        // their symbol ids would collide on the KG primary key and stall the whole
        // promotion pipeline. They must differ in column.
        let idx = extract_rust("impl A{fn x(&self){}} impl B{fn x(&self){}}");
        let xs: Vec<&Symbol> = idx
            .symbols
            .iter()
            .filter(|s| s.name == "x" && s.kind == SymbolKind::Method)
            .collect();
        assert_eq!(xs.len(), 2, "both methods extracted");
        assert_eq!(xs[0].line, xs[1].line, "on the same line");
        assert_ne!(xs[0].column, xs[1].column, "but distinct columns");
    }

    #[test]
    fn dogfoods_this_files_own_symbols() {
        // Parse the extractor's own source - the CG-R1 promise (index Arlen's
        // own Rust) and a regression guard over the real grammar shapes.
        let idx = extract_rust(include_str!("extract.rs"));
        let fns = names_of(&idx, SymbolKind::Function);
        assert!(fns.contains(&"extract_rust"), "the public entry point");
        assert!(fns.contains(&"import_names") && fns.contains(&"is_ident"), "the private helpers");
        let methods = names_of(&idx, SymbolKind::Method);
        assert!(methods.contains(&"as_key"), "the impl methods");
        let structs = names_of(&idx, SymbolKind::Struct);
        assert!(structs.contains(&"Symbol") && structs.contains(&"FileIndex"));
        let enums = names_of(&idx, SymbolKind::Enum);
        assert!(enums.contains(&"Confidence") && enums.contains(&"SymbolKind") && enums.contains(&"RefKind"));
        // It also records its own outgoing calls (e.g. into tree-sitter).
        assert!(!idx.references.is_empty(), "the file makes calls/imports");
    }

    #[test]
    fn confidence_and_kind_keys_are_stable() {
        assert_eq!(Confidence::Extracted.as_key(), "extracted");
        assert_eq!(Confidence::Ambiguous.as_key(), "ambiguous");
        assert_eq!(SymbolKind::TypeAlias.as_key(), "type_alias");
        assert_eq!(SymbolKind::Class.as_key(), "class");
        assert_eq!(SymbolKind::Interface.as_key(), "interface");
        assert_eq!(RefKind::Call.as_key(), "calls");
        assert_eq!(RefKind::Import.as_key(), "imports");
        assert_eq!(Language::Rust.as_key(), "rust");
        assert_eq!(Language::Python.as_key(), "python");
        assert_eq!(Language::TypeScript.as_key(), "typescript");
        assert_eq!(Language::Tsx.as_key(), "typescript", "tsx tags as typescript");
    }

    #[test]
    fn extracts_python_definitions_and_references() {
        let src = r#"
import os
from collections import OrderedDict
from typing import List as L

def helper(x):
    return x

class Widget:
    def render(self):
        helper(1)
        os.getcwd()
"#;
        let idx = extract(Language::Python, src);
        assert!(names_of(&idx, SymbolKind::Function).contains(&"helper"), "module function");
        assert!(names_of(&idx, SymbolKind::Class).contains(&"Widget"), "class");
        assert_eq!(names_of(&idx, SymbolKind::Method), vec!["render"], "class method");
        assert!(!names_of(&idx, SymbolKind::Function).contains(&"render"), "method not double-counted");
        let imports = refs_of(&idx, RefKind::Import);
        assert!(imports.contains(&"os"), "plain import");
        assert!(imports.contains(&"OrderedDict"), "from-import name");
        assert!(imports.contains(&"L"), "aliased import binds the alias");
        let calls = refs_of(&idx, RefKind::Call);
        assert!(calls.contains(&"helper"), "direct call");
        assert!(calls.contains(&"getcwd"), "attribute call");
    }

    #[test]
    fn extracts_typescript_definitions_and_references() {
        let src = r#"
import { foo, bar } from "./mod";
import baz from "pkg";
import * as ns from "ns";

interface Shape { area(): number; }
type Id = string;
enum Color { Red, Green }

function freeFn(): void { foo(); }

class Widget {
    render(): void { bar(); ns.helper(); }
}
"#;
        let idx = extract(Language::TypeScript, src);
        assert!(names_of(&idx, SymbolKind::Function).contains(&"freeFn"), "function declaration");
        assert!(names_of(&idx, SymbolKind::Class).contains(&"Widget"), "class");
        assert!(names_of(&idx, SymbolKind::Method).contains(&"render"), "method");
        assert!(names_of(&idx, SymbolKind::Interface).contains(&"Shape"), "interface");
        assert!(names_of(&idx, SymbolKind::TypeAlias).contains(&"Id"), "type alias");
        assert!(names_of(&idx, SymbolKind::Enum).contains(&"Color"), "enum");
        let imports = refs_of(&idx, RefKind::Import);
        assert!(imports.contains(&"foo") && imports.contains(&"bar"), "named imports");
        assert!(imports.contains(&"baz"), "default import");
        assert!(imports.contains(&"ns"), "namespace import");
        let calls = refs_of(&idx, RefKind::Call);
        assert!(calls.contains(&"foo"), "direct call");
        assert!(calls.contains(&"helper"), "member call");
    }

    #[test]
    fn extracts_svelte_script_symbols_with_correct_line_offset() {
        // Symbols live in the <script> block; the markup declares none. Two
        // blocks (module + instance) are both extracted, and line numbers are
        // offset back to the .svelte file.
        let src = "<script context=\"module\">\nexport function load() {}\n</script>\n\n<script lang=\"ts\">\nimport { onMount } from \"svelte\";\nfunction handleClick() { doThing(); }\n</script>\n\n<button on:click={handleClick}>x</button>\n";
        let idx = extract(Language::Svelte, src);
        let fns = names_of(&idx, SymbolKind::Function);
        assert!(fns.contains(&"load"), "module-block function");
        assert!(fns.contains(&"handleClick"), "instance-block function");
        assert!(refs_of(&idx, RefKind::Import).contains(&"onMount"), "script import");
        assert!(refs_of(&idx, RefKind::Call).contains(&"doThing"), "script call");
        // `handleClick` is on file line 7 (1=module-open, 2=load, 3=/script,
        // 4=blank, 5=instance-open, 6=import, 7=handleClick), proving the offset.
        let hc = idx.symbols.iter().find(|s| s.name == "handleClick").unwrap();
        assert_eq!(hc.line, 7, "line offset maps the inner line to the .svelte file");
    }

    #[test]
    fn a_svelte_file_with_no_script_yields_nothing() {
        let idx = extract(Language::Svelte, "<h1>hello</h1>\n");
        assert!(idx.symbols.is_empty() && idx.references.is_empty());
        // A malformed/unterminated script tag must not panic.
        let _ = extract(Language::Svelte, "<script>fn broken(");
        let _ = extract(Language::Svelte, "<script");
    }

    #[test]
    fn tsx_grammar_parses_a_component() {
        // The TSX grammar (distinct from plain TS) handles JSX in a .tsx file.
        let src = r#"
function App(): JSX.Element {
    return <div>{greet()}</div>;
}
"#;
        let idx = extract(Language::Tsx, src);
        assert!(names_of(&idx, SymbolKind::Function).contains(&"App"));
        assert!(refs_of(&idx, RefKind::Call).contains(&"greet"));
    }
}

//! The per-file Rust extraction core (code-graph-layer.md CG-R1).
//!
//! Parses ONE Rust file in complete isolation (tree-sitter, purely syntactic, no
//! build, no cross-file visibility) and returns its [`FileIndex`]: the
//! `CodeSymbol` definitions it declares, and the outgoing references (calls,
//! imports) it makes. This is the load-bearing "per-file isolation at index"
//! design - a file-save re-runs only this, never a whole-project reindex.
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

/// The tree-sitter query that captures Rust definitions + references in one pass.
/// Each pattern tags its capture so [`extract_rust`] can classify it. Methods
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

/// Extract the [`FileIndex`] from one Rust source file, in isolation.
///
/// Purely syntactic: no build, no other-file visibility. A malformed file yields
/// whatever tree-sitter can recover (error-tolerant parse) plus the symbols it
/// did recognise, never a panic. Definitions are deduplicated only by
/// (name, kind, line); a genuinely duplicated name on different lines is kept
/// (it is a real second definition the resolver must disambiguate).
pub fn extract_rust(source: &str) -> FileIndex {
    let mut parser = tree_sitter::Parser::new();
    let language: tree_sitter::Language = tree_sitter_rust::LANGUAGE.into();
    if parser.set_language(&language).is_err() {
        return FileIndex::default();
    }
    let Some(tree) = parser.parse(source, None) else {
        return FileIndex::default();
    };
    let Ok(query) = tree_sitter::Query::new(&language, RUST_QUERY) else {
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
        assert_eq!(RefKind::Call.as_key(), "calls");
        assert_eq!(RefKind::Import.as_key(), "imports");
    }
}

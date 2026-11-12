//! `arlen-code-indexer` - the Tier-2 code-graph ingestion library
//! (code-graph-layer.md).
//!
//! The code structure layer of the KG that capture-at-source cannot provide:
//! the event bus sees "file opened / part of project X", never "`foo()` calls
//! `bar()`". That comes only from parsing. This crate carries the per-file
//! syntactic [`extract`]ion core (tree-sitter) that turns one file, in
//! isolation, into its `CodeSymbol` definitions and the names it refers to, and
//! the [`index`] helpers that put both on the wire; the daemon binary follows
//! the project file events, re-parses only the changed file, and emits
//! `code.indexed`. Binding a name to a definition needs the whole project, so
//! the knowledge daemon does that when it promotes the event, and a file-save
//! costs one file's parse here, never a whole-project reindex.

pub mod extract;
pub mod index;

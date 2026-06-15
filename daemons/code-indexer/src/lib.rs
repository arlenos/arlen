//! `arlen-code-indexer` - the Tier-2 code-graph ingestion library
//! (code-graph-layer.md CG-R1).
//!
//! The code structure layer of the KG that capture-at-source cannot provide:
//! the event bus sees "file opened / part of project X", never "`foo()` calls
//! `bar()`". That comes only from parsing. This crate carries the per-file
//! syntactic [`extract`]ion core (tree-sitter, Rust first) that turns one file,
//! in isolation, into its `CodeSymbol` definitions and outgoing references; the
//! daemon binary follows the project file-write events, re-parses only the
//! changed file, and emits `code.*` events the knowledge daemon promotes into
//! the graph. Cross-file name-resolution is deferred to query time (CG-R2), so
//! a file-save costs one file's parse, never a whole-project reindex.

pub mod extract;
pub mod index;

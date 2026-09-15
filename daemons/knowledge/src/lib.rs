//! The knowledge daemon: the event store, the graph, and the sockets over both.
//!
//! ONE CRATE, ONE COMPILATION. Until 15 September this file and `main.rs` each
//! declared their own copy of the module tree - 31 of 33 modules in both - so the
//! whole daemon was compiled and tested twice into two different crates, a type
//! from `knowledge::graph` was not the same type as one from `crate::graph` in
//! the binary, and the bin's build reported ~480 unused imports because the lib's
//! consumers were not in it. `main.rs` is a thin `fn main()` over this library
//! now, and everything it needs is declared here.

#![allow(clippy::module_name_repetitions)]
#![allow(clippy::must_use_candidate)]

// One pedantic lint IS on: `doc_markdown`, so an identifier in a doc comment is
// in code font. It needed `clippy.toml`'s `doc-valid-idents` first - without that
// list the lint asks for backticks around SQLite and Ladybug too, which reads as
// "a symbol you could type" about a product you cannot. The list is beside this
// file and says the rest.
#![warn(clippy::doc_markdown)]

// The REST of `#![warn(clippy::pedantic)]` is still off, and the absence is a
// decision rather than an oversight. `main.rs` carried that line while it declared the whole module tree;
// now it is eight lines of glue, so moving the attribute here would not preserve
// a standard, it would impose one on thirty-three modules that never met it.
// Measured on 15 September: 450 findings, of which 130 are "item in documentation
// is missing backticks" and another 130 "docs for function returning `Result`
// missing `# Errors`". Both are worth having and both are a deliberate sweep, not
// a side effect of merging two crates into one.

pub mod consumer;
pub mod auth;
pub mod backup;
pub mod code_analysis;
pub mod cypher;
pub mod db;
pub mod derivation;
pub mod drift;
pub mod fts;
pub mod fuse;
pub mod graph;
pub mod identity;
pub mod lcg;
pub mod meeting;
pub mod lifecycle;
pub mod links;
pub mod migration;
pub mod permission;
pub mod project;
pub mod provenance;
pub mod retrieval;
pub mod timeline_config;
pub mod revoke;
pub mod quota;
pub mod schema;
pub mod seed;
pub mod shared;
pub mod temporal;
pub mod time;
pub mod token;
pub mod utils;
pub mod write;
pub mod activity_delete;
pub mod audit;
pub mod capsule;
pub mod daemon;
pub mod entity_precision;
pub mod events;
pub mod git_ingest;
pub mod library;
pub mod list;
pub mod prep;
pub mod promotion;
pub mod retention;
pub mod typed_read;
pub mod working_set;
pub mod writer;

pub mod proto {
    #![allow(dead_code)]
    #![allow(clippy::doc_markdown)]
    include!(concat!(env!("OUT_DIR"), "/arlen.eventbus.rs"));
}

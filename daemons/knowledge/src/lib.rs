// Library interface for knowledge crate.
// Used by benchmarks and integration tests.

pub mod auth;
pub mod backup;
pub mod code_analysis;
pub mod cypher;
pub mod db;
pub mod derivation;
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
pub mod entity_precision;
pub mod project;
pub mod provenance;
pub mod retrieval;
pub mod revoke;
pub mod quota;
pub mod schema;
pub mod seed;
pub mod shared;
pub mod temporal;
pub mod time;
pub mod token;
pub mod token_cache;
pub mod utils;
pub mod write;

pub mod proto {
    #![allow(dead_code)]
    #![allow(clippy::doc_markdown)]
    include!(concat!(env!("OUT_DIR"), "/arlen.eventbus.rs"));
}

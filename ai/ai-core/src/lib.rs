//! Shared infrastructure for the Arlen AI layer.
//!
//! Defines the [`provider::AIProvider`] trait and the surface area that
//! both `ai-daemon` and `ai-agent` build on: the routing engine, the MCP
//! client wrapper, the audit-log producer, the capability gate, the
//! content-origin tagging API, and the two-step Cypher pipeline. See
//! `docs/architecture/phase-9-plan.md` for the full mapping.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod audit;
pub mod capability;
pub mod compress;
pub mod cypher;
pub mod graph_query;
// The graph schema lives in its own light, zero-dep crate so the knowledge
// MCP server (and other consumers) can use it without pulling the whole AI
// core. Re-exported here so existing `arlen_ai_core::graph_schema::*` paths
// keep working.
pub use arlen_graph_schema as graph_schema;
pub mod mcp;
pub mod pipeline;
pub mod provider;
pub mod proxied;
pub mod routing;
pub mod screen;
pub mod tagging;

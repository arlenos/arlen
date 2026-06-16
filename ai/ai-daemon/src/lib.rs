//! Query daemon library for the Arlen AI layer.
//!
//! Hosts the D-Bus interface (`org.arlen.AI1`), the query registry,
//! the dispatch pipeline, and per-session authorization issuance for
//! MCP callers. The binary in `main.rs` wires this library into a
//! zbus service.
//!
//! Phase 9-α S5 scope: query registry + dispatch skeleton plus the
//! D-Bus surface. S6 fills in the Cypher pipeline; S7 wires the
//! Settings panel. Streaming signal emission is implemented as a
//! per-query broadcast channel on top of which the D-Bus surface
//! converts events into `QueryProgress` / `QueryComplete` /
//! `QueryFailed` signals.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod active_project;
pub mod authz;
pub mod config_watch;
pub mod skill_route;
pub mod graph_adapter;
pub mod mcp_discovery;
pub mod peer;
pub mod registry;
pub mod service;
pub mod tool_loop;

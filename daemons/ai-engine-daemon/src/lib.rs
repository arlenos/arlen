//! The Arlen AI engine daemon: supervises a swappable agent engine (pi) behind
//! the engine-neutral five-verb contract, authenticates the engine process via
//! SO_PEERCRED + a daemon-minted session token, and bounds every action to the
//! session grant server-side. See `pi-agent-adoption.md`.
//!
//! This is built BESIDE the existing ai-daemon/ai-agent (two systems side by
//! side); nothing here touches them. Phase 0 lands the session-auth core; the
//! contract socket, verb dispatch, and pi-sidecar supervision follow.

pub mod session;

pub mod dispatch;

pub mod wire;

pub mod placeholder;

pub mod supervisor;

pub mod capability_map;

pub mod read_executor;

/// The Phase-1 reporter seam: audit + S17/S18 screening of tool results.
pub mod reporter;

/// The os-sdk -> ai-core `GraphQuerier` bridge for the read executor.
pub mod graph_adapter;

/// The Execute-seam router: dispatches a tool to its registered sub-executor.
pub mod proxy_executor;

/// The graph.write executor: an atomic, op-id-keyed relation create.
pub mod write_executor;

/// Report-side compensation: op-id-keyed retract receipts for committed writes.
pub mod compensation;

/// The pi engine sidecar: the confined `pi --mode rpc` spawn (argv builder).
pub mod sidecar;

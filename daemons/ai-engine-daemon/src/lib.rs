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

/// The Authorize->Execute one-time proof (HIGH-1 gate enforcement).
pub mod execution_proof;

pub mod wire;

pub mod placeholder;

pub mod supervisor;

pub mod capability_map;

/// The consent seam: resolve a gate `Confirm` via the trusted-path consent surface.
pub mod consent;

/// The real consent-broker client implementing the consent seam.
pub mod consent_client;

/// The Phase-2-A drive-channel relay: a faithful JSONL bridge shell <-> pi RPC.
pub mod rpc_proxy;

pub mod read_executor;

/// Deterministic curation (§E): the zero-token, no-pi graph curation the daemon
/// runs directly for a workflow behaviour (auto-tag-by-project).
pub mod curation;

/// The autonomous-curator orchestrator (§E), re-homed from the native ai-agent:
/// event-bus trigger spine, fire-storm coalescing, dispatch, ephemeral pi runs.
pub mod orchestrator;

/// The ephemeral pi run (§D/§E): the per-trigger bounded confined pi session for
/// a `kind: agent` behaviour (the PiRun route).
pub mod pi_run;

/// An internal pi rpc driver: submit a prompt to a spawned pi + return its answer.
pub mod pi_driver;

/// The System Explanation Mode D-Bus surface (org.arlen.AI1).
pub mod explain_iface;

/// The production autonomous-curator route handler (§E): composes the
/// deterministic-curation + ephemeral-pi-run route bodies behind the orchestrator.
pub mod curator;

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

/// The daemon's minimal `ai.toml` read: the `[ai] enabled` master switch.
pub mod engine_config;

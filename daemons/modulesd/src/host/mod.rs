/// Capability-gated host imports for Tier 1 WASM modules.
///
/// Each submodule provides the Rust-side implementation that backs one
/// of the WIT host interfaces in `sdk/module-sdk/wit/host.wit`. The
/// backing functions take `&CapabilityContext` so they can return
/// typed denials when a request falls outside the module's manifest
/// allowlist.
///
/// The point of putting this in `modulesd` rather than in the SDK is
/// that the SDK's job is to surface a stable contract to module
/// authors; the daemon's job is to *enforce* the contract by deciding
/// when host calls succeed.

pub mod context;
pub mod events;
pub mod graph;
pub mod log;
pub mod network;

pub use context::CapabilityContext;

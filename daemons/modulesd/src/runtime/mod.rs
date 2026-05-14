/// Runtime hosts for sandboxed modules.
///
/// `tier1` runs WASM components in-process via Wasmtime.
/// `tier2` brokers iframe lifecycle for the desktop-shell webview;
/// the iframe DOM lives in the shell, the daemon owns the policy.
/// `crash` is the shared crash-recovery state machine that both tiers
/// drive on every clean run and every failure.

pub mod crash;
pub mod csp;
pub mod host_bindings;
pub mod tier1;
pub mod tier2;
pub mod wit;

pub use crash::{CrashState, Recovery};
pub use csp::build_csp;

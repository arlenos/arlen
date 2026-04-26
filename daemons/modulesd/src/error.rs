/// Errors surfaced by the module runtime daemon.
///
/// Distinguishes between the three failure modes documented in
/// `docs/architecture/module-system.md` "Error Handling":
///
///  1. **Load** failures (manifest invalid, link error). Permanent.
///     The module is marked failed without retry.
///  2. **Runtime** crashes (WASM trap, fuel exhaustion). Counted toward
///     crash recovery and retried per Foundation Table 08.
///  3. **Capability** denials. Surface a typed error to the caller; not
///     a crash.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum DaemonError {
    #[error("module not found: {0}")]
    NotFound(String),

    #[error("manifest invalid for {module_id}: {reason}")]
    ManifestInvalid { module_id: String, reason: String },

    #[error("WASM load failed for {module_id}: {reason}")]
    WasmLoad { module_id: String, reason: String },

    #[error("WASM trap in {module_id}: {reason}")]
    WasmTrap { module_id: String, reason: String },

    #[error("capability denied: {capability} for {module_id}")]
    CapabilityDenied {
        module_id: String,
        capability: String,
    },

    #[error("fuel exhausted in {module_id} after {instructions} instructions")]
    FuelExhausted {
        module_id: String,
        instructions: u64,
    },

    #[error("module marked failed (crash count {crashes}); manual retry required")]
    PermanentlyFailed { module_id: String, crashes: u32 },

    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("serialization: {0}")]
    Serde(#[from] serde_json::Error),

    #[error("internal: {0}")]
    Internal(String),
}

pub type Result<T> = std::result::Result<T, DaemonError>;

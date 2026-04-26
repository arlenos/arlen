/// Tier 1 (WASM Component) runtime.
///
/// Hosts third-party `waypointer.search`, `waypointer.action`,
/// `mcp.server`, and `keybinding.profile` modules as Wasmtime
/// components. Each module gets its own `Store` (isolated linear
/// memory) and its own `CapabilityContext` (read at link time, immutable
/// for the module's lifetime).
///
/// Resource limits:
///   * memory: 64 MB per instance (cap, not reservation)
///   * fuel: 1 M instructions per host call (search/execute)
///
/// Crash containment: WASM traps are caught and converted to typed
/// `DaemonError::WasmTrap` errors; the manager's crash state machine
/// decides whether to restart the module.

use std::path::Path;
use std::sync::Arc;

use tokio::sync::Mutex;
use wasmtime::component::{Component, Linker};
use wasmtime::{Config, Engine, Store, StoreLimits, StoreLimitsBuilder};

use crate::error::{DaemonError, Result};
use crate::host::CapabilityContext;

/// Default per-instance memory cap. Modules that need more must
/// request it explicitly in their manifest's `[capabilities.storage]`
/// section, which Settings surfaces to the user at install time.
pub const DEFAULT_MEMORY_LIMIT: usize = 64 * 1024 * 1024;

/// Default fuel budget per host call. One million Wasmtime fuel units
/// is roughly ten milliseconds of typical numeric work; modules that
/// exceed it trap and are counted toward crash recovery, so a runaway
/// loop in a third-party module never freezes the launcher.
pub const DEFAULT_FUEL_BUDGET: u64 = 1_000_000;

/// Store data carried by every Wasmtime instance the daemon spawns.
/// Host imports look this up via `caller.data()` to make capability
/// decisions without a global table.
pub struct ModuleStore {
    pub ctx: CapabilityContext,
    pub limits: StoreLimits,
}

impl ModuleStore {
    pub fn new(ctx: CapabilityContext) -> Self {
        Self {
            ctx,
            limits: StoreLimitsBuilder::new()
                .memory_size(DEFAULT_MEMORY_LIMIT)
                .build(),
        }
    }
}

/// Engine + linker pair shared across all Tier 1 modules. Wasmtime
/// engines are heavy to construct (JIT compiler init); reusing one
/// engine is the documented best practice. Each module instantiation
/// spins up its own `Store`, which is cheap.
pub struct Tier1Runtime {
    engine: Engine,
    linker: Arc<Mutex<Linker<ModuleStore>>>,
}

impl Tier1Runtime {
    pub fn new() -> Result<Self> {
        let mut config = Config::new();
        config.async_support(true);
        config.wasm_component_model(true);
        config.consume_fuel(true);
        // Cooperative cancellation: long-running modules can be
        // interrupted by setting `Store::epoch_deadline_trap`.
        config.epoch_interruption(true);

        let engine = Engine::new(&config)
            .map_err(|e| DaemonError::Internal(format!("wasmtime engine init: {e}")))?;

        let linker = Linker::<ModuleStore>::new(&engine);

        Ok(Self {
            engine,
            linker: Arc::new(Mutex::new(linker)),
        })
    }

    /// Compile a WASM component from disk. Compilation can be slow on
    /// first load; the daemon caches compiled artefacts via Wasmtime's
    /// own cache infrastructure when configured.
    pub async fn compile(&self, path: &Path) -> Result<Component> {
        let bytes = tokio::fs::read(path).await?;
        Component::new(&self.engine, &bytes).map_err(|e| DaemonError::WasmLoad {
            module_id: path
                .parent()
                .and_then(|p| p.file_name())
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default(),
            reason: e.to_string(),
        })
    }

    /// Build a `Store` for a fresh instance, preloaded with the
    /// module's capability context and default resource limits.
    pub fn create_store(&self, ctx: CapabilityContext) -> Store<ModuleStore> {
        let mut store = Store::new(&self.engine, ModuleStore::new(ctx));
        store.limiter(|s| &mut s.limits);
        // Initial fuel budget; refilled per host call by the manager.
        let _ = store.set_fuel(DEFAULT_FUEL_BUDGET);
        store
    }

    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    /// Hand out the linker for host-import registration. Caller holds
    /// the lock for the duration of registration.
    pub async fn linker(&self) -> tokio::sync::MutexGuard<'_, Linker<ModuleStore>> {
        self.linker.lock().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn runtime_constructs_without_module() {
        let r = Tier1Runtime::new().expect("runtime init");
        // Engine pointer must be live; we can not easily probe it
        // without a compiled module, so a successful `new()` is the
        // assertion.
        let _ = r.engine();
    }

    #[tokio::test]
    async fn create_store_carries_capability_context() {
        let r = Tier1Runtime::new().unwrap();
        let store = r.create_store(CapabilityContext::empty("com.example.test"));
        assert_eq!(store.data().ctx.module_id, "com.example.test");
    }

    #[tokio::test]
    async fn compile_rejects_invalid_bytes() {
        let r = Tier1Runtime::new().unwrap();
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), b"this is not a wasm module").unwrap();
        match r.compile(tmp.path()).await {
            Err(DaemonError::WasmLoad { .. }) => {}
            Err(other) => panic!("expected WasmLoad, got {other:?}"),
            Ok(_) => panic!("expected error, got Ok"),
        }
    }
}

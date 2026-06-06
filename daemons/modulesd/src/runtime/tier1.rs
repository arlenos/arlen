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
use std::time::Duration;

use tokio::sync::Mutex;
use wasmtime::component::{Component, Linker};
use wasmtime::{Config, Engine, Store, StoreLimits, StoreLimitsBuilder};

use os_sdk::{UnixEventEmitter, UnixGraphClient};

use crate::error::{DaemonError, Result};
use crate::host::CapabilityContext;
use crate::runtime::wit::WaypointerProvider;

/// Default per-instance memory cap. Modules that need more must
/// request it explicitly in their manifest's `[capabilities.storage]`
/// section, which Settings surfaces to the user at install time.
pub const DEFAULT_MEMORY_LIMIT: usize = 64 * 1024 * 1024;

/// Default fuel budget per host call. One million Wasmtime fuel units
/// is roughly ten milliseconds of typical numeric work; modules that
/// exceed it trap and are counted toward crash recovery, so a runaway
/// loop in a third-party module never freezes the launcher.
pub const DEFAULT_FUEL_BUDGET: u64 = 1_000_000;

/// Max time a module's `init()` may take before it is considered stuck
/// and trapped. Per `phase-7-sprint-s5.md` lifecycle edge analysis: the
/// fuel clock does not cover host-call hangs (e.g. a malicious server
/// holding a connection open inside `network::fetch`), so init() is
/// wrapped in an additional wall-clock timeout.
pub const INIT_TIMEOUT: Duration = Duration::from_secs(15);

/// Store data carried by every Wasmtime instance the daemon spawns.
/// Host imports look this up via `caller.data()` to make capability
/// decisions without a global table.
///
/// S6: `graph_client` and `event_emitter` are clones of the
/// Manager-owned originals. Tier 1 host trait impls reach the real
/// backends through these handles after the per-module capability
/// gate passes. Both fields are `Arc` so they clone cheaply and the
/// underlying `UnixStream` mutex is shared across every loaded
/// module.
pub struct ModuleStore {
    pub ctx: CapabilityContext,
    pub limits: StoreLimits,
    pub graph_client: Arc<UnixGraphClient>,
    pub event_emitter: Arc<UnixEventEmitter>,
}

impl ModuleStore {
    pub fn new(
        ctx: CapabilityContext,
        graph_client: Arc<UnixGraphClient>,
        event_emitter: Arc<UnixEventEmitter>,
    ) -> Self {
        Self {
            ctx,
            limits: StoreLimitsBuilder::new()
                .memory_size(DEFAULT_MEMORY_LIMIT)
                .build(),
            graph_client,
            event_emitter,
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

        // S7.3: wasmtime caches compiled components under
        // `$XDG_CACHE_HOME/wasmtime/` (or `~/.cache/wasmtime/`) so
        // subsequent modulesd starts skip the 100-300 ms compile
        // step per loaded module. Cache misses (e.g. read-only
        // home, sealed image, missing dir) downgrade to "always
        // recompile" rather than fail; the daemon stays usable
        // without cache.
        //
        // Wasmtime 36 API: `Cache::from_file(None)` loads the
        // default per-user cache config. Earlier wasmtime exposed
        // this as `Config::cache_config_load_default()`.
        match wasmtime::Cache::from_file(None) {
            Ok(cache) => {
                config.cache(Some(cache));
            }
            Err(err) => {
                tracing::info!(
                    "modulesd: wasmtime cache disabled ({err}); modules will recompile each start"
                );
            }
        }

        let engine = Engine::new(&config)
            .map_err(|e| DaemonError::Internal(format!("wasmtime engine init: {e}")))?;

        let mut linker = Linker::<ModuleStore>::new(&engine);
        populate_linker(&mut linker)?;

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
    /// module's capability context, default resource limits, and
    /// (S6) handles to the shared graph + event-bus backend clients.
    pub fn create_store(
        &self,
        ctx: CapabilityContext,
        graph_client: Arc<UnixGraphClient>,
        event_emitter: Arc<UnixEventEmitter>,
    ) -> Store<ModuleStore> {
        let mut store = Store::new(
            &self.engine,
            ModuleStore::new(ctx, graph_client, event_emitter),
        );
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

    /// Instantiate a freshly-compiled component against this runtime's
    /// engine + linker, then call its `init()` export with a wall-clock
    /// timeout. Returns the live `Tier1Instance` ready for subsequent
    /// `call_search` / `call_execute`.
    ///
    /// Errors:
    /// - `WasmLoad` when the component fails to link (missing import,
    ///   ABI mismatch, etc.). Permanent — caller marks the module
    ///   `PermanentlyFailed` without retry.
    /// - `WasmTrap` when `init()` traps or times out. Counts toward
    ///   crash recovery; caller drives the Foundation Table 08 ladder.
    pub async fn instantiate(
        &self,
        module_id: &str,
        component: &Component,
        ctx: CapabilityContext,
        graph_client: Arc<UnixGraphClient>,
        event_emitter: Arc<UnixEventEmitter>,
    ) -> Result<Tier1Instance> {
        let linker = self.linker.lock().await;
        let mut store = self.create_store(ctx, graph_client, event_emitter);
        let provider = WaypointerProvider::instantiate_async(&mut store, component, &linker)
            .await
            .map_err(|e| DaemonError::WasmLoad {
                module_id: module_id.to_string(),
                reason: format!("instantiate: {e}"),
            })?;
        drop(linker);

        // Init with wall-clock timeout. Modules that block forever
        // inside init (e.g. via a slow host call that fuel cannot
        // catch) get trapped here rather than wedging the daemon.
        let init_result = tokio::time::timeout(
            INIT_TIMEOUT,
            provider
                .lunaris_waypointer_provider()
                .call_init(&mut store),
        )
        .await;

        match init_result {
            Ok(Ok(Ok(()))) => Ok(Tier1Instance { store, provider }),
            Ok(Ok(Err(module_err))) => Err(DaemonError::WasmTrap {
                module_id: module_id.to_string(),
                reason: format!("init returned error: {module_err}"),
            }),
            Ok(Err(trap)) => Err(DaemonError::WasmTrap {
                module_id: module_id.to_string(),
                reason: format!("init trapped: {trap}"),
            }),
            Err(_elapsed) => Err(DaemonError::WasmTrap {
                module_id: module_id.to_string(),
                reason: format!("init exceeded {}s wall-clock timeout", INIT_TIMEOUT.as_secs()),
            }),
        }
    }

    /// Instantiate an `mcp.server` module against this runtime.
    ///
    /// The mcp-server world reuses the same four `lunaris:host/*`
    /// imports as the waypointer world, so the same populated linker
    /// satisfies it. Mirrors [`instantiate`](Self::instantiate): the
    /// guest `init()` export runs under the same wall-clock timeout
    /// and the same `WasmLoad` / `WasmTrap` error split.
    pub async fn instantiate_mcp(
        &self,
        module_id: &str,
        component: &Component,
        ctx: CapabilityContext,
        graph_client: Arc<UnixGraphClient>,
        event_emitter: Arc<UnixEventEmitter>,
    ) -> Result<McpInstance> {
        use crate::runtime::wit::mcp::McpServer;

        let linker = self.linker.lock().await;
        let mut store = self.create_store(ctx, graph_client, event_emitter);
        let provider = McpServer::instantiate_async(&mut store, component, &linker)
            .await
            .map_err(|e| DaemonError::WasmLoad {
                module_id: module_id.to_string(),
                reason: format!("instantiate: {e}"),
            })?;
        drop(linker);

        let init_result = tokio::time::timeout(
            INIT_TIMEOUT,
            provider
                .lunaris_waypointer_server()
                .call_init(&mut store),
        )
        .await;

        match init_result {
            Ok(Ok(Ok(()))) => Ok(McpInstance { store, provider }),
            Ok(Ok(Err(module_err))) => Err(DaemonError::WasmTrap {
                module_id: module_id.to_string(),
                reason: format!("init returned error: {module_err}"),
            }),
            Ok(Err(trap)) => Err(DaemonError::WasmTrap {
                module_id: module_id.to_string(),
                reason: format!("init trapped: {trap}"),
            }),
            Err(_elapsed) => Err(DaemonError::WasmTrap {
                module_id: module_id.to_string(),
                reason: format!("init exceeded {}s wall-clock timeout", INIT_TIMEOUT.as_secs()),
            }),
        }
    }
}

/// One loaded Tier 1 module instance. Holds its own `Store` (linear
/// memory + fuel + capability context) and a `WaypointerProvider`
/// view for calling guest exports. `Tier1Instance` is `!Sync` because
/// wasmtime `Store` is `!Sync`, so the manager wraps each instance in
/// `tokio::sync::Mutex` and serialises calls per module.
pub struct Tier1Instance {
    pub store: Store<ModuleStore>,
    pub provider: WaypointerProvider,
}

impl Tier1Instance {
    /// Best-effort call into the guest's `shutdown()` export. Used by
    /// the daemon SIGTERM handler so modules with persistent state
    /// (file handles, open connections, in-flight writes) get a
    /// chance to flush before the process exits. A trapping shutdown
    /// is logged but does not block: this is a politeness signal,
    /// not a correctness requirement.
    pub async fn graceful_shutdown(&mut self, module_id: &str) {
        if let Err(err) = self
            .provider
            .lunaris_waypointer_provider()
            .call_shutdown(&mut self.store)
            .await
        {
            tracing::warn!(
                module = module_id,
                "shutdown trapped: {err}",
            );
        }
    }
}

/// One loaded `mcp.server` Tier 1 module instance. The `mcp-server`
/// counterpart of [`Tier1Instance`]: same `Store` discipline, but it
/// holds the `mcp-server` world's provider rather than the
/// waypointer one. The higher-level hosting (per-call fuel + timeout,
/// the rmcp socket bridge) lives in `runtime::mcp`.
pub struct McpInstance {
    pub store: Store<ModuleStore>,
    pub provider: crate::runtime::wit::mcp::McpServer,
}

impl McpInstance {
    /// Best-effort call into the guest's `shutdown()` export, used by
    /// the daemon SIGTERM handler. A trapping shutdown is logged but
    /// does not block: this is a politeness signal, not correctness.
    pub async fn graceful_shutdown(&mut self, module_id: &str) {
        if let Err(err) = self
            .provider
            .lunaris_waypointer_server()
            .call_shutdown(&mut self.store)
            .await
        {
            tracing::warn!(module = module_id, "mcp shutdown trapped: {err}");
        }
    }
}

/// Wire every `lunaris:host/*` interface into the linker so that any
/// Tier 1 component instantiated against this engine can reach the
/// host imports it declared in its manifest. Capability gating runs
/// inside each host trait method (`host_bindings::*`), not here —
/// `add_to_linker` just registers the symbols, the host trait
/// rejects requests at call time.
///
/// The wasmtime 36 `add_to_linker` API requires a plain `fn`
/// pointer (not a closure) and a `HasData` marker that pins the
/// associated `Data<'a>` lifetime. `HasSelf<T>` is the wasmtime-
/// provided marker for "host data lives directly on the store and
/// is `T` itself" — exactly our layout because every `Host` trait
/// is implemented on `ModuleStore` directly.
fn populate_linker(linker: &mut Linker<ModuleStore>) -> Result<()> {
    use crate::runtime::wit;
    use wasmtime::component::HasSelf;

    fn host_getter(store: &mut ModuleStore) -> &mut ModuleStore {
        store
    }

    wit::lunaris::host::graph::add_to_linker::<_, HasSelf<ModuleStore>>(linker, host_getter)
        .map_err(|e| DaemonError::Internal(format!("link graph: {e}")))?;
    wit::lunaris::host::network::add_to_linker::<_, HasSelf<ModuleStore>>(linker, host_getter)
        .map_err(|e| DaemonError::Internal(format!("link network: {e}")))?;
    wit::lunaris::host::events::add_to_linker::<_, HasSelf<ModuleStore>>(linker, host_getter)
        .map_err(|e| DaemonError::Internal(format!("link events: {e}")))?;
    wit::lunaris::host::log::add_to_linker::<_, HasSelf<ModuleStore>>(linker, host_getter)
        .map_err(|e| DaemonError::Internal(format!("link log: {e}")))?;

    Ok(())
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
        // S6: stores now carry handles to the backend clients too.
        // Tests use real client constructors with throwaway socket
        // paths; the clients connect lazily so this never touches
        // the filesystem.
        let graph = Arc::new(UnixGraphClient::new("/tmp/lunaris-test-knowledge.sock"));
        let events = Arc::new(UnixEventEmitter::new("/tmp/lunaris-test-events.sock"));
        let store = r.create_store(
            CapabilityContext::empty("com.example.test"),
            graph,
            events,
        );
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

    /// `Tier1Runtime::new` must call `populate_linker`. Verify by
    /// repopulating: wasmtime rejects duplicate registration of the
    /// same interface, so a second `populate_linker` call against the
    /// same `Linker` errors with a duplicate-key message. If `new`
    /// had skipped registration this second call would succeed.
    #[tokio::test]
    async fn populate_linker_is_idempotent_only_in_new() {
        let r = Tier1Runtime::new().expect("runtime init");
        let mut linker = r.linker.lock().await;
        let result = super::populate_linker(&mut linker);
        assert!(
            result.is_err(),
            "second populate_linker must fail; first call already registered the host interfaces",
        );
        // Sanity-check the message names one of our four interfaces
        // so this test does not silently green on an unrelated error
        // (e.g. allocation failure).
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("link graph")
                || err.contains("link network")
                || err.contains("link events")
                || err.contains("link log"),
            "duplicate-registration error did not name a host interface: {err}",
        );
    }
}

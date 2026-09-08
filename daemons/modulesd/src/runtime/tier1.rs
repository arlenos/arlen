//! Tier 1 (WASM Component) runtime.
//!
//! Hosts third-party `waypointer.search`, `waypointer.action`,
//! `mcp.server`, and `keybinding.profile` modules as Wasmtime
//! components. Each module gets its own `Store` (isolated linear
//! memory) and its own `CapabilityContext` (read at link time, immutable
//! for the module's lifetime).
//!
//! Resource limits:
//!   * memory: 64 MB per instance (cap, not reservation)
//!   * fuel: 1 M instructions per host call (search/execute)
//!
//! Crash containment: WASM traps are caught and converted to typed
//! `DaemonError::WasmTrap` errors; the manager's crash state machine
//! decides whether to restart the module.

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

/// How often the epoch advances. The unit the deadline below is counted in.
const EPOCH_TICK: std::time::Duration = std::time::Duration::from_millis(100);

/// How many ticks a guest may run before it is interrupted, so five seconds.
///
/// Deliberately longer than anything a waypointer search should take and shorter
/// than a person will wait: the fuel budget is what bounds ordinary work, and
/// this is the backstop for the case fuel cannot see - a guest blocked in a host
/// call rather than executing instructions.
const EPOCH_DEADLINE_TICKS: u64 = 50;

/// The one-time budget `init` gets, which is not the per-call one.
///
/// **Ruled on 8 September, and the reasoning is what makes the number make
/// sense.** `search` runs per keystroke with a person waiting, so 1 M - about ten
/// milliseconds of work - is right for it and stays. `init` is called once, off
/// that path, with nobody waiting, and until now `refuel` handed it the same
/// constant: a module could not build anything at startup that it did not have
/// to rebuild on every keystroke.
///
/// **1.5 G, and the number moved once it was measured against a real index.** The
/// ruling said hundreds of millions, from a report that had measured a single
/// SCAN of the Unicode name space at 200-400 M. Building an index over the same
/// data costs more than scanning it once: the first module's name buffer plus
/// its trigram buckets came in between 1 G and 1.5 G, and 500 M could not build
/// it at all. The scan was the floor, not the whole cost.
///
/// What this does NOT loosen is the guard that matters. Fuel counts instructions,
/// not time, and the five-second wall clock covers `init` unchanged - 1.5 G of
/// wasm is a second or two, well inside it. A guest that hangs is stopped by the
/// deadline either way; this only stops the daemon killing a module for doing
/// legitimate one-time work.
///
/// This is not a way around the search budget. The wall-clock deadline covers
/// `init` unchanged at five seconds, and a guest that hangs in a host call is
/// stopped by that rather than by fuel.
pub const INIT_FUEL_BUDGET: u64 = 1_500_000_000;

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

/// Give a store its budgets for one call: fuel, and time.
///
/// **Both, always, together.** The epoch deadline is a count from the CURRENT
/// epoch, not a per-call allowance, so setting it once at store creation buys
/// the instance five seconds of wall clock in total and then kills it - not five
/// seconds of running. Probed on 8 September against the first real module: a
/// search, a seven-second pause, and the next search trapped with `wasm trap:
/// interrupt` on an instance that had done nothing in between. A module would
/// have worked for five seconds after being enabled and been dead for the rest
/// of the session.
///
/// That was my own bug from an hour earlier, and it is the reason both budgets
/// live in one function: fuel was already refilled per call in four places, and
/// a deadline that has to be refilled in the same four places will not stay in
/// step unless it is the same line.
pub fn refuel(store: &mut Store<ModuleStore>) {
    let _ = store.set_fuel(DEFAULT_FUEL_BUDGET);
    store.set_epoch_deadline(EPOCH_DEADLINE_TICKS);
}

/// The same, with the one-time budget `init` is allowed.
///
/// Time is unchanged: `init` gets the same five seconds every call does, because
/// the thing that budget exists to stop - a guest wedged in a host call - is not
/// something a bigger fuel allowance should buy its way out of.
pub fn refuel_for_init(store: &mut Store<ModuleStore>) {
    let _ = store.set_fuel(INIT_FUEL_BUDGET);
    store.set_epoch_deadline(EPOCH_DEADLINE_TICKS);
}

impl Tier1Runtime {
    pub fn new() -> Result<Self> {
        let mut config = Config::new();
        config.async_support(true);
        config.wasm_component_model(true);
        config.consume_fuel(true);
        // Cooperative cancellation: long-running modules can be interrupted at
        // an epoch deadline.
        //
        // ENABLING THIS ALONE IS A TRAP, and it is why this runtime had never
        // hosted a guest. With epoch interruption on, a fresh `Store` carries
        // deadline 0 while the engine starts at epoch 0, so the FIRST instruction
        // of the FIRST call is already past its deadline: every module trapped
        // with `wasm trap: interrupt` before running a line of its own code. And
        // nothing incremented the epoch, so a deadline could not have fired
        // usefully even where one had been set. Measured on 8 September with
        // `modules/unicode`, the first real guest, whose own test header had
        // concluded the ABI or the build was at fault.
        //
        // The two halves live together now and neither is optional: the ticker
        // below is the clock, and the store gets a deadline where it is created.
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

        // The clock the epoch deadline counts in. One thread for the process,
        // ticking whether or not a module is running, which is what lets a store
        // say "trap after N ticks" without the host having to watch it.
        //
        // A weak handle, so the ticker does not keep the engine alive after the
        // runtime is dropped; the loop ends with the last strong reference.
        {
            let ticker = engine.weak();
            std::thread::Builder::new()
                .name("modulesd-epoch".into())
                .spawn(move || {
                    while let Some(engine) = ticker.upgrade() {
                        std::thread::sleep(EPOCH_TICK);
                        engine.increment_epoch();
                    }
                })
                .map_err(|e| DaemonError::Internal(format!("epoch ticker: {e}")))?;
        }

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
        store.epoch_deadline_trap();
        // Both budgets, through the one function that sets both. See its doc for
        // why the deadline cannot be set once here.
        refuel(&mut store);
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
                reason: format!("instantiate: {e:#}"),
            })?;
        drop(linker);

        // Init with wall-clock timeout. Modules that block forever
        // inside init (e.g. via a slow host call that fuel cannot
        // catch) get trapped here rather than wedging the daemon.
        refuel_for_init(&mut store);
        let init_result = tokio::time::timeout(
            INIT_TIMEOUT,
            provider
                .arlen_waypointer_provider()
                .call_init(&mut store),
        )
        .await;

        match init_result {
            Ok(Ok(Ok(()))) => {
                // Back to the per-call budget the moment init is done: the large
                // one is for building, not for the first search after it.
                refuel(&mut store);
                Ok(Tier1Instance { store, provider })
            }
            Ok(Ok(Err(module_err))) => Err(DaemonError::WasmTrap {
                module_id: module_id.to_string(),
                reason: format!("init returned error: {module_err}"),
            }),
            // `{trap:#}` and not `{trap}`: anyhow's plain Display prints only the
            // outermost message, and wasmtime puts the REASON in the source below
            // it. So a trapping module reported "error while executing at wasm
            // backtrace: 0: 0x2282 - <unknown>!<wasm function 18>" and nothing
            // about what went wrong - which cost an hour on the first real guest
            // on 8 September, reading the module's own bytecode to guess. The
            // daemon had the sentence the whole time and dropped it.
            Ok(Err(trap)) => Err(DaemonError::WasmTrap {
                module_id: module_id.to_string(),
                reason: format!("init trapped: {trap:#}"),
            }),
            Err(_elapsed) => Err(DaemonError::WasmTrap {
                module_id: module_id.to_string(),
                reason: format!("init exceeded {}s wall-clock timeout", INIT_TIMEOUT.as_secs()),
            }),
        }
    }

    /// Instantiate an `mcp.server` module against this runtime.
    ///
    /// The mcp-server world reuses the same four `arlen:host/*`
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
                reason: format!("instantiate: {e:#}"),
            })?;
        drop(linker);

        // The same one-time budget as the waypointer path: an MCP server that
        // needs to build something at startup has the same reason to.
        refuel_for_init(&mut store);
        let init_result = tokio::time::timeout(
            INIT_TIMEOUT,
            provider
                .arlen_waypointer_server()
                .call_init(&mut store),
        )
        .await;

        match init_result {
            Ok(Ok(Ok(()))) => {
                refuel(&mut store);
                Ok(McpInstance { store, provider })
            }
            Ok(Ok(Err(module_err))) => Err(DaemonError::WasmTrap {
                module_id: module_id.to_string(),
                reason: format!("init returned error: {module_err}"),
            }),
            Ok(Err(trap)) => Err(DaemonError::WasmTrap {
                module_id: module_id.to_string(),
                reason: format!("init trapped: {trap:#}"),
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
            .arlen_waypointer_provider()
            .call_shutdown(&mut self.store)
            .await
        {
            tracing::warn!(
                module = module_id,
                "shutdown trapped: {err:#}",
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
            .arlen_waypointer_server()
            .call_shutdown(&mut self.store)
            .await
        {
            tracing::warn!(module = module_id, "mcp shutdown trapped: {err:#}");
        }
    }
}

/// Wire every `arlen:host/*` interface into the linker so that any
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

    wit::arlen::host::graph::add_to_linker::<_, HasSelf<ModuleStore>>(linker, host_getter)
        .map_err(|e| DaemonError::Internal(format!("link graph: {e}")))?;
    wit::arlen::host::network::add_to_linker::<_, HasSelf<ModuleStore>>(linker, host_getter)
        .map_err(|e| DaemonError::Internal(format!("link network: {e}")))?;
    wit::arlen::host::events::add_to_linker::<_, HasSelf<ModuleStore>>(linker, host_getter)
        .map_err(|e| DaemonError::Internal(format!("link events: {e}")))?;
    wit::arlen::host::log::add_to_linker::<_, HasSelf<ModuleStore>>(linker, host_getter)
        .map_err(|e| DaemonError::Internal(format!("link log: {e}")))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The epoch deadline actually stops a guest that will not stop itself.
    ///
    /// A guard that never fires is the same as no guard, and this one spent
    /// months in the opposite failure - it fired on every guest's first
    /// instruction because nothing gave the store a deadline or advanced the
    /// epoch. So both directions are now pinned: the module tests prove a real
    /// guest runs and survives being idle, and this proves a spinning one is
    /// still cut off.
    ///
    /// A core module rather than a component, and one tick rather than fifty:
    /// the claim is about the ENGINE's epoch configuration and the store's trap
    /// mode, which are the same either way, and a test that waited the full five
    /// seconds would be a test people skip.
    #[tokio::test]
    async fn a_guest_that_will_not_stop_is_stopped() {
        let runtime = Tier1Runtime::new().expect("runtime init");
        let module = wasmtime::Module::new(
            runtime.engine(),
            r#"(module (func (export "spin") (loop br 0)))"#,
        )
        .expect("the spinner compiles");

        let mut store = Store::new(runtime.engine(), ());
        store.epoch_deadline_trap();
        store.set_epoch_deadline(1);
        // Fuel enough that it cannot be what stops this. The engine has
        // `consume_fuel` on, so a store starts at zero and the first cut of this
        // test trapped on fuel while claiming to be about the deadline - which is
        // the same shape of wrong answer the whole morning has been about.
        store.set_fuel(u64::MAX).expect("fuel");
        let instance = wasmtime::Instance::new_async(&mut store, &module, &[])
            .await
            .expect("instantiate");
        let spin = instance
            .get_typed_func::<(), ()>(&mut store, "spin")
            .expect("the export is there");

        let err = spin.call_async(&mut store, ()).await.expect_err("must not return");
        let reason = format!("{err:#}");
        assert!(
            reason.contains("interrupt"),
            "the deadline is what stopped it, not something else: {reason}"
        );
    }

    #[tokio::test]
    async fn runtime_constructs_without_module() {
        let r = Tier1Runtime::new().expect("runtime init");
        // Engine pointer must be live; we can not easily probe it
        // without a compiled module, so a successful `new()` is the
        // assertion.
        let _ = r.engine();
    }

    /// The memory cap is armed, not merely configured.
    ///
    /// Third of the three budgets, and it is here for what happened to the other
    /// two: the epoch deadline was configured and never given a value, so it
    /// fired on every guest's first instruction, and the same store's fuel starts
    /// at zero unless somebody sets it. A limit that is built and never installed
    /// looks identical from the outside to one that works.
    ///
    /// `memory.grow` past the cap returns -1 rather than trapping, which is the
    /// wasm semantic: the guest is told it cannot have the pages and decides what
    /// to do. A guest that ignores the answer traps on its own access.
    ///
    /// Checked against its own control rather than trusted: with the cap raised
    /// to 512 MB the same call returns 1, the previous page count, so the -1 is
    /// the limiter answering and not the request failing for some other reason.
    #[tokio::test]
    async fn a_guest_cannot_grow_past_the_memory_cap() {
        let r = Tier1Runtime::new().expect("runtime init");
        let graph = Arc::new(UnixGraphClient::new("/tmp/arlen-test-knowledge.sock"));
        let events = Arc::new(UnixEventEmitter::new("/tmp/arlen-test-events.sock"));
        let mut store = r.create_store(
            CapabilityContext::empty("com.example.test"),
            graph,
            events,
        );

        // 2000 pages is 128 MB, twice the cap.
        let module = wasmtime::Module::new(
            r.engine(),
            r#"(module (memory 1) (func (export "grow") (result i32) i32.const 2000 memory.grow))"#,
        )
        .expect("the grower compiles");
        let instance = wasmtime::Instance::new_async(&mut store, &module, &[])
            .await
            .expect("instantiate");
        let grow = instance
            .get_typed_func::<(), i32>(&mut store, "grow")
            .expect("the export is there");

        let answer = grow.call_async(&mut store, ()).await.expect("grow returns");
        assert_eq!(answer, -1, "the cap refused the pages rather than handing them over");
    }

    #[tokio::test]
    async fn create_store_carries_capability_context() {
        let r = Tier1Runtime::new().unwrap();
        // S6: stores now carry handles to the backend clients too.
        // Tests use real client constructors with throwaway socket
        // paths; the clients connect lazily so this never touches
        // the filesystem.
        let graph = Arc::new(UnixGraphClient::new("/tmp/arlen-test-knowledge.sock"));
        let events = Arc::new(UnixEventEmitter::new("/tmp/arlen-test-events.sock"));
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

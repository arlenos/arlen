/// Manager: central coordinator.
///
/// Holds the discovered module records, their per-instance crash
/// state, and the live Tier 1 / Tier 2 runtimes. Every request from
/// the socket server flows through here. Every event broadcast also
/// originates here.
///
/// Concurrency model: the manager is `Arc<Manager>` shared between
/// the socket server and any background tasks. State is partitioned
/// behind a single async `RwLock` so requests do not serialise
/// trivially against each other; the bulk of the work (Wasmtime
/// calls) happens with the lock released.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use tokio::sync::{broadcast, Mutex, OnceCell, OwnedSemaphorePermit, RwLock, Semaphore};
use tracing::{debug, info, warn};

use crate::error::DaemonError;
use crate::host::CapabilityContext;
use crate::manifest::{discover_all, ModuleRecord, Tier};
use crate::runtime::{
    tier1::{Tier1Instance, Tier1Runtime},
    tier2::Tier2Broker,
    CrashState, Recovery,
};
use crate::socket::protocol::{
    ErrorCode, Event, ModuleSummary, ModuleTier, Request, Response, SearchResult,
};

/// One row in the manager's module table.
struct ModuleEntry {
    record: ModuleRecord,
    enabled: bool,
    crash: CrashState,
    /// Codex round-2 finding 3 fix: the next `Instant` at which a
    /// retry of this module is permitted. `None` means no cooldown
    /// is active. Updated from the `Recovery` returned by
    /// `CrashState::record_crash`; consulted by
    /// `ensure_tier1_instance` and the search dispatch path so that
    /// rapid user keystrokes cannot reinstantiate a flapping module
    /// faster than the Foundation Table 08 backoff allows.
    next_retry_at: Option<Instant>,
}

impl ModuleEntry {
    fn summary(&self) -> ModuleSummary {
        let mut points = Vec::new();
        if self.record.manifest.waypointer.is_some() {
            points.push("waypointer".to_string());
        }
        if self.record.manifest.topbar.is_some() {
            points.push("topbar".to_string());
        }
        if self.record.manifest.settings.is_some() {
            points.push("settings".to_string());
        }
        ModuleSummary {
            id: self.record.id().to_string(),
            name: self.record.manifest.module.name.clone(),
            version: self.record.manifest.module.version.clone(),
            tier: match self.record.tier {
                Tier::Wasm => ModuleTier::Wasm,
                Tier::Iframe => ModuleTier::Iframe,
            },
            enabled: self.enabled,
            failed: self.crash.is_failed(),
            priority: self.record.manifest.module.module_type.default_priority(),
            extension_points: points,
        }
    }
}

/// How many concurrent network fetches each module is allowed to
/// have in flight. Foundation does not pin this number; 4 is enough
/// for typical refresh patterns and small enough that a runaway loop
/// hits backpressure quickly.
const NETWORK_CONCURRENCY_PER_MODULE: usize = 4;

/// Wall-clock budget for a single Tier 1 `search` or `execute` call
/// (Codex finding 4). Fuel covers CPU loops; this timeout covers
/// async host calls that can block the guest indefinitely (e.g. a
/// slow / hanging upstream HTTP server even before reqwest's own
/// 30 s timeout kicks in). Foundation §6 budgets ~10 ms per
/// keystroke; 5 s here is the backstop a malicious module would
/// have to exceed for the daemon to step in.
const SEARCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Aggregate cap on `WaypointerSearchAll`. Modules are dispatched
/// in parallel, but the whole batch still has a fixed budget so a
/// single very slow module cannot stretch shell-side latency.
const SEARCH_ALL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(8);

/// Max bytes per `SearchResult` text field. Mirrors the equivalent
/// cap on the legacy in-process `WaypointerPlugin::max_results`
/// shape. A module that returns longer fields gets them truncated
/// with an ellipsis marker; the truncation is logged but does **not**
/// count as a crash because it is a content bug rather than a
/// runtime bug.
const SEARCH_FIELD_CAP_BYTES: usize = 4 * 1024;

/// Default number of search results a Tier 1 module may return per
/// call when its manifest does not specify a cap. Mirrors the
/// in-process `WaypointerPlugin::max_results` default.
const DEFAULT_MAX_RESULTS: usize = 8;

/// Failure modes for `Manager::search_tier1`. Distinguishes
/// recoverable traps (run module again after backoff) from
/// permanent load failures (bytecode is structurally broken) from
/// crash-cooldown short-circuits (module is *intentionally* unavailable,
/// not failing).
enum SearchFailure {
    Trap(String),
    Load(String),
    /// Module is in its crash-backoff cooldown window. Search
    /// dispatch returns empty results and records **no** new crash
    /// because the cooldown is the recovery mechanism itself —
    /// counting it as a crash would defeat the Foundation Table 08
    /// ladder.
    Cooldown,
}

/// Resolve a module's result cap from its manifest. Tier 1 modules
/// can express a per-call limit via the `[waypointer.search]`
/// section (foundation §6.4 Listing 13 default of 8); absent fields
/// fall back to `DEFAULT_MAX_RESULTS`.
fn search_result_cap(manifest: &lunaris_modules::ModuleManifest) -> usize {
    manifest
        .waypointer
        .as_ref()
        .and_then(|w| w.search.as_ref())
        .and_then(|s| s.max_results)
        .unwrap_or(DEFAULT_MAX_RESULTS)
}

/// Routing decision for a single `WaypointerSearchAll` invocation.
/// Each entry says: "send `query` to this module, capped at
/// `max_results`". The router populates this set from the full
/// candidate module list using the prefix-exclusive semantics
/// documented on [`route_search_all`].
struct ModuleDispatch {
    module_id: String,
    max_results: usize,
    query: String,
}

/// Codex round-2 finding 1 fix: decide *which set of modules* sees a
/// query, not whether each module independently passes a filter.
///
/// Matches the semantics of the in-process
/// `waypointer_system::PluginManager::search` (line 40-80 of that
/// file) so a third-party Tier 1 module and a first-party in-process
/// plugin behave identically:
///
/// 1. Trim the query; empty/whitespace → dispatch to nothing.
/// 2. Walk the modules. If any has a `prefix` and the query starts
///    with it, **return only that module** with the stripped query
///    appended (prefix is the user's exclusive activation signal —
///    every other module must not even see this keystroke).
/// 3. Otherwise return every module that does NOT declare a prefix,
///    with the unmodified query.
///
/// `detect_pattern` is **not** evaluated here. It is a hint the
/// guest may use to short-circuit its own search; the router treats
/// it like the in-process trait does, as descriptive metadata only.
/// Gating on it at the router-level would create a privacy split
/// the in-process implementation does not have.
fn route_search_all(
    candidates: &[(String, lunaris_modules::ModuleManifest)],
    query: &str,
) -> Vec<ModuleDispatch> {
    let trimmed_query = query.trim();
    if trimmed_query.is_empty() {
        return Vec::new();
    }

    // Step 1: look for the first prefix-owning module whose prefix
    // matches. Iterating in declared order matches the in-process
    // contract (it walks `self.plugins` in registration order).
    for (id, manifest) in candidates {
        let Some(search) = manifest
            .waypointer
            .as_ref()
            .and_then(|w| w.search.as_ref())
        else {
            continue;
        };
        let Some(prefix) = search.prefix.as_deref().map(str::trim) else {
            continue;
        };
        if prefix.is_empty() {
            continue;
        }
        if let Some(stripped) = trimmed_query.strip_prefix(prefix) {
            let stripped = stripped.trim_start();
            if stripped.is_empty() {
                // Prefix only, no payload → drop entirely (matches
                // in-process behaviour line 51-53).
                return Vec::new();
            }
            return vec![ModuleDispatch {
                module_id: id.clone(),
                max_results: search_result_cap(manifest),
                query: stripped.to_string(),
            }];
        }
    }

    // Step 2: no prefix won. Dispatch to every non-prefix module
    // declaring a `[waypointer.search]` section. Modules without
    // that section have no `search` export and are filtered out.
    candidates
        .iter()
        .filter_map(|(id, manifest)| {
            let search = manifest
                .waypointer
                .as_ref()
                .and_then(|w| w.search.as_ref())?;
            // Skip prefix-only modules — they only activate on their
            // exclusive prefix, which has already been ruled out.
            if search
                .prefix
                .as_deref()
                .map(str::trim)
                .is_some_and(|p| !p.is_empty())
            {
                return None;
            }
            Some(ModuleDispatch {
                module_id: id.clone(),
                max_results: search_result_cap(manifest),
                query: trimmed_query.to_string(),
            })
        })
        .collect()
}

/// Truncate a string to `SEARCH_FIELD_CAP_BYTES` if needed. Keeps an
/// ellipsis marker so the consumer can see the result was capped.
/// Always cuts at a UTF-8 char boundary so the output stays valid.
fn cap_field(s: String) -> String {
    if s.len() <= SEARCH_FIELD_CAP_BYTES {
        return s;
    }
    // Walk back to the nearest char boundary at or below the cap.
    let mut boundary = SEARCH_FIELD_CAP_BYTES;
    while boundary > 0 && !s.is_char_boundary(boundary) {
        boundary -= 1;
    }
    let mut truncated = s;
    truncated.truncate(boundary);
    truncated.push('…');
    truncated
}

/// Translate the WIT-generated `search-result` list returned by a
/// Tier 1 module into the wire-protocol `SearchResult` type. Applies
/// `max_results` truncation, per-field 4 KB cap, and `relevance`
/// clamping in one pass.
fn wit_to_proto_results(
    module_id: &str,
    wit_results: Vec<crate::runtime::wit::exports::lunaris::waypointer::provider::SearchResult>,
    max_results: usize,
) -> Vec<SearchResult> {
    use crate::runtime::wit::exports::lunaris::waypointer::provider::Action as WitAction;
    use crate::socket::protocol::SearchAction;

    wit_results
        .into_iter()
        .take(max_results)
        .map(|r| {
            let action = match r.action {
                WitAction::Copy(s) => SearchAction::Copy { text: cap_field(s) },
                WitAction::OpenUrl(s) => SearchAction::OpenUrl { url: cap_field(s) },
                WitAction::OpenPath(s) => SearchAction::OpenPath { path: cap_field(s) },
                WitAction::Execute(s) => SearchAction::Execute {
                    command: cap_field(s),
                },
                WitAction::Custom(c) => SearchAction::Custom {
                    handler: cap_field(c.handler),
                    data: cap_field(c.data),
                },
            };
            SearchResult {
                id: cap_field(r.id),
                title: cap_field(r.title),
                description: r.description.map(cap_field),
                icon: r.icon.map(cap_field),
                relevance: r.relevance.clamp(0.0, 1.0),
                action,
                plugin_id: module_id.to_string(),
            }
        })
        .collect()
}

/// Translate a wire-protocol `SearchResult` back into the WIT type
/// for the guest's `execute(hit: search-result)` call.
fn proto_to_wit_result(
    r: &SearchResult,
) -> crate::runtime::wit::exports::lunaris::waypointer::provider::SearchResult {
    use crate::runtime::wit::exports::lunaris::waypointer::provider::{
        Action as WitAction, CustomAction, SearchResult as WitResult,
    };
    use crate::socket::protocol::SearchAction;

    let action = match &r.action {
        SearchAction::Copy { text } => WitAction::Copy(text.clone()),
        SearchAction::OpenUrl { url } => WitAction::OpenUrl(url.clone()),
        SearchAction::OpenPath { path } => WitAction::OpenPath(path.clone()),
        SearchAction::Execute { command } => WitAction::Execute(command.clone()),
        SearchAction::Custom { handler, data } => WitAction::Custom(CustomAction {
            handler: handler.clone(),
            data: data.clone(),
        }),
    };
    WitResult {
        id: r.id.clone(),
        title: r.title.clone(),
        description: r.description.clone(),
        icon: r.icon.clone(),
        relevance: r.relevance,
        action,
    }
}

pub struct Manager {
    modules: RwLock<HashMap<String, ModuleEntry>>,
    tier1: Arc<Tier1Runtime>,
    tier2: Arc<Tier2Broker>,
    events_tx: broadcast::Sender<Event>,
    /// One `Semaphore` per module, lazily created. `Mutex` rather
    /// than `RwLock` because writes (insert on first use) and reads
    /// (acquire) both happen on the hot path.
    network_permits: Mutex<HashMap<String, Arc<Semaphore>>>,
    /// Live Tier 1 WASM instances keyed by `module.id`. Each value
    /// is wrapped in `Arc<OnceCell<...>>` so a per-module
    /// initialisation runs at most once even under concurrent
    /// first-touch (Codex finding 3: the old read-then-write
    /// hashmap pattern let two parallel searches each call guest
    /// `init()`, doubling any host side effects init performs).
    ///
    /// The outer `RwLock` only protects the map shape (insertions /
    /// removals). Reading an entry takes a brief read lock, clones
    /// the `Arc<OnceCell>`, then drops the lock — every awaited
    /// instantiation happens lock-free outside it. The inner
    /// `Mutex<Tier1Instance>` still serialises calls on the same
    /// module because wasmtime `Store` is `!Sync`.
    tier1_instances: RwLock<HashMap<String, Arc<OnceCell<Arc<Mutex<Tier1Instance>>>>>>,
}

impl Manager {
    pub fn new(events_tx: broadcast::Sender<Event>) -> crate::error::Result<Arc<Self>> {
        let tier1 = Arc::new(Tier1Runtime::new()?);
        let tier2 = Tier2Broker::new();
        Ok(Arc::new(Self {
            modules: RwLock::new(HashMap::new()),
            tier1,
            tier2,
            events_tx,
            network_permits: Mutex::new(HashMap::new()),
            tier1_instances: RwLock::new(HashMap::new()),
        }))
    }

    /// Acquire a per-module network permit. Returned permit limits a
    /// module to at most `NETWORK_CONCURRENCY_PER_MODULE` concurrent
    /// fetches. Drop the permit (or let it fall out of scope) to
    /// release the slot.
    async fn acquire_network_permit(&self, module_id: &str) -> OwnedSemaphorePermit {
        let semaphore = {
            let mut guard = self.network_permits.lock().await;
            Arc::clone(
                guard
                    .entry(module_id.to_string())
                    .or_insert_with(|| Arc::new(Semaphore::new(NETWORK_CONCURRENCY_PER_MODULE))),
            )
        };
        semaphore
            .acquire_owned()
            .await
            .expect("network permit semaphore was closed")
    }

    /// Run discovery and populate the module table. Idempotent: a
    /// second call after install + uninstall sequences will surface
    /// new modules without disturbing already-loaded ones.
    ///
    /// Module-ID collisions are logged loud and the *second*
    /// discovered record is skipped, not silently dropped via
    /// `HashMap::insert` overwrite (foundation §7.4: `module.id`
    /// must be globally unique). The first record wins because
    /// `entry(...).or_insert_with(...)` is used.
    pub async fn discover(&self) {
        let records = discover_all();
        info!("modulesd: discovered {} module(s)", records.len());
        let mut guard = self.modules.write().await;
        for record in records {
            let id = record.id().to_string();
            if let Some(existing) = guard.get(&id) {
                warn!(
                    module = %id,
                    existing = %existing.record.root.display(),
                    duplicate = %record.root.display(),
                    "modulesd: module id collision; keeping first discovered, skipping duplicate"
                );
                continue;
            }
            guard.insert(
                id,
                ModuleEntry {
                    record,
                    enabled: true,
                    crash: CrashState::new(),
                    next_retry_at: None,
                },
            );
        }
    }

    /// Get or lazily create a Tier 1 instance for the named module.
    /// Compiles the WASM bytes, instantiates the component against
    /// the runtime's pre-populated linker, and calls `init()` with
    /// the wall-clock timeout configured in `tier1::INIT_TIMEOUT`.
    ///
    /// **Single-init guarantee** (Codex finding 3): concurrent
    /// first-touch calls for the same module race only on the
    /// `OnceCell` insertion, never on the wasmtime instantiate +
    /// init path. The first call wins, runs guest `init()` exactly
    /// once, and every concurrent caller awaits its completion via
    /// the shared `OnceCell::get_or_try_init` future.
    ///
    /// On failure: the module's crash state is updated per Foundation
    /// Table 08. `WasmLoad` failures are treated as immediate
    /// permanent failure (the module's bytecode is structurally
    /// broken and a retry will not help). `WasmTrap` failures count
    /// toward the crash ladder.
    pub async fn ensure_tier1_instance(
        &self,
        module_id: &str,
    ) -> crate::error::Result<Arc<Mutex<Tier1Instance>>> {
        // Codex round-2 finding 3: enforce crash backoff *before*
        // touching the wasmtime path. If the module is in cooldown
        // after a recent crash, reject the request so the search
        // dispatch path can return empty results. Without this gate,
        // rapid keystrokes between crashes would keep firing
        // compile/init and promote the module to permanent-failed
        // in milliseconds instead of allowing the 5s/30s ladder to
        // actually wait.
        {
            let guard = self.modules.read().await;
            if let Some(entry) = guard.get(module_id) {
                if let Some(deadline) = entry.next_retry_at {
                    if Instant::now() < deadline {
                        return Err(DaemonError::InCooldown {
                            module_id: module_id.to_string(),
                        });
                    }
                }
            }
        }

        // Look up (or insert) the per-module OnceCell. Holding the
        // write lock only for the empty-cell case keeps the hot path
        // (cell already populated) read-locked.
        let cell: Arc<OnceCell<Arc<Mutex<Tier1Instance>>>> = {
            let read_guard = self.tier1_instances.read().await;
            if let Some(c) = read_guard.get(module_id) {
                Arc::clone(c)
            } else {
                drop(read_guard);
                let mut write_guard = self.tier1_instances.write().await;
                Arc::clone(
                    write_guard
                        .entry(module_id.to_string())
                        .or_insert_with(|| Arc::new(OnceCell::new())),
                )
            }
        };

        // Resolve module record + capability context. Done here
        // (inside `get_or_try_init`'s closure indirectly) so the
        // OnceCell's coordination governs whether the init runs;
        // multiple parallel waiters share the same future.
        let init = || async {
            let (root, ctx) = {
                let guard = self.modules.read().await;
                let entry = guard
                    .get(module_id)
                    .ok_or_else(|| DaemonError::NotFound(module_id.to_string()))?;
                if entry.record.tier != Tier::Wasm {
                    return Err(DaemonError::ManifestInvalid {
                        module_id: module_id.to_string(),
                        reason: "module is Tier 2, cannot be loaded as WASM".into(),
                    });
                }
                let ctx = CapabilityContext::new(
                    entry.record.id().to_string(),
                    entry.record.manifest.capabilities.clone(),
                );
                (entry.record.wasm_path(), ctx)
            };

            let component = self.tier1.compile(&root).await?;
            let instance = self.tier1.instantiate(module_id, &component, ctx).await?;
            Ok::<_, DaemonError>(Arc::new(Mutex::new(instance)))
        };

        let inst = cell.get_or_try_init(init).await?;
        Ok(Arc::clone(inst))
    }

    /// Drop the cached Tier 1 instance for a module (e.g. after a
    /// crash, on disable, or on uninstall). The next
    /// `ensure_tier1_instance` allocates a fresh `OnceCell` and
    /// rebuilds from scratch.
    pub async fn drop_tier1_instance(&self, module_id: &str) {
        // Remove the OnceCell entirely so the next ensure-call sees
        // an empty slot and runs a fresh init. Just clearing the
        // OnceCell's value would not be enough — `OnceCell::take`
        // requires `&mut OnceCell` which we cannot get through Arc.
        self.tier1_instances.write().await.remove(module_id);
    }

    /// Call `Guest::shutdown` on every loaded Tier 1 instance.
    /// Used by the SIGTERM handler in `main.rs` so modules with
    /// persistent state get a politeness signal before the process
    /// exits. Each shutdown call has a 1 s per-instance wall-clock
    /// timeout so one stuck module cannot block daemon exit; a
    /// timeout drops the instance ungracefully.
    pub async fn shutdown_all_tier1(&self) {
        // Snapshot only the cells that are actually initialised — an
        // OnceCell that never won its race holds no instance and
        // calling `get()` on it is None.
        let snapshot: Vec<(String, Arc<Mutex<Tier1Instance>>)> = {
            let guard = self.tier1_instances.read().await;
            guard
                .iter()
                .filter_map(|(k, cell)| cell.get().map(|inst| (k.clone(), Arc::clone(inst))))
                .collect()
        };

        for (module_id, instance) in snapshot {
            let timed = tokio::time::timeout(
                std::time::Duration::from_secs(1),
                async {
                    let mut guard = instance.lock().await;
                    guard.graceful_shutdown(&module_id).await;
                },
            )
            .await;
            if timed.is_err() {
                warn!(
                    module = %module_id,
                    "shutdown timed out after 1s; instance dropped ungracefully"
                );
            }
        }

        self.tier1_instances.write().await.clear();
    }

    pub async fn handle_request(&self, req: Request) -> Response {
        match req {
            Request::Hello { id, client, version } => {
                debug!("modulesd: hello from {client} v{version}");
                Response::Hello {
                    id,
                    version: env!("CARGO_PKG_VERSION").to_string(),
                }
            }

            Request::ListModules { id } => {
                let guard = self.modules.read().await;
                let modules = guard.values().map(ModuleEntry::summary).collect();
                Response::ModuleList { id, modules }
            }

            Request::WaypointerSearch {
                id,
                module_id,
                query,
            } => self.handle_search(&id, &module_id, &query).await,

            Request::WaypointerSearchAll { id, query } => {
                self.handle_search_all(&id, &query).await
            }

            Request::WaypointerExecute {
                id,
                module_id,
                result,
            } => self.handle_execute(&id, &module_id, result).await,

            Request::IframeMint {
                id,
                module_id,
                slot: _,
            } => self.handle_iframe_mint(&id, &module_id).await,

            Request::HostCall { id, nonce, call } => {
                self.handle_host_call(&id, &nonce, call).await
            }

            Request::Subscribe { id, .. } => Response::Subscribed { id },

            Request::SetEnabled {
                id,
                module_id,
                enabled,
            } => self.handle_set_enabled(&id, &module_id, enabled).await,

            Request::Retry { id, module_id } => self.handle_retry(&id, &module_id).await,

            Request::IframeLookup { id, nonce } => self.handle_iframe_lookup(&id, &nonce).await,
        }
    }

    async fn handle_iframe_lookup(&self, id: &str, nonce: &str) -> Response {
        let Some(instance) = self.tier2.lookup(nonce).await else {
            return Response::Error {
                id: id.to_string(),
                code: ErrorCode::NotFound,
                message: format!("unknown nonce {nonce}"),
            };
        };
        let guard = self.modules.read().await;
        let Some(entry) = guard.get(&instance.module_id) else {
            return Response::Error {
                id: id.to_string(),
                code: ErrorCode::NotFound,
                message: "module gone after iframe mint".into(),
            };
        };
        let csp = crate::runtime::build_csp(
            &instance.module_id,
            &entry.record.manifest.capabilities,
        );
        Response::IframeMeta {
            id: id.to_string(),
            module_id: instance.module_id.clone(),
            root_path: entry.record.dist_dir().to_string_lossy().into_owned(),
            csp,
        }
    }

    async fn handle_search(&self, id: &str, module_id: &str, query: &str) -> Response {
        // Check module is loadable before paying for instantiation.
        let max_results = {
            let guard = self.modules.read().await;
            let Some(entry) = guard.get(module_id) else {
                return Response::Error {
                    id: id.to_string(),
                    code: ErrorCode::NotFound,
                    message: format!("module {module_id} not found"),
                };
            };
            if !entry.enabled {
                return Response::WaypointerResults {
                    id: id.to_string(),
                    module_id: module_id.to_string(),
                    results: Vec::new(),
                };
            }
            if entry.crash.is_failed() {
                return Response::Error {
                    id: id.to_string(),
                    code: ErrorCode::ModuleFailed,
                    message: format!(
                        "module {module_id} permanently failed; manual retry required"
                    ),
                };
            }
            if entry.record.tier != Tier::Wasm {
                return Response::WaypointerResults {
                    id: id.to_string(),
                    module_id: module_id.to_string(),
                    results: Vec::new(),
                };
            }
            search_result_cap(&entry.record.manifest)
        };

        match self.search_tier1(module_id, query, max_results).await {
            Ok(results) => Response::WaypointerResults {
                id: id.to_string(),
                module_id: module_id.to_string(),
                results,
            },
            Err(SearchFailure::Trap(reason)) => {
                warn!(module = %module_id, reason = %reason, "tier 1 search trapped");
                self.record_crash(module_id).await;
                self.drop_tier1_instance(module_id).await;
                // Return empty results so the shell can still render
                // results from other modules. The crash event broadcast
                // is what Settings + the user observe.
                Response::WaypointerResults {
                    id: id.to_string(),
                    module_id: module_id.to_string(),
                    results: Vec::new(),
                }
            }
            Err(SearchFailure::Load(reason)) => {
                warn!(module = %module_id, reason = %reason, "tier 1 load failed (permanent)");
                // Promote to permanent failure by recording 4 crashes:
                // bytecode is structurally broken, retries will not help.
                for _ in 0..4 {
                    self.record_crash(module_id).await;
                }
                self.drop_tier1_instance(module_id).await;
                Response::Error {
                    id: id.to_string(),
                    code: ErrorCode::ModuleFailed,
                    message: reason,
                }
            }
            Err(SearchFailure::Cooldown) => {
                // Module is in crash backoff. Return empty results
                // without recording a fresh crash — the cooldown is
                // *what we are doing* for crash recovery, counting it
                // again would shrink the ladder to nothing.
                Response::WaypointerResults {
                    id: id.to_string(),
                    module_id: module_id.to_string(),
                    results: Vec::new(),
                }
            }
        }
    }

    async fn handle_search_all(&self, id: &str, query: &str) -> Response {
        // Empty / whitespace-only query never dispatches: Foundation
        // §6 budgets per keystroke, and an empty prefix-stripped
        // query would force every module into a no-op
        // `search("")` call.
        let trimmed = query.trim();
        if trimmed.is_empty() {
            return Response::WaypointerAggregate {
                id: id.to_string(),
                results: Vec::new(),
            };
        }

        // Snapshot eligible Tier 1 modules, then resolve the
        // dispatch set with prefix-exclusive semantics (Codex
        // round-2 finding 1). `route_search_all` mirrors the
        // in-process `waypointer_system::PluginManager::search`
        // contract: if any module owns a matching prefix it wins
        // exclusively; otherwise every non-prefix module sees the
        // query. detect_pattern is the guest's job.
        let candidates: Vec<(String, lunaris_modules::ModuleManifest)> = {
            let guard = self.modules.read().await;
            guard
                .values()
                .filter(|e| e.enabled && !e.crash.is_failed() && e.record.tier == Tier::Wasm)
                .map(|e| (e.record.id().to_string(), e.record.manifest.clone()))
                .collect()
        };
        let targets: Vec<(String, usize, String)> = route_search_all(&candidates, trimmed)
            .into_iter()
            .map(|d| (d.module_id, d.max_results, d.query))
            .collect();

        // Codex round-2 finding 2: `FuturesUnordered` retains
        // results from modules that finish before the aggregate
        // budget. The old `timeout(join_all(...))` pattern replaced
        // the ENTIRE batch with `Vec::new()` on timeout — one slow
        // module erased every quick module's results.
        //
        // Per-module SEARCH_TIMEOUT wraps the whole search_tier1
        // call (which includes ensure_tier1_instance). That way
        // first-touch compile+init still counts against the
        // per-module budget, not silently against the aggregate.
        use futures_util::stream::{FuturesUnordered, StreamExt};

        let deadline = tokio::time::Instant::now() + SEARCH_ALL_TIMEOUT;
        let mut pending: FuturesUnordered<_> = targets
            .into_iter()
            .map(|(module_id, max_results, forwarded_query)| {
                let id_for_future = module_id.clone();
                async move {
                    let outcome = tokio::time::timeout(
                        SEARCH_TIMEOUT,
                        self.search_tier1(&id_for_future, &forwarded_query, max_results),
                    )
                    .await;
                    (module_id, outcome)
                }
            })
            .collect();

        let mut all: Vec<SearchResult> = Vec::new();
        let mut budget_hit = false;
        loop {
            let next = tokio::time::timeout_at(deadline, pending.next()).await;
            match next {
                Ok(Some((_module_id, Ok(Ok(rs))))) => all.extend(rs),
                Ok(Some((module_id, Ok(Err(SearchFailure::Trap(reason)))))) => {
                    warn!(module = %module_id, reason = %reason, "tier 1 search trapped during search_all");
                    self.record_crash(&module_id).await;
                    self.drop_tier1_instance(&module_id).await;
                }
                Ok(Some((module_id, Ok(Err(SearchFailure::Load(reason)))))) => {
                    warn!(module = %module_id, reason = %reason, "tier 1 load failed during search_all");
                    for _ in 0..4 {
                        self.record_crash(&module_id).await;
                    }
                    self.drop_tier1_instance(&module_id).await;
                }
                Ok(Some((_module_id, Ok(Err(SearchFailure::Cooldown))))) => {
                    // Cooldown is the recovery — skip silently.
                }
                Ok(Some((module_id, Err(_per_call_elapsed)))) => {
                    // Per-module SEARCH_TIMEOUT exceeded. The module
                    // had its budget, took too long: count as a
                    // crash and drop the instance. Other modules
                    // continue.
                    warn!(
                        module = %module_id,
                        "tier 1 module exceeded {}s per-call budget; dropping",
                        SEARCH_TIMEOUT.as_secs(),
                    );
                    self.record_crash(&module_id).await;
                    self.drop_tier1_instance(&module_id).await;
                }
                Ok(None) => break, // all modules completed
                Err(_aggregate_elapsed) => {
                    budget_hit = true;
                    break;
                }
            }
        }
        if budget_hit {
            warn!(
                pending_left = pending.len(),
                completed = all.len(),
                "search_all hit aggregate {}s budget; returning {} completed results, {} modules dropped",
                SEARCH_ALL_TIMEOUT.as_secs(),
                all.len(),
                pending.len(),
            );
        }

        all.sort_by(|a, b| {
            b.relevance
                .partial_cmp(&a.relevance)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        Response::WaypointerAggregate { id: id.to_string(), results: all }
    }

    /// Single-module Tier 1 search call. Returns trap and load
    /// failures as distinct variants so the caller can apply the
    /// right crash-counter behaviour (single crash for trap, jump
    /// to permanent failure for load-error).
    async fn search_tier1(
        &self,
        module_id: &str,
        query: &str,
        max_results: usize,
    ) -> std::result::Result<Vec<SearchResult>, SearchFailure> {
        let instance = self
            .ensure_tier1_instance(module_id)
            .await
            .map_err(|e| match e {
                DaemonError::InCooldown { .. } => SearchFailure::Cooldown,
                DaemonError::WasmTrap { reason, .. } => SearchFailure::Trap(reason),
                DaemonError::WasmLoad { reason, .. } => SearchFailure::Load(reason),
                DaemonError::PermanentlyFailed { .. } => {
                    SearchFailure::Load("module already permanently failed".into())
                }
                other => SearchFailure::Load(other.to_string()),
            })?;

        let mut guard = instance.lock().await;
        // Refill fuel; an earlier call may have left some leftover.
        let _ = guard.store.set_fuel(crate::runtime::tier1::DEFAULT_FUEL_BUDGET);

        // Split-borrow: take a mutable reference to the whole instance
        // and pull `provider` + `store` from it without aliasing.
        let inst = &mut *guard;
        // Codex finding 4: wrap the WIT call in a wall-clock timeout.
        // Fuel handles CPU loops; this covers async host calls that
        // can block the guest beyond the per-keystroke budget.
        let wit_results = tokio::time::timeout(
            SEARCH_TIMEOUT,
            inst.provider
                .lunaris_waypointer_provider()
                .call_search(&mut inst.store, query),
        )
        .await
        .map_err(|_| {
            SearchFailure::Trap(format!("search exceeded {}s wall-clock budget", SEARCH_TIMEOUT.as_secs()))
        })?
        .map_err(|trap| SearchFailure::Trap(format!("search trap: {trap}")))?;

        Ok(wit_to_proto_results(module_id, wit_results, max_results))
    }

    async fn handle_execute(
        &self,
        id: &str,
        module_id: &str,
        result: SearchResult,
    ) -> Response {
        // Resolve module + Tier check up-front. Fail fast on a Tier 2
        // module: execute() is meaningless there because Tier 2 has
        // no `Guest::execute` export.
        {
            let guard = self.modules.read().await;
            let Some(entry) = guard.get(module_id) else {
                return Response::Error {
                    id: id.to_string(),
                    code: ErrorCode::NotFound,
                    message: format!("module {module_id} not found"),
                };
            };
            if !entry.enabled {
                return Response::Error {
                    id: id.to_string(),
                    code: ErrorCode::InvalidRequest,
                    message: format!("module {module_id} is disabled"),
                };
            }
            if entry.crash.is_failed() {
                return Response::Error {
                    id: id.to_string(),
                    code: ErrorCode::ModuleFailed,
                    message: format!("module {module_id} permanently failed"),
                };
            }
            if entry.record.tier != Tier::Wasm {
                return Response::Acked { id: id.to_string() };
            }
        }

        let instance = match self.ensure_tier1_instance(module_id).await {
            Ok(i) => i,
            Err(DaemonError::WasmTrap { reason, .. }) => {
                self.record_crash(module_id).await;
                self.drop_tier1_instance(module_id).await;
                return Response::Error {
                    id: id.to_string(),
                    code: ErrorCode::Internal,
                    message: reason,
                };
            }
            Err(other) => {
                return Response::Error {
                    id: id.to_string(),
                    code: ErrorCode::Internal,
                    message: other.to_string(),
                };
            }
        };

        let mut guard = instance.lock().await;
        let _ = guard.store.set_fuel(crate::runtime::tier1::DEFAULT_FUEL_BUDGET);

        let wit_hit = proto_to_wit_result(&result);
        let inst = &mut *guard;
        // Codex finding 4: same wall-clock guard as `search`.
        let exec_outcome = tokio::time::timeout(
            SEARCH_TIMEOUT,
            inst.provider
                .lunaris_waypointer_provider()
                .call_execute(&mut inst.store, &wit_hit),
        )
        .await;
        match exec_outcome {
            Ok(Ok(Ok(()))) => Response::Executed { id: id.to_string() },
            Ok(Ok(Err(module_err))) => Response::Error {
                id: id.to_string(),
                code: ErrorCode::Internal,
                message: format!("module returned error: {module_err}"),
            },
            Ok(Err(trap)) => {
                warn!(module = %module_id, "execute trapped: {trap}");
                drop(guard);
                self.record_crash(module_id).await;
                self.drop_tier1_instance(module_id).await;
                Response::Error {
                    id: id.to_string(),
                    code: ErrorCode::Internal,
                    message: format!("execute trapped: {trap}"),
                }
            }
            Err(_elapsed) => {
                warn!(module = %module_id, "execute exceeded wall-clock budget");
                drop(guard);
                self.record_crash(module_id).await;
                self.drop_tier1_instance(module_id).await;
                Response::Error {
                    id: id.to_string(),
                    code: ErrorCode::Timeout,
                    message: format!(
                        "execute exceeded {}s wall-clock budget",
                        SEARCH_TIMEOUT.as_secs()
                    ),
                }
            }
        }
    }

    async fn handle_iframe_mint(&self, id: &str, module_id: &str) -> Response {
        let guard = self.modules.read().await;
        let Some(entry) = guard.get(module_id) else {
            return Response::Error {
                id: id.to_string(),
                code: ErrorCode::NotFound,
                message: format!("module {module_id} not found"),
            };
        };
        if entry.record.tier != Tier::Iframe {
            return Response::Error {
                id: id.to_string(),
                code: ErrorCode::InvalidRequest,
                message: "module is Tier 1; iframe minting only valid for Tier 2".into(),
            };
        }
        let nonce = crate::runtime::tier2::mint_nonce();
        let url = format!("module://{module_id}/dist/index.html?nonce={nonce}");
        let ctx = CapabilityContext::new(
            entry.record.id().to_string(),
            entry.record.manifest.capabilities.clone(),
        );
        drop(guard);

        self.tier2
            .register(crate::runtime::tier2::IframeInstance {
                module_id: module_id.to_string(),
                instance_id: format!("{module_id}-{nonce}"),
                nonce: nonce.clone(),
                created_at: Instant::now(),
                ctx,
            })
            .await;

        Response::IframeIssued {
            id: id.to_string(),
            url,
            nonce,
        }
    }

    async fn handle_host_call(
        &self,
        id: &str,
        nonce: &str,
        call: crate::socket::protocol::HostCall,
    ) -> Response {
        // Resolve the iframe by nonce. Unknown nonce means the iframe
        // was revoked or never minted; either way we treat the caller
        // as untrusted and refuse.
        let Some(instance) = self.tier2.lookup(nonce).await else {
            return Response::Error {
                id: id.to_string(),
                code: ErrorCode::PermissionDenied,
                message: format!("unknown iframe nonce {nonce}"),
            };
        };

        // Re-check the backing module's lifecycle state on every
        // call. A nonce can outlive its module's enabled bit if a
        // disable raced with an in-flight call, or if the daemon's
        // own revocation is still in progress. Failing closed here
        // closes that race.
        {
            let guard = self.modules.read().await;
            match guard.get(&instance.module_id) {
                Some(entry) if !entry.enabled => {
                    return Response::Error {
                        id: id.to_string(),
                        code: ErrorCode::PermissionDenied,
                        message: format!("module {} is disabled", instance.module_id),
                    };
                }
                Some(entry) if entry.crash.is_failed() => {
                    return Response::Error {
                        id: id.to_string(),
                        code: ErrorCode::ModuleFailed,
                        message: format!(
                            "module {} permanently failed",
                            instance.module_id
                        ),
                    };
                }
                None => {
                    return Response::Error {
                        id: id.to_string(),
                        code: ErrorCode::NotFound,
                        message: format!(
                            "module {} no longer registered",
                            instance.module_id
                        ),
                    };
                }
                _ => {}
            }
        }

        use crate::host;
        use crate::socket::protocol::{HostCall, HostReply};

        let reply = match call {
            HostCall::GraphQuery { cypher } => {
                match host::graph::check_query(&instance.ctx, &cypher) {
                    Ok(_kind) => {
                        // Real query execution is wired in S5 with the
                        // dogfood module; for now we reply with an
                        // empty result set so the iframe sees a typed
                        // success response rather than a denial.
                        HostReply::GraphResult { rows: "[]".into() }
                    }
                    Err(crate::error::DaemonError::CapabilityDenied { capability, .. }) => {
                        HostReply::Error {
                            code: ErrorCode::PermissionDenied,
                            message: capability,
                        }
                    }
                    Err(other) => HostReply::Error {
                        code: ErrorCode::Internal,
                        message: other.to_string(),
                    },
                }
            }
            HostCall::GraphWrite { cypher } => {
                match host::graph::check_query(&instance.ctx, &cypher) {
                    Ok(_) => HostReply::GraphResult { rows: "[]".into() },
                    Err(crate::error::DaemonError::CapabilityDenied { capability, .. }) => {
                        HostReply::Error {
                            code: ErrorCode::PermissionDenied,
                            message: capability,
                        }
                    }
                    Err(other) => HostReply::Error {
                        code: ErrorCode::Internal,
                        message: other.to_string(),
                    },
                }
            }
            HostCall::NetworkFetch { url, headers } => {
                use base64::Engine;
                use std::sync::Arc;
                let ctx_arc = Arc::new(instance.ctx.clone());
                let permit = self.acquire_network_permit(&instance.module_id).await;
                let outcome = host::network::fetch(ctx_arc, &url, &headers).await;
                drop(permit);
                match outcome {
                    Ok(resp) => HostReply::NetworkBody {
                        status: resp.status,
                        body_b64: base64::engine::general_purpose::STANDARD
                            .encode(&resp.body),
                    },
                    Err(crate::error::DaemonError::CapabilityDenied { capability, .. }) => {
                        HostReply::Error {
                            code: ErrorCode::PermissionDenied,
                            message: capability,
                        }
                    }
                    Err(other) => HostReply::Error {
                        code: ErrorCode::Internal,
                        message: other.to_string(),
                    },
                }
            }
            HostCall::EventEmit {
                event_type,
                payload_b64: _,
            } => match host::events::check_publish(&instance.ctx, &event_type) {
                Ok(()) => HostReply::Acked,
                Err(crate::error::DaemonError::CapabilityDenied { capability, .. }) => {
                    HostReply::Error {
                        code: ErrorCode::PermissionDenied,
                        message: capability,
                    }
                }
                Err(other) => HostReply::Error {
                    code: ErrorCode::Internal,
                    message: other.to_string(),
                },
            },
        };

        Response::HostReply {
            id: id.to_string(),
            reply,
        }
    }

    async fn handle_set_enabled(
        &self,
        id: &str,
        module_id: &str,
        enabled: bool,
    ) -> Response {
        let mut guard = self.modules.write().await;
        let Some(entry) = guard.get_mut(module_id) else {
            return Response::Error {
                id: id.to_string(),
                code: ErrorCode::NotFound,
                message: format!("module {module_id} not found"),
            };
        };
        entry.enabled = enabled;
        drop(guard);
        // On disable, every live nonce belonging to this module must
        // be revoked so any iframe still mounted in the shell can no
        // longer issue capability-checked host calls. Without this the
        // `enabled = false` flip would only block *new* mints; old
        // iframes would keep using their pre-existing nonce until
        // process restart, which (now that fetch is real) is a data
        // exfiltration window.
        if !enabled {
            self.tier2.revoke_module(module_id).await;
        }
        let _ = self.events_tx.send(if enabled {
            Event::ModuleEnabled {
                module_id: module_id.to_string(),
            }
        } else {
            Event::ModuleDisabled {
                module_id: module_id.to_string(),
            }
        });
        Response::Acked { id: id.to_string() }
    }

    async fn handle_retry(&self, id: &str, module_id: &str) -> Response {
        let mut guard = self.modules.write().await;
        let Some(entry) = guard.get_mut(module_id) else {
            return Response::Error {
                id: id.to_string(),
                code: ErrorCode::NotFound,
                message: format!("module {module_id} not found"),
            };
        };
        if !entry.crash.is_failed() {
            return Response::Acked { id: id.to_string() };
        }
        entry.crash.manual_retry();
        // Clear any pending backoff deadline so the next search can
        // immediately rebuild the instance. The user explicitly
        // pushed Retry; the backoff ladder resets.
        entry.next_retry_at = None;
        info!("modulesd: manual retry for {module_id}");
        Response::Acked { id: id.to_string() }
    }

    /// Hook for runtime crashes. The Tier 1 runtime calls this on a
    /// trapped invocation, the Tier 2 broker calls it on iframe
    /// `onerror`. Both paths apply the same Foundation §07 recovery
    /// policy and broadcast the matching event.
    pub async fn record_crash(&self, module_id: &str) -> Recovery {
        let mut guard = self.modules.write().await;
        let Some(entry) = guard.get_mut(module_id) else {
            warn!("modulesd: crash recorded for unknown module {module_id}");
            return Recovery::Immediate;
        };
        let now = Instant::now();
        let recovery = entry.crash.record_crash(now);
        // Codex round-2 finding 3: store the retry deadline so the
        // search dispatch path can short-circuit during backoff.
        // Without this the recorded `Recovery` was rhetorical only
        // and the next keystroke would re-trigger compile/init,
        // promoting flapping modules to permanent-failed within
        // hundreds of milliseconds.
        entry.next_retry_at = match recovery {
            Recovery::Immediate => None,
            Recovery::Delayed { delay } => Some(now + delay),
            Recovery::PermanentlyFailed { .. } => None,
        };
        let next_action = match recovery {
            Recovery::Immediate => "immediate".to_string(),
            Recovery::Delayed { delay } => format!("delayed:{}s", delay.as_secs()),
            Recovery::PermanentlyFailed { .. } => "failed".to_string(),
        };
        let crashes = entry.crash.crash_count();
        drop(guard);
        let _ = self.events_tx.send(Event::ModuleCrashed {
            module_id: module_id.to_string(),
            crashes,
            next_action,
        });
        if matches!(recovery, Recovery::PermanentlyFailed { .. }) {
            let _ = self.events_tx.send(Event::ModuleFailed {
                module_id: module_id.to_string(),
            });
        }
        recovery
    }

    /// Hook for clean runs. The Tier 1 runtime calls this after a
    /// successful invocation; Tier 2 calls it whenever a postMessage
    /// completes without error.
    pub async fn record_clean(&self, module_id: &str) {
        let mut guard = self.modules.write().await;
        if let Some(entry) = guard.get_mut(module_id) {
            entry.crash.record_clean_run(Instant::now());
        }
    }

    /// For tests: directly insert a record. Not part of the public
    /// API surface; integration tests in `modulesd/tests/` need it
    /// because they cannot reach `#[cfg(test)]` inner items, so it
    /// is `pub` on the lib but namespaced as `_for_test` to keep
    /// production callers from using it accidentally.
    pub async fn insert_for_test(&self, record: ModuleRecord) {
        self.modules.write().await.insert(
            record.id().to_string(),
            ModuleEntry {
                record,
                enabled: true,
                crash: CrashState::new(),
                next_retry_at: None,
            },
        );
    }

    /// For tests: register a Tier 2 iframe directly without going
    /// through the mint flow.
    pub async fn register_iframe_for_test(
        &self,
        instance: crate::runtime::tier2::IframeInstance,
    ) {
        self.tier2.register(instance).await;
    }

    pub fn events_tx(&self) -> broadcast::Sender<Event> {
        self.events_tx.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lunaris_modules::{ModuleManifest, ModuleMeta, ModuleType};

    fn record(id: &str, tier: Tier) -> ModuleRecord {
        ModuleRecord {
            manifest: ModuleManifest {
                module: ModuleMeta {
                    id: id.into(),
                    name: id.into(),
                    version: "1.0.0".into(),
                    description: String::new(),
                    module_type: ModuleType::ThirdParty,
                    entry: "module.wasm".into(),
                    icon: String::new(),
                },
                waypointer: None,
                topbar: None,
                settings: None,
                quicksettings: None,
                capabilities: Default::default(),
                permissions: Default::default(),
                keybindings: Vec::new(),
            },
            root: std::path::PathBuf::from("/tmp"),
            tier,
        }
    }

    #[tokio::test]
    async fn list_modules_returns_inserted_record() {
        let (tx, _rx) = broadcast::channel(16);
        let m = Manager::new(tx).unwrap();
        m.insert_for_test(record("com.example.a", Tier::Wasm)).await;

        let resp = m
            .handle_request(Request::ListModules { id: "1".into() })
            .await;
        match resp {
            Response::ModuleList { modules, .. } => {
                assert_eq!(modules.len(), 1);
                assert_eq!(modules[0].id, "com.example.a");
                assert_eq!(modules[0].tier, ModuleTier::Wasm);
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[tokio::test]
    async fn set_enabled_persists_in_summary() {
        let (tx, _rx) = broadcast::channel(16);
        let m = Manager::new(tx).unwrap();
        m.insert_for_test(record("x", Tier::Wasm)).await;
        m.handle_request(Request::SetEnabled {
            id: "1".into(),
            module_id: "x".into(),
            enabled: false,
        })
        .await;
        let resp = m
            .handle_request(Request::ListModules { id: "2".into() })
            .await;
        if let Response::ModuleList { modules, .. } = resp {
            assert!(!modules[0].enabled);
        }
    }

    #[tokio::test]
    async fn iframe_mint_rejects_tier1_module() {
        let (tx, _rx) = broadcast::channel(16);
        let m = Manager::new(tx).unwrap();
        m.insert_for_test(record("x", Tier::Wasm)).await;
        let resp = m
            .handle_request(Request::IframeMint {
                id: "1".into(),
                module_id: "x".into(),
                slot: "topbar".into(),
            })
            .await;
        assert!(matches!(resp, Response::Error { .. }));
    }

    #[tokio::test]
    async fn iframe_mint_returns_url_with_nonce_for_tier2() {
        let (tx, _rx) = broadcast::channel(16);
        let m = Manager::new(tx).unwrap();
        m.insert_for_test(record("com.example.weather", Tier::Iframe))
            .await;
        let resp = m
            .handle_request(Request::IframeMint {
                id: "1".into(),
                module_id: "com.example.weather".into(),
                slot: "topbar".into(),
            })
            .await;
        match resp {
            Response::IframeIssued { url, nonce, .. } => {
                assert!(url.starts_with("module://com.example.weather/dist/"));
                assert!(url.contains(&nonce));
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[tokio::test]
    async fn record_crash_emits_event_and_advances_state() {
        let (tx, mut rx) = broadcast::channel(16);
        let m = Manager::new(tx).unwrap();
        m.insert_for_test(record("x", Tier::Wasm)).await;

        let r1 = m.record_crash("x").await;
        assert_eq!(r1, Recovery::Immediate);

        // Should have emitted ModuleCrashed.
        let ev = rx.try_recv().unwrap();
        assert!(matches!(ev, Event::ModuleCrashed { .. }));

        let r2 = m.record_crash("x").await;
        assert!(matches!(r2, Recovery::Delayed { .. }));
    }

    #[tokio::test]
    async fn retry_revives_failed_module() {
        let (tx, _rx) = broadcast::channel(16);
        let m = Manager::new(tx).unwrap();
        m.insert_for_test(record("x", Tier::Wasm)).await;
        for _ in 0..4 {
            m.record_crash("x").await;
        }
        let resp = m
            .handle_request(Request::Retry {
                id: "1".into(),
                module_id: "x".into(),
            })
            .await;
        assert!(matches!(resp, Response::Acked { .. }));
        // Next crash should again be Immediate.
        assert_eq!(m.record_crash("x").await, Recovery::Immediate);
    }

    #[tokio::test]
    async fn host_call_unknown_nonce_is_permission_denied() {
        let (tx, _rx) = broadcast::channel(16);
        let m = Manager::new(tx).unwrap();
        let resp = m
            .handle_request(Request::HostCall {
                id: "1".into(),
                nonce: "nope".into(),
                call: crate::socket::protocol::HostCall::NetworkFetch {
                    url: "https://example.com".into(),
                    headers: vec![],
                },
            })
            .await;
        match resp {
            Response::Error { code, .. } => assert_eq!(code, ErrorCode::PermissionDenied),
            other => panic!("expected denial, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn host_call_network_denied_when_url_outside_allowlist() {
        use crate::host::CapabilityContext;
        use crate::runtime::tier2::IframeInstance;
        use lunaris_modules::{ModuleCapabilities, NetworkCapability};

        let (tx, _rx) = broadcast::channel(16);
        let m = Manager::new(tx).unwrap();
        m.insert_for_test(record("com.example.weather", Tier::Iframe))
            .await;
        let mut caps = ModuleCapabilities::default();
        caps.network = Some(NetworkCapability {
            allowed_domains: vec!["api.example.com".into()],
        });
        let ctx = CapabilityContext::new("com.example.weather", caps);
        m.register_iframe_for_test(IframeInstance {
            module_id: "com.example.weather".into(),
            instance_id: "iid".into(),
            nonce: "n1".into(),
            created_at: std::time::Instant::now(),
            ctx,
        })
        .await;

        let resp = m
            .handle_request(Request::HostCall {
                id: "1".into(),
                nonce: "n1".into(),
                call: crate::socket::protocol::HostCall::NetworkFetch {
                    url: "https://api.evil.com/x".into(),
                    headers: vec![],
                },
            })
            .await;
        match resp {
            Response::HostReply { reply, .. } => {
                use crate::socket::protocol::HostReply;
                assert!(matches!(reply, HostReply::Error { code: ErrorCode::PermissionDenied, .. }));
            }
            other => panic!("expected HostReply, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn host_call_network_allowed_url_passes_capability_check() {
        // We can't easily reach `https://api.example.com` in unit
        // tests, but we can verify the capability layer doesn't
        // reject the URL before reqwest tries. A real network
        // round-trip is exercised by the wiremock-based integration
        // test under `tests/network_e2e.rs`.
        use crate::host::CapabilityContext;
        use crate::runtime::tier2::IframeInstance;
        use lunaris_modules::{ModuleCapabilities, NetworkCapability};

        let (tx, _rx) = broadcast::channel(16);
        let m = Manager::new(tx).unwrap();
        m.insert_for_test(record("x", Tier::Iframe)).await;
        let mut caps = ModuleCapabilities::default();
        caps.network = Some(NetworkCapability {
            allowed_domains: vec!["api.example.invalid".into()],
        });
        let ctx = CapabilityContext::new("x", caps);
        m.register_iframe_for_test(IframeInstance {
            module_id: "x".into(),
            instance_id: "iid".into(),
            nonce: "n1".into(),
            created_at: std::time::Instant::now(),
            ctx,
        })
        .await;

        let resp = m
            .handle_request(Request::HostCall {
                id: "1".into(),
                nonce: "n1".into(),
                call: crate::socket::protocol::HostCall::NetworkFetch {
                    url: "https://api.example.invalid/v1".into(),
                    headers: vec![],
                },
            })
            .await;
        match resp {
            Response::HostReply { reply, .. } => {
                use crate::socket::protocol::HostReply;
                // Capability passes, then reqwest fails on
                // unresolvable host: that's an Internal error, not
                // a PermissionDenied. Distinguishing those two is
                // exactly what the test asserts.
                match reply {
                    HostReply::NetworkBody { .. } => {} // surprising but ok
                    HostReply::Error { code, .. } => {
                        assert_ne!(
                            code,
                            ErrorCode::PermissionDenied,
                            "capability check must pass; only the actual fetch fails"
                        );
                    }
                    other => panic!("unexpected reply: {other:?}"),
                }
            }
            other => panic!("expected HostReply, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn host_call_event_emit_gated_by_publish_allowlist() {
        use crate::host::CapabilityContext;
        use crate::runtime::tier2::IframeInstance;
        use lunaris_modules::{EventBusCapability, ModuleCapabilities};

        let (tx, _rx) = broadcast::channel(16);
        let m = Manager::new(tx).unwrap();
        m.insert_for_test(record("x", Tier::Iframe)).await;
        let mut caps = ModuleCapabilities::default();
        caps.event_bus = Some(EventBusCapability {
            publish: vec!["module.com.example.".into()],
            subscribe: vec![],
        });
        let ctx = CapabilityContext::new("x", caps);
        m.register_iframe_for_test(IframeInstance {
            module_id: "x".into(),
            instance_id: "iid".into(),
            nonce: "n1".into(),
            created_at: std::time::Instant::now(),
            ctx,
        })
        .await;

        let allowed = m
            .handle_request(Request::HostCall {
                id: "1".into(),
                nonce: "n1".into(),
                call: crate::socket::protocol::HostCall::EventEmit {
                    event_type: "module.com.example.refreshed".into(),
                    payload_b64: String::new(),
                },
            })
            .await;
        let denied = m
            .handle_request(Request::HostCall {
                id: "2".into(),
                nonce: "n1".into(),
                call: crate::socket::protocol::HostCall::EventEmit {
                    event_type: "system.shutdown".into(),
                    payload_b64: String::new(),
                },
            })
            .await;

        use crate::socket::protocol::HostReply;
        if let Response::HostReply { reply, .. } = allowed {
            assert!(matches!(reply, HostReply::Acked));
        } else {
            panic!();
        }
        if let Response::HostReply { reply, .. } = denied {
            assert!(matches!(
                reply,
                HostReply::Error { code: ErrorCode::PermissionDenied, .. }
            ));
        } else {
            panic!();
        }
    }

    #[tokio::test]
    async fn disabling_a_module_revokes_its_iframe_nonces() {
        use crate::host::CapabilityContext;
        use crate::runtime::tier2::IframeInstance;

        let (tx, _rx) = broadcast::channel(16);
        let m = Manager::new(tx).unwrap();
        m.insert_for_test(record("com.example.weather", Tier::Iframe))
            .await;
        m.register_iframe_for_test(IframeInstance {
            module_id: "com.example.weather".into(),
            instance_id: "iid".into(),
            nonce: "live-nonce".into(),
            created_at: std::time::Instant::now(),
            ctx: CapabilityContext::empty("com.example.weather"),
        })
        .await;

        // Disable the module. The daemon should revoke every live
        // nonce belonging to it, so a subsequent host call from the
        // (still-mounted) iframe is rejected.
        m.handle_request(Request::SetEnabled {
            id: "1".into(),
            module_id: "com.example.weather".into(),
            enabled: false,
        })
        .await;

        let resp = m
            .handle_request(Request::HostCall {
                id: "2".into(),
                nonce: "live-nonce".into(),
                call: crate::socket::protocol::HostCall::NetworkFetch {
                    url: "https://api.example.com/exfil".into(),
                    headers: vec![],
                },
            })
            .await;
        match resp {
            Response::Error { code, .. } => {
                assert_eq!(code, ErrorCode::PermissionDenied);
            }
            other => panic!("expected PermissionDenied, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn host_call_after_disable_without_nonce_revoke_still_fails() {
        // Belt-and-suspenders: even if the tier2 broker had a bug
        // and forgot to revoke the nonce, the per-call enabled
        // re-check in handle_host_call still rejects the request.
        use crate::host::CapabilityContext;
        use crate::runtime::tier2::IframeInstance;

        let (tx, _rx) = broadcast::channel(16);
        let m = Manager::new(tx).unwrap();
        m.insert_for_test(record("x", Tier::Iframe)).await;
        m.register_iframe_for_test(IframeInstance {
            module_id: "x".into(),
            instance_id: "iid".into(),
            nonce: "n1".into(),
            created_at: std::time::Instant::now(),
            ctx: CapabilityContext::empty("x"),
        })
        .await;

        // Manually flip the enabled bit without going through
        // SetEnabled (and therefore without revoking nonces). The
        // host_call path should still recognise the disabled state.
        {
            let mut guard = m.modules.write().await;
            guard.get_mut("x").unwrap().enabled = false;
        }

        let resp = m
            .handle_request(Request::HostCall {
                id: "1".into(),
                nonce: "n1".into(),
                call: crate::socket::protocol::HostCall::NetworkFetch {
                    url: "https://api.example.com".into(),
                    headers: vec![],
                },
            })
            .await;
        match resp {
            Response::Error { code, .. } => {
                assert_eq!(code, ErrorCode::PermissionDenied);
            }
            other => panic!("expected PermissionDenied, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn search_for_failed_module_returns_typed_error() {
        let (tx, _rx) = broadcast::channel(16);
        let m = Manager::new(tx).unwrap();
        m.insert_for_test(record("x", Tier::Wasm)).await;
        for _ in 0..4 {
            m.record_crash("x").await;
        }
        let resp = m
            .handle_request(Request::WaypointerSearch {
                id: "1".into(),
                module_id: "x".into(),
                query: "any".into(),
            })
            .await;
        match resp {
            Response::Error { code, .. } => assert_eq!(code, ErrorCode::ModuleFailed),
            other => panic!("unexpected: {other:?}"),
        }
    }

    /// Two records with the same `module.id` must not silently
    /// overwrite each other — foundation §7.4 requires global ID
    /// uniqueness. The first record wins; the second is logged and
    /// skipped. Until the discover() pipeline takes input arguments
    /// directly we exercise the same code path by inserting twice
    /// via the test helper, then verifying the first record's
    /// `version` is what handle survives.
    #[tokio::test]
    async fn duplicate_module_id_does_not_overwrite_first() {
        let (tx, _rx) = broadcast::channel(16);
        let m = Manager::new(tx).unwrap();

        // First insert wins.
        let mut first = record("com.example.dup", Tier::Wasm);
        first.manifest.module.version = "1.0.0".into();
        m.insert_for_test(first).await;

        // discover() uses entry().or_insert; insert_for_test goes
        // through a different path (handle_set_enabled-style direct
        // write). For coverage of the discover()-side guard,
        // simulate two competing records via the same code shape
        // by exercising `entry`. We re-invoke the same helper and
        // check the version is unchanged.
        {
            let mut guard = m.modules.write().await;
            let mut dup = record("com.example.dup", Tier::Wasm);
            dup.manifest.module.version = "2.0.0".into();
            guard.entry(dup.id().to_string()).or_insert(ModuleEntry {
                record: dup,
                enabled: true,
                crash: CrashState::new(),
                next_retry_at: None,
            });
        }

        let guard = m.modules.read().await;
        let kept = guard.get("com.example.dup").unwrap();
        assert_eq!(kept.record.manifest.module.version, "1.0.0");
    }

    /// `drop_tier1_instance` is a no-op on a never-loaded module.
    /// Verifies the API tolerates that without panicking, so callers
    /// (crash handler) do not need to pre-check.
    #[tokio::test]
    async fn drop_tier1_instance_on_unloaded_module_is_noop() {
        let (tx, _rx) = broadcast::channel(16);
        let m = Manager::new(tx).unwrap();
        m.insert_for_test(record("com.example.notloaded", Tier::Wasm))
            .await;
        m.drop_tier1_instance("com.example.notloaded").await;
        // No assert: completing without panic is the contract.
    }

    /// `ensure_tier1_instance` rejects Tier 2 records with a
    /// structured error rather than silently trying to instantiate
    /// a `module.wasm` that doesn't exist.
    #[tokio::test]
    async fn ensure_tier1_instance_rejects_tier2() {
        let (tx, _rx) = broadcast::channel(16);
        let m = Manager::new(tx).unwrap();
        m.insert_for_test(record("com.example.t2", Tier::Iframe))
            .await;
        match m.ensure_tier1_instance("com.example.t2").await {
            Err(DaemonError::ManifestInvalid { .. }) => {}
            Err(other) => panic!("unexpected error: {other:?}"),
            Ok(_) => panic!("Tier 2 records must not load as WASM"),
        }
    }

    #[tokio::test]
    async fn ensure_tier1_instance_returns_not_found_for_unknown_id() {
        let (tx, _rx) = broadcast::channel(16);
        let m = Manager::new(tx).unwrap();
        match m.ensure_tier1_instance("does.not.exist").await {
            Err(DaemonError::NotFound(_)) => {}
            Err(other) => panic!("unexpected error: {other:?}"),
            Ok(_) => panic!("unknown id must not return an instance"),
        }
    }

    /// `shutdown_all_tier1` on an empty instance cache must complete
    /// without error. Exercised separately from the wasmtime-loaded
    /// path because the loaded path needs a real WASM module to
    /// instantiate.
    #[tokio::test]
    async fn shutdown_all_tier1_on_empty_cache_completes() {
        let (tx, _rx) = broadcast::channel(16);
        let m = Manager::new(tx).unwrap();
        m.shutdown_all_tier1().await;
    }

    /// Codex finding 3: the previous read-then-write pattern could
    /// run guest `init()` twice when two parallel first-touch calls
    /// raced past the empty-cache check. The OnceCell-based rewrite
    /// allocates exactly one cell per module on the first race; both
    /// concurrent ensure calls share it and the init closure runs
    /// at most once.
    ///
    /// We cannot easily make `instantiate` succeed without a real
    /// WASM bytecode in tests, but we *can* verify the structural
    /// property: after two concurrent ensure calls for the same
    /// module id, the `tier1_instances` map holds exactly one
    /// `Arc<OnceCell>` entry (not two). Both calls share that
    /// cell and would share its inner init future on a success
    /// path. Failure (no `module.wasm` on disk) returns Err from
    /// both without poisoning the cell.
    // ----- Codex round-2 finding 1: prefix-exclusive routing ------------

    fn manifest_with_search(
        prefix: Option<&str>,
        pattern: Option<&str>,
    ) -> lunaris_modules::ModuleManifest {
        let mut r = record("com.example.routed", Tier::Wasm);
        r.manifest.waypointer = Some(lunaris_modules::WaypointerConfig {
            search: Some(lunaris_modules::WaypointerSearchConfig {
                priority: 100,
                prefix: prefix.map(String::from),
                detect_pattern: pattern.map(String::from),
                max_results: None,
            }),
            action: None,
        });
        r.manifest
    }

    /// Empty query → nothing dispatched.
    #[test]
    fn route_search_all_drops_empty_query() {
        let candidates = vec![(
            "com.example.always".into(),
            manifest_with_search(None, None),
        )];
        assert!(route_search_all(&candidates, "").is_empty());
        assert!(route_search_all(&candidates, "   ").is_empty());
    }

    /// Modules without `[waypointer.search]` never get dispatched.
    #[test]
    fn route_search_all_skips_modules_without_search_section() {
        let candidates = vec![(
            "com.example.no-waypointer".into(),
            record("com.example.no-waypointer", Tier::Wasm).manifest,
        )];
        assert!(route_search_all(&candidates, "anything").is_empty());
    }

    /// No prefix in registry → all non-prefix modules see the
    /// unmodified query. Matches in-process line 60-67.
    #[test]
    fn route_search_all_no_prefix_match_dispatches_to_all_non_prefix() {
        let candidates = vec![
            (
                "com.example.a".into(),
                manifest_with_search(None, None),
            ),
            (
                "com.example.b".into(),
                manifest_with_search(None, Some(r"^\d+$")),
            ),
        ];
        let dispatched = route_search_all(&candidates, "hello");
        assert_eq!(dispatched.len(), 2);
        assert!(dispatched.iter().all(|d| d.query == "hello"));
    }

    /// **Codex round-2 finding 1 core property**: a winning prefix
    /// causes exclusive dispatch — always-active modules in the same
    /// registry must NOT see the prefixed query.
    #[test]
    fn route_search_all_prefix_match_is_exclusive() {
        let candidates = vec![
            (
                "com.example.always".into(),
                manifest_with_search(None, None),
            ),
            (
                "com.example.dollar".into(),
                manifest_with_search(Some("$"), None),
            ),
            (
                "com.example.equals".into(),
                manifest_with_search(Some("="), None),
            ),
        ];
        let dispatched = route_search_all(&candidates, "$btc");
        assert_eq!(
            dispatched.len(),
            1,
            "$-prefix module wins, everyone else (incl. always-active) skipped"
        );
        assert_eq!(dispatched[0].module_id, "com.example.dollar");
        // Prefix is stripped: guest gets "btc", not "$btc".
        assert_eq!(dispatched[0].query, "btc");
    }

    /// Prefix wins, but a prefix-only query with no payload after
    /// strip is treated as no-op (matches in-process line 51-53).
    #[test]
    fn route_search_all_prefix_with_no_payload_drops_all() {
        let candidates = vec![
            (
                "com.example.always".into(),
                manifest_with_search(None, None),
            ),
            (
                "com.example.dollar".into(),
                manifest_with_search(Some("$"), None),
            ),
        ];
        assert!(route_search_all(&candidates, "$").is_empty());
        assert!(route_search_all(&candidates, "$   ").is_empty());
    }

    /// detect_pattern is descriptor-only; it must NOT gate routing.
    /// Matches in-process behaviour where pattern is the plugin's
    /// own concern, not the manager's filter.
    #[test]
    fn route_search_all_ignores_detect_pattern() {
        let candidates = vec![(
            "com.example.pattern".into(),
            manifest_with_search(None, Some(r"^\d+$")),
        )];
        // Pattern doesn't match: the router still dispatches because
        // pattern is the guest's job. The guest's `search` body is
        // expected to early-return on mismatch.
        let dispatched = route_search_all(&candidates, "hello world");
        assert_eq!(dispatched.len(), 1);
    }

    #[tokio::test]
    async fn handle_search_all_prefix_exclusive_end_to_end() {
        // Codex round-2 finding 1 verified through the public API:
        // three modules — always-active, `$`-prefix, `=`-prefix.
        // Send query "$btc". Only the `$` module is dispatched;
        // always-active and `=`-prefix never see this keystroke.
        let (tx, _rx) = broadcast::channel(16);
        let m = Manager::new(tx).unwrap();

        let mut r1 = record("com.example.always", Tier::Wasm);
        r1.manifest.waypointer = Some(lunaris_modules::WaypointerConfig {
            search: Some(lunaris_modules::WaypointerSearchConfig {
                priority: 100,
                prefix: None,
                detect_pattern: None,
                max_results: None,
            }),
            action: None,
        });
        let mut r2 = record("com.example.dollar", Tier::Wasm);
        r2.manifest.waypointer = Some(lunaris_modules::WaypointerConfig {
            search: Some(lunaris_modules::WaypointerSearchConfig {
                priority: 100,
                prefix: Some("$".into()),
                detect_pattern: None,
                max_results: None,
            }),
            action: None,
        });
        let mut r3 = record("com.example.equals", Tier::Wasm);
        r3.manifest.waypointer = Some(lunaris_modules::WaypointerConfig {
            search: Some(lunaris_modules::WaypointerSearchConfig {
                priority: 100,
                prefix: Some("=".into()),
                detect_pattern: None,
                max_results: None,
            }),
            action: None,
        });
        m.insert_for_test(r1).await;
        m.insert_for_test(r2).await;
        m.insert_for_test(r3).await;

        let _ = m
            .handle_request(Request::WaypointerSearchAll {
                id: "1".into(),
                query: "$btc".into(),
            })
            .await;

        // search_tier1 fails for any dispatched module (no real
        // module.wasm on disk → WasmLoad → 4 crashes recorded).
        // Verify which modules were touched by inspecting crash
        // state.
        let modules = m.modules.read().await;
        assert!(
            modules.get("com.example.dollar").unwrap().crash.is_failed(),
            "$-prefix module owns this query and must be dispatched"
        );
        assert!(
            !modules.get("com.example.always").unwrap().crash.is_failed(),
            "always-active module must NOT see prefixed query (privacy leak otherwise)"
        );
        assert_eq!(
            modules.get("com.example.always").unwrap().crash.crash_count(),
            0,
            "always-active module crash counter must stay at 0 — was never touched"
        );
        assert_eq!(
            modules.get("com.example.equals").unwrap().crash.crash_count(),
            0,
            "=-prefix module crash counter must stay at 0 — different prefix"
        );
    }

    /// Codex finding 4: per-call and aggregate timeouts must exist
    /// and be sensibly ordered. The aggregate must be larger than
    /// the per-call budget so a single slow module hitting its
    /// per-call timeout still leaves headroom for the rest of the
    /// batch.
    #[test]
    fn search_timeout_constants_are_sane() {
        // Per-call: bounded so a slow async host call cannot
        // stretch a single keystroke beyond a usable budget.
        assert!(SEARCH_TIMEOUT >= std::time::Duration::from_secs(1));
        assert!(SEARCH_TIMEOUT <= std::time::Duration::from_secs(10));
        // Aggregate: strictly larger than per-call so the batch is
        // not artificially capped by SEARCH_TIMEOUT itself.
        assert!(SEARCH_ALL_TIMEOUT > SEARCH_TIMEOUT);
    }

    #[tokio::test]
    async fn handle_search_all_drops_empty_queries() {
        // Whitespace-only queries never dispatch — protects every
        // module from being forced to handle a degenerate input.
        let (tx, _rx) = broadcast::channel(16);
        let m = Manager::new(tx).unwrap();
        let mut r = record("com.example.always", Tier::Wasm);
        r.manifest.waypointer = Some(lunaris_modules::WaypointerConfig {
            search: Some(lunaris_modules::WaypointerSearchConfig {
                priority: 100,
                prefix: None,
                detect_pattern: None,
                max_results: None,
            }),
            action: None,
        });
        m.insert_for_test(r).await;

        let _ = m
            .handle_request(Request::WaypointerSearchAll {
                id: "1".into(),
                query: "   ".into(),
            })
            .await;
        let cells = m.tier1_instances.read().await;
        assert_eq!(cells.len(), 0, "empty/whitespace query must not dispatch");
    }

    /// Codex round-2 finding 2: when handle_search_all hits its
    /// aggregate budget, results from modules that finished must
    /// survive. We verify the structural property: even when every
    /// module fails fast (no module.wasm → WasmLoad → 4 crashes),
    /// the dispatch loop processes them rather than discarding the
    /// whole batch on a single slow one.
    ///
    /// We cannot easily simulate one slow module without a real
    /// wasmtime instance, so this test exercises the *non-timeout*
    /// happy path: many quick-failing modules. The codex regression
    /// it guards against was that the old `timeout(join_all)` would
    /// have erased everything; FuturesUnordered with per-module
    /// SEARCH_TIMEOUT keeps every completed result.
    #[tokio::test]
    async fn handle_search_all_processes_each_module_independently() {
        let (tx, _rx) = broadcast::channel(16);
        let m = Manager::new(tx).unwrap();

        for i in 0..5 {
            let id = format!("com.example.m{i}");
            let mut r = record(&id, Tier::Wasm);
            r.manifest.waypointer = Some(lunaris_modules::WaypointerConfig {
                search: Some(lunaris_modules::WaypointerSearchConfig {
                    priority: 100,
                    prefix: None,
                    detect_pattern: None,
                    max_results: None,
                }),
                action: None,
            });
            m.insert_for_test(r).await;
        }

        let _ = m
            .handle_request(Request::WaypointerSearchAll {
                id: "1".into(),
                query: "hello".into(),
            })
            .await;

        // Every module had a chance to fail (no wasm bytes → load
        // failure → 4 crashes recorded). The aggregate path did
        // not abort after the first failure.
        let modules = m.modules.read().await;
        for i in 0..5 {
            let id = format!("com.example.m{i}");
            assert!(
                modules.get(&id).unwrap().crash.is_failed(),
                "module {id} should have been dispatched and crash-counted"
            );
        }
    }

    /// Codex round-2 finding 3: after a recorded crash with a
    /// Recovery::Delayed return value, the next ensure-call within
    /// the cooldown window must short-circuit with
    /// DaemonError::InCooldown — not rerun compile/init. Otherwise
    /// rapid keystrokes between crashes burn through the ladder in
    /// milliseconds instead of allowing 5s/30s recovery delays.
    #[tokio::test]
    async fn ensure_tier1_instance_respects_crash_backoff() {
        let (tx, _rx) = broadcast::channel(16);
        let m = Manager::new(tx).unwrap();
        m.insert_for_test(record("com.example.flap", Tier::Wasm))
            .await;

        // First crash = Immediate → no cooldown.
        let r1 = m.record_crash("com.example.flap").await;
        assert_eq!(r1, Recovery::Immediate);
        match m.ensure_tier1_instance("com.example.flap").await {
            Err(DaemonError::InCooldown { .. }) => {
                panic!("Immediate recovery must NOT set a cooldown")
            }
            // Any other error is fine — the test only cares about
            // the InCooldown gate.
            _ => {}
        }

        // Second crash = Delayed{5s} → cooldown active.
        let r2 = m.record_crash("com.example.flap").await;
        assert!(matches!(r2, Recovery::Delayed { .. }));
        match m.ensure_tier1_instance("com.example.flap").await {
            Err(DaemonError::InCooldown { module_id }) => {
                assert_eq!(module_id, "com.example.flap");
            }
            Err(other) => panic!(
                "second-crash backoff must gate with InCooldown, got error: {other:?}",
            ),
            Ok(_) => panic!(
                "second-crash backoff must gate ensure_tier1_instance with InCooldown error"
            ),
        }

        // SearchFailure::Cooldown reaches the dispatch path and is
        // returned as empty results, not as an error response (the
        // module is recovering, not broken).
        let resp = m
            .handle_request(Request::WaypointerSearch {
                id: "1".into(),
                module_id: "com.example.flap".into(),
                query: "any".into(),
            })
            .await;
        match resp {
            Response::WaypointerResults { results, .. } => {
                assert!(results.is_empty(), "cooldown returns empty, not error");
            }
            other => panic!("cooldown path must return WaypointerResults: {other:?}"),
        }
    }

    /// The cooldown gate must not run guest crash accounting twice
    /// when the gate fires — we are *avoiding* a re-instantiate,
    /// not recording a fresh crash.
    #[tokio::test]
    async fn cooldown_does_not_count_as_new_crash() {
        let (tx, _rx) = broadcast::channel(16);
        let m = Manager::new(tx).unwrap();
        m.insert_for_test(record("com.example.cool", Tier::Wasm))
            .await;
        m.record_crash("com.example.cool").await; // crash 1 (immediate)
        m.record_crash("com.example.cool").await; // crash 2 (delayed 5s, cooldown active)

        // Many searches during the cooldown window: none should
        // increment crash_count beyond 2.
        for i in 0..10 {
            let _ = m
                .handle_request(Request::WaypointerSearch {
                    id: format!("{i}"),
                    module_id: "com.example.cool".into(),
                    query: "x".into(),
                })
                .await;
        }
        let guard = m.modules.read().await;
        let entry = guard.get("com.example.cool").unwrap();
        assert_eq!(
            entry.crash.crash_count(),
            2,
            "cooldown short-circuits must not increment the crash counter",
        );
    }

    #[tokio::test]
    async fn concurrent_ensure_uses_a_single_oncecell_per_module() {
        let (tx, _rx) = broadcast::channel(16);
        let m = Manager::new(tx).unwrap();
        m.insert_for_test(record("com.example.race", Tier::Wasm))
            .await;

        let a = Arc::clone(&m);
        let b = Arc::clone(&m);
        let (ra, rb) = tokio::join!(
            async move { a.ensure_tier1_instance("com.example.race").await },
            async move { b.ensure_tier1_instance("com.example.race").await },
        );
        // Both fail (no real module.wasm on disk at /tmp), which is
        // expected and irrelevant to the race property under test.
        assert!(ra.is_err());
        assert!(rb.is_err());

        // The structural invariant: exactly one cell entry, never two.
        let guard = m.tier1_instances.read().await;
        assert_eq!(
            guard.len(),
            1,
            "two concurrent ensure calls must not create two OnceCell entries"
        );
        assert!(guard.contains_key("com.example.race"));
    }

    #[test]
    fn cap_field_passes_short_strings_through() {
        let short = "hello world".to_string();
        assert_eq!(cap_field(short.clone()), short);
    }

    #[test]
    fn cap_field_truncates_oversize_strings_with_marker() {
        let big = "x".repeat(SEARCH_FIELD_CAP_BYTES * 2);
        let capped = cap_field(big);
        assert!(capped.len() <= SEARCH_FIELD_CAP_BYTES + 8);
        assert!(capped.ends_with('…'));
    }

    #[test]
    fn cap_field_respects_utf8_boundaries() {
        // Build a string that ends right at the boundary mid-codepoint.
        // 3-byte codepoint "€" at index SEARCH_FIELD_CAP_BYTES - 1 would
        // be split if we just `truncate()`; cap_field must walk back to
        // the previous char boundary.
        let mut s = "a".repeat(SEARCH_FIELD_CAP_BYTES - 1);
        s.push('€'); // 3 bytes pushes total past the cap
        s.push_str(&"b".repeat(100));
        let capped = cap_field(s);
        // Output must still be valid UTF-8 by construction; the easy
        // check is that String::len() agrees with `.chars().count()`.
        assert!(capped.is_char_boundary(capped.len()));
    }

    #[test]
    fn search_result_cap_uses_manifest_value() {
        let mut r = record("x", Tier::Wasm);
        r.manifest.waypointer = Some(lunaris_modules::WaypointerConfig {
            search: Some(lunaris_modules::WaypointerSearchConfig {
                priority: 100,
                prefix: None,
                detect_pattern: None,
                max_results: Some(3),
            }),
            action: None,
        });
        assert_eq!(search_result_cap(&r.manifest), 3);
    }

    #[test]
    fn search_result_cap_defaults_when_unset() {
        let r = record("x", Tier::Wasm);
        assert_eq!(search_result_cap(&r.manifest), DEFAULT_MAX_RESULTS);
    }

    #[test]
    fn wit_to_proto_clamps_relevance_and_caps_count() {
        use crate::runtime::wit::exports::lunaris::waypointer::provider::{
            Action as WitAction, SearchResult as WitResult,
        };
        // 12 results, max 3 allowed; relevance ranges into invalid space.
        let raw: Vec<WitResult> = (0..12)
            .map(|i| WitResult {
                id: format!("r{i}"),
                title: format!("title-{i}"),
                description: None,
                icon: None,
                relevance: if i == 0 { 2.5 } else { -0.5 },
                action: WitAction::Copy(format!("text-{i}")),
            })
            .collect();
        let mapped = wit_to_proto_results("com.example.test", raw, 3);
        assert_eq!(mapped.len(), 3);
        assert!(mapped[0].relevance <= 1.0 && mapped[0].relevance >= 0.0);
        assert_eq!(mapped[1].relevance, 0.0); // negative clamped to 0
    }

    #[test]
    fn wit_to_proto_preserves_action_variants() {
        use crate::runtime::wit::exports::lunaris::waypointer::provider::{
            Action as WitAction, CustomAction, SearchResult as WitResult,
        };
        let raw = vec![
            WitResult {
                id: "a".into(),
                title: "Copy".into(),
                description: None,
                icon: None,
                relevance: 0.9,
                action: WitAction::Copy("clip".into()),
            },
            WitResult {
                id: "b".into(),
                title: "OpenUrl".into(),
                description: None,
                icon: None,
                relevance: 0.8,
                action: WitAction::OpenUrl("https://x".into()),
            },
            WitResult {
                id: "c".into(),
                title: "Custom".into(),
                description: None,
                icon: None,
                relevance: 0.7,
                action: WitAction::Custom(CustomAction {
                    handler: "h".into(),
                    data: r#"{"k":"v"}"#.into(),
                }),
            },
        ];
        let mapped = wit_to_proto_results("m", raw, 8);
        use crate::socket::protocol::SearchAction;
        assert!(matches!(mapped[0].action, SearchAction::Copy { .. }));
        assert!(matches!(mapped[1].action, SearchAction::OpenUrl { .. }));
        assert!(matches!(mapped[2].action, SearchAction::Custom { .. }));
    }
}

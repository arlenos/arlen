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

use tokio::sync::{broadcast, RwLock};
use tracing::{debug, info, warn};

use crate::host::CapabilityContext;
use crate::manifest::{discover_all, ModuleRecord, Tier};
use crate::runtime::{tier1::Tier1Runtime, tier2::Tier2Broker, CrashState, Recovery};
use crate::socket::protocol::{
    ErrorCode, Event, ModuleSummary, ModuleTier, Request, Response, SearchResult,
};

/// One row in the manager's module table.
struct ModuleEntry {
    record: ModuleRecord,
    enabled: bool,
    crash: CrashState,
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

pub struct Manager {
    modules: RwLock<HashMap<String, ModuleEntry>>,
    tier1: Arc<Tier1Runtime>,
    tier2: Arc<Tier2Broker>,
    events_tx: broadcast::Sender<Event>,
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
        }))
    }

    /// Run discovery and populate the module table. Idempotent.
    pub async fn discover(&self) {
        let records = discover_all();
        info!("modulesd: discovered {} module(s)", records.len());
        let mut guard = self.modules.write().await;
        for record in records {
            let id = record.id().to_string();
            guard
                .entry(id)
                .or_insert_with(|| ModuleEntry {
                    record,
                    enabled: true,
                    crash: CrashState::new(),
                });
        }
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

            Request::WaypointerExecute { id, .. } => {
                // Tier 1 execute path is wired up in S5 alongside the
                // Currency-Konverter dogfood. Until then we ack; the
                // shell will fall back to its own action handler for
                // copy/open-url which do not need the module to run.
                Response::Acked { id }
            }

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
        // Real Tier 1 invocation lands in S4/S5 with the SDK macro
        // and the dogfooded Currency-Konverter. Until then we return
        // an empty result set with the query echoed in a debug field
        // visible during testing.
        let _ = (query, &self.tier1);
        Response::WaypointerResults {
            id: id.to_string(),
            module_id: module_id.to_string(),
            results: Vec::new(),
        }
    }

    async fn handle_search_all(&self, id: &str, query: &str) -> Response {
        let guard = self.modules.read().await;
        let mut all: Vec<SearchResult> = Vec::new();
        for entry in guard.values() {
            if !entry.enabled || entry.crash.is_failed() {
                continue;
            }
            if entry.record.tier != Tier::Wasm {
                continue;
            }
            let _ = query;
            // Same stub as handle_search; filled in S5.
        }
        all.sort_by(|a, b| {
            b.relevance
                .partial_cmp(&a.relevance)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        Response::WaypointerAggregate { id: id.to_string(), results: all }
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
            HostCall::NetworkFetch { url, headers: _ } => {
                match host::network::check_fetch(&instance.ctx, &url) {
                    Ok(()) => HostReply::NetworkBody {
                        // Empty body until the real HTTP client lands
                        // in S5; the policy layer is what S3 ships.
                        status: 200,
                        body_b64: String::new(),
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
        let recovery = entry.crash.record_crash(Instant::now());
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
    /// API surface.
    #[cfg(test)]
    pub async fn insert_for_test(&self, record: ModuleRecord) {
        self.modules.write().await.insert(
            record.id().to_string(),
            ModuleEntry {
                record,
                enabled: true,
                crash: CrashState::new(),
            },
        );
    }

    /// For tests: register a Tier 2 iframe directly without going
    /// through the mint flow.
    #[cfg(test)]
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
    async fn host_call_network_allowed_when_url_in_allowlist() {
        use crate::host::CapabilityContext;
        use crate::runtime::tier2::IframeInstance;
        use lunaris_modules::{ModuleCapabilities, NetworkCapability};

        let (tx, _rx) = broadcast::channel(16);
        let m = Manager::new(tx).unwrap();
        let mut caps = ModuleCapabilities::default();
        caps.network = Some(NetworkCapability {
            allowed_domains: vec!["api.example.com".into()],
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
                    url: "https://api.example.com/v1".into(),
                    headers: vec![],
                },
            })
            .await;
        match resp {
            Response::HostReply { reply, .. } => {
                use crate::socket::protocol::HostReply;
                assert!(matches!(reply, HostReply::NetworkBody { status: 200, .. }));
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
}

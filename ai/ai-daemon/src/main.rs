//! `arlen-ai-daemon` entry point.
//!
//! Wires the service core into a zbus session-bus connection and
//! exposes `org.arlen.AI1`. Design:
//!
//! * All outbound LLM traffic transits the proxy via
//!   [`ProxiedProvider`] (Foundation §8.4.6). The daemon never
//!   speaks HTTP directly.
//! * Results are not broadcast on the session bus. Callers poll
//!   `take_result(query_id, retrieval_token)` and the daemon checks
//!   both the caller's unique bus name and the per-query retrieval
//!   token before handing back the result text.
//! * The `Enabled` property is read-only over D-Bus. Toggling AI
//!   on/off happens through Settings writing the canonical TOML
//!   config, which the daemon's config watcher picks up.

use std::sync::Arc;

use arlen_ai_core::audit::{AuditSink, LedgerAuditSink};
use arlen_ai_core::capability::access_tier_from_level;
use arlen_ai_core::graph_query::QueryScope;
use arlen_ai_core::graph_schema::GraphSchema;
use arlen_ai_core::pipeline::{CypherPipeline, GraphQuerier, QueryRunner};
use arlen_ai_core::provider::AIProvider;
use arlen_ai_daemon::active_project::ActiveProject;
use arlen_ai_daemon::authz::AuthorizationStore;
use arlen_ai_daemon::config_watch;
use arlen_ai_daemon::graph_adapter::OsSdkGraphQuerier;
use arlen_ai_daemon::live_provider::LiveProvider;
use arlen_ai_daemon::mcp_discovery::McpDiscovery;
use arlen_ai_daemon::peer::{self, PeerError};
use arlen_ai_daemon::registry::{AuthError, CompletionOutcome};
use arlen_ai_daemon::selection::{ActiveSelection, ModelEntry, ModelKind};
use arlen_ai_daemon::service::{AiDaemonService, ExplainError, QueryError};
use arlen_ai_providers::proxied::{ProxiedConfig, ProxiedProvider};
use os_sdk::UnixEventConsumer;
use zbus::Connection;

const BUS_NAME: &str = "org.arlen.AI1";
const OBJECT_PATH: &str = "/org/arlen/AI1";

/// Step budget for the interactive tool-routing loop. Bounds how many
/// tool calls one query may chain before the loop must produce a final
/// answer (or report exhaustion). Mirrors the agent loop's bounded shape.
const TOOL_LOOP_MAX_STEPS: u32 = 6;

/// Resolve the Knowledge Daemon query socket the same way every
/// other Arlen client does: an explicit
/// `ARLEN_DAEMON_SOCKET` override wins; otherwise the per-user
/// runtime path `$XDG_RUNTIME_DIR/arlen/knowledge.sock` is used
/// when it exists (the daemon listens there in an unprivileged
/// session); the system path `/run/arlen/knowledge.sock` is the
/// final fallback.
fn resolve_knowledge_socket() -> String {
    if let Ok(explicit) = std::env::var("ARLEN_DAEMON_SOCKET") {
        if !explicit.is_empty() {
            return explicit;
        }
    }
    if let Ok(xdg) = std::env::var("XDG_RUNTIME_DIR") {
        if !xdg.is_empty() {
            let runtime = format!("{xdg}/arlen/knowledge.sock");
            if std::path::Path::new(&runtime).exists() {
                return runtime;
            }
        }
    }
    "/run/arlen/knowledge.sock".to_string()
}

/// Resolve the Event Bus consumer socket the same way: an explicit
/// `ARLEN_CONSUMER_SOCKET` override wins; otherwise the per-user
/// runtime path is used when it exists; the system path is the
/// final fallback.
fn resolve_event_consumer_socket() -> String {
    if let Ok(explicit) = std::env::var("ARLEN_CONSUMER_SOCKET") {
        if !explicit.is_empty() {
            return explicit;
        }
    }
    if let Ok(xdg) = std::env::var("XDG_RUNTIME_DIR") {
        if !xdg.is_empty() {
            let runtime = format!("{xdg}/arlen/event-bus-consumer.sock");
            if std::path::Path::new(&runtime).exists() {
                return runtime;
            }
        }
    }
    "/run/arlen/event-bus-consumer.sock".to_string()
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    // Read ai.toml once at startup. `provider` is applied here;
    // `enabled` is applied after the service is built and then kept
    // live by the config watcher (Phase 9-α S7).
    let settings = config_watch::load_ai_settings();
    tracing::info!(
        enabled = settings.enabled,
        provider = %settings.provider.name,
        model = %settings.provider.model,
        "loaded ai.toml"
    );

    // One session-bus connection for the daemon's whole lifetime: it
    // owns `org.arlen.AI1` and is also the connection the proxied
    // provider forwards on. The proxy authorises a forward by the
    // calling connection owning that name, so the provider and the
    // name must live on the same connection.
    let connection = zbus::Connection::session().await?;

    // Foundation §8.4.6: outbound LLM traffic goes through ai-proxy.
    // The provider config comes from `ai.toml` (name via `ai.provider`,
    // model/window/token via the optional `[provider]` section), so a
    // deployment points the daemon at any catalogued backend without a
    // rebuild. Read once at startup: changing the provider needs a restart.
    let provider_inner: Arc<dyn AIProvider> = Arc::new(
        ProxiedProvider::with_connection(
            ProxiedConfig {
                name: settings.provider.name.clone(),
                model: settings.provider.model.clone(),
                audit_token: settings.provider.audit_token.clone(),
                context_window: settings.provider.context_window,
            },
            &connection,
        )
        .await?,
    );
    // Wrap the startup provider in the live-swappable indirection: the pipeline,
    // the explain path, and the tool loop all route through this one handle, so
    // `ai_set_active` can swap the backend at runtime without a restart (the
    // consumers below were each handed an independent provider arc before). The
    // D-Bus interface keeps `live` to read (`ai_active`) and swap it.
    let live = Arc::new(LiveProvider::new(
        provider_inner,
        ActiveSelection::new(
            settings.provider.name.clone(),
            settings.provider.model.clone(),
        ),
    ));
    let provider: Arc<dyn AIProvider> = live.clone();

    // Graph queries run against the Knowledge Daemon. The pipeline
    // turns NL into a validated structured query, compiles Cypher,
    // executes it here, then formats the result back to NL.
    let knowledge_socket = resolve_knowledge_socket();
    tracing::info!(socket = %knowledge_socket, "knowledge daemon socket");
    // System Explanation Mode reads the same knowledge socket through
    // its own read-only seam, and shares the provider that the pipeline
    // forwards through. Clone both before they are moved into the
    // pipeline so the explainer can be wired onto the service below.
    let explain_reader: Arc<dyn arlen_ai_explanation::GraphReader> = Arc::new(
        arlen_ai_explanation::UnixGraphReader::new(knowledge_socket.clone()),
    );
    let explain_provider = provider.clone();
    // The anomaly source for the explanation: the Anomaly Detector's findings
    // file under the data dir (`$XDG_DATA_HOME` or `~/.local/share`). Resolved
    // here without a `dirs` dependency; a missing path or file just reads as no
    // anomalies (the reader is fail-soft), so the explanation degrades to
    // graph-only rather than failing.
    let explain_anomaly_reader: Option<Arc<dyn arlen_ai_explanation::AnomalyReader>> =
        std::env::var_os("XDG_DATA_HOME")
            .map(std::path::PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(|h| std::path::PathBuf::from(h).join(".local/share"))
            })
            .map(|base| {
                Arc::new(arlen_ai_explanation::FileAnomalyReader::new(
                    base.join("arlen/anomaly/alerts.json")
                        .to_string_lossy()
                        .into_owned(),
                )) as Arc<dyn arlen_ai_explanation::AnomalyReader>
            });
    // The tool-routing loop (default-off) drives the same provider as the
    // pipeline; clone before `provider` is moved into the pipeline.
    let tool_provider = provider.clone();
    let graph: Arc<dyn GraphQuerier> =
        Arc::new(OsSdkGraphQuerier::new(knowledge_socket));
    let runner: Arc<dyn QueryRunner> =
        Arc::new(CypherPipeline::new(provider, graph));

    // The audit sink submits to `arlen-auditd` over its ingest
    // socket. It is shared: the service gates every query on a
    // dispatch entry, and the MCP discovery layer audits tool calls
    // through the same ledger.
    let audit: Arc<dyn AuditSink> = Arc::new(LedgerAuditSink::at_default_socket());

    // The service is constructed fail-closed: disabled, with the Minimal
    // (no graph access) scope. The config watcher is the sole owner of
    // the admission state (enabled flag + read scope from `ai.toml`'s
    // `access_level`, 0..=4, Foundation §8.4); it publishes the
    // configured admission once its file watch is armed and keeps it live
    // on every change. Starting fail-closed means there is no window in
    // which a stale startup snapshot serves access before the watcher is
    // live. The Settings tier slider that writes `access_level` is S24.
    // MCP server discovery owns the daemon's `McpClient`. It is built
    // before the service so the tool-routing loop can share the same
    // client handle that discovery keeps connected.
    let discovery = Arc::new(McpDiscovery::new(audit.clone()));

    // The live active-project source (Focus Mode, over the Event Bus). It
    // anchors project-scoped reads to the active project (GAP-21); shared
    // between the service (reads it per query) and the bus listener spawned
    // below (updates it on focus.activated/deactivated).
    let active_project = ActiveProject::new();

    // Loaded skills, for routing a fitting query to the agent. Discovered from
    // the shared behaviour dirs and enabled from the same `ai.toml` the agent
    // reads (only an enabled skill matches), so the daemon routes exactly the
    // skills the agent would run. Loaded once at startup.
    let loaded_skills = arlen_ai_skills::loader::load(
        &arlen_ai_skills::loader::behaviour_sources(),
        &arlen_ai_skills::loader::enabled_from_ai_toml(&config_watch::load_ai_text()),
    )
    .loaded;

    let service = Arc::new(
        AiDaemonService::new(
            runner,
            QueryScope::for_tier(access_tier_from_level(0), &GraphSchema::knowledge_graph()),
            audit.clone(),
        )
        .with_explain(explain_provider, explain_reader)
        .with_explanation_anomalies(explain_anomaly_reader)
        .with_explanation_processes(Some(Arc::new(
            arlen_ai_explanation::ProcProcessReader::new(),
        )))
        .with_tool_routing(
            settings.tool_routing,
            discovery.client(),
            tool_provider,
            TOOL_LOOP_MAX_STEPS,
        )
        .with_screening(arlen_ai_core::screen::Screener::from_config(
            &config_watch::load_ai_text(),
        ))
        .with_active_project(active_project.clone())
        .with_skills(loaded_skills)
        .with_skill_router(std::sync::Arc::new(
            arlen_ai_daemon::skill_route::DbusSkillRouter::new(connection.clone()),
        )),
    );
    config_watch::spawn_config_watch(service.clone());

    // Auto-sweep terminal records once per minute. The handle is
    // kept alive for the daemon's lifetime; aborting it on shutdown
    // is fine because ctrl_c().await is the only exit path.
    let _sweep = service.spawn_sweep_task();

    // Per-session authorization for MCP action servers. Grants live
    // here only; nothing is persisted, and the store is dropped with
    // the process at session end.
    let authz = Arc::new(AuthorizationStore::new());

    let dbus = AiInterface {
        service: service.clone(),
        authz: authz.clone(),
        live: live.clone(),
    };

    // Register the interface, then claim the well-known name on the
    // same connection the provider forwards on. The interface is up
    // before the name is claimed so a client cannot reach the name
    // before the object is served.
    connection.object_server().at(OBJECT_PATH, dbus).await?;
    connection.request_name(BUS_NAME).await?;

    tracing::info!(bus = BUS_NAME, path = OBJECT_PATH, "arlen-ai-daemon serving");

    // Discover Tier-1 module MCP servers over the Event Bus `module.`
    // namespace. `run` subscribes (retrying if the bus is late),
    // reconciles against the sockets already on disk, and tracks
    // installs and removals for the rest of the session.
    let consumer = UnixEventConsumer::new(resolve_event_consumer_socket());
    tokio::spawn(discovery.run(consumer));

    // Track the active Focus-Mode project over the Event Bus `focus.`
    // namespace, so a project-scoped read anchors to it (its own consumer
    // connection: the bus enforces one filter per connection).
    let focus_consumer = UnixEventConsumer::new(resolve_event_consumer_socket());
    tokio::spawn(active_project.run(focus_consumer));

    tokio::signal::ctrl_c().await?;
    tracing::info!("arlen-ai-daemon shutting down");
    Ok(())
}

/// D-Bus surface (`org.arlen.AI1`).
struct AiInterface {
    service: Arc<AiDaemonService>,
    authz: Arc<AuthorizationStore>,
    /// The live-swappable provider handle, for the model picker (`ai_active`,
    /// and later `ai_set_active`). Shared with the pipeline/explain/tool-loop.
    live: Arc<LiveProvider>,
}

#[zbus::interface(name = "org.arlen.AI1")]
impl AiInterface {
    /// Submit a new query. Returns a `(query_id, retrieval_token)`
    /// pair as a JSON object. The caller must store both and present
    /// them on every follow-up method; the daemon also verifies the
    /// follow-up's D-Bus sender matches the submitter.
    #[zbus(name = "query")]
    async fn query(
        &self,
        prompt: &str,
        _context_hints: &str,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] connection: &Connection,
    ) -> zbus::fdo::Result<String> {
        // Resolve the caller's stable executable identity for
        // rate-limit accounting. Fails closed if the PID/exe cannot
        // be resolved.
        let caller = peer::resolve(&header, connection)
            .await
            .map_err(map_peer_error)?;
        match self.service.query(prompt.to_string(), caller).await {
            Ok(handle) => Ok(serde_json::json!({
                "query_id": handle.query_id,
                "retrieval_token": handle.retrieval_token,
            })
            .to_string()),
            Err(QueryError::Disabled) => Err(zbus::fdo::Error::AccessDenied(
                "ai layer is disabled".to_string(),
            )),
            Err(QueryError::TooManyInflight) => Err(zbus::fdo::Error::LimitsExceeded(
                "too many in-flight queries for this caller".to_string(),
            )),
            Err(QueryError::GlobalCapacityReached) => Err(zbus::fdo::Error::LimitsExceeded(
                "daemon at global query capacity".to_string(),
            )),
            Err(QueryError::PromptTooLarge(n)) => Err(zbus::fdo::Error::LimitsExceeded(
                format!("prompt too large: {n} bytes"),
            )),
            Err(QueryError::NoGraphAccess) => Err(zbus::fdo::Error::NotSupported(
                "ai layer has no graph access configured".to_string(),
            )),
            Err(QueryError::AuditUnavailable) => Err(zbus::fdo::Error::Failed(
                "audit log unavailable; query refused".to_string(),
            )),
        }
    }

    /// The current live provider+model selection as a JSON object
    /// `{ "provider", "model" }` (the model picker's `ai_active`). Read from the
    /// daemon's live `LiveProvider`, never from `ai.toml` (Settings owns the
    /// file; an in-chat `ai_set_active` overrides it for the session). Read-only,
    /// no auth: it discloses only which catalogued backend the daemon routes to,
    /// not any user data.
    #[zbus(name = "ai_active")]
    async fn ai_active(&self) -> String {
        serde_json::to_string(&self.live.active()).unwrap_or_else(|_| "{}".to_string())
    }

    /// Cumulative token usage since daemon start, as JSON
    /// `{ inputTokens, outputTokens, totalTokens }`, for the harness
    /// transparency "Cost" feed (harness-redesign emit seam 5). Daemon-lifetime
    /// totals across every completion routed through the live provider (the chat
    /// runner and the tool loop), surviving a provider swap so the figure is the
    /// user's session spend, not per-backend. A provider that reports no usage
    /// (a local Ollama without token counts) contributes nothing, so the figure
    /// is honest rather than fabricated. Read-only; discloses only counts, no
    /// content. The harness renders it; it must not read the daemon's internals.
    #[zbus(name = "ai_usage")]
    async fn ai_usage(&self) -> String {
        let (input, output) = self.live.usage();
        serde_json::json!({
            "inputTokens": input,
            "outputTokens": output,
            "totalTokens": input + output,
        })
        .to_string()
    }

    /// The model catalog as a JSON array of
    /// `{ provider, model, contextWindow, kind, available }` (the picker's
    /// `ai_models_list`). Today the daemon catalogues one local provider (its
    /// live selection) with full metadata - the model + context window it is
    /// configured for, and a live availability probe (through the proxy, which
    /// owns egress unchanged). Structured for 1..n: the full multi-provider
    /// enumeration (every allowed provider's models) needs the proxy to expose
    /// per-provider catalog metadata + Ollama `/api/tags`, a cross-component
    /// follow-up; the proxy's `list_allowed_providers` gives only names today, so
    /// listing other providers here would be metadata-less placeholders. `kind`
    /// is `local` for the local Ollama provider; cloud providers report `cloud`
    /// once the proxy surfaces per-provider kind (Phase 9-β/γ).
    #[zbus(name = "ai_models_list")]
    async fn ai_models_list(&self) -> String {
        let active = self.live.active();
        let entry = ModelEntry {
            provider: active.provider,
            model: active.model,
            context_window: self.live.context_window(),
            kind: ModelKind::Local,
            available: self.live.available().await,
        };
        serde_json::to_string(&[entry]).unwrap_or_else(|_| "[]".to_string())
    }

    /// Live-swap the active provider+model (the picker's `ai_set_active`), no
    /// restart. Returns the new selection as `{ "provider", "model" }`. The
    /// `provider` is validated against the proxy's authoritative allowlist
    /// (`list_allowed_providers`); the `model` is accepted and validated by the
    /// backend at forward time (per-provider model enumeration is the
    /// cross-component follow-up, so the daemon cannot pre-check it). A fresh
    /// `ProxiedProvider` for the pair is built on the daemon's own connection
    /// (the proxy still authorises the egress by that connection owning
    /// `org.arlen.AI1`, unchanged) and swapped into the shared `LiveProvider`, so
    /// the pipeline, explain path, and tool loop all route to it at once.
    ///
    /// Fail-closed: if the proxy cannot be reached to validate, or the provider
    /// is not allowlisted, or building the provider fails, the swap is refused
    /// and the previous selection stays live. The new model's context window is
    /// the configured default (`ai.toml`); a real per-model window needs the
    /// catalog metadata the enumeration follow-up brings.
    #[zbus(name = "ai_set_active")]
    async fn ai_set_active(
        &self,
        provider: &str,
        model: &str,
        #[zbus(connection)] connection: &Connection,
    ) -> zbus::fdo::Result<String> {
        if provider.is_empty() || model.is_empty() {
            return Err(zbus::fdo::Error::InvalidArgs(
                "provider and model must be non-empty".to_string(),
            ));
        }
        // The proxy's allowlist is the authoritative provider catalog. Fail
        // closed if it cannot be reached.
        let proxy = zbus::Proxy::new(
            connection,
            "org.arlen.AIProxy1",
            "/org/arlen/AIProxy1",
            "org.arlen.AIProxy1",
        )
        .await
        .map_err(|e| zbus::fdo::Error::Failed(format!("proxy unreachable: {e}")))?;
        let allowed: Vec<String> = proxy
            .call("list_allowed_providers", &())
            .await
            .map_err(|e| zbus::fdo::Error::Failed(format!("could not list providers: {e}")))?;
        if !allowed.iter().any(|p| p == provider) {
            return Err(zbus::fdo::Error::InvalidArgs(format!(
                "provider '{provider}' is not in the proxy allowlist"
            )));
        }
        // The audit token + default context window come from ai.toml (read live;
        // Settings owns the file). The proxy records the token; the window is a
        // safe default until per-model catalog metadata lands.
        let settings = config_watch::load_ai_settings();
        let new_provider = ProxiedProvider::with_connection(
            ProxiedConfig {
                name: provider.to_string(),
                model: model.to_string(),
                audit_token: settings.provider.audit_token.clone(),
                context_window: settings.provider.context_window,
            },
            connection,
        )
        .await
        .map_err(|e| zbus::fdo::Error::Failed(format!("could not build provider: {e}")))?;
        self.live.swap(
            Arc::new(new_provider),
            ActiveSelection::new(provider, model),
        );
        tracing::info!(provider, model, "live-switched the active provider/model");
        serde_json::to_string(&self.live.active())
            .map_err(|e| zbus::fdo::Error::Failed(format!("serialize active: {e}")))
    }

    /// The Settings AI-providers MANAGER list - the management surface, distinct
    /// from the in-chat `ai_models_list` picker. A JSON array of
    /// `{ id, name, kind, enabled, configured, status }` (camelCase): `id/name/
    /// kind/configured` come from the proxy's authoritative catalog view
    /// (`list_providers` - display metadata only, no endpoint or credential);
    /// `enabled` is the per-provider on/off (every provider is enabled until
    /// `ai_provider_set_enabled` turns one off - the disabled set lands with that
    /// setter) and `status` is the last connection-test verdict (`untested` until
    /// `ai_provider_test` runs). Fail-closed to `[]` if the proxy is unreachable,
    /// so the manager shows nothing rather than stale data.
    #[zbus(name = "ai_providers_list")]
    async fn ai_providers_list(&self, #[zbus(connection)] connection: &Connection) -> String {
        let empty = || "[]".to_string();
        let Ok(proxy) = zbus::Proxy::new(
            connection,
            "org.arlen.AIProxy1",
            "/org/arlen/AIProxy1",
            "org.arlen.AIProxy1",
        )
        .await
        else {
            return empty();
        };
        let json: String = match proxy.call("list_providers", &()).await {
            Ok(j) => j,
            Err(_) => return empty(),
        };
        let Ok(mut views) = serde_json::from_str::<Vec<serde_json::Value>>(&json) else {
            return empty();
        };
        // Augment each catalog view with the manage-state the proxy does not own:
        // `enabled` (from the daemon-owned disabled set; absent = enabled) and
        // `status` (untested until a connection test runs).
        let disabled = arlen_ai_daemon::provider_state::state_path()
            .map(|p| arlen_ai_daemon::provider_state::load_disabled(&p))
            .unwrap_or_default();
        for view in &mut views {
            if let Some(obj) = view.as_object_mut() {
                let id = obj.get("id").and_then(|v| v.as_str()).unwrap_or("");
                obj.insert(
                    "enabled".to_string(),
                    serde_json::Value::Bool(!disabled.contains(id)),
                );
                obj.insert(
                    "status".to_string(),
                    serde_json::Value::String("untested".to_string()),
                );
            }
        }
        serde_json::to_string(&views).unwrap_or_else(|_| empty())
    }

    /// Enable or disable a catalogued provider in the AI-providers manager. A
    /// disabled provider is configured-but-dormant: it stays catalogued (and
    /// `ai_providers_list` still lists it, `enabled: false`) but is not offered
    /// for selection. Persisted in the daemon-owned disabled set (not `ai.toml`,
    /// which Settings owns for the default/ranking), so there is no co-ownership.
    /// Returns `ok`, or `error: <reason>` on a persistence failure. Idempotent.
    #[zbus(name = "ai_provider_set_enabled")]
    async fn ai_provider_set_enabled(&self, id: &str, enabled: bool) -> String {
        if id.is_empty() {
            return "error: provider id must be non-empty".to_string();
        }
        let Some(path) = arlen_ai_daemon::provider_state::state_path() else {
            return "error: no state directory (set XDG_STATE_HOME or HOME)".to_string();
        };
        match arlen_ai_daemon::provider_state::set_enabled(&path, id, enabled) {
            Ok(()) => "ok".to_string(),
            Err(e) => format!("error: {e}"),
        }
    }

    /// The configured default provider/model for the manager's Default-Models
    /// page (`ai_defaults_get`), as JSON `{ provider, model, ranking }`. Distinct
    /// from `ai_active` (the live in-session selection, which a `ai_set_active`
    /// may have overridden): this is the persisted default read from `ai.toml`
    /// (`ai.provider` + `[provider] model`), the value the daemon starts on.
    /// `ranking` is the priority-ordered fallback list - empty until the
    /// Settings-written ranking schema lands (Settings owns the `ai.toml` write;
    /// the daemon reads it). Read live so a Settings edit shows without a restart.
    #[zbus(name = "ai_defaults_get")]
    async fn ai_defaults_get(&self) -> String {
        let settings = config_watch::load_ai_settings();
        serde_json::json!({
            "provider": settings.provider.name,
            "model": settings.provider.model,
            "ranking": Vec::<String>::new(),
        })
        .to_string()
    }

    /// Test a catalogued provider's connectivity for the AI-providers manager.
    /// Relays the proxy's `test_provider`, which GETs the provider's catalogued
    /// model-list endpoint (the URL is proxy-owned, never the caller's, so no
    /// egress-consent step), and returns the verdict as JSON
    /// `{ ok, httpStatus?, network? }`. A policy refusal or an unreachable proxy
    /// maps to `{ ok: false, network: <reason> }` so the manager always gets the
    /// uniform shape. The proxy mints the actual egress + audits it; the daemon
    /// only forwards the call with its configured audit token.
    #[zbus(name = "ai_provider_test")]
    async fn ai_provider_test(
        &self,
        id: &str,
        #[zbus(connection)] connection: &Connection,
    ) -> String {
        let network = |reason: String| {
            serde_json::json!({ "ok": false, "network": reason }).to_string()
        };
        if id.is_empty() {
            return network("provider id must be non-empty".to_string());
        }
        let proxy = match zbus::Proxy::new(
            connection,
            "org.arlen.AIProxy1",
            "/org/arlen/AIProxy1",
            "org.arlen.AIProxy1",
        )
        .await
        {
            Ok(p) => p,
            Err(e) => return network(format!("proxy unreachable: {e}")),
        };
        // The proxy records the audit token; the actual attribution is the
        // daemon's kernel-attested peer credentials. Read the token live from
        // ai.toml (Settings owns the file).
        let token = config_watch::load_ai_settings().provider.audit_token;
        match proxy
            .call::<_, _, String>("test_provider", &(id, token.as_str()))
            .await
        {
            Ok(json) => json,
            Err(zbus::Error::MethodError(_, detail, _)) => {
                network(detail.unwrap_or_else(|| "test failed".to_string()))
            }
            Err(e) => network(format!("test failed: {e}")),
        }
    }

    /// Run System Explanation Mode (Foundation §5.8): return a
    /// plain-language summary of what the computer is doing right now.
    /// The summary is returned directly to the caller (not broadcast),
    /// so unlike `query` there is no id/token or poll cycle. Gated like a
    /// query (enabled + graph access), audited, and bounded by the same
    /// in-flight caps keyed on the caller's executable identity.
    #[zbus(name = "explain_system")]
    async fn explain_system(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] connection: &Connection,
    ) -> zbus::fdo::Result<String> {
        let caller = peer::resolve(&header, connection)
            .await
            .map_err(map_peer_error)?;
        // Unix seconds; the snapshot uses it for its recency window. A
        // pre-epoch clock floors at 0 (the source treats that as "match
        // everything", never a panic).
        let now_unix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        self.service
            .explain_system(caller, now_unix)
            .await
            .map_err(map_explain_error)
    }

    /// Poll a query for completion. Returns a JSON envelope of the
    /// form `{ "status": "...", ... }`. Result text is only included
    /// for the single-shot `completed` status; subsequent polls
    /// return `drained`.
    #[zbus(name = "take_result")]
    async fn take_result(
        &self,
        query_id: &str,
        retrieval_token: &str,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> zbus::fdo::Result<String> {
        let caller = sender(&header)?;
        let outcome = self
            .service
            .take_result(query_id, &caller, retrieval_token)
            .await
            .map_err(map_auth_error)?;
        Ok(serialise_outcome(outcome))
    }

    /// Retrieve the tool-call transcript for a completed tool-routing query.
    ///
    /// Returns a JSON array of step objects of the form
    /// `[{"server":"...", "tool":"...", "arguments":"...", "result":"..."}, ...]`.
    /// The array is empty for queries that ran through the single-shot runner
    /// path (no tool loop) or that have not yet produced any completed steps.
    ///
    /// Authorisation is identical to `take_result`: only the connection that
    /// submitted the query, presenting the same retrieval token, may call this.
    /// Unlike `take_result` the trace is not consumed: the caller may fetch it
    /// more than once while the query record exists.
    #[zbus(name = "take_trace")]
    async fn take_trace(
        &self,
        query_id: &str,
        retrieval_token: &str,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> zbus::fdo::Result<String> {
        let caller = sender(&header)?;
        let trace = self
            .service
            .take_trace(query_id, &caller, retrieval_token)
            .await
            .map_err(map_auth_error)?;
        let steps: Vec<serde_json::Value> = trace
            .into_iter()
            .map(|s| {
                serde_json::json!({
                    "server": s.server,
                    "tool": s.tool,
                    "arguments": s.arguments,
                    "result": s.result,
                    "status": s.status.as_str(),
                })
            })
            .collect();
        Ok(serde_json::Value::Array(steps).to_string())
    }

    /// Cancel an in-flight query. Returns true if the query existed,
    /// was not already terminated, and the caller passed authz.
    #[zbus(name = "cancel")]
    async fn cancel(
        &self,
        query_id: &str,
        retrieval_token: &str,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> zbus::fdo::Result<bool> {
        let caller = sender(&header)?;
        self.service
            .cancel(query_id, &caller, retrieval_token)
            .await
            .map_err(map_auth_error)
    }

    /// Whether the daemon is currently accepting new queries. Read
    /// only; writers must update the canonical TOML config and the
    /// daemon picks it up through its config watcher (S7).
    #[zbus(property)]
    fn enabled(&self) -> bool {
        self.service.is_enabled()
    }

    /// Answer an open authorization prompt.
    ///
    /// Only the desktop shell may call this: the `AuthorizationPrompt`
    /// signal that carries a prompt id is a session-bus broadcast, so
    /// without a caller check any peer that observed the id could
    /// approve a scope itself. The caller's executable is resolved
    /// and checked against the trusted shell binary before the
    /// decision is recorded.
    ///
    /// Returns `true` if a matching pending prompt existed.
    async fn respond_authorization(
        &self,
        prompt_id: &str,
        granted: bool,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] connection: &Connection,
    ) -> zbus::fdo::Result<bool> {
        let caller = peer::resolve(&header, connection)
            .await
            .map_err(map_peer_error)?;
        if !is_trusted_shell(&caller.stable_id) {
            return Err(zbus::fdo::Error::AccessDenied(
                "only the desktop shell may answer authorization prompts"
                    .to_string(),
            ));
        }
        match uuid::Uuid::parse_str(prompt_id) {
            Ok(id) => Ok(self.authz.resolve(id, granted).await),
            Err(_) => Ok(false),
        }
    }

    /// Emitted when a scope needs the user's authorization. The
    /// payload is only a prompt id and a scope label, never query
    /// content. The prompt id is not a bearer token: a response is
    /// authorised by the caller's identity in `respond_authorization`,
    /// not by knowing the id. Phase 9-δ's tool dispatch emits this
    /// once it can request authorization for a real tool call.
    #[zbus(signal)]
    async fn authorization_prompt(
        emitter: &zbus::object_server::SignalEmitter<'_>,
        prompt_id: &str,
        scope: &str,
    ) -> zbus::Result<()>;
}

/// Canonical install paths of the desktop shell binary. Only a
/// process running one of these may answer authorization prompts.
const TRUSTED_SHELL_BINS: &[&str] = &[
    "/usr/bin/arlen-desktop-shell",
    "/usr/lib/arlen/libexec/arlen-desktop-shell",
];

/// Whether `exe_path` is the trusted desktop shell.
///
/// In debug builds a `ARLEN_AI_TRUSTED_SHELL_BIN` env var adds a
/// dev path (the repo-relative `cargo tauri dev` binary). The
/// override is compiled out of release builds so it cannot become
/// part of the production trust boundary.
fn is_trusted_shell(exe_path: &str) -> bool {
    if TRUSTED_SHELL_BINS.contains(&exe_path) {
        return true;
    }
    #[cfg(debug_assertions)]
    if let Ok(dev) = std::env::var("ARLEN_AI_TRUSTED_SHELL_BIN") {
        if !dev.is_empty() && dev == exe_path {
            return true;
        }
    }
    false
}

fn sender(header: &zbus::message::Header<'_>) -> zbus::fdo::Result<String> {
    header
        .sender()
        .map(|s| s.to_string())
        .ok_or_else(|| zbus::fdo::Error::AccessDenied("message has no sender".to_string()))
}

fn map_peer_error(err: PeerError) -> zbus::fdo::Error {
    match err {
        PeerError::NoSender => {
            zbus::fdo::Error::AccessDenied("message has no sender".to_string())
        }
        PeerError::PidLookup(detail) => {
            zbus::fdo::Error::AccessDenied(format!("caller PID lookup failed: {detail}"))
        }
        PeerError::ExeLookup { pid, error } => zbus::fdo::Error::AccessDenied(format!(
            "caller exe lookup failed for pid {pid}: {error}"
        )),
    }
}

fn map_explain_error(err: ExplainError) -> zbus::fdo::Error {
    match err {
        ExplainError::Disabled => {
            zbus::fdo::Error::AccessDenied("ai layer is disabled".to_string())
        }
        ExplainError::NoGraphAccess => zbus::fdo::Error::NotSupported(
            "ai layer has no graph access configured".to_string(),
        ),
        ExplainError::InsufficientScope => zbus::fdo::Error::NotSupported(
            "read tier does not permit system explanation".to_string(),
        ),
        ExplainError::NotConfigured => {
            zbus::fdo::Error::NotSupported("system explanation is not configured".to_string())
        }
        ExplainError::TooManyInflight => zbus::fdo::Error::LimitsExceeded(
            "too many in-flight requests for this caller".to_string(),
        ),
        ExplainError::GlobalCapacityReached => {
            zbus::fdo::Error::LimitsExceeded("daemon at global capacity".to_string())
        }
        ExplainError::AuditUnavailable => zbus::fdo::Error::Failed(
            "audit log unavailable; explanation refused".to_string(),
        ),
        ExplainError::Failed(detail) => {
            zbus::fdo::Error::Failed(format!("explanation failed: {detail}"))
        }
    }
}

fn map_auth_error(err: AuthError) -> zbus::fdo::Error {
    match err {
        AuthError::UnknownQuery => zbus::fdo::Error::InvalidArgs("unknown query".to_string()),
        AuthError::CallerMismatch => {
            zbus::fdo::Error::AccessDenied("caller does not match submitter".to_string())
        }
        AuthError::TokenMismatch => {
            zbus::fdo::Error::AccessDenied("retrieval token mismatch".to_string())
        }
    }
}

fn serialise_outcome(outcome: CompletionOutcome) -> String {
    let value = match outcome {
        CompletionOutcome::Pending => serde_json::json!({ "status": "pending" }),
        CompletionOutcome::InProgress => serde_json::json!({ "status": "in-progress" }),
        CompletionOutcome::Completed { result } => {
            serde_json::json!({ "status": "completed", "result": result })
        }
        CompletionOutcome::Drained => serde_json::json!({ "status": "drained" }),
        CompletionOutcome::Failed { code, reason } => serde_json::json!({
            "status": "failed",
            "code": code,
            "reason": reason,
        }),
        CompletionOutcome::Cancelled => serde_json::json!({ "status": "cancelled" }),
    };
    value.to_string()
}

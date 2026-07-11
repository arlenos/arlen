//! The Arlen AI engine daemon binary (`pi-agent-adoption.md` Phase 1).
//!
//! Binds the contract Unix socket (0600), and for each connection authenticates
//! the peer with SO_PEERCRED via `ConnectionAuth` (cross-uid rejected), then
//! serves the five-verb contract through the session-bound dispatcher with the
//! attested pid. Built BESIDE the existing ai-daemon/ai-agent; nothing here
//! touches them. The gate/executor/reporter seams are wired to the real Rust:
//! the gate is `CapabilityGate` (`Capability::decide`), the reporter is
//! `ScreeningReporter` (content-free audit + S17/S18 screening), and the
//! executor is a `ProxyExecutor` routing graph.read/graph.write. graph.read runs
//! over the live `CypherPipeline` when AI is enabled and a provider is configured
//! (else the fail-closed `DeniedRunner`); graph.write over the live
//! `UnixRelationWriter` when `[agent] executor_live` is on (else `DeniedWriter`).
//! The screener is `Off` until a classifier model is provisioned - the gate's
//! confirm-on-external-trigger is the containment meanwhile.

use arlen_ai_core::screen::Screener;
use arlen_ai_engine_daemon::capability_map::CapabilityGate;
use arlen_ai_engine_daemon::compensation::CompensationStore;
use arlen_ai_engine_daemon::consent_client::ConsentBrokerClient;
use arlen_ai_engine_daemon::dispatch::Dispatcher;
use arlen_ai_engine_daemon::dispatch::Executor;
use arlen_ai_engine_daemon::file_executor::FileSystemExecutor;
use arlen_ai_engine_daemon::settings_executor::SettingsExecutor;
use arlen_ai_engine_daemon::proxy_executor::ProxyExecutor;
use arlen_ai_engine_daemon::read_executor::{DeniedRunner, GraphReadExecutor};
use arlen_ai_core::pipeline::{CypherPipeline, GraphQuerier, QueryRunner};
use arlen_ai_core::provider::AIProvider;
use arlen_ai_engine_daemon::graph_adapter::OsSdkGraphQuerier;
use arlen_ai_core::proxied::{ProxiedConfig, ProxiedProvider};
use arlen_ai_engine_daemon::engine_config;
use arlen_ai_engine_daemon::reporter::ScreeningReporter;
use arlen_ai_engine_daemon::curation::GraphProjectReader;
use arlen_ai_engine_daemon::curator::CuratorHandler;
use arlen_ai_engine_daemon::orchestrator;
use arlen_ai_engine_daemon::explain_iface;
use arlen_ai_engine_daemon::pi_run::SessionBinder;
use arlen_ai_engine_daemon::sidecar::{PiSidecar, SidecarPaths};
use arlen_ai_engine_daemon::supervisor::supervise;
use arlen_ai_engine_daemon::write_executor::{
    DeniedWriter, GraphWriteExecutor, RelationWriter, UnixRelationWriter,
};
use arlen_ai_engine_daemon::wire::serve_connection;
use ai_engine_contract::{CapabilityContext, ReadTier, SessionInit};
use arlen_permissions::connection_auth::ConnectionAuth;
use audit_proto::sink::{AuditSink, LedgerAuditSink};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::net::UnixListener;
use tracing::{error, info, warn};

/// The contract socket path: `$XDG_RUNTIME_DIR/arlen/ai-engine.sock`, falling
/// back to `/run/user/<uid>/arlen/...` when the env var is unset.
fn socket_path() -> PathBuf {
    let base = std::env::var("XDG_RUNTIME_DIR")
        .unwrap_or_else(|_| format!("/run/user/{}", current_uid()));
    PathBuf::from(base).join("arlen").join("ai-engine.sock")
}

/// The consent-broker INTAKE socket: `$XDG_RUNTIME_DIR/arlen/consent-intake.sock`,
/// else `/run/arlen/consent-intake.sock` (mirrors the broker's own resolution).
/// The [`ConsentBrokerClient`] connects here to drive a gate `Confirm`; an
/// unreachable broker fails closed to a denial, so this is safe even when the
/// broker is not running.
fn consent_intake_socket() -> PathBuf {
    match std::env::var_os("XDG_RUNTIME_DIR") {
        Some(dir) => PathBuf::from(dir).join("arlen").join("consent-intake.sock"),
        None => PathBuf::from("/run/arlen/consent-intake.sock"),
    }
}

/// The Phase-2-A drive socket: `$XDG_RUNTIME_DIR/arlen/ai-engine-drive.sock`
/// (else `/run/user/<uid>/arlen/...`), where the harness shell connects to drive
/// the pi RPC session. The supervisor relays one shell connection here against
/// each pi instance's stdio.
fn drive_socket_path() -> PathBuf {
    let base =
        std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| format!("/run/user/{}", current_uid()));
    PathBuf::from(base).join("arlen").join("ai-engine-drive.sock")
}

/// Bind the drive socket at 0600 (replacing a stale file). The 0600 perms are
/// the same-uid boundary: only the owning user can connect, so the drive channel
/// carries prompts to the user's OWN pi.
fn bind_drive_socket() -> std::io::Result<UnixListener> {
    let path = drive_socket_path();
    // Tighten the runtime dir to 0700 BEFORE binding so the drive socket's
    // same-uid boundary does not rest on the parent dir's default creation mode
    // (the socket's own bind-then-chmod window is then behind a 0700 dir a
    // non-owner cannot even traverse).
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))?;
    }
    // Only clear a STALE socket; refuse to hijack one a live daemon still serves
    // (a successful connect means another instance owns the drive endpoint).
    if path.exists() {
        match std::os::unix::net::UnixStream::connect(&path) {
            Ok(_) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::AddrInUse,
                    "drive socket already served by a live daemon",
                ))
            }
            Err(_) => {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
    let listener = UnixListener::bind(&path)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}

/// The uid the daemon runs as; cross-uid IPC is rejected by `ConnectionAuth`.
fn current_uid() -> u32 {
    // SAFETY: getuid is always safe; it reads the real uid and never fails.
    unsafe { libc::getuid() }
}

/// The session grant for the daemon-spawned engine: the safe default. No tools
/// granted (so the gate denies every Authorize) and the narrowest read tier, so
/// a freshly-supervised engine is inert until a real per-session grant policy
/// lands. The supervise loop binds this to the sandboxed engine's pid.
fn default_engine_session() -> SessionInit {
    SessionInit {
        system_prompt: String::new(),
        behaviour: None,
        capability_context: CapabilityContext { generic_tools: vec![], proxy_tools: vec![] },
        project_anchor: None,
        read_tier: ReadTier::Minimal,
        // The trustworthy session-origin bit. It is false here because the daemon
        // spawns ONE inert supervisor session with no external trigger, so false is
        // correct today. HARD PRE-FLIP GATE (review HIGH-1): before ANY external-
        // trigger source (an event-bus-triggered agent run) drives a session under
        // executor_live, the supervisor MUST derive this bit from the run origin.
        // The gate ORs it with the engine's untrusted per-call flag escalate-only,
        // so a run the daemon KNOWS is externally-originated escalates every action
        // to a confirmation - but only if this bit is set. Leaving it hardwired
        // false while wiring an external trigger would reduce external-content
        // containment to "the engine self-reports honestly", which a prompt-injected
        // engine will not. So: derive-before-external-trigger, not optional.
        externally_triggered: false,
    }
}

/// The Knowledge Daemon socket the read pipeline queries: the `ARLEN_DAEMON_SOCKET`
/// override, else the XDG runtime path if it exists, else the system path (mirrors
/// the ai-daemon resolver so both read the same socket).
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

/// Build the `graph.read` runner. Live ONLY when AI is enabled AND a provider is
/// configured AND the session bus + proxy client come up; any of those missing
/// falls back to the fail-closed [`DeniedRunner`] (the read then reports
/// provider-unavailable rather than reading). The live runner is the ai-core
/// [`CypherPipeline`]: it turns the caller's NL query into validated Cypher via
/// the `ProxiedProvider` (forwarded through ai-proxy, which peer-auths this
/// daemon's binary as `org.arlen.AI1`) and runs it against the Knowledge Daemon,
/// so the read is bounded by the scope the gate already resolved. Only reachable
/// when pi is running (AI enabled) and a `graph.read` Execute presents a valid
/// HIGH-1 proof, so wiring it live carries no autonomy of its own.
async fn build_read_runner(connection: Option<&zbus::Connection>) -> Arc<dyn QueryRunner> {
    if !engine_config::ai_enabled() {
        return Arc::new(DeniedRunner);
    }
    let settings = engine_config::provider_settings();
    if settings.name.is_empty() {
        tracing::warn!("no ai.provider configured; graph.read stays fail-closed");
        return Arc::new(DeniedRunner);
    }
    // The ProxiedProvider must forward on the connection that OWNS `org.arlen.AI1`:
    // the ai-proxy authorizes an LLM forward by the owned name (planner ruling, pi
    // as the drop-in ai-daemon). Without that connection, graph.read fails closed.
    let Some(connection) = connection else {
        tracing::warn!("no org.arlen.AI1 connection; graph.read stays fail-closed");
        return Arc::new(DeniedRunner);
    };
    let provider: Arc<dyn AIProvider> = match ProxiedProvider::with_connection(
        ProxiedConfig {
            name: settings.name,
            model: settings.model,
            audit_token: settings.audit_token,
            context_window: settings.context_window,
        },
        connection,
    )
    .await
    {
        Ok(p) => Arc::new(p),
        Err(e) => {
            tracing::warn!(error = %e, "read provider build failed; graph.read stays fail-closed");
            return Arc::new(DeniedRunner);
        }
    };
    let graph: Arc<dyn GraphQuerier> = Arc::new(OsSdkGraphQuerier::new(resolve_knowledge_socket()));
    tracing::info!("graph.read wired to the live CypherPipeline over the proxied provider");
    Arc::new(CypherPipeline::new(provider, graph))
}

/// Build the session-bus connection that OWNS `org.arlen.AI1` - pi is the drop-in
/// replacement for the retired ai-daemon (planner ruling, 8 July). No interface is
/// attached here; `explain_system` is served on this same connection once the
/// sidecar is up, and it is the connection the `ProxiedProvider` forwards on (the
/// ai-proxy authorizes an LLM forward by the owned name). A name conflict (the old
/// ai-daemon still owning it) surfaces as an error and fails the AI paths closed.
async fn build_ai1_connection() -> zbus::Result<zbus::Connection> {
    zbus::connection::Builder::session()?
        .name(explain_iface::EXPLAIN_BUS_NAME)?
        .build()
        .await
}

/// Build the `graph.write` runner. Live ONLY when AI is enabled AND
/// `[agent] executor_live` is on; otherwise the fail-closed [`DeniedWriter`]
/// (a write then reports the not-permitted error and applies nothing). The live
/// writer performs a single atomic, op-id-keyed relation create through the
/// Knowledge Daemon; the reporter's compensation store then registers the op-id
/// retract that undoes exactly this write when the Report verb arrives. A write
/// only reaches here after the gate ALLOWED it (which under executor-live means an
/// authorized reversible action - a high-impact or externally-triggered one still
/// confirms) and it presented a valid HIGH-1 execution proof.
fn build_write_runner() -> Arc<dyn RelationWriter> {
    // The runner is chosen only by whether AI is on at all; executor_live is gated
    // PER CALL by the executor (below), not baked into the runner at startup. That
    // avoids the startup/per-call skew: a runtime executor_live change (either
    // direction) takes effect on the next write with no restart, and a mid-flight
    // disable is honoured at Execute even for an already-minted proof.
    if engine_config::ai_enabled() {
        Arc::new(UnixRelationWriter::new(resolve_knowledge_socket()))
    } else {
        Arc::new(DeniedWriter)
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let path = socket_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // Replace a stale socket left by an unclean exit.
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    let uid = current_uid();
    info!(socket = %path.display(), "ai-engine-daemon listening (Phase 1: real seams; live tool runners pending cutover)");

    // The dispatcher is shared across connection tasks. Phase 1 has wired the
    // gate seam to Capability::decide (CapabilityGate), the reporter seam to the
    // audit ledger + S17/S18 screening (ScreeningReporter: a reported tool result
    // is recorded content-free fail-closed and screened before it can re-enter
    // the engine context; the screener is Off until a classifier model is
    // provisioned, the gate's confirm-on-external-trigger being the containment
    // meanwhile), and the executor seam to the graph read executor over a fail-
    // closed runner. The live read runner is the proxied CypherPipeline, which
    // forwards only over an ai-proxy-authorized bus name the engine daemon cannot
    // own while the old ai-daemon runs (side-by-side Phase 1), so the runner is
    // DeniedRunner until the Phase-2 cutover swaps the real pipeline in (a one-
    // line change). Under the suggest_only baseline the gate never returns Allow,
    // so nothing executes yet regardless.
    // A bounded, daemon-lived store of op-id-keyed retract receipts: a reported
    // graph.write records its undo here for the activity-view undo trigger (a
    // later consumer) to reverse. In-memory + bounded; on startup it is re-armed
    // from the separate-uid signer's signed, HMAC-chained log (below), so a
    // graph compensation recorded before a restart is still undoable.
    let compensation = Arc::new(Mutex::new(CompensationStore::new(256)));
    // Restart restore: fetch the signer's still-undoable (non-terminal) entries and
    // re-arm the in-memory store, so an undo issued after a restart finds a write
    // the signed log recorded. Best-effort - an absent/failing signer just means no
    // restore (the session starts with an empty store), never a startup failure.
    {
        let signer = arlen_ai_undo_proto::socket_path();
        match arlen_ai_engine_daemon::undo_signer::fetch_live(&signer).await {
            Ok(entries) => {
                let armed = compensation
                    .lock()
                    .map(|mut store| store.restore(&entries))
                    .unwrap_or(0);
                if armed > 0 {
                    info!("re-armed {armed} persisted compensation(s) from the signer");
                }
            }
            Err(e) => tracing::debug!(error = %e, "no persisted compensation restored"),
        }
    }
    let audit: Arc<dyn AuditSink> = Arc::new(LedgerAuditSink::at_default_socket());
    // The curator loop publishes its live status here; the AIAgent1 `status` method
    // reads the same handle (the orchestrator writes idle/busy, the interface reads).
    // Hoisted so both the served surface and the orchestrator loop share one handle.
    let status = arlen_ai_engine_daemon::agent_iface::new_status_handle();
    let reporter = ScreeningReporter::new(audit.clone(), Screener::off());
    // The executor seam is a router so the daemon hosts several proxy tools
    // (graph.read + graph.write now; OS/MCP tools as they land), each enforcing
    // its own scope. graph.read runs over the LIVE CypherPipeline when AI is
    // enabled + a provider is configured (else the fail-closed DeniedRunner);
    // graph.write over the LIVE UnixRelationWriter when executor_live (else
    // DeniedWriter). The write executor AUDITS before it applies and registers the
    // op-id-keyed compensation at apply time, from the daemon's own op id - so a
    // committed write is audited + undoable regardless of the engine's Report. An
    // unregistered tool is UnknownTool.
    // The single `org.arlen.AI1`-owning connection (pi as the drop-in ai-daemon
    // replacement): it both authorizes the ProxiedProvider's LLM forwards and
    // serves explain_system (below). Built once, only when AI is enabled; a name
    // conflict or missing bus leaves it None and the AI paths fail closed. Held in
    // scope so it outlives the accept loop.
    let ai_connection: Option<zbus::Connection> = if engine_config::ai_enabled() {
        match build_ai1_connection().await {
            Ok(c) => Some(c),
            Err(e) => {
                warn!(error = %e, "could not own org.arlen.AI1; graph.read + explain fail-closed");
                None
            }
        }
    } else {
        None
    };
    // Serve the AIAgent1 pull-transparency + undo surface (status / completed_actions
    // / working_set / compensate) on the same connection that owns org.arlen.AI1. The
    // name request is GRACEFUL: while ai-agent still owns org.arlen.AIAgent1 (the
    // transition before it is deleted) the request fails, the surface stays dormant,
    // and AI1 + explain are unaffected; once ai-agent is removed the engine acquires
    // the name on the next start. compensate is gated to the harness/Settings caller.
    if let Some(conn) = &ai_connection {
        let surface = arlen_ai_engine_daemon::agent_iface::AgentAdminInterface::new(
            status.clone(),
            compensation.clone(),
            build_write_runner(),
            audit.clone(),
        );
        match conn
            .object_server()
            .at(arlen_ai_engine_daemon::agent_iface::AGENT_OBJECT_PATH, surface)
            .await
        {
            Ok(_) => match conn
                .request_name_with_flags(
                    arlen_ai_engine_daemon::agent_iface::AGENT_BUS_NAME,
                    zbus::fdo::RequestNameFlags::DoNotQueue.into(),
                )
                .await
            {
                Ok(_) => info!("serving org.arlen.AIAgent1 (pull-transparency + undo)"),
                Err(e) => warn!(error = %e, "org.arlen.AIAgent1 owned elsewhere (ai-agent transition); the engine surface activates once ai-agent is removed"),
            },
            Err(e) => warn!(error = %e, "could not serve the AIAgent1 surface"),
        }
    }
    let read_executor: Arc<dyn Executor> =
        Arc::new(GraphReadExecutor::new(build_read_runner(ai_connection.as_ref()).await));
    let write_executor: Arc<dyn Executor> = Arc::new(
        GraphWriteExecutor::new(build_write_runner())
            .with_audit(audit.clone())
            .with_compensation(compensation)
            // Persist each created write's compensation to the separate-uid signed
            // undo log too (best-effort), so an undo survives a restart. Absent
            // signer -> the in-memory store still covers the session.
            .with_undo_signer(arlen_ai_undo_proto::socket_path()),
    );
    // The autonomous curator's deterministic auto-tag write applies through the
    // SAME gated write executor (executor-live gated, audited, undo-registered);
    // clone the handle before the ProxyExecutor consumes it below.
    let curator_writer = write_executor.clone();
    // The ACT layer's live non-graph acts: fs.move + fs.trash, both gated
    // ReversibleAction. Same discipline as graph.write (executor-live gated,
    // audit-before-act, RestorePath/RestoreFromTrash compensation persisted to the
    // signed undo log). One instance serves both tools (registered under each name);
    // neither has an in-memory session store (the graph store is graph-only), so the
    // signer is their record.
    let fs_executor: Arc<dyn Executor> = Arc::new(
        FileSystemExecutor::new()
            .with_audit(audit.clone())
            .with_undo_signer(arlen_ai_undo_proto::socket_path()),
    );
    // The ACT layer's reversible settings write: settings.set, gated
    // ReversibleAction, confined to ~/.config/arlen and refusing the protected AI
    // master-switch file (ai.toml) so an autonomous write cannot self-escalate. Same
    // discipline (executor-live gated, audit-before-act, RestoreValue compensation to
    // the signed undo log).
    let settings_executor: Arc<dyn Executor> = Arc::new(
        SettingsExecutor::new()
            .with_audit(audit.clone())
            .with_undo_signer(arlen_ai_undo_proto::socket_path()),
    );
    // GATE-CLASSIFIER PRE-CONDITION (review MEDIUM): only tools registered here are
    // executable, and the gate's `action_kind_for_tool` classifies by NAME segment.
    // The irreversible graph mutators `graph.set_field` / `graph.retract_node`
    // segment to no always-confirm keyword, so under executor_live they would
    // resolve to Allow (Ordinary) even though the D1 `gate_class_for_tool` table
    // marks them Confirm. They are NOT registered here, so they cannot execute - but
    // BEFORE any executor for them is wired, route the gate decision through
    // `gate_class_for_tool` (or an effect-based irreversibility check), else an
    // irreversible action would auto-apply autonomously. Only reversible edge
    // writes are wired below.
    let executor = ProxyExecutor::new()
        .register("graph.read", read_executor)
        // D2 (pi-gate-class-registry.md): the fine-grained reversible graph-write
        // tools route to the same write executor as the coarse graph.write, so each
        // NAME carries one fixed gate class (ReversibleAction) - the coarse
        // graph.write's mixed reversibility can never be classified soundly. Under
        // executor_live the write applies (gated per-call); the coarse graph.write
        // stays registered through the transition. Write scope is any-relation-any-
        // nodes bounded only by the Knowledge Daemon's own tier auth (review MEDIUM:
        // a per-session write scope is the follow-up; acceptable for the reversible
        // tier since every write is op-id-keyed and undoable).
        .register("graph.assert_edge", write_executor.clone())
        .register("graph.retract_edge", write_executor.clone())
        .register("graph.write", write_executor)
        // The filesystem forward acts (ai-act-layer-plan.md §⟳). Both are
        // gate-classified ReversibleAction; the executor captures each act's inverse
        // write-ahead (RestorePath / RestoreFromTrash) and persists it to the undo
        // signer. One instance runs both.
        .register("fs.move", fs_executor.clone())
        .register("fs.trash", fs_executor)
        // The settings forward act (ai-act-layer-plan.md §⟳): a reversible scalar
        // config write, RestoreValue inverse write-ahead, ai.toml protected.
        .register("settings.set", settings_executor);
    // A gate Confirm is resolved daemon-side by driving the consent-broker over
    // its intake socket (the trusted-path dialog). An unreachable broker fails
    // closed to a denial, so wiring the real client is safe even headless.
    let dispatcher = Arc::new(
        Dispatcher::new(CapabilityGate::new(), executor, reporter)
            .with_consent(Arc::new(ConsentBrokerClient::new(consent_intake_socket()))),
    );

    // Spawn + supervise the pi sidecar only when AI is enabled (the §D master
    // switch; default off). The supervisor shares this dispatcher's session
    // store, so the session it binds to the sandboxed engine's pid (read from
    // bwrap's --info-fd) is the one the engine's contract calls resolve against.
    // A disabled config, or paths that do not resolve, leaves pi unspawned while
    // the contract socket still serves (the gate denies, the runners fail-closed).
    if engine_config::ai_enabled() {
        match SidecarPaths::resolve(|k| std::env::var(k).ok(), path.to_string_lossy().into_owned()) {
            Ok(paths) => {
                // System Explanation Mode: serve org.arlen.AI1.explain_system
                // via a fresh ephemeral pi. Held by its own task so the served
                // connection outlives this block (which ends before the accept loop).
                match explain_iface::load_explain_behaviour() {
                    Some(behaviour) => {
                        let iface = explain_iface::ExplainInterface::new(
                            Arc::new(behaviour),
                            Arc::new(PiSidecar::new(paths.clone())),
                            Arc::clone(&dispatcher) as Arc<dyn SessionBinder>,
                        );
                        match &ai_connection {
                            Some(conn) => match conn
                                .object_server()
                                .at(explain_iface::EXPLAIN_OBJECT_PATH, iface)
                                .await
                            {
                                Ok(true) => info!("serving org.arlen.AI1.explain_system"),
                                Ok(false) => warn!("explain object path already served"),
                                Err(e) => warn!(error = %e, "could not serve the explain interface"),
                            },
                            None => warn!("no org.arlen.AI1 connection; explain unavailable"),
                        }
                    }
                    None => warn!("explain skill not found; System Explanation Mode unavailable"),
                }

                // A second confined pi engine for the curator's ephemeral runs
                // (distinct from the persistent shell-driven supervisor below).
                let curator_paths = paths.clone();
                let sidecar = PiSidecar::new(paths);
                let disp = Arc::clone(&dispatcher);
                // The drive socket lets the harness shell drive this pi session;
                // a bind failure degrades to headless (the gate/contract path is
                // unaffected). Owned by the supervision task for its lifetime.
                let drive_listener = match bind_drive_socket() {
                    Ok(l) => Some(l),
                    Err(e) => {
                        warn!(error = %e, "could not bind the drive socket; pi runs without a shell drive");
                        None
                    }
                };
                tokio::spawn(async move {
                    let init = default_engine_session();
                    match supervise(&sidecar, &disp, &init, drive_listener.as_ref()).await {
                        Ok(pid) => info!(pid, "pi sidecar supervision ended"),
                        Err(e) => error!(error = %e, "pi sidecar supervision could not mint a session"),
                    }
                });
                info!("AI enabled: supervising the pi sidecar");

                // The autonomous curator (§E): subscribe the event bus and dispatch
                // the enabled behaviours - deterministic curation daemon-direct, an
                // agent behaviour to a bounded ephemeral pi run. Runs beside the
                // persistent (shell-driven) supervisor above. An event never reaches
                // an action without the gate: a deterministic write is executor-live
                // gated + audited, and an agent run is gated + scoped + audited per
                // call, its origin marked external so mutating actions confirm.
                let behaviours = Arc::new(orchestrator::load_behaviours().loaded);
                let sub_types = orchestrator::subscription_types(&behaviours);
                if sub_types.is_empty() {
                    info!("no enabled event-triggered behaviours; the curator does not subscribe");
                } else {
                    let consumer_socket = std::env::var("ARLEN_CONSUMER_SOCKET")
                        .unwrap_or_else(|_| orchestrator::DEFAULT_CONSUMER_SOCKET.to_string());
                    match orchestrator::EventBusSource::subscribe(consumer_socket, sub_types).await {
                        Ok(source) => {
                            let handler = CuratorHandler::new(
                                GraphProjectReader::new(resolve_knowledge_socket()),
                                curator_writer,
                                behaviours.clone(),
                                Arc::new(PiSidecar::new(curator_paths)),
                                Arc::clone(&dispatcher) as Arc<dyn SessionBinder>,
                            );
                            // The curator loop publishes its live status into the
                            // shared handle the AIAgent1 `status` method reads.
                            tokio::spawn(async move {
                                let mut coalescer = orchestrator::Coalescer::new(
                                    orchestrator::DEFAULT_COALESCE_WINDOW,
                                );
                                orchestrator::run_orchestrator(
                                    source,
                                    &behaviours,
                                    &mut coalescer,
                                    &handler,
                                    std::time::SystemTime::now,
                                    &status,
                                )
                                .await;
                                info!("curator orchestrator loop ended");
                            });
                            info!("autonomous curator orchestrator running");
                        }
                        Err(e) => warn!(
                            error = %e,
                            "curator could not subscribe to the event bus; autonomous curation disabled this run"
                        ),
                    }
                }
            }
            Err(e) => warn!(error = %e, "AI enabled but the pi sidecar paths do not resolve; not spawning pi"),
        }
    } else {
        info!("AI disabled ([ai] enabled=false); not spawning the pi sidecar");
    }

    let accept = async {
        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    // Authenticate the peer from the kernel (SO_PEERCRED), never
                    // a wire value; cross-uid is rejected, and the attested pid
                    // is what binds every verb to its session.
                    let auth = match ConnectionAuth::extract_from(&stream, uid) {
                        Ok(a) => a,
                        Err(e) => {
                            warn!(error = %e, "rejecting unauthenticated engine connection");
                            continue;
                        }
                    };
                    let pid = auth.pid();
                    let disp = Arc::clone(&dispatcher);
                    tokio::spawn(async move {
                        let mut stream = stream;
                        if let Err(e) = serve_connection(&mut stream, &disp, pid).await {
                            warn!(pid, error = %e, "engine connection ended with an error");
                        }
                    });
                }
                Err(e) => warn!(error = %e, "accept failed"),
            }
        }
    };

    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    tokio::select! {
        _ = accept => {}
        _ = tokio::signal::ctrl_c() => info!("SIGINT, shutting down"),
        _ = sigterm.recv() => info!("SIGTERM, shutting down"),
    }

    if let Err(e) = std::fs::remove_file(&path) {
        error!(error = %e, "failed to remove the contract socket on shutdown");
    }
    Ok(())
}

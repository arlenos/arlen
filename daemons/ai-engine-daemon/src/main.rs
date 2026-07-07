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
use arlen_ai_engine_daemon::proxy_executor::ProxyExecutor;
use arlen_ai_engine_daemon::read_executor::{DeniedRunner, GraphReadExecutor};
use arlen_ai_core::pipeline::{CypherPipeline, GraphQuerier, QueryRunner};
use arlen_ai_core::provider::AIProvider;
use arlen_ai_engine_daemon::graph_adapter::OsSdkGraphQuerier;
use arlen_ai_providers::proxied::{ProxiedConfig, ProxiedProvider};
use arlen_ai_engine_daemon::engine_config;
use arlen_ai_engine_daemon::reporter::ScreeningReporter;
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
async fn build_read_runner() -> Arc<dyn QueryRunner> {
    if !engine_config::ai_enabled() {
        return Arc::new(DeniedRunner);
    }
    let settings = engine_config::provider_settings();
    if settings.name.is_empty() {
        tracing::warn!("no ai.provider configured; graph.read stays fail-closed");
        return Arc::new(DeniedRunner);
    }
    let connection = match zbus::Connection::session().await {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "no session bus; graph.read stays fail-closed");
            return Arc::new(DeniedRunner);
        }
    };
    let provider: Arc<dyn AIProvider> = match ProxiedProvider::with_connection(
        ProxiedConfig {
            name: settings.name,
            model: settings.model,
            audit_token: settings.audit_token,
            context_window: settings.context_window,
        },
        &connection,
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
    // later consumer) to reverse. In-memory + bounded (a persisted/signed undo
    // log is a separate increment).
    let compensation = Arc::new(Mutex::new(CompensationStore::new(256)));
    let audit: Arc<dyn AuditSink> = Arc::new(LedgerAuditSink::at_default_socket());
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
    let read_executor: Arc<dyn Executor> =
        Arc::new(GraphReadExecutor::new(build_read_runner().await));
    let write_executor: Arc<dyn Executor> = Arc::new(
        GraphWriteExecutor::new(build_write_runner())
            .with_audit(audit.clone())
            .with_compensation(compensation),
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
        .register("graph.write", write_executor);
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

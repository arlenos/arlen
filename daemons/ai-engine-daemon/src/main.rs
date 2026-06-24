//! The Arlen AI engine daemon binary (`pi-agent-adoption.md` Phase 1).
//!
//! Binds the contract Unix socket (0600), and for each connection authenticates
//! the peer with SO_PEERCRED via `ConnectionAuth` (cross-uid rejected), then
//! serves the five-verb contract through the session-bound dispatcher with the
//! attested pid. Built BESIDE the existing ai-daemon/ai-agent; nothing here
//! touches them. The gate/executor/reporter seams are wired to the real Rust:
//! the gate is `CapabilityGate` (`Capability::decide`), the reporter is
//! `ScreeningReporter` (content-free audit + S17/S18 screening), and the
//! executor is a `ProxyExecutor` routing graph.read/graph.write. The live tool
//! runners stay fail-closed (`DeniedRunner`/`DeniedWriter`) pending the gated
//! cutovers (the Phase-2 read pipeline; the human-gated executor-live write),
//! and the screener is `Off` until a classifier model is provisioned - the
//! gate's confirm-on-external-trigger is the containment meanwhile.

use arlen_ai_core::screen::Screener;
use arlen_ai_engine_daemon::capability_map::CapabilityGate;
use arlen_ai_engine_daemon::compensation::CompensationStore;
use arlen_ai_engine_daemon::consent_client::ConsentBrokerClient;
use arlen_ai_engine_daemon::dispatch::Dispatcher;
use arlen_ai_engine_daemon::dispatch::Executor;
use arlen_ai_engine_daemon::proxy_executor::ProxyExecutor;
use arlen_ai_engine_daemon::read_executor::{DeniedRunner, GraphReadExecutor};
use arlen_ai_engine_daemon::engine_config;
use arlen_ai_engine_daemon::reporter::ScreeningReporter;
use arlen_ai_engine_daemon::sidecar::{PiSidecar, SidecarPaths};
use arlen_ai_engine_daemon::supervisor::supervise;
use arlen_ai_engine_daemon::write_executor::{DeniedWriter, GraphWriteExecutor};
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
    let _ = std::fs::remove_file(&path);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
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
    let reporter = ScreeningReporter::new(
        Arc::new(LedgerAuditSink::at_default_socket()) as Arc<dyn AuditSink>,
        Screener::off(),
    )
    .with_compensation(compensation);
    // The executor seam is a router so the daemon hosts several proxy tools
    // (graph.read + graph.write now; OS/MCP tools as they land), each enforcing
    // its own scope. Both are wired over fail-closed backends: graph.read over
    // DeniedRunner (the live read provider is the Phase-2 cutover) and
    // graph.write over DeniedWriter (the live write + its Report-side
    // compensation land at the human-gated executor-live cutover). An
    // unregistered tool is UnknownTool.
    let read_executor: Arc<dyn Executor> =
        Arc::new(GraphReadExecutor::new(Arc::new(DeniedRunner)));
    let write_executor: Arc<dyn Executor> =
        Arc::new(GraphWriteExecutor::new(Arc::new(DeniedWriter)));
    let executor = ProxyExecutor::new()
        .register("graph.read", read_executor)
        .register("graph.write", write_executor);
    // A gate Confirm is resolved daemon-side by driving the consent-broker over
    // its intake socket (the trusted-path dialog). An unreachable broker fails
    // closed to a denial, so wiring the real client is safe even headless.
    let dispatcher = Arc::new(
        Dispatcher::new(CapabilityGate, executor, reporter)
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

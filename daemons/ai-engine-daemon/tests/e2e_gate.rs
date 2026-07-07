//! End-to-end gate proof (`pi-agent-adoption.md` §C): the REAL Node gate plugin
//! (`ai/pi-plugins`) talks to the REAL Rust daemon contract over an actual Unix
//! socket, and the daemon's `CapabilityGate` (`Capability::decide`) verdict is
//! enforced inline by the plugin. This is the cross-language seam the unit
//! suites can only mock: a Node child runs the real `makeGate` + `ContractClient`,
//! fires one synthetic `tool_call`, and the daemon DENIES it because suggest mode
//! proposes a mutating action rather than auto-executing it. That is Tim's named
//! safe proof: a suggest-only session must Deny/Propose, never silently execute.
//!
//! The Deny carries a reason mentioning "proposal" (the gate's suggest-mode
//! wording), distinct from the session-guard's "no valid session" Deny - so the
//! assertion proves the REAL gate ran (Capability::decide was reached) AND that
//! SO_PEERCRED bound the spawned child's pid to the minted session.
//!
//! `#[ignore]`d: it needs node22 and the built pi-plugins dist
//! (`npm --prefix ai/pi-plugins run build`), so normal CI skips it. Run with
//! `cargo test -p arlen-ai-engine-daemon --test e2e_gate -- --ignored` on a host
//! with node; point `ARLEN_E2E_NODE` at the node binary if it is not on PATH or
//! at `~/.local/share/arlen-node22/bin/node`.

use ai_engine_contract::{
    CapabilityContext, ContractError, Execute, ExecuteOutcome, ReadTier, Report, ReportAck,
    ScreenVerdict, SessionInit,
};
use arlen_ai_engine_daemon::capability_map::CapabilityGate;
use arlen_ai_engine_daemon::dispatch::{Dispatcher, Executor, Reporter};
use arlen_ai_engine_daemon::proxy_executor::ProxyExecutor;
use arlen_ai_engine_daemon::read_executor::{DeniedRunner, GraphReadExecutor};
use arlen_ai_engine_daemon::session::SessionGrant;
use arlen_ai_engine_daemon::wire::serve_connection;
use arlen_ai_engine_daemon::write_executor::{DeniedWriter, GraphWriteExecutor};
use async_trait::async_trait;
use std::os::fd::AsRawFd;
use std::os::unix::fs::PermissionsExt;
use std::sync::Arc;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;
use tokio::net::UnixListener;

// The proof only drives Authorize, so the executor + reporter seams are inert
// stubs that are never reached; the gate is the REAL CapabilityGate.
struct UnusedExecutor;
#[async_trait]
impl Executor for UnusedExecutor {
    async fn execute(&self, _: &Execute, _: &SessionGrant) -> ExecuteOutcome {
        ExecuteOutcome::Error { code: ContractError::Internal, message: "unused in this test".into() }
    }
}
struct UnusedReporter;
#[async_trait]
impl Reporter for UnusedReporter {
    async fn report(&self, _: &Report, _: &SessionGrant) -> ReportAck {
        ReportAck { screen: ScreenVerdict::Block }
    }
}
// The audit Report proof needs a reporter that returns a non-Block verdict, so
// passthrough (audit.ts lets the content through) is observable - distinct from
// the no-session fallback, which fails closed to Block.
struct CleanReporter;
#[async_trait]
impl Reporter for CleanReporter {
    async fn report(&self, _: &Report, _: &SessionGrant) -> ReportAck {
        ReportAck { screen: ScreenVerdict::Clean }
    }
}

/// The kernel-attested pid of a connected Unix peer (SO_PEERCRED).
fn peer_pid(stream: &tokio::net::UnixStream) -> u32 {
    let mut cred = libc::ucred { pid: 0, uid: 0, gid: 0 };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: SO_PEERCRED writes a ucred into our buffer; len is its exact size.
    let r = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            &mut cred as *mut libc::ucred as *mut libc::c_void,
            &mut len,
        )
    };
    assert_eq!(r, 0, "SO_PEERCRED getsockopt failed");
    cred.pid as u32
}

/// Resolve the node binary: `ARLEN_E2E_NODE`, else the arlen-node22 runtime, else
/// `node` on PATH.
fn node_binary() -> String {
    if let Ok(n) = std::env::var("ARLEN_E2E_NODE") {
        return n;
    }
    if let Ok(home) = std::env::var("HOME") {
        let p = format!("{home}/.local/share/arlen-node22/bin/node");
        if std::path::Path::new(&p).exists() {
            return p;
        }
    }
    "node".to_string()
}

/// Spawn the Node e2e driver with `driver_args` against a real dispatcher over
/// the REAL `CapabilityGate` and the given `reporter` (the executor is inert; the
/// gate and audit proofs drive Authorize and Report respectively). Mints a
/// session bound to the child's SO_PEERCRED pid (so the real verb path resolves,
/// not the no-session fallback), serves the single verb, and returns the driver's
/// parsed JSON line.
async fn run_driver<E: Executor, R: Reporter>(
    executor: E,
    reporter: R,
    driver_args: &[&str],
) -> serde_json::Value {
    let dir = std::env::temp_dir()
        .join(format!("arlen-pi-e2e-{}-{}", std::process::id(), driver_args.join("-")));
    std::fs::create_dir_all(&dir).unwrap();
    let socket = dir.join("ai-engine.sock");
    let token_file = dir.join("token");

    let driver = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../ai/pi-plugins/dist/e2e.js");
    assert!(
        driver.exists(),
        "build the pi-plugins first (npm --prefix ai/pi-plugins run build): {}",
        driver.display(),
    );

    let dispatcher = Dispatcher::new(CapabilityGate, executor, reporter);

    let listener = UnixListener::bind(&socket).unwrap();
    std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600)).unwrap();

    // Spawn the Node child; it waits for the token file before connecting (the
    // daemon writes the token only after learning this child's pid).
    let mut cmd = std::process::Command::new(node_binary());
    cmd.arg(&driver);
    for a in driver_args {
        cmd.arg(a);
    }
    let child = cmd
        .env("ARLEN_AI_ENGINE_SOCKET", &socket)
        .env("ARLEN_AI_ENGINE_TOKEN_FILE", &token_file)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn node e2e driver");

    // Mint a session for the child's pid, then hand the token to the waiting child.
    let init = SessionInit {
        system_prompt: String::new(),
        behaviour: None,
        capability_context: CapabilityContext { generic_tools: vec![], proxy_tools: vec![] },
        project_anchor: None,
        read_tier: ReadTier::Minimal,
        externally_triggered: false,
    };
    let pid = child.id();
    let token = dispatcher.init_session(&init, pid).unwrap();
    std::fs::write(&token_file, token.as_str()).unwrap();
    std::fs::set_permissions(&token_file, std::fs::Permissions::from_mode(0o600)).unwrap();

    // Accept the child's connection, resolve its pid from the kernel
    // (SO_PEERCRED), and serve until the child closes after its single verb. The
    // daemon binary additionally resolves the peer's Arlen IDENTITY via
    // ConnectionAuth (the confined pi at a recognized path); that attestation is a
    // separate deploy concern, so this wire+enforcement proof uses the kernel-
    // attested pid directly (raw node is not a recognized Arlen app id).
    let serve = async {
        let stream = listener.accept().await.unwrap().0;
        let peer = peer_pid(&stream);
        assert_eq!(peer, pid, "the connecting peer is the spawned child");
        let mut stream = stream;
        serve_connection(&mut stream, &dispatcher, peer).await.unwrap();
    };
    tokio::time::timeout(Duration::from_secs(30), serve)
        .await
        .expect("the child connected and was served within the timeout");

    let output = child.wait_with_output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "the e2e driver failed: {stderr}");
    let line = stdout.trim().lines().last().unwrap_or("").to_string();
    let v: serde_json::Value =
        serde_json::from_str(&line).unwrap_or_else(|e| panic!("driver json ({e}): {line:?}"));
    std::fs::remove_dir_all(&dir).ok();
    v
}

#[tokio::test]
#[ignore = "needs node22 + the built pi-plugins dist (npm --prefix ai/pi-plugins run build)"]
async fn the_real_gate_plugin_is_denied_a_suggest_mode_tool_call_end_to_end() {
    // The gate proof reaches neither the executor nor the reporter (Authorize only).
    let v = run_driver(UnusedExecutor, UnusedReporter, &["gate", "note.append"]).await;
    assert_eq!(
        v["result"]["block"],
        serde_json::json!(true),
        "the suggest-mode tool call is blocked end to end: {v}",
    );
    let reason = v["result"]["reason"].as_str().unwrap_or("");
    assert!(
        reason.contains("proposal"),
        "the REAL gate decided (suggest -> propose -> deny), not the no-session fallback: {reason:?}",
    );
}

#[tokio::test]
#[ignore = "needs node22 + the built pi-plugins dist (npm --prefix ai/pi-plugins run build)"]
async fn the_real_audit_shim_reports_a_tool_result_and_clean_content_passes_end_to_end() {
    // A Clean screen verdict from the real reporter lets the content through:
    // audit.ts returns an empty patch. Passthrough is the strong proof that the
    // REAL reporter was reached - a no-session Report fails closed to Block ->
    // WITHHELD content, so an empty result only happens when the session bound and
    // the reporter both resolved over the actual socket.
    let v = run_driver(UnusedExecutor, CleanReporter, &["audit", "graph.read"]).await;
    let result = &v["result"];
    assert!(
        result.get("content").is_none(),
        "clean content passes through unchanged (not withheld): {v}",
    );
    assert!(
        result.get("isError").is_none(),
        "a clean pass-through sets no error: {v}",
    );
}

#[tokio::test]
#[ignore = "needs node22 + the built pi-plugins dist (npm --prefix ai/pi-plugins run build)"]
async fn the_real_proxy_tool_forwards_execute_and_fails_closed_end_to_end() {
    // The KG read proxy tool's execute() forwards to the daemon's Execute verb. The
    // daemon routes graph.read through the REAL ProxyExecutor -> GraphReadExecutor
    // -> the fail-closed DeniedRunner (the live read provider lands at the Phase-2
    // cutover), so the proxy tool surfaces a tool error. The "provider-unavailable"
    // message is the load-bearing distinguisher: it comes only from the real runner
    // reached PAST the session bound and the read-scope check - not from the
    // no-session or no-scope fallbacks - so it proves the Execute verb round-trips
    // through the real executor end to end.
    let read_executor: Arc<dyn Executor> = Arc::new(GraphReadExecutor::new(Arc::new(DeniedRunner)));
    let write_executor: Arc<dyn Executor> = Arc::new(GraphWriteExecutor::new(Arc::new(DeniedWriter)));
    let executor = ProxyExecutor::new()
        .register("graph.read", read_executor)
        .register("graph.write", write_executor);

    let v = run_driver(executor, UnusedReporter, &["execute", "graph.read"]).await;
    let result = &v["result"];
    assert_eq!(
        result["isError"],
        serde_json::json!(true),
        "the fail-closed Execute surfaces a tool error: {v}",
    );
    let text = result["content"][0]["text"].as_str().unwrap_or("");
    assert!(
        text.contains("provider-unavailable") || text.contains("Phase-2"),
        "the REAL ProxyExecutor -> GraphReadExecutor -> DeniedRunner was reached (not a session/scope fallback): {text:?}",
    );
}

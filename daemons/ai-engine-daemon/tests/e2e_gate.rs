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
use arlen_ai_engine_daemon::session::SessionGrant;
use arlen_ai_engine_daemon::wire::serve_connection;
use async_trait::async_trait;
use std::os::fd::AsRawFd;
use std::os::unix::fs::PermissionsExt;
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

#[tokio::test]
#[ignore = "needs node22 + the built pi-plugins dist (npm --prefix ai/pi-plugins run build)"]
async fn the_real_gate_plugin_is_denied_a_suggest_mode_tool_call_end_to_end() {
    let dir = std::env::temp_dir().join(format!("arlen-pi-e2e-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let socket = dir.join("ai-engine.sock");
    let token_file = dir.join("token");

    let driver = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../ai/pi-plugins/dist/e2e.js");
    assert!(
        driver.exists(),
        "build the pi-plugins first (npm --prefix ai/pi-plugins run build): {}",
        driver.display(),
    );

    // The REAL Phase-1 gate; the executor/reporter are inert (Authorize-only proof).
    let dispatcher = Dispatcher::new(CapabilityGate, UnusedExecutor, UnusedReporter);

    let listener = UnixListener::bind(&socket).unwrap();
    std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600)).unwrap();

    // Spawn the Node child; it waits for the token file before connecting (the
    // daemon writes the token only after learning this child's pid).
    let child = std::process::Command::new(node_binary())
        .arg(&driver)
        .arg("note.append")
        .env("ARLEN_AI_ENGINE_SOCKET", &socket)
        .env("ARLEN_AI_ENGINE_TOKEN_FILE", &token_file)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn node e2e driver");

    // Mint a session for the child's pid (empty grant -> the gate proposes ->
    // denies), then hand the token to the waiting child.
    let init = SessionInit {
        system_prompt: String::new(),
        behaviour: None,
        capability_context: CapabilityContext { generic_tools: vec![], proxy_tools: vec![] },
        project_anchor: None,
        read_tier: ReadTier::Minimal,
    };
    let pid = child.id();
    let token = dispatcher.init_session(&init, pid).unwrap();
    std::fs::write(&token_file, token.as_str()).unwrap();
    std::fs::set_permissions(&token_file, std::fs::Permissions::from_mode(0o600)).unwrap();

    // Accept the child's connection, resolve its pid from the kernel
    // (SO_PEERCRED), and serve until the child closes after its single Authorize.
    // The daemon binary additionally resolves the peer's Arlen IDENTITY via
    // ConnectionAuth (the confined pi at a recognized path); that attestation is
    // a separate deploy concern, so this wire+gate proof uses the kernel-attested
    // pid directly (raw node is not a recognized Arlen app id).
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

    let line = stdout.trim().lines().last().unwrap_or("");
    let v: serde_json::Value = serde_json::from_str(line).unwrap_or_else(|e| panic!("driver json ({e}): {line:?}"));
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

    std::fs::remove_dir_all(&dir).ok();
}

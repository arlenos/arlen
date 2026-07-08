//! The ephemeral pi run (`pi-agent-adoption.md` §D/§E): for a `kind: agent`
//! behaviour, the autonomous curator spawns a BOUNDED, headless, bwrap-confined
//! pi process for ONE trigger, drives it through the gated contract, and tears it
//! down - distinct from the persistent interactive supervisor. This module builds
//! the per-trigger session; the spawn + teardown is a later increment.

use crate::pi_driver::{drive_for_answer, drive_kick};
use crate::session::SessionToken;
use crate::supervisor::{EngineExit, SpawnEngine};
use ai_engine_contract::{CapabilityContext, ReadTier, SessionInit};
use arlen_ai_skills::behaviour::{Behaviour, ReadScope};
use std::path::PathBuf;
use std::time::Duration;
use tokio::net::{UnixListener, UnixStream};

/// The initial turn that kicks an ephemeral explain run.
const EXPLAIN_PROMPT: &str = "Explain what the computer is doing right now.";

/// The minimal turn that kicks a fire-and-forget behaviour run. It carries NO
/// trigger data (the event fields are external content, a prompt-injection
/// surface); pi reads the triggering event from the Knowledge Graph via its
/// gated, screened tools instead.
const KICK_PROMPT: &str = "Run your behaviour now.";

/// Wall-clock bound for an ephemeral run when the behaviour declares no budget
/// (an agent behaviour always declares one, so this is a defensive default).
const DEFAULT_EPHEMERAL_WALL_MS: u64 = 30_000;

/// The session lifecycle an ephemeral pi run drives: bind the minted session to
/// the spawned pid, then end it when the run is over. The production impl is the
/// [`Dispatcher`](crate::dispatch::Dispatcher); tests inject a recorder. (The
/// token is minted directly via [`SessionToken::mint`], as the supervisor does.)
pub trait SessionBinder: Send + Sync {
    /// Bind the session token to the kernel-attested spawned pid.
    fn bind_session(&self, token: SessionToken, init: &SessionInit, pid: u32);
    /// End the session (its run is over; a fresh run mints a new one).
    fn end_session(&self, token: &SessionToken);
}

impl<G, E, R> SessionBinder for crate::dispatch::Dispatcher<G, E, R>
where
    G: crate::dispatch::Gate,
    E: crate::dispatch::Executor,
    R: crate::dispatch::Reporter,
{
    fn bind_session(&self, token: SessionToken, init: &SessionInit, pid: u32) {
        crate::dispatch::Dispatcher::bind_session(self, token, init, pid)
    }
    fn end_session(&self, token: &SessionToken) {
        crate::dispatch::Dispatcher::end_session(self, token)
    }
}

/// The outcome of an ephemeral pi run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EphemeralOutcome {
    /// The run finished (the engine exited) within its wall-clock budget.
    Ran(EngineExit),
    /// The behaviour's turn ran to completion (pi emitted `agent_end` after the
    /// kick); the confined pi was then torn down. The normal success outcome for a
    /// kicked ephemeral run, since a turn-based pi does not exit on its own.
    Completed,
    /// The run exceeded its wall-clock budget and was aborted: the `run_once`
    /// future is dropped, which (via the sidecar's `kill_on_drop`) KILLS the
    /// confined pi tree, and `end_session` revokes its authority.
    TimedOut,
    /// The session token could not be minted (CSPRNG failure); the run is skipped.
    SessionMintFailed,
}

/// The wall-clock bound for a behaviour's ephemeral run (its budget's
/// `max_wall_ms`, or a defensive default).
fn ephemeral_wall(behaviour: &Behaviour) -> Duration {
    Duration::from_millis(
        behaviour.manifest.budget.as_ref().map(|b| b.max_wall_ms).unwrap_or(DEFAULT_EPHEMERAL_WALL_MS),
    )
}

/// Run ONE bounded, headless, confined pi process for a `kind: agent` trigger
/// (§D): mint a session, spawn pi with the per-trigger [`SessionInit`], bind the
/// session to the spawned pid, and drive it under a wall-clock timeout - then end
/// the session. Distinct from the persistent `supervise` loop: a single run, no
/// restart, no shell drive (`drive: None`). Every model-proposed action inside the
/// run is still gated + scoped + audited by the SAME contract path; this only
/// bounds the run.
pub async fn run_ephemeral_pi<S: SpawnEngine, B: SessionBinder + ?Sized>(
    behaviour: &Behaviour,
    project_anchor: Option<String>,
    engine: &S,
    binder: &B,
) -> EphemeralOutcome {
    let init = build_ephemeral_session_init(behaviour, project_anchor);
    let token = match SessionToken::mint() {
        Ok(t) => t,
        Err(_) => return EphemeralOutcome::SessionMintFailed,
    };
    let on_spawned = |pid: u32| binder.bind_session(token.clone(), &init, pid);
    let on_spawned: &(dyn Fn(u32) + Send + Sync) = &on_spawned;
    let wall = ephemeral_wall(behaviour);

    // pi is turn-based: with only a system prompt it idles and the behaviour never
    // runs. So bind a private drive socket and KICK it into acting. If the drive
    // socket cannot be set up, fall back to an un-driven run (degraded: pi will
    // idle, but the caller is never failed and the session is still cleaned up).
    let outcome = match ephemeral_drive_socket().and_then(|path| {
        UnixListener::bind(&path)
            .map(|listener| (path, listener))
            .map_err(|e| format!("could not bind the ephemeral drive socket: {e}"))
    }) {
        Ok((socket_path, listener)) => {
            let _socket_guard = SocketGuard(socket_path.clone());
            let run =
                engine.run_once(token.as_str(), &init.system_prompt, on_spawned, Some(&listener));
            let kick = async {
                // The listener is bound, so this connect queues even before
                // serve_drive accepts it; serve_drive relays it to pi's stdio.
                let stream = UnixStream::connect(&socket_path)
                    .await
                    .map_err(|e| format!("could not reach the ephemeral engine: {e}"))?;
                let (read, write) = stream.into_split();
                drive_kick(read, write, KICK_PROMPT).await
            };
            // The turn ending (agent_end) drops the run, which kills the confined
            // pi via kill_on_drop; the wall-clock and an early engine exit both
            // fail closed.
            let driven = async {
                tokio::select! {
                    exit = run => EphemeralOutcome::Ran(exit),
                    kicked = kick => match kicked {
                        Ok(()) => EphemeralOutcome::Completed,
                        Err(_) => EphemeralOutcome::Ran(EngineExit::Crashed),
                    },
                }
            };
            match tokio::time::timeout(wall, driven).await {
                Ok(o) => o,
                Err(_) => EphemeralOutcome::TimedOut,
            }
        }
        Err(e) => {
            tracing::warn!("ephemeral run falling back to an un-driven engine (no kick): {e}");
            match tokio::time::timeout(
                wall,
                engine.run_once(token.as_str(), &init.system_prompt, on_spawned, None),
            )
            .await
            {
                Ok(exit) => EphemeralOutcome::Ran(exit),
                Err(_) => EphemeralOutcome::TimedOut,
            }
        }
    };

    // End the session whether the run finished or timed out (its authority must
    // not outlive the run).
    binder.end_session(&token);
    outcome
}

/// Resolve a private drive-socket path for one ephemeral run, under the Arlen
/// runtime dir with a random suffix so concurrent runs never collide.
fn ephemeral_drive_socket() -> Result<PathBuf, String> {
    let base = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/run/arlen".to_string());
    let dir = PathBuf::from(base).join("arlen");
    std::fs::create_dir_all(&dir).map_err(|e| format!("could not create the runtime dir: {e}"))?;
    let mut bytes = [0u8; 8];
    getrandom::getrandom(&mut bytes).map_err(|e| e.to_string())?;
    let nonce: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    Ok(dir.join(format!("ephemeral-{nonce}.sock")))
}

/// Removes the private drive socket file on drop.
struct SocketGuard(PathBuf);
impl Drop for SocketGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Run `behaviour` (the explain skill) on a fresh ephemeral confined pi and RETURN
/// its assistant answer. Unlike [`run_ephemeral_pi`] (fire-and-forget curation),
/// this is REQUEST-RESPONSE: it drives pi over a PRIVATE drive socket, submits an
/// initial turn, and captures pi's reply via [`drive_for_answer`]. The session is
/// bound for the run (so pi's gated reads authorise) and ended after; the run is
/// dropped once the answer is in hand, so `kill_on_drop` reaps the confined pi.
/// Bounded by the behaviour's wall-clock. (System Explanation Mode, §D.)
pub async fn run_ephemeral_explain<S, B>(
    behaviour: &Behaviour,
    project_anchor: Option<String>,
    engine: &S,
    binder: &B,
) -> Result<String, String>
where
    S: SpawnEngine,
    B: SessionBinder + ?Sized,
{
    let init = build_ephemeral_session_init(behaviour, project_anchor);
    let token = SessionToken::mint().map_err(|_| "could not mint a session".to_string())?;

    let socket_path = ephemeral_drive_socket()?;
    let listener = UnixListener::bind(&socket_path)
        .map_err(|e| format!("could not bind the explain drive socket: {e}"))?;
    let _socket_guard = SocketGuard(socket_path.clone());

    let on_spawned = |pid: u32| binder.bind_session(token.clone(), &init, pid);
    let on_spawned: &(dyn Fn(u32) + Send + Sync) = &on_spawned;

    let run = engine.run_once(token.as_str(), &init.system_prompt, on_spawned, Some(&listener));
    let drive = async {
        // The listener is already bound, so this connect queues even before
        // `serve_drive` accepts it; serve_drive then relays it to pi's stdio.
        let stream = UnixStream::connect(&socket_path)
            .await
            .map_err(|e| format!("could not reach the explain engine: {e}"))?;
        let (read, write) = stream.into_split();
        drive_for_answer(read, write, EXPLAIN_PROMPT).await
    };

    // The answer arriving ends the run (dropping `run` kills the confined pi via
    // kill_on_drop); a wall-clock bound and an early engine exit both fail closed.
    let answer = tokio::select! {
        result = tokio::time::timeout(ephemeral_wall(behaviour), drive) => match result {
            Ok(driven) => driven,
            Err(_) => Err("the explanation timed out".to_string()),
        },
        _ = run => Err("the explain engine exited before answering".to_string()),
    };

    binder.end_session(&token);
    answer
}

/// Map a behaviour's declared [`ReadScope`] to the contract [`ReadTier`] the
/// session reads under. Both are five-level and ordinally aligned, so this is the
/// order-preserving correspondence (no graph -> no read, ... full -> full). The
/// graph compiler enforces the resulting tier + its active-project anchor.
pub fn read_tier_for_scope(scope: ReadScope) -> ReadTier {
    match scope {
        ReadScope::Minimal => ReadTier::None,
        ReadScope::Session => ReadTier::Minimal,
        ReadScope::Project => ReadTier::Standard,
        ReadScope::Time => ReadTier::Extended,
        ReadScope::Full => ReadTier::Full,
    }
}

/// Whether a declared tool name is a PRIVILEGED proxy tool - one the daemon runs
/// in trusted Rust via `Execute` (KG + OS mutations) - vs a generic in-engine
/// tool. Used only to split the SessionInit's coarse capability context; the gate
/// enforces every call regardless of this classification.
pub fn is_privileged_proxy_tool(tool: &str) -> bool {
    tool.starts_with("graph.") || tool.starts_with("fs.") || tool.starts_with("os.")
}

/// Build the [`SessionInit`] for an ephemeral autonomous pi run of `behaviour`.
///
/// The system prompt is the behaviour's body (the skill instructions); the
/// capability context lists the behaviour's declared tools split into generic vs
/// privileged-proxy; the read tier comes from the behaviour's declared read scope;
/// and `externally_triggered` is TRUE - an autonomous-curator run is started by an
/// event (external origin, HIGH-2), so the gate escalates every action to a
/// confirmation unless the deterministic-workflow carve-out applies (which it does
/// NOT for a `kind: agent` run, so an agent's mutating action always confirms).
///
/// NOTE (§F2, defense-in-depth follow-up): the capability context is PROMPT
/// CONTEXT ONLY - the gate is the real per-call authority - so a behaviour cannot
/// escalate by over-declaring tools here. Supplying a CURATED least-authority tool
/// set (rather than the behaviour's self-declared list) is the §F2 hardening, not
/// yet wired.
pub fn build_ephemeral_session_init(
    behaviour: &Behaviour,
    project_anchor: Option<String>,
) -> SessionInit {
    let (proxy_tools, generic_tools): (Vec<String>, Vec<String>) = behaviour
        .manifest
        .tools
        .keys()
        .cloned()
        .partition(|t| is_privileged_proxy_tool(t));
    SessionInit {
        system_prompt: behaviour.body.clone(),
        behaviour: Some(behaviour.manifest.name.clone()),
        capability_context: CapabilityContext { generic_tools, proxy_tools },
        project_anchor,
        read_tier: read_tier_for_scope(behaviour.manifest.reads),
        externally_triggered: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent_behaviour(name: &str) -> Behaviour {
        agent_behaviour_with_wall(name, 15000)
    }

    fn agent_behaviour_with_wall(name: &str, wall_ms: u64) -> Behaviour {
        // A complete agent SKILL.md declaring one privileged + one generic tool.
        let src = format!(
            "---\nname: {name}\ndescription: d\nkind: agent\nreads: project\nmode: suggest\n\
             trigger:\n  type: event\n  event: calendar.event.upcoming\ntools:\n  graph.query: []\n  \
             web.search: []\nbudget:\n  max_steps: 10\n  max_tokens: 12000\n  max_wall_ms: {wall_ms}\n\
             terminal:\n  done: silent\n---\nGather related notes.\n"
        );
        arlen_ai_skills::behaviour::parse(&src).expect("valid agent SKILL.md")
    }

    struct ScriptedEngine {
        exit: EngineExit,
        sleep_ms: u64,
    }
    #[async_trait::async_trait]
    impl SpawnEngine for ScriptedEngine {
        async fn run_once(
            &self,
            _token: &str,
            _system_prompt: &str,
            on_spawned: &(dyn Fn(u32) + Send + Sync),
            _drive: Option<&tokio::net::UnixListener>,
        ) -> EngineExit {
            on_spawned(4242); // the daemon binds the session to this attested pid
            if self.sleep_ms > 0 {
                tokio::time::sleep(Duration::from_millis(self.sleep_ms)).await;
            }
            self.exit
        }
    }

    #[derive(Default)]
    struct MockBinder {
        bound_pid: std::sync::Mutex<Option<u32>>,
        ended: std::sync::Mutex<bool>,
    }
    impl SessionBinder for MockBinder {
        fn bind_session(&self, _token: SessionToken, _init: &SessionInit, pid: u32) {
            *self.bound_pid.lock().unwrap() = Some(pid);
        }
        fn end_session(&self, _token: &SessionToken) {
            *self.ended.lock().unwrap() = true;
        }
    }

    #[tokio::test]
    async fn ephemeral_run_binds_the_spawned_pid_and_returns_the_exit() {
        let b = agent_behaviour("meeting-prep");
        let engine = ScriptedEngine { exit: EngineExit::Clean, sleep_ms: 0 };
        let binder = MockBinder::default();
        let out = run_ephemeral_pi(&b, Some("p".to_string()), &engine, &binder).await;
        assert_eq!(out, EphemeralOutcome::Ran(EngineExit::Clean));
        // The session was bound to the spawned pid, then ended after the run.
        assert_eq!(*binder.bound_pid.lock().unwrap(), Some(4242));
        assert!(*binder.ended.lock().unwrap());
    }

    #[tokio::test(start_paused = true)]
    async fn ephemeral_run_times_out_and_still_ends_the_session() {
        // A tiny wall budget with an engine that sleeps far past it.
        let b = agent_behaviour_with_wall("slow", 100);
        let engine = ScriptedEngine { exit: EngineExit::Clean, sleep_ms: 60_000 };
        let binder = MockBinder::default();
        let out = run_ephemeral_pi(&b, None, &engine, &binder).await;
        assert_eq!(out, EphemeralOutcome::TimedOut);
        // The session is ended even on timeout (its authority must not outlive it).
        assert!(*binder.ended.lock().unwrap());
    }

    #[test]
    fn read_tier_for_scope_is_the_ordinal_alignment() {
        assert_eq!(read_tier_for_scope(ReadScope::Minimal), ReadTier::None);
        assert_eq!(read_tier_for_scope(ReadScope::Session), ReadTier::Minimal);
        assert_eq!(read_tier_for_scope(ReadScope::Project), ReadTier::Standard);
        assert_eq!(read_tier_for_scope(ReadScope::Time), ReadTier::Extended);
        assert_eq!(read_tier_for_scope(ReadScope::Full), ReadTier::Full);
    }

    #[test]
    fn privileged_proxy_classification() {
        assert!(is_privileged_proxy_tool("graph.read"));
        assert!(is_privileged_proxy_tool("fs.move"));
        assert!(is_privileged_proxy_tool("os.notify"));
        assert!(!is_privileged_proxy_tool("web.search"));
        assert!(!is_privileged_proxy_tool("bash"));
    }

    #[test]
    fn build_session_init_carries_body_tools_tier_and_external() {
        let b = agent_behaviour("meeting-prep");
        let init = build_ephemeral_session_init(&b, Some("proj-1".to_string()));
        // The body is the verbatim skill instructions (trailing newline kept).
        assert_eq!(init.system_prompt.trim(), "Gather related notes.");
        assert_eq!(init.behaviour, Some("meeting-prep".to_string()));
        // Tools split by privilege.
        assert_eq!(init.capability_context.proxy_tools, vec!["graph.query".to_string()]);
        assert_eq!(init.capability_context.generic_tools, vec!["web.search".to_string()]);
        // reads: project -> the project (standard) tier; anchored.
        assert_eq!(init.read_tier, ReadTier::Standard);
        assert_eq!(init.project_anchor, Some("proj-1".to_string()));
        // An autonomous-curator run is externally triggered (HIGH-2).
        assert!(init.externally_triggered);
    }
}

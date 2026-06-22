//! Supervision of the agent-engine (pi) sidecar (`pi-agent-adoption.md` Phase 0).
//!
//! The daemon owns the engine process: it spawns `pi --mode rpc` bwrap-sandboxed
//! (no network except the ai-proxy socket), and restarts it on a crash with a
//! backoff that gives up after repeated rapid crashes. This module is the
//! engine-neutral restart POLICY (the same Foundation §07 Table 08 shape
//! modulesd uses) plus the [`SpawnEngine`] seam the daemon binary implements
//! with the real bwrap+pi invocation; the live spawn is gated on a node>=22.19
//! runtime (the de-risk spike's packaging note), but the policy + the loop are
//! exercised here over a mock.

use async_trait::async_trait;
use std::time::{Duration, Instant};

/// How long crashes accumulate before the window resets. A clean run longer
/// than this clears the count, so an engine that ran fine for minutes and then
/// crashes once still gets an immediate restart.
const CRASH_WINDOW: Duration = Duration::from_secs(60);

/// What to do after an engine exit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recovery {
    /// Restart now.
    Immediate,
    /// Restart after the given delay (back off from a repeated crash).
    Delayed { delay: Duration },
    /// Give up; too many crashes inside the window. Manual retry only.
    PermanentlyFailed { crashes: u32 },
}

/// The engine's crash/restart accounting. Pure + clock-injected (pass `now`), so
/// the policy is unit-tested without real time.
#[derive(Debug)]
pub struct RestartPolicy {
    crashes: u32,
    window_start: Instant,
    failed: bool,
}

impl RestartPolicy {
    /// A fresh policy starting its window at `now`.
    pub fn new(now: Instant) -> Self {
        Self { crashes: 0, window_start: now, failed: false }
    }

    /// Crashes recorded in the current window.
    pub fn crash_count(&self) -> u32 {
        self.crashes
    }

    /// Whether the engine has permanently failed (no auto-restart).
    pub fn is_failed(&self) -> bool {
        self.failed
    }

    /// Record a clean run that ended at `now`. A run longer than the window
    /// resets the crash count (a long-lived engine that exits cleanly is not
    /// "flapping"); a clean run inside the window leaves the count, since a
    /// crash-restart-clean-crash cycle should still escalate.
    pub fn record_clean_run(&mut self, now: Instant) {
        if now.duration_since(self.window_start) > CRASH_WINDOW {
            self.crashes = 0;
            self.window_start = now;
        }
    }

    /// Record a crash at `now` and decide the recovery. 1st crash in the window
    /// restarts immediately, 2nd after 5s, 3rd after 30s, 4th+ permanently
    /// fails. The window resets first if it elapsed without a fresh crash.
    pub fn record_crash(&mut self, now: Instant) -> Recovery {
        if now.duration_since(self.window_start) > CRASH_WINDOW {
            self.crashes = 0;
            self.window_start = now;
        }
        self.crashes += 1;
        match self.crashes {
            1 => Recovery::Immediate,
            2 => Recovery::Delayed { delay: Duration::from_secs(5) },
            3 => Recovery::Delayed { delay: Duration::from_secs(30) },
            _ => {
                self.failed = true;
                Recovery::PermanentlyFailed { crashes: self.crashes }
            }
        }
    }

    /// Clear the failure (the user asked to retry). The window restarts at `now`.
    pub fn manual_retry(&mut self, now: Instant) {
        self.crashes = 0;
        self.window_start = now;
        self.failed = false;
    }
}

/// How an engine process exited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineExit {
    /// The engine exited 0 (a clean shutdown, not a fault).
    Clean,
    /// The engine crashed (non-zero exit, signal, or spawn failure).
    Crashed,
}

/// Spawns and runs the agent engine to completion. The daemon binary implements
/// this with `pi --mode rpc` under bwrap (FS deny-by-default, NET only to the
/// ai-proxy socket), passing `session_token` in the child's env so the pi
/// gate-plugin can authenticate to the contract socket. The future resolves
/// when the process exits.
#[async_trait]
pub trait SpawnEngine: Send + Sync {
    /// Run one engine instance to exit, with `session_token` in its environment.
    /// `on_spawned` is called with the child's pid as soon as it is known, so
    /// the supervisor can bind the session to that attested pid before any
    /// contract call arrives. The future resolves when the process exits.
    async fn run_once(
        &self,
        session_token: &str,
        on_spawned: &(dyn Fn(u32) + Send + Sync),
    ) -> EngineExit;
}

/// Supervise the engine: spawn it, bind a fresh session to the spawned pid, run
/// to exit, and restart per [`RestartPolicy`]. Each run gets a freshly-minted
/// token bound to that run's pid (so a restarted engine cannot reuse a prior
/// run's session). Returns when the engine permanently fails (too many rapid
/// crashes) or when minting a token fails closed.
pub async fn supervise<S, G, E, R>(
    engine: &S,
    dispatcher: &crate::dispatch::Dispatcher<G, E, R>,
    init: &ai_engine_contract::SessionInit,
) -> Result<u32, crate::session::CsprngError>
where
    S: SpawnEngine,
    G: crate::dispatch::Gate,
    E: crate::dispatch::Executor,
    R: crate::dispatch::Reporter,
{
    let mut policy = RestartPolicy::new(Instant::now());
    loop {
        let token = crate::session::SessionToken::mint()?;
        let on_spawned = |pid: u32| dispatcher.bind_session(token.clone(), init, pid);
        let exit = engine.run_once(token.as_str(), &on_spawned).await;
        // The engine exited; its session is over (a restart mints a fresh one).
        dispatcher.end_session(&token);
        let now = Instant::now();
        match exit {
            // A long-lived RPC sidecar exiting cleanly is unexpected for an open
            // session; restart it so the daemon stays ready. A future intentional
            // -shutdown signal would break here instead.
            EngineExit::Clean => policy.record_clean_run(now),
            EngineExit::Crashed => match policy.record_crash(now) {
                Recovery::Immediate => {}
                Recovery::Delayed { delay } => tokio::time::sleep(delay).await,
                Recovery::PermanentlyFailed { crashes } => return Ok(crashes),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_backoff_escalates_then_permanently_fails() {
        let t0 = Instant::now();
        let mut p = RestartPolicy::new(t0);
        assert_eq!(p.record_crash(t0), Recovery::Immediate);
        assert_eq!(p.record_crash(t0), Recovery::Delayed { delay: Duration::from_secs(5) });
        assert_eq!(p.record_crash(t0), Recovery::Delayed { delay: Duration::from_secs(30) });
        assert_eq!(p.record_crash(t0), Recovery::PermanentlyFailed { crashes: 4 });
        assert!(p.is_failed());
    }

    #[test]
    fn a_clean_run_past_the_window_resets_the_count() {
        let t0 = Instant::now();
        let mut p = RestartPolicy::new(t0);
        p.record_crash(t0); // 1
        p.record_crash(t0); // 2
        assert_eq!(p.crash_count(), 2);
        // A clean run lasting longer than the window resets.
        p.record_clean_run(t0 + CRASH_WINDOW + Duration::from_secs(1));
        assert_eq!(p.crash_count(), 0);
        // So the next crash is treated as the first again.
        assert_eq!(p.record_crash(t0 + CRASH_WINDOW + Duration::from_secs(2)), Recovery::Immediate);
    }

    #[test]
    fn a_clean_run_inside_the_window_does_not_reset() {
        let t0 = Instant::now();
        let mut p = RestartPolicy::new(t0);
        p.record_crash(t0); // 1
        p.record_clean_run(t0 + Duration::from_secs(5)); // inside the window
        assert_eq!(p.crash_count(), 1, "a quick clean run does not clear flapping");
        assert_eq!(p.record_crash(t0 + Duration::from_secs(6)), Recovery::Delayed { delay: Duration::from_secs(5) });
    }

    #[test]
    fn a_crash_after_the_window_starts_a_fresh_count() {
        let t0 = Instant::now();
        let mut p = RestartPolicy::new(t0);
        p.record_crash(t0); // 1
        p.record_crash(t0); // 2
        // A crash long after the window started counts as the first again.
        assert_eq!(
            p.record_crash(t0 + CRASH_WINDOW + Duration::from_secs(1)),
            Recovery::Immediate
        );
    }

    #[test]
    fn manual_retry_clears_a_permanent_failure() {
        let t0 = Instant::now();
        let mut p = RestartPolicy::new(t0);
        for _ in 0..4 {
            p.record_crash(t0);
        }
        assert!(p.is_failed());
        p.manual_retry(t0);
        assert!(!p.is_failed());
        assert_eq!(p.crash_count(), 0);
        assert_eq!(p.record_crash(t0), Recovery::Immediate);
    }

    use crate::dispatch::Dispatcher;
    use crate::placeholder::{BlockReporter, DenyGate, UnavailableExecutor};
    use ai_engine_contract::{CapabilityContext, ReadTier, SessionInit};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex as StdMutex;

    /// A mock engine that returns a scripted sequence of exits (defaulting to
    /// Crashed once exhausted), counting runs and spawn-callbacks.
    struct ScriptedEngine {
        exits: StdMutex<std::collections::VecDeque<EngineExit>>,
        runs: AtomicUsize,
        spawns: AtomicUsize,
    }
    #[async_trait]
    impl SpawnEngine for ScriptedEngine {
        async fn run_once(
            &self,
            _token: &str,
            on_spawned: &(dyn Fn(u32) + Send + Sync),
        ) -> EngineExit {
            self.runs.fetch_add(1, Ordering::SeqCst);
            on_spawned(4242); // the daemon binds the session to this attested pid
            self.spawns.fetch_add(1, Ordering::SeqCst);
            self.exits.lock().unwrap().pop_front().unwrap_or(EngineExit::Crashed)
        }
    }

    fn dispatcher() -> Dispatcher<DenyGate, UnavailableExecutor, BlockReporter> {
        Dispatcher::new(DenyGate, UnavailableExecutor, BlockReporter)
    }

    fn init() -> SessionInit {
        SessionInit {
            system_prompt: "p".into(),
            behaviour: None,
            capability_context: CapabilityContext { generic_tools: vec![], proxy_tools: vec![] },
            project_anchor: None,
            read_tier: ReadTier::Minimal,
        }
    }

    /// The supervise loop restarts on a clean exit, escalates the backoff over
    /// crashes, and stops (returns) when the engine permanently fails. The
    /// backoff sleeps complete instantly under a paused clock.
    #[tokio::test(start_paused = true)]
    async fn supervise_restarts_then_gives_up_after_repeated_crashes() {
        use std::collections::VecDeque;
        let engine = ScriptedEngine {
            exits: StdMutex::new(VecDeque::from(vec![
                EngineExit::Clean, // a clean exit -> restart
                EngineExit::Crashed, // 1 -> immediate
                EngineExit::Crashed, // 2 -> 5s
                EngineExit::Crashed, // 3 -> 30s
                EngineExit::Crashed, // 4 -> permanently failed, supervise returns
            ])),
            runs: AtomicUsize::new(0),
            spawns: AtomicUsize::new(0),
        };
        let disp = dispatcher();
        let crashes = supervise(&engine, &disp, &init()).await.unwrap();
        assert_eq!(crashes, 4, "gives up on the 4th crash in the window");
        assert_eq!(engine.runs.load(Ordering::SeqCst), 5, "1 clean restart + 4 crashes");
        assert_eq!(engine.spawns.load(Ordering::SeqCst), 5, "each run bound a session to its pid");
        // The last session was ended on exit (no live session lingers).
        assert_eq!(disp.session_count(), 0, "the session is ended on each exit; none lingers");
    }
}

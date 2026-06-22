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
    async fn run_once(&self, session_token: &str) -> EngineExit;
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
}

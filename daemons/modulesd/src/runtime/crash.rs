/// Crash recovery state machine per Foundation §07 Table 08.
///
/// Each module instance carries a `CrashState` that the daemon updates
/// on every clean run and every crash. The state alone determines what
/// happens next: immediate restart, delayed restart, or permanent
/// failure that requires a manual retry.
///
/// The 60 second window resets on a clean run, so a module that has
/// been alive for several minutes and crashes once gets its immediate
/// restart even if it crashed three times last week.

use std::time::{Duration, Instant};

/// Time window during which crashes accumulate. Foundation §07.
const CRASH_WINDOW: Duration = Duration::from_secs(60);

/// What the daemon should do after the latest event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recovery {
    /// Restart the module immediately.
    Immediate,
    /// Restart the module after `delay`.
    Delayed { delay: Duration },
    /// Module is dead; only a manual retry can revive it.
    PermanentlyFailed { crashes: u32 },
}

#[derive(Debug, Clone)]
pub struct CrashState {
    /// Crashes within the current window.
    crashes: u32,
    /// Start of the current window.
    window_start: Instant,
    /// Last time the module ran cleanly long enough to reset the window.
    /// `None` means the module has never had a clean run.
    last_clean: Option<Instant>,
    /// True after we have given up on automatic restart. Manual retry
    /// is the only path out.
    failed: bool,
}

impl Default for CrashState {
    fn default() -> Self {
        Self::new()
    }
}

impl CrashState {
    pub fn new() -> Self {
        Self {
            crashes: 0,
            window_start: Instant::now(),
            last_clean: None,
            failed: false,
        }
    }

    pub fn is_failed(&self) -> bool {
        self.failed
    }

    pub fn crash_count(&self) -> u32 {
        self.crashes
    }

    /// Called when the module produced a result without trapping.
    /// A clean run that lasted longer than the crash window resets the
    /// crash counter. A clean run inside the window does not, because
    /// the module may still be flapping.
    pub fn record_clean_run(&mut self, now: Instant) {
        if let Some(prev) = self.last_clean {
            if now.duration_since(prev) > CRASH_WINDOW {
                self.crashes = 0;
                self.window_start = now;
            }
        }
        self.last_clean = Some(now);
    }

    /// Called when the module crashed (WASM trap, fuel exhaustion,
    /// iframe `onerror`). Returns the recovery decision. `Immediate`
    /// means caller should respawn at once; `Delayed` means schedule
    /// a respawn after the duration; `PermanentlyFailed` means stop
    /// trying.
    pub fn record_crash(&mut self, now: Instant) -> Recovery {
        // Clear the window if it has elapsed without a fresh crash.
        if now.duration_since(self.window_start) > CRASH_WINDOW {
            self.crashes = 0;
            self.window_start = now;
        }
        self.crashes += 1;

        match self.crashes {
            1 => Recovery::Immediate,
            2 => Recovery::Delayed {
                delay: Duration::from_secs(5),
            },
            3 => Recovery::Delayed {
                delay: Duration::from_secs(30),
            },
            _ => {
                self.failed = true;
                Recovery::PermanentlyFailed {
                    crashes: self.crashes,
                }
            }
        }
    }

    /// Manual reset (user clicked "retry" in Settings). Crash count
    /// returns to zero and the module is no longer marked failed.
    pub fn manual_retry(&mut self) {
        self.crashes = 0;
        self.window_start = Instant::now();
        self.failed = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_crash_is_immediate() {
        let mut s = CrashState::new();
        assert_eq!(s.record_crash(Instant::now()), Recovery::Immediate);
        assert!(!s.is_failed());
    }

    #[test]
    fn second_crash_within_window_delays_5s() {
        let mut s = CrashState::new();
        let t0 = Instant::now();
        s.record_crash(t0);
        let r = s.record_crash(t0 + Duration::from_secs(2));
        assert_eq!(r, Recovery::Delayed { delay: Duration::from_secs(5) });
    }

    #[test]
    fn third_crash_within_window_delays_30s() {
        let mut s = CrashState::new();
        let t0 = Instant::now();
        s.record_crash(t0);
        s.record_crash(t0 + Duration::from_secs(2));
        let r = s.record_crash(t0 + Duration::from_secs(4));
        assert_eq!(r, Recovery::Delayed { delay: Duration::from_secs(30) });
    }

    #[test]
    fn fourth_crash_within_window_permanently_fails() {
        let mut s = CrashState::new();
        let t0 = Instant::now();
        s.record_crash(t0);
        s.record_crash(t0 + Duration::from_secs(1));
        s.record_crash(t0 + Duration::from_secs(2));
        let r = s.record_crash(t0 + Duration::from_secs(3));
        assert_eq!(r, Recovery::PermanentlyFailed { crashes: 4 });
        assert!(s.is_failed());
    }

    #[test]
    fn crash_after_window_resets_to_immediate() {
        let mut s = CrashState::new();
        let t0 = Instant::now();
        // Three crashes inside the window.
        s.record_crash(t0);
        s.record_crash(t0 + Duration::from_secs(1));
        s.record_crash(t0 + Duration::from_secs(2));
        // Crash long after the window has elapsed: counter resets,
        // recovery is Immediate again.
        let r = s.record_crash(t0 + Duration::from_secs(120));
        assert_eq!(r, Recovery::Immediate);
        assert!(!s.is_failed());
    }

    #[test]
    fn clean_run_inside_window_does_not_reset() {
        let mut s = CrashState::new();
        let t0 = Instant::now();
        s.record_crash(t0);
        s.record_crash(t0 + Duration::from_secs(1));
        // Single clean run inside the window: module may still be
        // flapping, so the counter must not reset.
        s.record_clean_run(t0 + Duration::from_secs(10));
        let r = s.record_crash(t0 + Duration::from_secs(11));
        assert_eq!(r, Recovery::Delayed { delay: Duration::from_secs(30) });
    }

    #[test]
    fn extended_clean_period_resets_counter() {
        let mut s = CrashState::new();
        let t0 = Instant::now();
        s.record_crash(t0);
        // Two clean runs separated by more than the crash window means
        // the module is healthy. The next crash is treated as the first.
        s.record_clean_run(t0 + Duration::from_secs(10));
        s.record_clean_run(t0 + Duration::from_secs(80));
        let r = s.record_crash(t0 + Duration::from_secs(81));
        assert_eq!(r, Recovery::Immediate);
    }

    #[test]
    fn manual_retry_revives_failed_module() {
        let mut s = CrashState::new();
        let t0 = Instant::now();
        for i in 0..4 {
            s.record_crash(t0 + Duration::from_secs(i));
        }
        assert!(s.is_failed());
        s.manual_retry();
        assert!(!s.is_failed());
        assert_eq!(s.crash_count(), 0);
        assert_eq!(s.record_crash(Instant::now()), Recovery::Immediate);
    }
}

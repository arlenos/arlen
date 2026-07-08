//! Graceful process stop: the SIGTERM->SIGKILL ladder behind the task manager's
//! "End task" (system-monitor-plan.md §a, build-order item 1). Send SIGTERM, give
//! the process a grace period to exit cleanly, then SIGKILL if it did not. The
//! signalling is a seam ([`Signaller`]) so the ladder logic is tested without
//! killing real processes; [`LibcSignaller`] is the real backend over `libc::kill`.

use std::io;
use std::time::{Duration, Instant};

/// How a stop resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopOutcome {
    /// The process was already gone before we signalled it.
    AlreadyGone,
    /// The process exited within the grace period after SIGTERM (clean shutdown).
    ExitedOnTerm,
    /// The process outlived the grace period and was SIGKILLed.
    KilledAfterGrace,
}

/// The signalling seam, so [`stop_process`] is testable without real processes.
pub trait Signaller {
    /// Send SIGTERM to `pid`.
    fn term(&self, pid: u32) -> io::Result<()>;
    /// Send SIGKILL to `pid`.
    fn kill(&self, pid: u32) -> io::Result<()>;
    /// Whether `pid` currently exists (alive, even if unsignalable).
    fn is_alive(&self, pid: u32) -> bool;
}

/// Gracefully stop `pid`: SIGTERM, poll for exit up to `grace`, then SIGKILL if it
/// is still alive. An already-dead process is [`StopOutcome::AlreadyGone`] (no
/// signal sent). A signal error (e.g. no permission, so the privileged helper is
/// needed, or the process vanished) is returned to the caller.
pub fn stop_process(sig: &dyn Signaller, pid: u32, grace: Duration) -> io::Result<StopOutcome> {
    if !sig.is_alive(pid) {
        return Ok(StopOutcome::AlreadyGone);
    }
    sig.term(pid)?;
    let poll = Duration::from_millis(20).min(grace);
    let deadline = Instant::now() + grace;
    while Instant::now() < deadline {
        if !sig.is_alive(pid) {
            return Ok(StopOutcome::ExitedOnTerm);
        }
        std::thread::sleep(poll);
    }
    if !sig.is_alive(pid) {
        return Ok(StopOutcome::ExitedOnTerm);
    }
    sig.kill(pid)?;
    Ok(StopOutcome::KilledAfterGrace)
}

/// The real signaller over `libc::kill`.
pub struct LibcSignaller;

impl LibcSignaller {
    fn send(pid: u32, signal: i32) -> io::Result<()> {
        // SAFETY: `kill(2)` is a plain syscall with no memory effects; a bad pid
        // just returns an error, which we surface.
        let rc = unsafe { libc::kill(pid as libc::pid_t, signal) };
        if rc == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }
}

impl Signaller for LibcSignaller {
    fn term(&self, pid: u32) -> io::Result<()> {
        Self::send(pid, libc::SIGTERM)
    }

    fn kill(&self, pid: u32) -> io::Result<()> {
        Self::send(pid, libc::SIGKILL)
    }

    fn is_alive(&self, pid: u32) -> bool {
        // Signal 0 tests for existence without delivering anything. `EPERM` means
        // the process exists but we may not signal it (still alive); `ESRCH` (any
        // other error) means it is gone.
        // SAFETY: `kill(2)` with signal 0 has no effect beyond the existence check.
        let rc = unsafe { libc::kill(pid as libc::pid_t, 0) };
        rc == 0 || io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    /// A mock: alive until `term` has been called `die_after_term` more polls,
    /// counting `is_alive` calls. `die_after_term = None` means it never exits on
    /// SIGTERM (forces the SIGKILL path).
    struct Mock {
        die_after_term: Option<u32>,
        termed: Cell<bool>,
        killed: Cell<bool>,
        polls_since_term: Cell<u32>,
    }
    impl Mock {
        fn new(die_after_term: Option<u32>) -> Self {
            Self {
                die_after_term,
                termed: Cell::new(false),
                killed: Cell::new(false),
                polls_since_term: Cell::new(0),
            }
        }
    }
    impl Signaller for Mock {
        fn term(&self, _pid: u32) -> io::Result<()> {
            self.termed.set(true);
            Ok(())
        }
        fn kill(&self, _pid: u32) -> io::Result<()> {
            self.killed.set(true);
            Ok(())
        }
        fn is_alive(&self, _pid: u32) -> bool {
            if !self.termed.get() {
                return true;
            }
            match self.die_after_term {
                None => true,
                Some(n) => {
                    let polls = self.polls_since_term.get();
                    self.polls_since_term.set(polls + 1);
                    polls < n
                }
            }
        }
    }

    #[test]
    fn an_already_dead_process_is_not_signalled() {
        struct Dead;
        impl Signaller for Dead {
            fn term(&self, _: u32) -> io::Result<()> {
                panic!("must not signal a dead process")
            }
            fn kill(&self, _: u32) -> io::Result<()> {
                panic!("must not signal a dead process")
            }
            fn is_alive(&self, _: u32) -> bool {
                false
            }
        }
        assert_eq!(stop_process(&Dead, 1, Duration::from_millis(1)).unwrap(), StopOutcome::AlreadyGone);
    }

    #[test]
    fn a_process_that_exits_on_term_is_not_killed() {
        let m = Mock::new(Some(0)); // dead on the first alive-check after term
        assert_eq!(stop_process(&m, 1, Duration::from_millis(50)).unwrap(), StopOutcome::ExitedOnTerm);
        assert!(m.termed.get());
        assert!(!m.killed.get());
    }

    #[test]
    fn a_process_that_outlives_the_grace_is_killed() {
        let m = Mock::new(None); // never exits on term
        assert_eq!(
            stop_process(&m, 1, Duration::from_millis(30)).unwrap(),
            StopOutcome::KilledAfterGrace
        );
        assert!(m.termed.get());
        assert!(m.killed.get());
    }
}

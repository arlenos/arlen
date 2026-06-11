//! The write gate: when an adapter edit may be written back (integration-packages-plan.md IP-R3).
//!
//! Some apps rewrite their config on exit and would discard a live edit, so an
//! adapter declares a [`WriteStrategy`](crate::adapter::WriteStrategy): `anytime`
//! writes immediately, `requires_app_closed` writes only while the app is NOT
//! running. [`write_gate`] is the pure decision; whether the app is running is the
//! [`AppPresence`] seam, so the gate is testable without a process table. The real
//! [`ProcAppPresence`] answers it from a declared lockfile (definitive when the
//! app keeps one, like Firefox's `.parentlock`) and a `/proc` scan by process
//! name. The format-preserving write + the read-after-write verify are
//! `arlen-config-format`'s `checked_set`; this module only decides WHETHER to
//! write now.

use crate::adapter::WriteStrategy;
use std::path::{Path, PathBuf};

/// The verdict on whether an edit may be written now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteGate {
    /// The edit may be written now.
    Allowed,
    /// `requires_app_closed` and the app is running: writing now risks the app
    /// overwriting the edit on exit, so defer until it closes.
    BlockedAppRunning,
}

/// Whether the target app is currently running.
pub trait AppPresence {
    /// True if the app is running (a write under `requires_app_closed` must wait).
    fn is_running(&self) -> bool;
}

/// Decide whether an edit under `strategy` may be written now, given `presence`.
pub fn write_gate(strategy: WriteStrategy, presence: &dyn AppPresence) -> WriteGate {
    match strategy {
        WriteStrategy::Anytime => WriteGate::Allowed,
        WriteStrategy::RequiresAppClosed => {
            if presence.is_running() {
                WriteGate::BlockedAppRunning
            } else {
                WriteGate::Allowed
            }
        }
    }
}

/// The kernel truncates a process `comm` to 15 bytes (`TASK_COMM_LEN - 1`), so a
/// longer process name only ever appears truncated in `/proc/<pid>/comm`.
const COMM_MAX: usize = 15;

/// Whether a `/proc/<pid>/comm` value identifies `process_name`, accounting for
/// the kernel's 15-byte `comm` truncation (a longer name matches its prefix).
pub fn comm_matches(comm: &str, process_name: &str) -> bool {
    let comm = comm.trim_end_matches('\n');
    if comm == process_name {
        return true;
    }
    process_name.len() > COMM_MAX && comm == &process_name[..COMM_MAX]
}

/// Detects whether an app is running from a declared lockfile and a `/proc` scan.
pub struct ProcAppPresence {
    /// The app's process name to match in `/proc/<pid>/comm`.
    process_name: String,
    /// An optional lockfile whose existence definitively means the app is running
    /// (e.g. Firefox's `.parentlock`); checked before the `/proc` scan.
    lockfile: Option<PathBuf>,
}

impl ProcAppPresence {
    /// A presence detector for `process_name`, optionally backed by a `lockfile`.
    pub fn new(process_name: impl Into<String>, lockfile: Option<PathBuf>) -> Self {
        Self {
            process_name: process_name.into(),
            lockfile,
        }
    }

    /// Whether any `/proc/<pid>/comm` matches the process name. A missing or
    /// unreadable `/proc` is treated as "running" - fail-safe for a write gate,
    /// where the cost of a false "not running" (clobbering an edit) is worse than
    /// a false "running" (the user retries when the app is closed).
    fn proc_scan_running(&self) -> bool {
        let Ok(entries) = std::fs::read_dir("/proc") else {
            return true;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            // Only numeric PID directories.
            if !name.to_string_lossy().bytes().all(|b| b.is_ascii_digit()) {
                continue;
            }
            let comm_path = Path::new("/proc").join(&name).join("comm");
            if let Ok(comm) = std::fs::read_to_string(&comm_path) {
                if comm_matches(&comm, &self.process_name) {
                    return true;
                }
            }
        }
        false
    }
}

impl AppPresence for ProcAppPresence {
    fn is_running(&self) -> bool {
        // A present lockfile is definitive; otherwise scan the process table.
        if let Some(lock) = &self.lockfile {
            if lock.exists() {
                return true;
            }
        }
        self.proc_scan_running()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Mock(bool);
    impl AppPresence for Mock {
        fn is_running(&self) -> bool {
            self.0
        }
    }

    #[test]
    fn anytime_always_allows() {
        assert_eq!(write_gate(WriteStrategy::Anytime, &Mock(true)), WriteGate::Allowed);
        assert_eq!(write_gate(WriteStrategy::Anytime, &Mock(false)), WriteGate::Allowed);
    }

    #[test]
    fn requires_app_closed_blocks_only_while_running() {
        assert_eq!(
            write_gate(WriteStrategy::RequiresAppClosed, &Mock(true)),
            WriteGate::BlockedAppRunning
        );
        assert_eq!(
            write_gate(WriteStrategy::RequiresAppClosed, &Mock(false)),
            WriteGate::Allowed
        );
    }

    #[test]
    fn comm_matches_handles_the_15_byte_truncation() {
        assert!(comm_matches("firefox\n", "firefox"));
        assert!(comm_matches("firefox", "firefox"));
        // A long name appears truncated to 15 bytes in comm.
        assert!(comm_matches("some-very-long-", "some-very-long-process"));
        assert!(!comm_matches("firefox", "chromium"));
        // A non-truncated comm must match in full.
        assert!(!comm_matches("fire", "firefox"));
    }

    #[test]
    fn proc_scan_detects_the_running_test_process() {
        // This test process is running, so a detector for its own comm reports it.
        let own_comm = std::fs::read_to_string("/proc/self/comm").unwrap();
        let name = own_comm.trim_end_matches('\n');
        let here = ProcAppPresence::new(name, None);
        assert!(here.is_running(), "the test's own process must be detected");
        // A process that does not exist is not running.
        let absent = ProcAppPresence::new("definitely-not-a-real-process-xyz", None);
        assert!(!absent.is_running());
    }

    #[test]
    fn a_present_lockfile_means_running_without_a_scan() {
        let tmp = tempfile::tempdir().unwrap();
        let lock = tmp.path().join(".parentlock");
        std::fs::write(&lock, b"").unwrap();
        // Even with a process name that is not running, the lockfile is definitive.
        let p = ProcAppPresence::new("not-running-xyz", Some(lock.clone()));
        assert!(p.is_running());
        // Remove the lockfile: now it falls back to the (negative) scan.
        std::fs::remove_file(&lock).unwrap();
        assert!(!p.is_running());
    }
}

//! Process and resource collection for the task manager (system-monitor-plan.md
//! §e), the coder's Rust collection sidecar. This crate is the easy ~70% - the
//! process list, CPU%, and memory - read WRITE-CLEAN via `sysinfo` (MIT). The
//! advanced 30% (precise per-pid `procfs`, GPU, PSI, cgroup v2) and the
//! capability-scoped privileged helper are later layers; the app's Tauri backend
//! consumes this library, and the surface (arlen-ui) presents it.
//!
//! CPU% is a rate, not a single read (§e, "the gotcha that bites first"): sysinfo
//! computes it from two refreshes, so [`Collector`] holds a `System` across
//! snapshots and the first snapshot reports 0% until there is an interval to
//! measure over.

use sysinfo::{ProcessesToUpdate, System};

/// One process for the task-manager list (the easy-70% fields).
#[derive(Debug, Clone, PartialEq)]
pub struct ProcessRow {
    /// Process id.
    pub pid: u32,
    /// Parent process id, if any.
    pub ppid: Option<u32>,
    /// Executable / process name.
    pub name: String,
    /// Resident memory in bytes (sysinfo reports bytes).
    pub memory_bytes: u64,
    /// CPU usage as sysinfo reports it: summed across cores, so a process
    /// saturating two cores reads ~200%. The default surface divides by
    /// [`Collector::cpu_count`] for the WHOLE-MACHINE figure (never >100%); the
    /// flat expert view shows this raw per-core number (system-monitor-plan.md §a).
    pub cpu_percent: f32,
}

/// Collects the process list over `sysinfo`, holding the `System` so CPU% is a
/// correct rate on the second and later snapshots.
pub struct Collector {
    sys: System,
}

impl Default for Collector {
    fn default() -> Self {
        Self::new()
    }
}

impl Collector {
    /// A fresh collector. The first [`snapshot`](Self::snapshot) reports 0% CPU
    /// for every process (a rate needs two samples); refresh again after an
    /// interval for real figures.
    pub fn new() -> Self {
        Self { sys: System::new() }
    }

    /// Number of logical CPUs, for turning the per-core [`ProcessRow::cpu_percent`]
    /// into a whole-machine figure on the default surface.
    pub fn cpu_count(&self) -> usize {
        self.sys.cpus().len().max(1)
    }

    /// Refresh the process table and return the current rows. Dead processes are
    /// dropped. CPU% is meaningful only from the second snapshot on.
    pub fn snapshot(&mut self) -> Vec<ProcessRow> {
        self.sys.refresh_cpu_all();
        self.sys.refresh_processes(ProcessesToUpdate::All, true);
        self.sys
            .processes()
            .values()
            .map(|p| ProcessRow {
                pid: p.pid().as_u32(),
                ppid: p.parent().map(|pp| pp.as_u32()),
                name: p.name().to_string_lossy().into_owned(),
                memory_bytes: p.memory(),
                cpu_percent: p.cpu_usage(),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_includes_this_process() {
        // A real-system smoke test: our own process is present and named.
        let mut c = Collector::new();
        let rows = c.snapshot();
        assert!(!rows.is_empty());
        let me = std::process::id();
        let mine = rows.iter().find(|r| r.pid == me).expect("own process must be listed");
        assert!(!mine.name.is_empty());
    }

    #[test]
    fn cpu_count_is_at_least_one() {
        assert!(Collector::new().cpu_count() >= 1);
    }

    #[test]
    fn a_second_snapshot_still_lists_processes() {
        // Two snapshots (the CPU%-rate path) do not panic and keep the list.
        let mut c = Collector::new();
        c.snapshot();
        assert!(!c.snapshot().is_empty());
    }
}

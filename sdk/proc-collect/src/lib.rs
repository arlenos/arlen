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

use std::collections::BTreeMap;
use sysinfo::{Networks, ProcessesToUpdate, System};

/// One process for the task-manager list (the easy-70% fields).
#[derive(Debug, Clone, PartialEq)]
pub struct ProcessRow {
    /// Process id.
    pub pid: u32,
    /// Parent process id, if any.
    pub ppid: Option<u32>,
    /// Executable / process name.
    pub name: String,
    /// Run state in plain words (sysinfo's status: "Runnable", "Sleeping",
    /// "Zombie", "Stopped", ...) - the design's Status column. The surface maps
    /// these to its friendly labels (Running / Not responding / Suspended).
    pub state: String,
    /// Resident memory in bytes (sysinfo reports bytes).
    pub memory_bytes: u64,
    /// CPU usage as sysinfo reports it: summed across cores, so a process
    /// saturating two cores reads ~200%. The default surface divides by
    /// [`Collector::cpu_count`] for the WHOLE-MACHINE figure (never >100%); the
    /// flat expert view shows this raw per-core number (system-monitor-plan.md §a).
    pub cpu_percent: f32,
}

/// Collects the process list over `sysinfo`, holding the `System` (and `Networks`)
/// so CPU% and network throughput are correct rates on the second and later reads.
pub struct Collector {
    sys: System,
    networks: Networks,
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
        Self { sys: System::new(), networks: Networks::new_with_refreshed_list() }
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
                state: p.status().to_string(),
                memory_bytes: p.memory(),
                cpu_percent: p.cpu_usage(),
            })
            .collect()
    }

    /// Read the system-wide totals for the Performance tab (system-monitor-plan.md
    /// §b). Memory is reported honestly as used-vs-AVAILABLE (`MemAvailable`, what
    /// a new allocation can actually get), never used-vs-free - the design's "the
    /// honest number". The overall CPU% is a rate, so it is meaningful from the
    /// second call on (this holds the same `System` as [`snapshot`](Self::snapshot)).
    pub fn totals(&mut self) -> SystemTotals {
        self.sys.refresh_cpu_all();
        self.sys.refresh_memory();
        self.networks.refresh(true);
        let mut net_received_bytes = 0u64;
        let mut net_transmitted_bytes = 0u64;
        for (_name, data) in &self.networks {
            net_received_bytes = net_received_bytes.saturating_add(data.received());
            net_transmitted_bytes = net_transmitted_bytes.saturating_add(data.transmitted());
        }
        SystemTotals {
            cpu_percent: self.sys.global_cpu_usage(),
            memory_total_bytes: self.sys.total_memory(),
            memory_available_bytes: self.sys.available_memory(),
            memory_used_bytes: self.sys.used_memory(),
            swap_total_bytes: self.sys.total_swap(),
            swap_used_bytes: self.sys.used_swap(),
            net_received_bytes,
            net_transmitted_bytes,
        }
    }
}

/// System-wide resource totals for the Performance tab. Memory is used-vs-AVAILABLE
/// (`MemAvailable`), not used-vs-free: available counts reclaimable cache, so it is
/// the honest "how much can I still allocate" figure (system-monitor-plan.md §b).
/// Disk and per-interface network totals are a later layer (net via sysinfo's
/// interface deltas, disk I/O throughput hand-rolled - neither is in this snapshot).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SystemTotals {
    /// Overall CPU usage, 0-100 (whole machine); a rate, so 0 on the first read.
    pub cpu_percent: f32,
    /// Total physical RAM, bytes.
    pub memory_total_bytes: u64,
    /// Available RAM (`MemAvailable`: free plus reclaimable), bytes - the honest
    /// "how much can still be allocated" figure.
    pub memory_available_bytes: u64,
    /// Used RAM, bytes.
    pub memory_used_bytes: u64,
    /// Total swap, bytes.
    pub swap_total_bytes: u64,
    /// Used swap, bytes.
    pub swap_used_bytes: u64,
    /// Bytes received across all interfaces since the previous [`Collector::totals`]
    /// call (0 on the first call) - divide by the inter-call interval for a rate.
    pub net_received_bytes: u64,
    /// Bytes transmitted across all interfaces since the previous call.
    pub net_transmitted_bytes: u64,
}

/// An app-grouped row: one named app aggregating its processes' resources (the
/// design's "Chrome is one row, not 15 nameless PIDs"). The default landing shows
/// these; a toggle flattens back to the raw [`ProcessRow`] list (the expert view).
#[derive(Debug, Clone, PartialEq)]
pub struct AppGroup {
    /// The app name that grouped these processes.
    pub name: String,
    /// The member pids, ascending.
    pub pids: Vec<u32>,
    /// Summed CPU% across the members (still per-core; the surface normalizes by
    /// [`Collector::cpu_count`]).
    pub total_cpu_percent: f32,
    /// Summed resident memory across the members, in bytes.
    pub total_memory_bytes: u64,
}

/// Group processes into named app rows by name and aggregate their CPU and memory,
/// sorted by CPU descending (the default landing order), name-ascending on ties.
/// Grouping by name covers the common case (one app spawns many like-named
/// helpers); grouping by executable path or process ancestry is a later refinement.
pub fn group_by_app(rows: &[ProcessRow]) -> Vec<AppGroup> {
    let mut by_name: BTreeMap<&str, AppGroup> = BTreeMap::new();
    for row in rows {
        let group = by_name.entry(row.name.as_str()).or_insert_with(|| AppGroup {
            name: row.name.clone(),
            pids: Vec::new(),
            total_cpu_percent: 0.0,
            total_memory_bytes: 0,
        });
        group.pids.push(row.pid);
        group.total_cpu_percent += row.cpu_percent;
        group.total_memory_bytes += row.memory_bytes;
    }
    let mut out: Vec<AppGroup> = by_name.into_values().collect();
    for group in &mut out {
        group.pids.sort_unstable();
    }
    out.sort_by(|a, b| {
        b.total_cpu_percent
            .partial_cmp(&a.total_cpu_percent)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.name.cmp(&b.name))
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(pid: u32, name: &str, cpu: f32, mem: u64) -> ProcessRow {
        ProcessRow {
            pid,
            ppid: None,
            name: name.to_string(),
            state: "Sleeping".to_string(),
            memory_bytes: mem,
            cpu_percent: cpu,
        }
    }

    #[test]
    fn groups_like_named_processes_into_one_app_row() {
        let rows = vec![
            row(10, "chrome", 5.0, 100),
            row(11, "chrome", 3.0, 200),
            row(12, "chrome", 2.0, 300),
            row(20, "kitty", 1.0, 50),
        ];
        let groups = group_by_app(&rows);
        assert_eq!(groups.len(), 2);
        let chrome = groups.iter().find(|g| g.name == "chrome").unwrap();
        assert_eq!(chrome.pids, vec![10, 11, 12]);
        assert_eq!(chrome.total_cpu_percent, 10.0);
        assert_eq!(chrome.total_memory_bytes, 600);
    }

    #[test]
    fn groups_are_sorted_by_cpu_descending() {
        let rows = vec![row(1, "a", 1.0, 0), row(2, "b", 9.0, 0), row(3, "c", 5.0, 0)];
        let names: Vec<String> = group_by_app(&rows).into_iter().map(|g| g.name).collect();
        assert_eq!(names, vec!["b", "c", "a"]);
    }

    #[test]
    fn empty_input_groups_to_nothing() {
        assert!(group_by_app(&[]).is_empty());
    }

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

    #[test]
    fn totals_report_sane_memory() {
        let mut c = Collector::new();
        let t = c.totals();
        assert!(t.memory_total_bytes > 0, "a machine has some RAM");
        assert!(t.memory_available_bytes <= t.memory_total_bytes, "available never exceeds total");
        assert!(t.memory_used_bytes <= t.memory_total_bytes, "used never exceeds total");
        assert!(t.swap_used_bytes <= t.swap_total_bytes, "used swap never exceeds total swap");
        assert!((0.0..=100.0).contains(&t.cpu_percent), "overall CPU% is a whole-machine 0-100");
    }
}

/// Graceful process stop: the SIGTERM->SIGKILL ladder ("End task").
pub mod stop;

/// Per-process detail (honest PSS memory, open-file count) read on demand.
pub mod detail;

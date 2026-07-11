//! The live process feed for the task manager (system-monitor-plan.md): read the
//! per-process `/proc` details, compute CPU% and disk-rate deltas against the
//! previous sample, and map each to the frontend `Process` shape.
//!
//! CPU% and disk KB/s are rates, so they need two samples spaced by an interval;
//! the [`Monitor`] holds the previous snapshot and computes the delta on each
//! call. The first call (no previous) reports 0 for the rates and the real memory
//! and names, so the manager shows real processes immediately and the rates
//! settle on the next poll. Per-process network is not in `/proc` (it needs eBPF/cgroup
//! attribution), so `netKBs` is 0 for now, honestly.

use std::sync::Mutex;
use std::time::Instant;

use arlen_system_monitor_mcp::sysinfo::{process_rates, ProcReader, ProcessDetail};
use serde::Serialize;

/// One process row as the frontend `Process` interface consumes it (camelCase for
/// the rate fields). A flat row per process; app-child grouping (one row over a
/// browser's tabs) is a later refinement.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Process {
    /// The process id.
    pub id: u32,
    /// The display name.
    pub name: String,
    /// `"app" | "background" | "system"`.
    pub group: &'static str,
    /// `"running" | "suspended" | "not-responding"`.
    pub status: &'static str,
    /// CPU% (share of total capacity in the sample window).
    pub cpu: f64,
    /// Resident memory in mebibytes.
    #[serde(rename = "memMB")]
    pub mem_mb: f64,
    /// Storage I/O rate in kibibytes per second.
    #[serde(rename = "diskKBs")]
    pub disk_kbs: f64,
    /// Per-process network rate in KB/s (0 until eBPF/cgroup attribution lands).
    #[serde(rename = "netKBs")]
    pub net_kbs: f64,
}

/// The known first-party background daemons: they show as ordinary rows in the
/// Background group (sovereignty by being an ordinary row, not a lecture).
const BACKGROUND: &[&str] = &[
    "knowledge",
    "arlen-graph-daemon",
    "ai-agent",
    "ai-daemon",
    "ai-engine-daemon",
    "arlen-ai-engine",
    "event-bus",
    "audit-daemon",
    "arlen-auditd",
    "modulesd",
    "notification-daemon",
    "arlen-notifyd",
    "power-daemon",
    "arlen-powerd",
    "anomaly-detector",
    "consent-broker",
    "online-accounts",
    "connections",
    "capsuled",
    "print",
];

/// Core OS / session infrastructure.
const SYSTEM: &[&str] = &[
    "systemd",
    "cosmic-comp",
    "Xwayland",
    "pipewire",
    "wireplumber",
    "dbus-daemon",
    "dbus-broker",
    "systemd-journal",
    "systemd-logind",
    "systemd-udevd",
    "udevd",
    "polkitd",
    "greetd",
    "seatd",
];

/// Classify a process into the task-manager's three groups by name. An `arlen-`
/// prefixed binary that is not core infrastructure is a first-party background
/// service; everything else the user launched is an app. Pure.
pub fn classify_group(name: &str) -> &'static str {
    if SYSTEM.contains(&name) {
        "system"
    } else if BACKGROUND.contains(&name) || name.starts_with("arlen-") {
        "background"
    } else {
        "app"
    }
}

/// Map a raw `/proc` state char to the plain-words display status. Stopped/traced
/// is a suspend; a zombie is not responding; everything else (running, sleeping,
/// disk-wait) reads as running to the user. Pure.
pub fn map_status(state: char) -> &'static str {
    match state {
        'T' | 't' => "suspended",
        'Z' => "not-responding",
        _ => "running",
    }
}

/// Build the frontend process rows from the current sample and, when present, the
/// previous one (its processes, total CPU jiffies and the interval since it) for
/// the rate deltas. A process with no match in the previous sample reports 0
/// rates (new since last poll). Sorted CPU-desc then memory-desc so the hog is on
/// top. Pure, so the mapping + rate wiring is unit-tested.
pub fn build_processes(
    procs: &[ProcessDetail],
    total: u64,
    prev: Option<(&[ProcessDetail], u64, f64)>,
) -> Vec<Process> {
    let mut out: Vec<Process> = procs
        .iter()
        .map(|d| {
            let (cpu, disk) = match prev {
                Some((prev_procs, prev_total, interval)) => {
                    match prev_procs.iter().find(|p| p.pid == d.pid) {
                        Some(pd) => {
                            let r = process_rates(pd, d, prev_total, total, interval);
                            (r.cpu_pct, r.disk_kbs)
                        }
                        None => (0.0, 0.0),
                    }
                }
                None => (0.0, 0.0),
            };
            Process {
                id: d.pid,
                name: d.name.clone(),
                group: classify_group(&d.name),
                status: map_status(d.state),
                cpu,
                mem_mb: d.mem_kb as f64 / 1024.0,
                disk_kbs: disk,
                net_kbs: 0.0,
            }
        })
        .collect();
    out.sort_by(|a, b| {
        b.cpu
            .partial_cmp(&a.cpu)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(b.mem_mb.partial_cmp(&a.mem_mb).unwrap_or(std::cmp::Ordering::Equal))
    });
    out
}

/// One captured snapshot: the per-process details, the system CPU jiffies at that
/// instant and when it was taken (for the next call's interval).
struct Snapshot {
    procs: Vec<ProcessDetail>,
    total: u64,
    at: Instant,
}

/// The live process monitor: holds the previous snapshot so each `sample()`
/// computes CPU% and disk-rate deltas. Held as Tauri managed state.
pub struct Monitor {
    prev: Mutex<Option<Snapshot>>,
}

impl Monitor {
    /// A fresh monitor with no previous sample (the first `sample()` reports 0
    /// rates).
    pub fn new() -> Self {
        Self { prev: Mutex::new(None) }
    }

    /// Read the current processes + resource totals, map them against the previous
    /// snapshot for the rates, then store this as the new previous. `now` is the
    /// capture instant (injected so the interval is testable).
    pub fn sample_at(&self, now: Instant) -> Vec<Process> {
        let reader = ProcReader::new();
        let procs = reader.list_processes_detailed();
        let total = reader.total_cpu_jiffies();
        let mut guard = self.prev.lock().unwrap_or_else(|e| e.into_inner());
        let out = match guard.as_ref() {
            Some(p) => {
                let interval = now.saturating_duration_since(p.at).as_secs_f64();
                build_processes(&procs, total, Some((&p.procs, p.total, interval)))
            }
            None => build_processes(&procs, total, None),
        };
        *guard = Some(Snapshot { procs, total, at: now });
        out
    }

    /// Sample now (the command entry point).
    pub fn sample(&self) -> Vec<Process> {
        self.sample_at(Instant::now())
    }
}

impl Default for Monitor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn detail(pid: u32, name: &str, state: char, mem_kb: u64, cpu: u64, io: u64) -> ProcessDetail {
        ProcessDetail {
            pid,
            name: name.to_string(),
            state,
            mem_kb,
            cpu_jiffies: cpu,
            io_read_bytes: io,
            io_write_bytes: 0,
        }
    }

    #[test]
    fn groups_classify_by_name() {
        assert_eq!(classify_group("cosmic-comp"), "system");
        assert_eq!(classify_group("knowledge"), "background");
        assert_eq!(classify_group("arlen-capsuled"), "background");
        assert_eq!(classify_group("Firefox"), "app");
    }

    #[test]
    fn status_maps_the_state_char() {
        assert_eq!(map_status('R'), "running");
        assert_eq!(map_status('S'), "running");
        assert_eq!(map_status('T'), "suspended");
        assert_eq!(map_status('Z'), "not-responding");
    }

    #[test]
    fn first_sample_has_zero_rates_and_real_memory() {
        let now = [detail(1, "bash", 'R', 2048, 100, 4096)];
        let rows = build_processes(&now, 1000, None);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, 1);
        assert_eq!(rows[0].cpu, 0.0);
        assert_eq!(rows[0].disk_kbs, 0.0);
        assert!((rows[0].mem_mb - 2.0).abs() < 1e-9);
        assert_eq!(rows[0].net_kbs, 0.0);
    }

    #[test]
    fn second_sample_computes_rates_against_the_matched_pid() {
        let prev = [detail(1, "bash", 'R', 2048, 100, 0)];
        let now = [detail(1, "bash", 'R', 2048, 150, 4096)];
        // 50 jiffies over 200 total = 25%; 4096 bytes over 2s = 2 KiB/s.
        let rows = build_processes(&now, 1200, Some((&prev, 1000, 2.0)));
        assert!((rows[0].cpu - 25.0).abs() < 1e-9);
        assert!((rows[0].disk_kbs - 2.0).abs() < 1e-9);
    }

    #[test]
    fn rows_are_sorted_cpu_desc() {
        let prev = [detail(1, "a", 'R', 0, 0, 0), detail(2, "b", 'R', 0, 0, 0)];
        let now = [detail(1, "a", 'R', 0, 10, 0), detail(2, "b", 'R', 0, 90, 0)];
        let rows = build_processes(&now, 100, Some((&prev, 0, 1.0)));
        assert_eq!(rows[0].id, 2, "the CPU hog is on top");
        assert_eq!(rows[1].id, 1);
    }

    #[test]
    fn a_new_pid_since_last_sample_reports_zero_rates() {
        let prev = [detail(1, "a", 'R', 0, 100, 0)];
        // pid 2 is new this sample -> no prev match -> 0 rates, not a panic.
        let now = [detail(2, "b", 'R', 1024, 500, 9999)];
        let rows = build_processes(&now, 200, Some((&prev, 100, 1.0)));
        assert_eq!(rows[0].id, 2);
        assert_eq!(rows[0].cpu, 0.0);
        assert_eq!(rows[0].disk_kbs, 0.0);
    }
}

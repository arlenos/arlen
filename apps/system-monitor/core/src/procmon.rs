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
    /// Does stopping this take something else down with it? Carried on the row
    /// rather than re-derived in the frontend, so the warning and the grouping
    /// read one list. See `is_critical`.
    pub critical: bool,
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
    /// The members this row stands for, when it is an app row.
    ///
    /// `None` on a plain per-pid row and on every child, which is what keeps the
    /// grouped and flat shapes the SAME type: the frontend's `Process` already
    /// models `children?`, so grouping adds no second contract for the table to
    /// learn. Skipped when absent so the flat list serialises exactly as before.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<Process>>,
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

/// Is this process one whose loss takes something else down with it?
///
/// The plan's guardrail (system-monitor-plan.md (d)1): the Arlen daemons appear
/// as ordinary rows you can stop like anything else - that IS the sovereignty
/// statement - but stopping one warns first, "the way Windows greys out critical
/// processes". Stopping `knowledge` from here does not fail; it silently ends
/// event promotion, and the next thing the user notices is a graph that stopped
/// growing an hour ago.
///
/// The two NAMED lists only, deliberately not the `arlen-` prefix: prefixed
/// binaries include ordinary first-party apps (the file manager, the terminal),
/// and a warning that fires on a text editor teaches people to click through it.
///
/// This is a name list, not an attested property. It is what the tree has - there
/// is no critical-process flag to read - and a daemon renamed without touching
/// this list stops warning. `classify_group` carries the same exposure and the
/// same lists, so the two drift together rather than apart. Pure.
pub fn is_critical(name: &str) -> bool {
    SYSTEM.contains(&name) || BACKGROUND.contains(&name)
}

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
                critical: is_critical(&d.name),
                status: map_status(d.state),
                cpu,
                mem_mb: d.mem_kb as f64 / 1024.0,
                disk_kbs: disk,
                net_kbs: 0.0,
                // The flat list is flat. Grouping is `group_processes`, applied
                // to this output, so the sample itself carries no hierarchy.
                children: None,
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

/// Rank a status so the worst one wins a group. Higher is worse.
fn status_rank(status: &str) -> u8 {
    match status {
        "not-responding" => 2,
        "suspended" => 1,
        _ => 0,
    }
}

/// Fold the flat per-pid rows into the app rows the landing view opens on.
///
/// The plan's first bullet, and first for a reason: "Chrome is one Chrome row,
/// not 15 nameless PIDs". Someone opening this to find the frozen thing is
/// looking for a NAME they recognise, and a flat pid list hides that name among
/// its own children. The flat list stays as the power-user toggle; it is just
/// not what opens.
///
/// Returns `Process` rather than a new type, because the frontend's own model
/// already carries `children?` - a parallel `AppRow` would be a second contract
/// describing the same rows, and the table would have to learn both.
///
/// KEYED ON `(group, name)`, NOT ON LINEAGE. Lineage is the textbook answer and
/// the wrong one here: a browser's renderers get re-parented, a daemon's workers
/// are not its children, and a pid tree files the same program in several places
/// depending on who forked it. The name is what the user reads off the row, so
/// the name is what the row is. Grouping never spans the three groups either -
/// that division is the plan's own, and merging across it would file a daemon
/// under an app.
///
/// The parent's `id` is the LOWEST member pid: a real pid rather than a
/// synthetic one, so a Stop on the row has something to aim at and a detail pane
/// opening from it lands on a process that exists.
///
/// Sorted CPU-desc then memory-desc, the same order as the flat list, so the
/// toggle between the two does not reshuffle the top of the screen.
pub fn group_processes(rows: &[Process]) -> Vec<Process> {
    let mut out: Vec<Process> = Vec::new();
    for p in rows {
        match out
            .iter_mut()
            .find(|r| r.name == p.name && r.group == p.group)
        {
            Some(r) => {
                r.cpu += p.cpu;
                r.mem_mb += p.mem_mb;
                r.disk_kbs += p.disk_kbs;
                r.net_kbs += p.net_kbs;
                r.id = r.id.min(p.id);
                // The worst status wins, not the first seen or the commonest: one
                // frozen tab in a browser that is otherwise fine is exactly the
                // case the landing view exists for, and averaging buries it.
                if status_rank(p.status) > status_rank(r.status) {
                    r.status = p.status;
                }
                r.children.get_or_insert_with(Vec::new).push(p.clone());
            }
            None => out.push(Process {
                children: None,
                ..p.clone()
            }),
        }
    }
    for r in &mut out {
        if let Some(kids) = r.children.as_mut() {
            kids.sort_by_key(|k| k.id);
        }
    }
    out.sort_by(|a, b| {
        b.cpu
            .partial_cmp(&a.cpu)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(
                b.mem_mb
                    .partial_cmp(&a.mem_mb)
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
    });
    out
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

    #[test]
    fn the_daemons_that_carry_the_session_are_flagged() {
        // A row you can stop, that warns first. Both lists, because losing the
        // compositor and losing the event bus are both more than losing a window.
        assert!(is_critical("knowledge"));
        assert!(is_critical("event-bus"));
        assert!(is_critical("cosmic-comp"));
        assert!(is_critical("pipewire"));
    }

    #[test]
    fn an_ordinary_first_party_app_is_not_flagged() {
        // The clause the `arlen-` prefix would get wrong. These are apps: closing
        // the file manager closes the file manager. A warning here would be the
        // kind people learn to click through, which is how the real warning stops
        // working.
        assert!(!is_critical("arlen-files"));
        assert!(!is_critical("arlen-terminal"));
        assert!(!is_critical("firefox"));
        // ...while `classify_group` still files them as background by prefix, so
        // the two rules are deliberately different questions about the same name.
        assert_eq!(classify_group("arlen-files"), "background");
    }

    #[test]
    fn the_flag_travels_on_the_row_through_grouping() {
        // The frontend must never re-derive this from a name it holds. A grouped
        // row keeps the member's flag, so a daemon with helper processes still
        // warns.
        let rows = vec![
            row(1, "knowledge", "background", "running", 1.0, 1.0),
            row(2, "knowledge", "background", "running", 1.0, 1.0),
        ];
        let out = group_processes(&rows);
        assert_eq!(out.len(), 1);
        assert!(out[0].critical);
        assert!(!row(3, "firefox", "app", "running", 1.0, 1.0).critical);
    }

    fn row(id: u32, name: &str, group: &'static str, status: &'static str, cpu: f64, mem: f64) -> Process {
        Process {
            critical: is_critical(name),
            id,
            name: name.to_string(),
            group,
            status,
            cpu,
            mem_mb: mem,
            disk_kbs: 0.0,
            net_kbs: 0.0,
            children: None,
        }
    }

    /// The plan's own example, and the whole reason the default view is grouped.
    #[test]
    fn fifteen_chrome_pids_are_one_chrome_row() {
        let flat: Vec<Process> = (1..=15)
            .map(|i| row(i, "chrome", "app", "running", 2.0, 100.0))
            .collect();
        let grouped = group_processes(&flat);
        assert_eq!(grouped.len(), 1, "one row, not fifteen");
        assert_eq!(grouped[0].children.as_ref().unwrap().len(), 14, "the row plus fourteen more");
        assert_eq!(grouped[0].cpu, 30.0, "the resource is what it costs together");
        assert_eq!(grouped[0].mem_mb, 1500.0);
        assert_eq!(grouped[0].id, 1, "the row takes the eldest pid, a real one");
    }

    /// The case the landing view exists for: one frozen child in an app that is
    /// otherwise fine has to reach the row a person is looking at.
    #[test]
    fn one_frozen_child_makes_the_whole_row_read_as_frozen() {
        let flat = [
            row(1, "chrome", "app", "running", 1.0, 10.0),
            row(2, "chrome", "app", "not-responding", 1.0, 10.0),
            row(3, "chrome", "app", "running", 1.0, 10.0),
        ];
        let grouped = group_processes(&flat);
        assert_eq!(grouped[0].status, "not-responding");
    }

    /// Suspended is worse than running and better than frozen, so a row carrying
    /// both a suspended and a running member says suspended.
    #[test]
    fn the_worst_status_wins_and_not_the_first_one_seen() {
        let flat = [
            row(1, "x", "app", "running", 0.0, 0.0),
            row(2, "x", "app", "suspended", 0.0, 0.0),
        ];
        assert_eq!(group_processes(&flat)[0].status, "suspended");
        // And the other order, so this is not passing on iteration luck.
        let flat = [
            row(1, "x", "app", "suspended", 0.0, 0.0),
            row(2, "x", "app", "running", 0.0, 0.0),
        ];
        assert_eq!(group_processes(&flat)[0].status, "suspended");
    }

    /// A name shared across two groups is two rows. Merging them would file a
    /// daemon under an app, which is the one thing the three groups are for.
    #[test]
    fn the_same_name_in_two_groups_stays_two_rows() {
        let flat = [
            row(1, "helper", "app", "running", 5.0, 0.0),
            row(2, "helper", "background", "running", 1.0, 0.0),
        ];
        let grouped = group_processes(&flat);
        assert_eq!(grouped.len(), 2);
        assert_eq!(grouped[0].group, "app", "and the busier one is on top");
    }

    /// The toggle between grouped and flat must not reshuffle the top of the
    /// screen, so both are ordered the same way.
    #[test]
    fn rows_are_sorted_by_summed_cpu_so_the_hog_is_on_top() {
        let flat = [
            row(1, "quiet", "app", "running", 40.0, 0.0),
            row(2, "busy", "app", "running", 25.0, 0.0),
            row(3, "busy", "app", "running", 25.0, 0.0),
        ];
        let grouped = group_processes(&flat);
        assert_eq!(grouped[0].name, "busy", "50 summed beats 40 alone");
        assert_eq!(grouped[0].cpu, 50.0);
    }

    /// An ordinary single process is an ordinary row: same numbers, count 1.
    #[test]
    fn a_lone_process_survives_grouping_unchanged() {
        let flat = [row(7, "solo", "system", "running", 3.5, 12.0)];
        let grouped = group_processes(&flat);
        assert_eq!(grouped.len(), 1);
        assert!(grouped[0].children.is_none(), "a lone process gets no expander");
        assert_eq!(grouped[0].cpu, 3.5);
        assert_eq!(grouped[0].id, 7);
    }

    #[test]
    fn no_processes_is_no_rows_rather_than_a_panic() {
        assert!(group_processes(&[]).is_empty());
    }
}

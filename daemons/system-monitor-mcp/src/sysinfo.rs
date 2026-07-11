//! Read-only system information from `/proc`, for the System Monitor MCP
//! server. The parsers are pure and the reader takes a configurable `/proc`
//! root, so everything is unit-tested against fixture content without a live
//! system. This is the same public information `ps`/`top`/`uptime` show any
//! user, so there is no per-path scope to enforce (unlike the File Manager).

use std::path::{Path, PathBuf};

use serde::Serialize;

/// How many active processes to surface. The AI asks "what is running", not
/// for the full process table, so a bounded list of the genuinely-active ones
/// is the point.
const MAX_PROCESSES: usize = 50;

/// One active process.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProcessInfo {
    /// The process/application name (`/proc/<pid>/comm`).
    pub name: String,
    /// What it is doing now: `running` or `waiting on I/O`.
    pub state: &'static str,
}

/// A point-in-time resource snapshot.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ResourceUsage {
    /// Load average over 1, 5 and 15 minutes.
    pub load1: f64,
    pub load5: f64,
    pub load15: f64,
    /// Total and available memory, in kibibytes (as `/proc/meminfo` reports).
    pub mem_total_kb: u64,
    pub mem_available_kb: u64,
    /// Coarse human-readable forms of the two memory figures (e.g. `"15.6 GiB"`),
    /// so a consumer can show memory without re-deriving units from the raw kB.
    pub mem_total_human: String,
    pub mem_available_human: String,
}

/// The process state character from a `/proc/<pid>/stat` line. The format is
/// `pid (comm) state ...`, and `comm` can contain spaces and parentheses, so
/// the state is the first non-space char after the **last** `)`. Pure.
fn proc_state(stat: &str) -> Option<char> {
    let close = stat.rfind(')')?;
    stat[close + 1..].trim_start().chars().next()
}

/// Decide whether a process is active and worth surfacing, from its `/proc`
/// files. Kernel threads (empty `cmdline`) are skipped; only Running or
/// uninterruptible-I/O-wait processes count as active. Pure, so the heuristic
/// is unit-tested.
fn select_process(comm: &str, cmdline: &str, stat: &str) -> Option<ProcessInfo> {
    if cmdline.trim().is_empty() {
        return None;
    }
    let state = match proc_state(stat)? {
        'R' => "running",
        'D' => "waiting on I/O",
        _ => return None,
    };
    let name = comm.trim();
    if name.is_empty() {
        return None;
    }
    Some(ProcessInfo {
        name: name.to_string(),
        state,
    })
}

/// Parse the first three whitespace-separated floats of `/proc/loadavg`
/// (`"0.52 0.40 0.31 1/853 12345"`). Pure.
fn parse_loadavg(text: &str) -> Option<(f64, f64, f64)> {
    let mut it = text.split_whitespace();
    let one = it.next()?.parse().ok()?;
    let five = it.next()?.parse().ok()?;
    let fifteen = it.next()?.parse().ok()?;
    Some((one, five, fifteen))
}

/// The kibibyte count from a `/proc/meminfo` value line tail (`"  16334072 kB"`).
fn parse_kb(rest: &str) -> Option<u64> {
    rest.split_whitespace().next()?.parse().ok()
}

/// Parse `MemTotal` and `MemAvailable` (in kB) from `/proc/meminfo`. Either may
/// be absent (returned as `None`). Pure.
fn parse_meminfo(text: &str) -> (Option<u64>, Option<u64>) {
    let mut total = None;
    let mut available = None;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("MemTotal:") {
            total = parse_kb(rest);
        } else if let Some(rest) = line.strip_prefix("MemAvailable:") {
            available = parse_kb(rest);
        }
    }
    (total, available)
}

/// Format a kibibyte count (as `/proc/meminfo` reports) as a coarse
/// human-readable size: GiB to one decimal at or above 1 GiB, whole MiB below.
/// Pure.
pub fn format_kib(kib: u64) -> String {
    const MIB: u64 = 1024;
    const GIB: u64 = 1024 * 1024;
    if kib >= GIB {
        format!("{:.1} GiB", kib as f64 / GIB as f64)
    } else {
        format!("{} MiB", kib / MIB)
    }
}

/// System uptime since boot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Uptime {
    /// Whole seconds since boot.
    pub seconds: u64,
    /// A coarse human-readable form, e.g. `"3d 4h 12m"`.
    pub human: String,
}

/// Parse the uptime in seconds: the first whitespace-separated float of
/// `/proc/uptime` (`"12345.67 9876.54"`, uptime then idle). Pure.
fn parse_uptime(text: &str) -> Option<f64> {
    text.split_whitespace().next()?.parse().ok()
}

/// Format whole seconds as a coarse `"Nd Nh Nm"` string. Days and hours are
/// shown only when non-zero; minutes are always shown, so an uptime under a
/// minute reads as `"0m"`. Pure.
pub fn format_uptime(seconds: u64) -> String {
    let days = seconds / 86_400;
    let hours = (seconds % 86_400) / 3_600;
    let mins = (seconds % 3_600) / 60;
    let mut parts = Vec::new();
    if days > 0 {
        parts.push(format!("{days}d"));
    }
    if hours > 0 {
        parts.push(format!("{hours}h"));
    }
    parts.push(format!("{mins}m"));
    parts.join(" ")
}

/// One network interface and its cumulative traffic counters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NetworkInterface {
    /// Interface name, e.g. `"eth0"` / `"lo"`.
    pub name: String,
    /// Cumulative received bytes since boot.
    pub rx_bytes: u64,
    /// Cumulative transmitted bytes since boot.
    pub tx_bytes: u64,
}

/// Parse `/proc/net/dev`: two header lines, then one line per interface
/// (`"  name: rx_bytes rx_packets ... tx_bytes tx_packets ..."`). The rx byte
/// count is the first stat field and the tx byte count is the ninth. A
/// malformed line is skipped, never an error. Pure.
fn parse_net_dev(text: &str) -> Vec<NetworkInterface> {
    let mut out = Vec::new();
    for line in text.lines().skip(2) {
        let Some((name, rest)) = line.split_once(':') else {
            continue;
        };
        let name = name.trim();
        if name.is_empty() {
            continue;
        }
        let fields: Vec<&str> = rest.split_whitespace().collect();
        if fields.len() < 9 {
            continue;
        }
        let rx_bytes = fields[0].parse().unwrap_or(0);
        let tx_bytes = fields[8].parse().unwrap_or(0);
        out.push(NetworkInterface {
            name: name.to_string(),
            rx_bytes,
            tx_bytes,
        });
    }
    out
}

/// Operating-system identity, from `/proc/sys/kernel/`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OsInfo {
    /// Kernel name, e.g. `"Linux"` (`/proc/sys/kernel/ostype`).
    pub kernel: String,
    /// Kernel release, e.g. `"6.9.3-arch1-1"` (`/proc/sys/kernel/osrelease`).
    pub kernel_release: String,
    /// Hostname (`/proc/sys/kernel/hostname`).
    pub hostname: String,
}

/// Disk usage of one filesystem.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DiskUsage {
    /// The path queried (the filesystem containing it is reported).
    pub path: String,
    /// Total and available bytes on that filesystem.
    pub total_bytes: u64,
    pub available_bytes: u64,
}

/// Total and available bytes from raw `statvfs` block counts and fragment
/// size. Pure (and overflow-guarded), so the arithmetic is unit-tested without
/// the syscall. `available` uses `f_bavail` (blocks free to unprivileged
/// users), the number that matters for "can I still write here".
fn bytes_from_statvfs(blocks: u64, bavail: u64, frsize: u64) -> (u64, u64) {
    (
        blocks.saturating_mul(frsize),
        bavail.saturating_mul(frsize),
    )
}

/// Disk usage of the filesystem containing `path` (e.g. `/`), via `statvfs`.
/// Returns `None` if the path is unrepresentable or the syscall fails.
pub fn disk_usage(path: &str) -> Option<DiskUsage> {
    let c_path = std::ffi::CString::new(path).ok()?;
    // SAFETY: `statvfs` fills a caller-owned, zeroed buffer and reports success
    // via its return code, which is checked before any field is read.
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    let rc = unsafe { libc::statvfs(c_path.as_ptr(), &mut stat) };
    if rc != 0 {
        return None;
    }
    let (total_bytes, available_bytes) = bytes_from_statvfs(
        stat.f_blocks as u64,
        stat.f_bavail as u64,
        stat.f_frsize as u64,
    );
    Some(DiskUsage {
        path: path.to_string(),
        total_bytes,
        available_bytes,
    })
}

/// A coarse battery/AC snapshot read from `/sys/class/power_supply` (the same
/// kernel surface `upower`/`acpi` use). A laptop exposes a Battery supply
/// (`capacity` + `status`) and a Mains supply (`online` = AC connected); a
/// desktop has neither battery, reported `battery_present: false`. Mirrors the
/// `org.arlen.Power1` shape so the AI's answer matches the power-daemon's, but is
/// read directly here so the MCP surface stays self-contained (no daemon dep).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PowerSupply {
    /// Whether a battery supply was found.
    pub battery_present: bool,
    /// Battery charge 0-100 (the first battery's `capacity`), if present.
    pub percentage: Option<u8>,
    /// Battery status string (`Charging`/`Discharging`/`Full`/`Not charging`/
    /// `Unknown`), if present.
    pub status: Option<String>,
    /// Whether mains/AC power is online (any Mains supply reporting `online` = 1).
    pub on_ac: bool,
}

/// Read the power-supply snapshot from `root` (the real path is
/// `/sys/class/power_supply`; configurable for fixture tests). Missing or
/// unreadable files are skipped, never an error: a machine with no
/// power-supply tree reports `battery_present: false, on_ac: false`. The first
/// battery supply found wins (the coarse surface answers "on battery? charge?",
/// not per-cell detail); read-dir order is unspecified, so a multi-battery
/// machine reports one of them.
pub fn read_power_supply(root: &Path) -> PowerSupply {
    let mut out = PowerSupply {
        battery_present: false,
        percentage: None,
        status: None,
        on_ac: false,
    };
    let Ok(entries) = std::fs::read_dir(root) else {
        return out;
    };
    let read_trim = |dir: &Path, field: &str| -> Option<String> {
        std::fs::read_to_string(dir.join(field))
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    };
    for entry in entries.flatten() {
        let dir = entry.path();
        let typ = read_trim(&dir, "type").unwrap_or_default();
        if typ.eq_ignore_ascii_case("Battery") {
            if !out.battery_present {
                out.battery_present = true;
                out.percentage = read_trim(&dir, "capacity")
                    .and_then(|s| s.parse::<u8>().ok())
                    .map(|p| p.min(100));
                out.status = read_trim(&dir, "status");
            }
        } else if typ.eq_ignore_ascii_case("Mains") || typ.eq_ignore_ascii_case("USB") {
            // An AC adapter (Mains) or a USB-PD source counts as wall power when
            // it reports itself online.
            if read_trim(&dir, "online").as_deref() == Some("1") {
                out.on_ac = true;
            }
        }
    }
    out
}

/// Reads `/proc` (root configurable for tests). All reads are local and fast,
/// and a vanished PID or missing file is just skipped, never an error.
pub struct ProcReader {
    root: PathBuf,
}

impl ProcReader {
    /// A reader over the real `/proc`.
    pub fn new() -> Self {
        Self {
            root: PathBuf::from("/proc"),
        }
    }

    /// A reader over an alternate root (for tests with a fixture tree).
    pub fn with_root(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// The currently-active processes, capped at [`MAX_PROCESSES`]. Returns an
    /// empty list if `/proc` cannot be read.
    pub fn list_processes(&self) -> Vec<ProcessInfo> {
        let Ok(entries) = std::fs::read_dir(&self.root) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for entry in entries.flatten() {
            if out.len() >= MAX_PROCESSES {
                break;
            }
            let file_name = entry.file_name();
            let Some(pid) = file_name
                .to_str()
                .filter(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
            else {
                continue;
            };
            let dir = self.root.join(pid);
            let comm = std::fs::read_to_string(dir.join("comm")).unwrap_or_default();
            let cmdline = std::fs::read(dir.join("cmdline")).unwrap_or_default();
            let cmdline = String::from_utf8_lossy(&cmdline);
            let stat = std::fs::read_to_string(dir.join("stat")).unwrap_or_default();
            if let Some(p) = select_process(&comm, &cmdline, &stat) {
                out.push(p);
            }
        }
        out
    }

    /// The current resource snapshot. A missing/unreadable source reads as
    /// zero rather than failing (the surface is informational).
    pub fn resource_usage(&self) -> ResourceUsage {
        let load = std::fs::read_to_string(self.root.join("loadavg"))
            .ok()
            .and_then(|t| parse_loadavg(&t))
            .unwrap_or((0.0, 0.0, 0.0));
        let (total, available) = std::fs::read_to_string(self.root.join("meminfo"))
            .ok()
            .map(|t| parse_meminfo(&t))
            .unwrap_or((None, None));
        let mem_total_kb = total.unwrap_or(0);
        let mem_available_kb = available.unwrap_or(0);
        ResourceUsage {
            load1: load.0,
            load5: load.1,
            load15: load.2,
            mem_total_kb,
            mem_available_kb,
            mem_total_human: format_kib(mem_total_kb),
            mem_available_human: format_kib(mem_available_kb),
        }
    }

    /// System uptime, or `None` if `/proc/uptime` is unreadable or unparseable.
    pub fn uptime(&self) -> Option<Uptime> {
        let secs = std::fs::read_to_string(self.root.join("uptime"))
            .ok()
            .and_then(|t| parse_uptime(&t))?;
        let seconds = secs.max(0.0) as u64;
        Some(Uptime {
            seconds,
            human: format_uptime(seconds),
        })
    }

    /// Network interfaces and their cumulative traffic counters, from
    /// `/proc/net/dev`. Empty if the file cannot be read; the same system-wide
    /// public information `ip -s link` shows.
    pub fn network_interfaces(&self) -> Vec<NetworkInterface> {
        std::fs::read_to_string(self.root.join("net/dev"))
            .map(|t| parse_net_dev(&t))
            .unwrap_or_default()
    }

    /// OS identity (kernel name, release, hostname) from `/proc/sys/kernel/`.
    /// A missing or unreadable field reads as empty rather than failing; this is
    /// the same system-wide public information `uname` shows.
    pub fn os_info(&self) -> OsInfo {
        let read = |name: &str| {
            std::fs::read_to_string(self.root.join("sys/kernel").join(name))
                .map(|s| s.trim().to_string())
                .unwrap_or_default()
        };
        OsInfo {
            kernel: read("ostype"),
            kernel_release: read("osrelease"),
            hostname: read("hostname"),
        }
    }

    /// A detailed sample of every user process (kernel threads skipped), for the
    /// task manager. Each carries the raw counters the caller turns into rates
    /// (CPU% and I/O KB/s are deltas between two samples); a vanished PID or an
    /// unreadable file is skipped or reads as zero, never an error. Bounded so a
    /// pathological `/proc` cannot blow the response.
    pub fn list_processes_detailed(&self) -> Vec<ProcessDetail> {
        let Ok(entries) = std::fs::read_dir(&self.root) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for entry in entries.flatten() {
            if out.len() >= MAX_DETAILED_PROCESSES {
                break;
            }
            let file_name = entry.file_name();
            let Some(pid) = file_name
                .to_str()
                .filter(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
                .and_then(|s| s.parse::<u32>().ok())
            else {
                continue;
            };
            let dir = self.root.join(pid.to_string());
            let cmdline_raw = std::fs::read(dir.join("cmdline")).unwrap_or_default();
            // A kernel thread has an empty cmdline (all NUL or truly empty); skip it,
            // the task manager surfaces user processes.
            if cmdline_raw.iter().all(|&b| b == 0) {
                continue;
            }
            let cmdline = String::from_utf8_lossy(&cmdline_raw);
            let comm = std::fs::read_to_string(dir.join("comm")).unwrap_or_default();
            let stat = std::fs::read_to_string(dir.join("stat")).unwrap_or_default();
            let status = std::fs::read_to_string(dir.join("status")).unwrap_or_default();
            let io = std::fs::read_to_string(dir.join("io")).unwrap_or_default();
            let (io_read_bytes, io_write_bytes) = parse_io_bytes(&io);
            out.push(ProcessDetail {
                pid,
                name: process_name(&comm, &cmdline),
                state: proc_state(&stat).unwrap_or('?'),
                mem_kb: parse_vmrss_kb(&status).unwrap_or(0),
                cpu_jiffies: parse_stat_cpu_jiffies(&stat).unwrap_or(0),
                io_read_bytes,
                io_write_bytes,
            });
        }
        out
    }
}

/// The upper bound on the detailed process list (a desktop runs a few hundred;
/// this only guards against a pathological `/proc`).
const MAX_DETAILED_PROCESSES: usize = 4096;

/// A detailed per-process sample for the task manager: the raw counters it needs
/// to compute CPU%, memory and I/O rates. The rates (CPU% and KB/s) are deltas the
/// caller derives from two samples spaced by an interval; `state` is the raw
/// `/proc/<pid>/stat` state char, which the caller maps to a display status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProcessDetail {
    /// The process id.
    pub pid: u32,
    /// A display name (the first cmdline arg's basename, else `comm`).
    pub name: String,
    /// The raw `/proc/<pid>/stat` state char (`R`/`S`/`D`/`T`/`Z`/...).
    pub state: char,
    /// Resident memory in kibibytes (`VmRSS`).
    pub mem_kb: u64,
    /// CPU time in clock ticks (`utime`+`stime`), for a rate delta.
    pub cpu_jiffies: u64,
    /// Cumulative storage bytes read (0 if `/proc/<pid>/io` is unreadable, which it
    /// is for another user's process).
    pub io_read_bytes: u64,
    /// Cumulative storage bytes written.
    pub io_write_bytes: u64,
}

/// `utime`+`stime` (fields 14 and 15 of `/proc/<pid>/stat`), the process CPU time
/// in clock ticks. `comm` (field 2) may contain spaces and parentheses, so the
/// fields are counted from after the **last** `)`: state is token 0, so `utime`
/// (field 14) is token 11 and `stime` (field 15) is token 12. Pure.
fn parse_stat_cpu_jiffies(stat: &str) -> Option<u64> {
    let close = stat.rfind(')')?;
    let toks: Vec<&str> = stat[close + 1..].split_whitespace().collect();
    let utime: u64 = toks.get(11)?.parse().ok()?;
    let stime: u64 = toks.get(12)?.parse().ok()?;
    Some(utime + stime)
}

/// `VmRSS` in kB from `/proc/<pid>/status` (`"VmRSS:\t   12345 kB"`). Pure.
fn parse_vmrss_kb(status: &str) -> Option<u64> {
    status
        .lines()
        .find_map(|l| l.strip_prefix("VmRSS:"))
        .and_then(|rest| rest.split_whitespace().next()?.parse().ok())
}

/// `read_bytes` and `write_bytes` from `/proc/<pid>/io` (0 each if absent or
/// unreadable). Pure.
fn parse_io_bytes(io: &str) -> (u64, u64) {
    let mut read = 0;
    let mut write = 0;
    for line in io.lines() {
        if let Some(v) = line.strip_prefix("read_bytes:") {
            read = v.trim().parse().unwrap_or(0);
        } else if let Some(v) = line.strip_prefix("write_bytes:") {
            write = v.trim().parse().unwrap_or(0);
        }
    }
    (read, write)
}

/// A process display name: the first `cmdline` argument's basename if present
/// (NUL-separated args), else the `comm`. Pure.
fn process_name(comm: &str, cmdline: &str) -> String {
    let first = cmdline.split('\0').next().unwrap_or("").trim();
    if !first.is_empty() {
        let base = first.rsplit('/').next().unwrap_or(first);
        if !base.is_empty() {
            return base.to_string();
        }
    }
    comm.trim().to_string()
}

impl Default for ProcReader {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_jiffies_sum_utime_and_stime_past_a_parenthesised_comm() {
        // comm with spaces AND parens: state is the token after the last ')',
        // utime is field 14 (token 11), stime field 15 (token 12).
        let stat = "42 (weird (name) ) S 1 1 1 0 -1 0 10 20 30 40 111 222 5 6";
        assert_eq!(parse_stat_cpu_jiffies(stat), Some(333));
        assert_eq!(parse_stat_cpu_jiffies("bad"), None);
    }

    #[test]
    fn vmrss_and_io_parse_from_their_files() {
        let status = "Name:\tbash\nVmRSS:\t   12345 kB\nThreads:\t1\n";
        assert_eq!(parse_vmrss_kb(status), Some(12345));
        assert_eq!(parse_vmrss_kb("no rss here"), None);
        let io = "rchar: 100\nread_bytes: 4096\nwrite_bytes: 8192\n";
        assert_eq!(parse_io_bytes(io), (4096, 8192));
        assert_eq!(parse_io_bytes(""), (0, 0));
    }

    #[test]
    fn process_name_prefers_the_cmdline_basename() {
        assert_eq!(process_name("firefox", "/usr/lib/firefox/firefox\0-P\0"), "firefox");
        // No cmdline arg -> fall back to comm.
        assert_eq!(process_name("kworker", ""), "kworker");
    }

    #[test]
    fn detailed_list_reads_a_user_process_and_skips_a_kernel_thread() {
        let tmp = tempfile::tempdir().unwrap();
        let proc = |pid: &str| {
            let d = tmp.path().join(pid);
            std::fs::create_dir_all(&d).unwrap();
            d
        };
        // A user process: non-empty cmdline, VmRSS, cpu jiffies, io.
        let u = proc("100");
        std::fs::write(u.join("cmdline"), b"/usr/bin/bash\0").unwrap();
        std::fs::write(u.join("comm"), "bash\n").unwrap();
        std::fs::write(u.join("stat"), "100 (bash) R 1 1 1 0 -1 0 0 0 0 0 70 30 0 0").unwrap();
        std::fs::write(u.join("status"), "VmRSS:\t 2048 kB\n").unwrap();
        std::fs::write(u.join("io"), "read_bytes: 1024\nwrite_bytes: 512\n").unwrap();
        // A kernel thread: empty cmdline -> skipped.
        let k = proc("2");
        std::fs::write(k.join("cmdline"), b"").unwrap();
        std::fs::write(k.join("comm"), "kthreadd\n").unwrap();
        std::fs::write(k.join("stat"), "2 (kthreadd) S 0 0 0 0 -1 0 0 0 0 0 0 0 0 0").unwrap();

        let list = ProcReader::with_root(tmp.path()).list_processes_detailed();
        assert_eq!(list.len(), 1, "the kernel thread is skipped");
        let p = &list[0];
        assert_eq!(p.pid, 100);
        assert_eq!(p.name, "bash");
        assert_eq!(p.state, 'R');
        assert_eq!(p.mem_kb, 2048);
        assert_eq!(p.cpu_jiffies, 100);
        assert_eq!((p.io_read_bytes, p.io_write_bytes), (1024, 512));
    }

    #[test]
    fn proc_state_reads_the_char_after_the_last_paren() {
        assert_eq!(proc_state("1 (bash) R 0 ..."), Some('R'));
        assert_eq!(proc_state("2 (od d (x)) S 0 ..."), Some('S'));
        assert_eq!(proc_state("no paren here"), None);
    }

    #[test]
    fn select_keeps_active_user_processes() {
        assert_eq!(
            select_process("bash\n", "bash\0", "1 (bash) R 0").unwrap().state,
            "running"
        );
        assert_eq!(
            select_process("dd\n", "dd\0", "2 (dd) D 0").unwrap().state,
            "waiting on I/O"
        );
        assert!(select_process("bash\n", "bash\0", "3 (bash) S 0").is_none(), "sleeping skipped");
        assert!(select_process("kworker\n", "", "4 (kworker) R 0").is_none(), "kernel thread skipped");
    }

    #[test]
    fn parse_loadavg_reads_three_floats() {
        assert_eq!(parse_loadavg("0.52 0.40 0.31 1/853 12345"), Some((0.52, 0.40, 0.31)));
        assert_eq!(parse_loadavg("bad"), None);
    }

    #[test]
    fn parse_meminfo_reads_total_and_available() {
        let text = "MemTotal:       16334072 kB\nMemFree:  100 kB\nMemAvailable:    9000000 kB\n";
        assert_eq!(parse_meminfo(text), (Some(16_334_072), Some(9_000_000)));
        assert_eq!(parse_meminfo("nothing useful"), (None, None));
    }

    #[test]
    fn format_kib_is_coarse_gib_or_mib() {
        assert_eq!(format_kib(16_334_072), "15.6 GiB");
        assert_eq!(format_kib(1024 * 1024), "1.0 GiB");
        assert_eq!(format_kib(512 * 1024), "512 MiB");
        assert_eq!(format_kib(0), "0 MiB");
    }

    #[test]
    fn parse_uptime_reads_the_first_float() {
        assert_eq!(parse_uptime("12345.67 9876.54"), Some(12345.67));
        assert_eq!(parse_uptime("bad"), None);
    }

    #[test]
    fn format_uptime_is_coarse_and_always_shows_minutes() {
        assert_eq!(format_uptime(0), "0m");
        assert_eq!(format_uptime(59), "0m");
        assert_eq!(format_uptime(90), "1m");
        assert_eq!(format_uptime(3 * 3600 + 12 * 60), "3h 12m");
        assert_eq!(format_uptime(2 * 86_400 + 4 * 3600 + 5 * 60), "2d 4h 5m");
        // A whole-hour boundary still shows the (zero) minutes.
        assert_eq!(format_uptime(3600), "1h 0m");
    }

    #[test]
    fn parse_net_dev_reads_interfaces_and_byte_counters() {
        let text = "Inter-|   Receive                          |  Transmit\n \
                    face |bytes    packets errs drop fifo frame compressed multicast|bytes    packets\n  \
                    lo:    1000     10    0    0    0     0          0         0      1000      10\n  \
                    eth0:  50000    400   0    0    0     0          0         0     20000     300\n";
        let ifaces = parse_net_dev(text);
        assert_eq!(ifaces.len(), 2);
        assert_eq!(ifaces[0].name, "lo");
        assert_eq!(ifaces[1].name, "eth0");
        assert_eq!(ifaces[1].rx_bytes, 50000);
        assert_eq!(ifaces[1].tx_bytes, 20000);
    }

    #[test]
    fn reader_reads_network_interfaces_from_proc() {
        let tmp = tempfile::tempdir().unwrap();
        let net = tmp.path().join("net");
        std::fs::create_dir_all(&net).unwrap();
        std::fs::write(
            net.join("dev"),
            "h1\nh2\n  lo: 5 1 0 0 0 0 0 0 5 1\n",
        )
        .unwrap();
        let ifaces = ProcReader::with_root(tmp.path()).network_interfaces();
        assert_eq!(ifaces.len(), 1);
        assert_eq!(ifaces[0].name, "lo");
    }

    #[test]
    fn reader_reads_os_info_from_proc_sys_kernel() {
        let tmp = tempfile::tempdir().unwrap();
        let k = tmp.path().join("sys/kernel");
        std::fs::create_dir_all(&k).unwrap();
        std::fs::write(k.join("ostype"), "Linux\n").unwrap();
        std::fs::write(k.join("osrelease"), "6.9.3-arch1-1\n").unwrap();
        std::fs::write(k.join("hostname"), "arlen-box\n").unwrap();
        let info = ProcReader::with_root(tmp.path()).os_info();
        assert_eq!(info.kernel, "Linux");
        assert_eq!(info.kernel_release, "6.9.3-arch1-1");
        assert_eq!(info.hostname, "arlen-box");
    }

    #[test]
    fn os_info_missing_fields_read_empty_not_error() {
        let tmp = tempfile::tempdir().unwrap();
        let info = ProcReader::with_root(tmp.path()).os_info();
        assert_eq!(info.kernel, "");
        assert_eq!(info.hostname, "");
    }

    #[test]
    fn reader_reads_uptime_from_proc() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::write(root.join("uptime"), "9000.42 5000.00\n").unwrap();
        let up = ProcReader::with_root(root).uptime().unwrap();
        assert_eq!(up.seconds, 9000);
        assert_eq!(up.human, "2h 30m");
        // No uptime file -> None.
        let tmp2 = tempfile::tempdir().unwrap();
        assert!(ProcReader::with_root(tmp2.path()).uptime().is_none());
    }

    #[test]
    fn bytes_from_statvfs_multiplies_and_guards_overflow() {
        assert_eq!(bytes_from_statvfs(100, 40, 4096), (409_600, 163_840));
        // Overflow saturates rather than wrapping.
        assert_eq!(bytes_from_statvfs(u64::MAX, u64::MAX, 4096), (u64::MAX, u64::MAX));
    }

    #[test]
    fn disk_usage_reads_the_root_filesystem() {
        // "/" exists on any build/CI host, so this exercises the real syscall.
        let usage = disk_usage("/").expect("root filesystem statvfs");
        assert_eq!(usage.path, "/");
        assert!(usage.total_bytes > 0);
        assert!(usage.available_bytes <= usage.total_bytes);
        // A nonexistent path fails cleanly to None.
        assert!(disk_usage("/no/such/path/here/at/all").is_none());
    }

    #[test]
    fn reader_lists_active_processes_and_reads_resources() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let p = root.join("100");
        std::fs::create_dir(&p).unwrap();
        std::fs::write(p.join("comm"), "myapp\n").unwrap();
        std::fs::write(p.join("cmdline"), "myapp\0").unwrap();
        std::fs::write(p.join("stat"), "100 (myapp) R 1").unwrap();
        // A kernel thread, skipped.
        let k = root.join("200");
        std::fs::create_dir(&k).unwrap();
        std::fs::write(k.join("comm"), "kthreadd\n").unwrap();
        std::fs::write(k.join("cmdline"), "").unwrap();
        std::fs::write(k.join("stat"), "200 (kthreadd) R 0").unwrap();
        std::fs::write(root.join("loadavg"), "1.0 0.5 0.25 1/100 999").unwrap();
        std::fs::write(root.join("meminfo"), "MemTotal: 8000 kB\nMemAvailable: 4000 kB\n").unwrap();

        let reader = ProcReader::with_root(root);
        let procs = reader.list_processes();
        assert_eq!(procs.len(), 1);
        assert_eq!(procs[0].name, "myapp");

        let res = reader.resource_usage();
        assert_eq!(res.load1, 1.0);
        assert_eq!(res.mem_total_kb, 8000);
        assert_eq!(res.mem_available_kb, 4000);
    }

    /// Write one `/sys/class/power_supply/<name>` supply directory.
    fn supply(root: &Path, name: &str, fields: &[(&str, &str)]) {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        for (k, v) in fields {
            std::fs::write(dir.join(k), format!("{v}\n")).unwrap();
        }
    }

    #[test]
    fn power_supply_reads_a_discharging_laptop() {
        let tmp = tempfile::tempdir().unwrap();
        supply(tmp.path(), "BAT0", &[("type", "Battery"), ("capacity", "73"), ("status", "Discharging")]);
        supply(tmp.path(), "AC", &[("type", "Mains"), ("online", "0")]);
        let p = read_power_supply(tmp.path());
        assert!(p.battery_present);
        assert_eq!(p.percentage, Some(73));
        assert_eq!(p.status.as_deref(), Some("Discharging"));
        assert!(!p.on_ac);
    }

    #[test]
    fn power_supply_reports_ac_when_mains_is_online() {
        let tmp = tempfile::tempdir().unwrap();
        supply(tmp.path(), "BAT0", &[("type", "Battery"), ("capacity", "80"), ("status", "Charging")]);
        supply(tmp.path(), "AC", &[("type", "Mains"), ("online", "1")]);
        let p = read_power_supply(tmp.path());
        assert_eq!(p.percentage, Some(80));
        assert_eq!(p.status.as_deref(), Some("Charging"));
        assert!(p.on_ac);
    }

    #[test]
    fn power_supply_on_a_desktop_has_no_battery() {
        let tmp = tempfile::tempdir().unwrap();
        supply(tmp.path(), "AC", &[("type", "Mains"), ("online", "1")]);
        let p = read_power_supply(tmp.path());
        assert!(!p.battery_present);
        assert_eq!(p.percentage, None);
        assert!(p.on_ac);
    }

    #[test]
    fn power_supply_missing_tree_is_all_absent_not_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        let p = read_power_supply(&tmp.path().join("nope"));
        assert!(!p.battery_present);
        assert!(!p.on_ac);
        assert_eq!(p.percentage, None);
    }
}

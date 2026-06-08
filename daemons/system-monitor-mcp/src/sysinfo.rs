//! Read-only system information from `/proc`, for the System Monitor MCP
//! server. The parsers are pure and the reader takes a configurable `/proc`
//! root, so everything is unit-tested against fixture content without a live
//! system. This is the same public information `ps`/`top`/`uptime` show any
//! user, so there is no per-path scope to enforce (unlike the File Manager).

use std::path::PathBuf;

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
        ResourceUsage {
            load1: load.0,
            load5: load.1,
            load15: load.2,
            mem_total_kb: total.unwrap_or(0),
            mem_available_kb: available.unwrap_or(0),
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
}

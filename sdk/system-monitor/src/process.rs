//! Process reading over `/proc`, WRITE-CLEAN (parsed directly, no GPL lib). The
//! task-manager landing's data core (system-monitor-plan.md §a): the process list
//! plus the raw counters CPU% and memory are derived from.

use std::fs;

/// A process's core `/proc/[pid]/stat` fields. CPU% is not here: it is a rate
/// (delta utime+stime over delta total CPU time between two samples), computed by
/// a sampler that holds successive [`ProcStat`]s, not readable from one snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcStat {
    /// Process id.
    pub pid: i32,
    /// Executable name (from field 2). May contain spaces and parentheses.
    pub comm: String,
    /// Run state: `R` running, `S` sleeping, `D` uninterruptible, `Z` zombie,
    /// `T` stopped, `t` traced, `I` idle.
    pub state: char,
    /// Parent process id.
    pub ppid: i32,
    /// User-mode CPU time, in clock ticks.
    pub utime: u64,
    /// Kernel-mode CPU time, in clock ticks.
    pub stime: u64,
    /// Number of threads.
    pub num_threads: i64,
    /// Resident set size, in pages (multiply by the page size for bytes).
    pub rss_pages: i64,
}

/// Parse one `/proc/[pid]/stat` line. Field 2 (`comm`) is paren-wrapped and may
/// itself contain spaces AND parentheses (e.g. `(Web Content)`, `(a) b`), so it is
/// taken as everything between the FIRST `(` and the LAST `)`, and the numeric
/// fields are read from after the last `)`. Returns `None` on a malformed line.
pub fn parse_proc_stat(line: &str) -> Option<ProcStat> {
    let open = line.find('(')?;
    let close = line.rfind(')')?;
    if close <= open {
        return None;
    }
    let pid: i32 = line[..open].trim().parse().ok()?;
    let comm = line[open + 1..close].to_string();
    // Fields from `state` on (proc(5) field 3 onward); `rest[n]` is field `n + 3`.
    let rest: Vec<&str> = line[close + 1..].split_whitespace().collect();
    Some(ProcStat {
        pid,
        comm,
        state: rest.first()?.chars().next()?, // field 3
        ppid: rest.get(1)?.parse().ok()?,     // field 4
        utime: rest.get(11)?.parse().ok()?,   // field 14
        stime: rest.get(12)?.parse().ok()?,   // field 15
        num_threads: rest.get(17)?.parse().ok()?, // field 20
        rss_pages: rest.get(21)?.parse().ok()?,   // field 24
    })
}

/// Read and parse `/proc/[pid]/stat` for one process. `None` if the process is
/// gone or its stat is unreadable/malformed.
pub fn read_proc_stat(pid: i32) -> Option<ProcStat> {
    let line = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    parse_proc_stat(&line)
}

/// List the currently-running processes by reading the numeric entries under
/// `/proc`. Processes that exit mid-scan are skipped (a live list is never a
/// perfect instant). Empty if `/proc` is unreadable.
pub fn list_processes() -> Vec<ProcStat> {
    let Ok(entries) = fs::read_dir("/proc") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        if let Some(pid) = entry.file_name().to_str().and_then(|n| n.parse::<i32>().ok()) {
            if let Some(stat) = read_proc_stat(pid) {
                out.push(stat);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_simple_stat_line() {
        let line = "1234 (bash) S 1000 1234 1234 0 -1 4194304 100 0 0 0 5 3 0 0 20 0 1 0 999 12345678 512 18446744073709551615";
        let s = parse_proc_stat(line).unwrap();
        assert_eq!(s.pid, 1234);
        assert_eq!(s.comm, "bash");
        assert_eq!(s.state, 'S');
        assert_eq!(s.ppid, 1000);
        assert_eq!(s.utime, 5);
        assert_eq!(s.stime, 3);
        assert_eq!(s.num_threads, 1);
        assert_eq!(s.rss_pages, 512);
    }

    #[test]
    fn comm_with_spaces_and_parens_does_not_break_the_field_split() {
        // The classic /proc/stat gotcha: comm can contain spaces and parens.
        let line = "42 (Web Content (tab)) R 100 42 42 0 -1 0 200 0 0 0 9 7 0 0 20 0 4 0 555 22222222 1024 0";
        let s = parse_proc_stat(line).unwrap();
        assert_eq!(s.pid, 42);
        assert_eq!(s.comm, "Web Content (tab)");
        assert_eq!(s.state, 'R');
        assert_eq!(s.ppid, 100);
        assert_eq!(s.utime, 9);
        assert_eq!(s.stime, 7);
        assert_eq!(s.num_threads, 4);
        assert_eq!(s.rss_pages, 1024);
    }

    #[test]
    fn a_malformed_line_is_rejected() {
        assert!(parse_proc_stat("not a stat line").is_none());
        assert!(parse_proc_stat("123 (only-comm)").is_none());
        assert!(parse_proc_stat("").is_none());
    }

    #[test]
    fn lists_at_least_this_process_from_real_proc() {
        // Smoke test against the real /proc: our own pid is present and named.
        let me = std::process::id() as i32;
        let mine = read_proc_stat(me).expect("own /proc stat must be readable");
        assert_eq!(mine.pid, me);
        assert!(!mine.comm.is_empty());
        assert!(list_processes().iter().any(|p| p.pid == me));
    }
}

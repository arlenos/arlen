//! Per-process detail beyond sysinfo's easy fields (system-monitor-plan.md item 3,
//! §e "genuinely hard: honest per-process memory"). PSS (proportional set size)
//! from `/proc/[pid]/smaps_rollup` splits each shared page by how many processes
//! map it, so - unlike RSS, which counts a shared page in full for every sharer -
//! summing PSS across processes does not double-count shared memory. It is read on
//! demand (smaps_rollup is costlier than the RSS shown in the list). WRITE-CLEAN.

use std::fs;

/// Parse the total `Pss:` value (in kB) from `/proc/[pid]/smaps_rollup` content.
/// Matches only the `Pss:` line, never the `Pss_Anon:`/`Pss_File:`/`Pss_Shmem:`
/// breakdown lines. `None` if there is no `Pss:` line or its value is unparseable.
pub fn parse_pss_kb(smaps_rollup: &str) -> Option<u64> {
    for line in smaps_rollup.lines() {
        if let Some(rest) = line.strip_prefix("Pss:") {
            // e.g. "Pss:               1234 kB"
            return rest.split_whitespace().next()?.parse().ok();
        }
    }
    None
}

/// Read a process's PSS (proportional set size) in BYTES from `smaps_rollup`.
/// `None` if the process is gone or `smaps_rollup` is unreadable - reading another
/// user's process needs `CAP_SYS_PTRACE` (the privileged-helper case), so a same-uid
/// read succeeds and a cross-uid one cleanly returns `None`.
pub fn read_pss_bytes(pid: u32) -> Option<u64> {
    let content = fs::read_to_string(format!("/proc/{pid}/smaps_rollup")).ok()?;
    parse_pss_kb(&content).map(|kb| kb.saturating_mul(1024))
}

/// Count a process's open file descriptors (`/proc/[pid]/fd` entries). `None` if
/// the directory is unreadable (the process is gone or not ours).
pub fn open_fd_count(pid: u32) -> Option<usize> {
    Some(fs::read_dir(format!("/proc/{pid}/fd")).ok()?.flatten().count())
}

/// Per-process detail fields from `/proc/[pid]/status` that sysinfo does not
/// expose: the thread count and the context-switch counters (a busy or thrashing
/// process shows high nonvoluntary switches). The per-process Statistics detail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ProcessDetail {
    /// Number of threads in the process.
    pub threads: u32,
    /// Context switches the process yielded voluntarily (e.g. blocking on I/O).
    pub voluntary_ctxt_switches: u64,
    /// Context switches forced on it (preempted); high values indicate CPU pressure.
    pub nonvoluntary_ctxt_switches: u64,
}

/// Parse the [`ProcessDetail`] fields from `/proc/[pid]/status` content. Each is a
/// `Key:\tvalue` line; a missing field defaults to 0. `strip_prefix` matches the
/// exact key, so `voluntary_ctxt_switches:` never matches the `nonvoluntary_` line.
pub fn parse_status(status: &str) -> ProcessDetail {
    let field = |key: &str| -> Option<u64> {
        status
            .lines()
            .find_map(|line| line.strip_prefix(key))
            .and_then(|rest| rest.split_whitespace().next())
            .and_then(|v| v.parse().ok())
    };
    ProcessDetail {
        threads: field("Threads:").unwrap_or(0) as u32,
        voluntary_ctxt_switches: field("voluntary_ctxt_switches:").unwrap_or(0),
        nonvoluntary_ctxt_switches: field("nonvoluntary_ctxt_switches:").unwrap_or(0),
    }
}

/// Read a process's [`ProcessDetail`] from `/proc/[pid]/status`. `None` if the
/// process is gone or its status is unreadable.
pub fn read_process_detail(pid: u32) -> Option<ProcessDetail> {
    let content = fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    Some(parse_status(&content))
}

#[cfg(test)]
mod tests {
    use super::*;

    const STATUS: &str = "\
Name:\tbash
State:\tS (sleeping)
Threads:\t4
voluntary_ctxt_switches:\t120
nonvoluntary_ctxt_switches:\t7
";

    #[test]
    fn parses_threads_and_context_switches() {
        let d = parse_status(STATUS);
        assert_eq!(d.threads, 4);
        assert_eq!(d.voluntary_ctxt_switches, 120);
        assert_eq!(d.nonvoluntary_ctxt_switches, 7);
    }

    #[test]
    fn nonvoluntary_line_does_not_match_the_voluntary_key() {
        // Only the nonvoluntary line present: the voluntary field must stay 0.
        let d = parse_status("nonvoluntary_ctxt_switches:\t9\n");
        assert_eq!(d.voluntary_ctxt_switches, 0);
        assert_eq!(d.nonvoluntary_ctxt_switches, 9);
    }

    #[test]
    fn missing_fields_default_to_zero() {
        assert_eq!(parse_status("Name:\tx\n"), ProcessDetail::default());
    }

    #[test]
    fn reads_own_detail_from_real_proc() {
        let d = read_process_detail(std::process::id()).expect("own /proc/self/status is readable");
        assert!(d.threads >= 1);
    }

    const SMAPS_ROLLUP: &str = "\
55e0d0a00000-7fff00000000 ---p 00000000 00:00 0                          [rollup]
Rss:               10240 kB
Pss:                4096 kB
Pss_Anon:           2048 kB
Pss_File:           2048 kB
Pss_Shmem:             0 kB
Shared_Clean:       6144 kB
Private_Dirty:      2048 kB
";

    #[test]
    fn parses_the_total_pss_not_the_breakdown_lines() {
        assert_eq!(parse_pss_kb(SMAPS_ROLLUP), Some(4096));
    }

    #[test]
    fn missing_or_malformed_pss_is_none() {
        assert_eq!(parse_pss_kb("Rss: 100 kB\n"), None);
        assert_eq!(parse_pss_kb("Pss: notanumber kB"), None);
        assert_eq!(parse_pss_kb(""), None);
    }

    #[test]
    fn reads_own_pss_from_real_proc() {
        // Our own smaps_rollup is readable and non-zero.
        let pss = read_pss_bytes(std::process::id());
        // smaps_rollup requires CONFIG_PROC_PAGE_MONITOR; where present it is > 0.
        if let Some(bytes) = pss {
            assert!(bytes > 0);
        }
    }

    #[test]
    fn counts_own_open_fds() {
        // At least stdin/stdout/stderr plus the dir handle being read.
        let n = open_fd_count(std::process::id()).expect("own /proc/self/fd is readable");
        assert!(n >= 3);
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;

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

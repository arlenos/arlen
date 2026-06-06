/// Waypointer process listing and killing.
///
/// Reads /proc to list user processes. Filters out kernel threads,
/// zombies, and other users' processes.

use serde::Serialize;
use std::fs;
use std::path::Path;

/// A single process entry.
#[derive(Clone, Serialize)]
pub struct ProcessInfo {
    /// Process ID.
    pub pid: u32,
    /// Process name (from /proc/pid/stat comm field).
    pub name: String,
    /// Resident memory in bytes (RSS from /proc/pid/statm * page_size).
    pub memory_bytes: u64,
}

/// Returns a list of the current user's processes, sorted by memory
/// (highest first). Filters out kernel threads, zombies, and other users.
/// Max 50 results.
#[tauri::command]
pub fn get_processes() -> Vec<ProcessInfo> {
    let my_uid = unsafe { libc::getuid() };
    let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) } as u64;

    let Ok(proc_dir) = fs::read_dir("/proc") else {
        return vec![];
    };

    let mut procs: Vec<ProcessInfo> = Vec::new();

    for entry in proc_dir.flatten() {
        let name = entry.file_name();
        let Some(pid_str) = name.to_str() else { continue };
        let Ok(pid) = pid_str.parse::<u32>() else { continue };

        let proc_path = entry.path();

        // Check ownership: only current user's processes.
        if let Ok(meta) = fs::metadata(&proc_path) {
            use std::os::unix::fs::MetadataExt;
            if meta.uid() != my_uid {
                continue;
            }
        }

        // Parse /proc/pid/stat for name, ppid, state.
        let Some((comm, ppid, state)) = parse_stat(&proc_path) else {
            continue;
        };

        // Filter: zombies.
        if state == 'Z' {
            continue;
        }
        // Filter: kernel threads (ppid == 2 = kthreadd, or name starts with k and ppid == 2).
        if ppid == 2 {
            continue;
        }
        // Filter: pid 1 and 2.
        if pid <= 2 {
            continue;
        }

        // Parse /proc/pid/statm for RSS.
        let rss_pages = parse_statm_rss(&proc_path).unwrap_or(0);
        let memory_bytes = rss_pages * page_size;

        procs.push(ProcessInfo {
            pid,
            name: comm,
            memory_bytes,
        });
    }

    // Sort by memory, highest first.
    procs.sort_by(|a, b| b.memory_bytes.cmp(&a.memory_bytes));
    procs.truncate(50);
    procs
}

/// Kills a process by PID.
///
/// `force=false` sends SIGTERM (graceful shutdown).
/// `force=true` sends SIGKILL (immediate).
#[tauri::command]
pub fn kill_process(pid: u32, force: bool) -> Result<(), String> {
    let signal = if force { libc::SIGKILL } else { libc::SIGTERM };
    let result = unsafe { libc::kill(pid as i32, signal) };
    if result == 0 {
        log::info!(
            "kill_process: sent {} to pid={}",
            if force { "SIGKILL" } else { "SIGTERM" },
            pid,
        );
        Ok(())
    } else {
        let err = std::io::Error::last_os_error();
        log::warn!("kill_process: failed for pid={}: {}", pid, err);
        Err(format!("Failed to kill process {pid}: {err}"))
    }
}

/// Parses /proc/pid/stat to extract (comm, ppid, state).
fn parse_stat(proc_path: &Path) -> Option<(String, u32, char)> {
    let content = fs::read_to_string(proc_path.join("stat")).ok()?;
    // Format: pid (comm) state ppid ...
    // comm can contain spaces and parens, so find the last ')'.
    let comm_start = content.find('(')?;
    let comm_end = content.rfind(')')?;
    let comm = content[comm_start + 1..comm_end].to_string();

    let rest = &content[comm_end + 2..]; // skip ") "
    let mut parts = rest.split_whitespace();
    let state = parts.next()?.chars().next()?;
    let ppid: u32 = parts.next()?.parse().ok()?;

    Some((comm, ppid, state))
}

/// Parses /proc/pid/statm to extract RSS (second field) in pages.
fn parse_statm_rss(proc_path: &Path) -> Option<u64> {
    let content = fs::read_to_string(proc_path.join("statm")).ok()?;
    let mut parts = content.split_whitespace();
    parts.next()?; // skip size
    let rss: u64 = parts.next()?.parse().ok()?;
    Some(rss)
}

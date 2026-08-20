//! Most-Recently-Used app history for the Waypointer "Recent Apps"
//! section. Persists a rolling buffer of `exec` strings to
//! `~/.local/share/arlen/app-history.json`.
//!
//! Design rationale — why file-based and not the Knowledge Graph:
//!
//!   1. **Source:** App launches through the Waypointer are 100%
//!      shell-initiated; no third party needs to consume this data.
//!      A local file is the narrowest scope.
//!   2. **Latency:** the Waypointer renders Recents synchronously on
//!      open; a graph round-trip (even cached) adds tens of ms and a
//!      failure mode. JSON at 50 entries is ~1 KB and loads in <1 ms.
//!   3. **Availability:** file-based works when the Knowledge daemon
//!      is down (common during dev, or on a user's first boot).
//!
//! Cross-app file opens are a different story — those live in the
//! graph via eBPF tracking (`recent_files.rs`). Keep the two paths
//! separate: local-origin history → file, cross-app/global history →
//! graph.

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Hard cap on retained entries. Each entry is ~50 bytes so 50 entries
/// is ~2.5 KB on disk — comfortable for a file the shell rewrites on
/// every launch. The front-end only shows the top 6-8 anyway.
const MAX_ENTRIES: usize = 50;

/// One recorded launch. Keyed by the exec command (what
/// `launchAppCmd` gets passed) — stable across user sessions because
/// .desktop `Exec=` doesn't change unless the package is replaced.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct HistoryEntry {
    /// The `AppEntry.exec` string the user launched.
    exec: String,
    /// Unix ms timestamp of the most recent launch (dedup collapses
    /// multiple launches of the same app into one entry).
    last_launched_at: i64,
    /// Total launches seen for this app. Not yet used by the UI; useful
    /// if we later want frequency-weighted MRU.
    #[serde(default = "default_launch_count")]
    launch_count: u32,
}

fn default_launch_count() -> u32 { 1 }

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct HistoryFile {
    #[serde(default)]
    entries: Vec<HistoryEntry>,
}

fn history_path() -> PathBuf {
    // Allow tests / sandboxed sessions to redirect the path without
    // touching the real user directory.
    if let Ok(p) = std::env::var("ARLEN_APP_HISTORY") {
        return PathBuf::from(p);
    }
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("arlen/app-history.json")
}

/// The launch history, or why it could not be read.
///
/// ABSENT IS NOT UNREADABLE. `record_launch_at` is read-modify-write and
/// `write_history` writes the whole file, so answering a corrupt or unreadable
/// history with the empty default meant the next app launch replaced the whole
/// list with one entry - and the launcher's "recent" ordering, which is built
/// from months of use, was gone on the first launch after a bad write.
fn read_history(path: &std::path::Path) -> Result<HistoryFile, String> {
    match fs::read_to_string(path) {
        Ok(text) => serde_json::from_str::<HistoryFile>(&text)
            .map_err(|e| format!("{} could not be read: {e}", path.display())),
        // Nothing launched yet is the ordinary first-run state.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(HistoryFile::default()),
        Err(e) => Err(format!("{} could not be read: {e}", path.display())),
    }
}

fn write_history(path: &std::path::Path, h: &HistoryFile) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("mkdir: {e}"))?;
    }
    let content = serde_json::to_string_pretty(h)
        .map_err(|e| format!("serialize: {e}"))?;
    fs::write(path, content).map_err(|e| format!("write: {e}"))
}

/// Now, as Unix milliseconds. Wrapped so tests can override via
/// `record_launch_at`.
fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// Core record logic. Exposed separately from the Tauri command so
/// tests can inject a path + timestamp.
fn record_launch_at(path: &std::path::Path, exec: &str, now: i64) -> Result<(), String> {
    if exec.trim().is_empty() {
        return Ok(()); // No-op — don't persist empty keys.
    }
    let mut history = read_history(path)?;

    // Find existing entry; bump timestamp + counter and move to front.
    if let Some(pos) = history.entries.iter().position(|e| e.exec == exec) {
        let mut entry = history.entries.remove(pos);
        entry.last_launched_at = now;
        entry.launch_count = entry.launch_count.saturating_add(1);
        history.entries.insert(0, entry);
    } else {
        history.entries.insert(
            0,
            HistoryEntry {
                exec: exec.to_string(),
                last_launched_at: now,
                launch_count: 1,
            },
        );
    }

    // Ring-buffer semantics: drop oldest beyond MAX_ENTRIES.
    history.entries.truncate(MAX_ENTRIES);

    write_history(path, &history)
}

// ── Tauri commands ───────────────────────────────────────────────────────

/// Record a Waypointer-launched app. Silent no-op on I/O failure —
/// the user doesn't care that history persistence hiccupped, they
/// care that the app launched (which already happened).
#[tauri::command]
pub fn record_app_launch(exec: String) {
    let path = history_path();
    if let Err(e) = record_launch_at(&path, &exec, now_ms()) {
        log::warn!("app_history: record_launch failed: {e}");
    }
}

/// Return the most-recently-launched `exec` strings, newest first.
/// Capped at `limit`. Returns an empty list (never errors) if the
/// history file is absent or unparseable.
#[tauri::command]
pub fn get_recent_apps(limit: u32) -> Vec<String> {
    // Read-only: an unreadable history is rendered as no history rather than
    // refusing to answer, and nothing is written back from here.
    let history = read_history(&history_path()).unwrap_or_default();
    history
        .entries
        .into_iter()
        .take(limit as usize)
        .map(|e| e.exec)
        .collect()
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn record_creates_entry_on_empty_history() {
        let dir = tmp();
        let path = dir.path().join("history.json");
        record_launch_at(&path, "firefox", 1_000).unwrap();
        let h = read_history(&path).unwrap();
        assert_eq!(h.entries.len(), 1);
        assert_eq!(h.entries[0].exec, "firefox");
        assert_eq!(h.entries[0].launch_count, 1);
    }

    #[test]
    fn repeat_launch_bumps_count_and_moves_to_front() {
        let dir = tmp();
        let path = dir.path().join("history.json");
        record_launch_at(&path, "firefox", 1_000).unwrap();
        record_launch_at(&path, "kitty", 2_000).unwrap();
        record_launch_at(&path, "firefox", 3_000).unwrap();

        let h = read_history(&path).unwrap();
        assert_eq!(h.entries.len(), 2);
        assert_eq!(h.entries[0].exec, "firefox");
        assert_eq!(h.entries[0].launch_count, 2);
        assert_eq!(h.entries[0].last_launched_at, 3_000);
        assert_eq!(h.entries[1].exec, "kitty");
    }

    #[test]
    fn ring_buffer_drops_oldest() {
        let dir = tmp();
        let path = dir.path().join("history.json");
        for i in 0..(MAX_ENTRIES + 10) {
            record_launch_at(&path, &format!("app{i}"), i as i64).unwrap();
        }
        let h = read_history(&path).unwrap();
        assert_eq!(h.entries.len(), MAX_ENTRIES);
        // Newest-first ordering: app(MAX+9) at front, app(10) at end.
        assert_eq!(h.entries[0].exec, format!("app{}", MAX_ENTRIES + 9));
        assert_eq!(h.entries[MAX_ENTRIES - 1].exec, "app10");
    }

    #[test]
    fn empty_exec_is_ignored() {
        let dir = tmp();
        let path = dir.path().join("history.json");
        record_launch_at(&path, "", 1).unwrap();
        record_launch_at(&path, "   ", 2).unwrap();
        assert_eq!(read_history(&path).unwrap().entries.len(), 0);
    }

    #[test]
    fn read_on_missing_file_returns_default() {
        let dir = tmp();
        let path = dir.path().join("nonexistent.json");
        let h = read_history(&path).unwrap();
        assert!(h.entries.is_empty());
    }

    /// This test asserted the opposite until 20 August - that a corrupt history
    /// reads as the empty default - which is exactly the behaviour that let one
    /// launch write that emptiness back over months of use. The safe answer is a
    /// refusal, and the file stays on disk for a person to fix.
    #[test]
    fn a_corrupt_history_is_refused_and_left_on_disk() {
        let dir = tmp();
        let path = dir.path().join("bad.json");
        fs::write(&path, "{not valid json").unwrap();

        assert!(read_history(&path).is_err(), "unreadable is not empty");
        assert!(
            record_launch_at(&path, "app.desktop", 1).is_err(),
            "and a launch does not write over what it could not read",
        );
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "{not valid json",
            "the file is still there to be fixed by hand",
        );
    }
}

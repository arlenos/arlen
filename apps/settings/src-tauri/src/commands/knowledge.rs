//! Knowledge Graph stats command.
//!
//! V1 reads only filesystem-level stats — DB file size, graph dir
//! size, FUSE mount status. The Cypher-aware stats (project count,
//! file count, edge count) require a token-authenticated daemon
//! query, which is bigger plumbing than Sprint C wants. Phase 8's
//! `app-knowledge` will get that surface.

use std::path::Path;
use std::process::Command;

use serde::Serialize;

const DB_PATH_DEFAULT: &str = "/var/lib/lunaris/knowledge/events.db";
const GRAPH_DIR_DEFAULT: &str = "/var/lib/lunaris/knowledge/graph";
const FUSE_MOUNT_DEFAULT: &str = "~/.timeline";

/// Daemon's listen-socket path. Created on startup, removed on
/// clean shutdown. Presence of this file is the most reliable
/// "the daemon is currently alive" signal we can read without a
/// token-authenticated socket round-trip.
const DAEMON_SOCKET_DEFAULT: &str = "/run/lunaris/knowledge.sock";

/// Whole-page stats payload for the Knowledge Graph settings page.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeStats {
    /// Daemon presence — `findmnt` reports a fuse mount only when
    /// the FUSE thread is actually serving. Other stats are only
    /// shown to the user when this is true.
    pub daemon_running: bool,
    /// Where the FUSE filesystem is mounted (path), independent of
    /// `daemon_running` — UI shows it as the link target either way.
    pub fuse_mount: String,
    /// Whether the FUSE mount is currently in the kernel mount
    /// table. Distinct from `daemon_running` so the UI can render
    /// "(mounted ✓)" or "(not mounted)" precisely.
    pub fuse_mounted: bool,
    /// SQLite event store size in bytes, or `null` if unreadable
    /// (root-only on hardened systems, or daemon-not-running).
    pub db_size_bytes: Option<u64>,
    /// Sum of all file sizes in the graph storage directory.
    /// `null` for the same reasons as `db_size_bytes`.
    pub graph_size_bytes: Option<u64>,
}

#[tauri::command]
pub fn knowledge_stats_get() -> Result<KnowledgeStats, String> {
    let fuse_mount = expand_tilde(FUSE_MOUNT_DEFAULT);
    let fuse_mounted = is_fuse_mounted(&fuse_mount);
    let db_size_bytes = file_size(Path::new(DB_PATH_DEFAULT));
    let graph_size_bytes = dir_size(Path::new(GRAPH_DIR_DEFAULT));

    // Daemon-liveness: socket file presence is the truthy signal
    // (created on startup, removed on clean shutdown). FUSE mount
    // is the secondary signal — the daemon owns the FUSE thread,
    // so a mounted FUSE means the daemon is alive in the kernel
    // sense even if the runtime socket dir is stale.
    //
    // We deliberately do NOT use "DB file readable" as a liveness
    // signal — it's stale on-disk data after a crash and would
    // misreport a dead daemon as running (Codex Sprint C review).
    let daemon_running = daemon_socket_exists() || fuse_mounted;

    Ok(KnowledgeStats {
        daemon_running,
        fuse_mount,
        fuse_mounted,
        db_size_bytes,
        graph_size_bytes,
    })
}

/// Resolve the daemon socket path with the same fallback chain
/// the desktop-shell client uses: `LUNARIS_DAEMON_SOCKET` env var
/// (set by `start-dev.sh`), then `$XDG_RUNTIME_DIR/lunaris/...`,
/// finally the hardcoded `/run/lunaris/...` system default.
fn daemon_socket_path() -> std::path::PathBuf {
    if let Ok(p) = std::env::var("LUNARIS_DAEMON_SOCKET") {
        return std::path::PathBuf::from(p);
    }
    if let Ok(xdg) = std::env::var("XDG_RUNTIME_DIR") {
        return std::path::PathBuf::from(xdg).join("lunaris/knowledge.sock");
    }
    std::path::PathBuf::from(DAEMON_SOCKET_DEFAULT)
}

fn daemon_socket_exists() -> bool {
    daemon_socket_path().exists()
}

fn expand_tilde(s: &str) -> String {
    if let Some(rest) = s.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return format!("{home}/{rest}");
        }
    }
    s.to_string()
}

fn file_size(path: &Path) -> Option<u64> {
    std::fs::metadata(path).ok().map(|m| m.len())
}

/// Recursive sum of file sizes under `path`. Returns `None` if the
/// directory itself is unreadable (typical for root-owned graph
/// stores on hardened distros).
fn dir_size(path: &Path) -> Option<u64> {
    if !path.exists() {
        return None;
    }
    walk_dir_size(path).ok()
}

fn walk_dir_size(path: &Path) -> std::io::Result<u64> {
    let mut total = 0u64;
    if path.is_file() {
        return Ok(std::fs::metadata(path)?.len());
    }
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        if ty.is_dir() {
            // Bounded recursion depth via path length — graph stores
            // are flat enough that any reasonable depth works, but
            // we don't want to wedge on a symlink loop.
            if entry.path().components().count() < 32 {
                total = total.saturating_add(walk_dir_size(&entry.path())?);
            }
        } else if ty.is_file() {
            total =
                total.saturating_add(entry.metadata().map(|m| m.len()).unwrap_or(0));
        }
    }
    Ok(total)
}

/// `findmnt -t fuse <path>` exits 0 when there's a fuse mount at
/// that path, non-zero otherwise. We don't need the output; the
/// exit code is the answer.
fn is_fuse_mounted(path: &str) -> bool {
    Command::new("findmnt")
        .args(["-t", "fuse", path])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expand_tilde_uses_home() {
        std::env::set_var("HOME", "/tmp/home-test");
        assert_eq!(expand_tilde("~/.timeline"), "/tmp/home-test/.timeline");
        // No tilde — pass-through.
        assert_eq!(expand_tilde("/var/x"), "/var/x");
    }

    #[test]
    fn file_size_missing_is_none() {
        assert!(file_size(Path::new("/nonexistent-file-99999")).is_none());
    }

    #[test]
    fn dir_size_missing_is_none() {
        assert!(dir_size(Path::new("/nonexistent-dir-99999")).is_none());
    }

    /// Walking a real tempdir returns the byte sum of the files in it.
    #[test]
    fn dir_size_sums_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a"), b"hello").unwrap();
        std::fs::write(dir.path().join("b"), b"world!").unwrap();
        std::fs::create_dir(dir.path().join("nested")).unwrap();
        std::fs::write(dir.path().join("nested/c"), b"x").unwrap();

        let total = dir_size(dir.path()).unwrap();
        assert_eq!(total, 5 + 6 + 1);
    }

    /// Daemon-running heuristic uses the socket file (or FUSE mount)
    /// as the truthy signal. Stale DB files MUST NOT count — that
    /// was the Codex Sprint C review MEDIUM finding: a leftover
    /// `events.db` from a previous run misreported a dead daemon
    /// as running.
    #[test]
    fn daemon_running_uses_socket_not_db_size() {
        let stats = knowledge_stats_get().unwrap();
        // The contract: daemon_running iff socket exists OR fuse
        // mount is up. DB-size presence is now decoupled from the
        // liveness signal.
        let socket_signal = daemon_socket_exists();
        assert_eq!(stats.daemon_running, socket_signal || stats.fuse_mounted);

        // Critical invariant: a system with stale DB but no socket
        // and no fuse mount must report daemon_running = false.
        // We can't easily fabricate that exact state on the test
        // runner without root, but we verify the rule above
        // doesn't accidentally fall back to db_size_bytes.
        if !socket_signal && !stats.fuse_mounted {
            assert!(
                !stats.daemon_running,
                "stale DB without socket/fuse must NOT mark daemon as running"
            );
        }
    }

    /// Socket-path resolution honours env-var fallbacks.
    #[test]
    fn socket_path_uses_env_overrides() {
        let prev_socket = std::env::var("LUNARIS_DAEMON_SOCKET").ok();
        let prev_xdg = std::env::var("XDG_RUNTIME_DIR").ok();

        // Explicit override wins.
        unsafe {
            std::env::set_var("LUNARIS_DAEMON_SOCKET", "/tmp/test-knowledge.sock");
        }
        assert_eq!(
            daemon_socket_path(),
            std::path::PathBuf::from("/tmp/test-knowledge.sock")
        );

        // Without explicit override, XDG_RUNTIME_DIR is the next stop.
        unsafe {
            std::env::remove_var("LUNARIS_DAEMON_SOCKET");
            std::env::set_var("XDG_RUNTIME_DIR", "/tmp/run-test");
        }
        assert_eq!(
            daemon_socket_path(),
            std::path::PathBuf::from("/tmp/run-test/lunaris/knowledge.sock")
        );

        // Restore env.
        unsafe {
            match prev_socket {
                Some(v) => std::env::set_var("LUNARIS_DAEMON_SOCKET", v),
                None => std::env::remove_var("LUNARIS_DAEMON_SOCKET"),
            }
            match prev_xdg {
                Some(v) => std::env::set_var("XDG_RUNTIME_DIR", v),
                None => std::env::remove_var("XDG_RUNTIME_DIR"),
            }
        }
    }
}

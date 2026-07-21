//! Knowledge-daemon filesystem-stat helpers the settings host wraps in the
//! `knowledge_stats_get` command: the socket-path fallback chain (the daemon's
//! liveness signal), tilde expansion, and the file/dir size walks. Pure of Tauri
//! and unit-tested in CI (the src-tauri host is not); the command that composes
//! them into the page payload stays in the host.

use std::path::Path;
use std::process::Command;

/// The daemon's listen-socket system default. Presence of the socket file is the
/// most reliable "the daemon is currently alive" signal readable without a
/// token-authenticated round-trip.
const DAEMON_SOCKET_DEFAULT: &str = "/run/arlen/knowledge.sock";

/// Resolve the daemon socket path with the same fallback chain the desktop-shell
/// client uses: `ARLEN_DAEMON_SOCKET` env var (set by `start-dev.sh`), then
/// `$XDG_RUNTIME_DIR/arlen/...`, finally the hardcoded `/run/arlen/...` default.
pub fn daemon_socket_path() -> std::path::PathBuf {
    if let Ok(p) = std::env::var("ARLEN_DAEMON_SOCKET") {
        return std::path::PathBuf::from(p);
    }
    if let Ok(xdg) = std::env::var("XDG_RUNTIME_DIR") {
        return std::path::PathBuf::from(xdg).join("arlen/knowledge.sock");
    }
    std::path::PathBuf::from(DAEMON_SOCKET_DEFAULT)
}

/// True when the daemon's listen socket exists (created on startup, removed on
/// clean shutdown).
pub fn daemon_socket_exists() -> bool {
    daemon_socket_path().exists()
}

/// Expand a leading `~/` to `$HOME/`, passing anything else through unchanged.
pub fn expand_tilde(s: &str) -> String {
    if let Some(rest) = s.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return format!("{home}/{rest}");
        }
    }
    s.to_string()
}

/// File size in bytes, or `None` if unreadable (root-only on hardened systems, or
/// the file is absent).
pub fn file_size(path: &Path) -> Option<u64> {
    std::fs::metadata(path).ok().map(|m| m.len())
}

/// Recursive sum of file sizes under `path`. Returns `None` if the directory
/// itself is unreadable (typical for root-owned graph stores on hardened distros).
pub fn dir_size(path: &Path) -> Option<u64> {
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
            // Bounded recursion depth via path length — graph stores are flat
            // enough that any reasonable depth works, but we don't want to wedge
            // on a symlink loop.
            if entry.path().components().count() < 32 {
                total = total.saturating_add(walk_dir_size(&entry.path())?);
            }
        } else if ty.is_file() {
            total = total.saturating_add(entry.metadata().map(|m| m.len()).unwrap_or(0));
        }
    }
    Ok(total)
}

/// `findmnt -t fuse <path>` exits 0 when there's a fuse mount at that path,
/// non-zero otherwise. The exit code is the answer; the output is not needed.
pub fn is_fuse_mounted(path: &str) -> bool {
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

    /// Socket-path resolution honours env-var fallbacks.
    #[test]
    fn socket_path_uses_env_overrides() {
        let prev_socket = std::env::var("ARLEN_DAEMON_SOCKET").ok();
        let prev_xdg = std::env::var("XDG_RUNTIME_DIR").ok();

        // Explicit override wins.
        std::env::set_var("ARLEN_DAEMON_SOCKET", "/tmp/test-knowledge.sock");
        assert_eq!(
            daemon_socket_path(),
            std::path::PathBuf::from("/tmp/test-knowledge.sock")
        );

        // Without explicit override, XDG_RUNTIME_DIR is the next stop.
        std::env::remove_var("ARLEN_DAEMON_SOCKET");
        std::env::set_var("XDG_RUNTIME_DIR", "/tmp/run-test");
        assert_eq!(
            daemon_socket_path(),
            std::path::PathBuf::from("/tmp/run-test/arlen/knowledge.sock")
        );

        // Restore env.
        match prev_socket {
            Some(v) => std::env::set_var("ARLEN_DAEMON_SOCKET", v),
            None => std::env::remove_var("ARLEN_DAEMON_SOCKET"),
        }
        match prev_xdg {
            Some(v) => std::env::set_var("XDG_RUNTIME_DIR", v),
            None => std::env::remove_var("XDG_RUNTIME_DIR"),
        }
    }
}

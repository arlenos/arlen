//! Knowledge Graph stats command.
//!
//! V1 reads only filesystem-level stats — DB file size, graph dir
//! size, FUSE mount status. The Cypher-aware stats (project count,
//! file count, edge count) require a token-authenticated daemon
//! query, which is bigger plumbing than Sprint C wants. Phase 8's
//! `app-knowledge` will get that surface. The filesystem-stat helpers
//! (socket-path fallback, tilde expansion, file/dir size walks) live
//! in `arlen-settings-core::knowledge`, unit-tested in CI.

use std::path::Path;

use serde::Serialize;

use arlen_settings_core::knowledge::{
    daemon_socket_exists, dir_size, expand_tilde, file_size, is_fuse_mounted,
};

const DB_PATH_DEFAULT: &str = "/var/lib/arlen/knowledge/events.db";
const GRAPH_DIR_DEFAULT: &str = "/var/lib/arlen/knowledge/graph";
const FUSE_MOUNT_DEFAULT: &str = "~/.timeline";

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

#[cfg(test)]
mod tests {
    use super::*;

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
}

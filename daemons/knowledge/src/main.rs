#![warn(clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]
#![allow(clippy::must_use_candidate)]

mod proto {
    #![allow(dead_code)]
    #![allow(clippy::doc_markdown)]
    include!(concat!(env!("OUT_DIR"), "/arlen.eventbus.rs"));
}

mod audit;
mod auth;
mod backup;
mod capsule;
mod code_analysis;
mod daemon;
mod db;
mod events;
mod fts;
mod fuse;
mod graph;
mod identity;
mod lcg;
mod lifecycle;
mod links;
mod migration;
mod permission;
mod project;
mod promotion;
mod quota;
mod retrieval;
mod retention;
mod revoke;
mod schema;
mod shared;
mod time;
mod token;
mod token_cache;
mod typed_read;
mod utils;
mod write;
mod writer;

use anyhow::{bail, Result};
use tracing::{info, warn};

const DEFAULT_DB_PATH: &str = "/var/lib/arlen/knowledge/events.db";
const DEFAULT_GRAPH_PATH: &str = "/var/lib/arlen/knowledge/graph";
const DEFAULT_TIMELINE_MOUNT: &str = ".timeline";

/// Pick the daemon socket path per the standard 3-tier convention:
/// `ARLEN_DAEMON_SOCKET` (non-empty) wins, else the per-user path
/// `$XDG_RUNTIME_DIR/arlen/knowledge.sock` (i.e.
/// `/run/user/{uid}/arlen/knowledge.sock`), else `/run/arlen/knowledge.sock`.
/// The XDG branch creates the `arlen/` parent so the daemon starts
/// cleanly in a normal dev session even if the launcher forgets the env
/// var; the `/run/arlen/` last resort requires the write access only a
/// privileged launcher has.
fn pick_daemon_socket() -> String {
    crate::utils::socket_path("ARLEN_DAEMON_SOCKET", "knowledge.sock")
}

/// Resolve a per-user data path: an explicit `env_var` wins (the
/// launcher / systemd-unit contract), else `$XDG_DATA_HOME/arlen/<name>`
/// (the per-user store, i.e. `~/.local/share/arlen/<name>`, matching the
/// unit's pinned paths), else the system-wide `system_default` as a last
/// resort. A per-uid default keeps two profiles from sharing one graph +
/// SQLite store even when no launcher pins the env, the same fail-safe the
/// socket layer already has (profile-system-plan.md PR-R1). The derived
/// per-user parent is created best-effort so the daemon opens cleanly in a
/// dev session; an env-pinned path's parent is the launcher's to own.
fn pick_data_path(env_var: &str, name: &str, system_default: &str) -> String {
    let pinned = std::env::var(env_var).ok().filter(|s| !s.is_empty());
    let xdg = std::env::var("XDG_DATA_HOME").ok();
    let home = std::env::var("HOME").ok();
    let path = crate::utils::resolve_data_path(
        pinned.as_deref(),
        xdg.as_deref(),
        home.as_deref(),
        name,
        system_default,
    );
    // Best-effort: create the per-user parent so the daemon opens cleanly
    // in a dev session. An env-pinned path's parent is the launcher's.
    if pinned.is_none() {
        if let Some(parent) = std::path::Path::new(&path).parent() {
            let _ = std::fs::create_dir_all(parent);
        }
    }
    path
}

/// Check whether `path` is currently a mount point. Reads
/// `/proc/self/mountinfo` directly so we don't depend on the
/// `mountpoint(1)` binary being installed. Returns `false` on any
/// error — the caller then tries to mount normally (which will fail
/// with a clear error if it actually IS a mount).
fn is_mountpoint(path: &str) -> bool {
    let Ok(content) = std::fs::read_to_string("/proc/self/mountinfo") else {
        return false;
    };
    // mountinfo layout per proc(5):
    //   id parent major:minor root mount-point mount-options ... - fstype source super-opts
    // Index 4 (0-based) is the mount point. Space-separated tokens;
    // paths with spaces are octal-escaped but we match literal so
    // that's fine for our `~/.timeline`.
    for line in content.lines() {
        if let Some(target) = line.split_whitespace().nth(4) {
            if target == path {
                return true;
            }
        }
    }
    false
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("knowledge=debug".parse()?),
        )
        .init();

    info!("starting knowledge daemon");

    let consumer_socket = crate::utils::socket_path("ARLEN_CONSUMER_SOCKET", "event-bus-consumer.sock");
    let db_path = pick_data_path("ARLEN_DB_PATH", "events.db", DEFAULT_DB_PATH);
    let graph_path = pick_data_path("ARLEN_GRAPH_PATH", "graph", DEFAULT_GRAPH_PATH);
    let daemon_socket = pick_daemon_socket();
    // The timeline FUSE mount is optional: an empty or "off"
    // `ARLEN_TIMELINE_MOUNT` disables it, so the daemon runs on a host without
    // FUSE (CI runners, the EphemeralStack integration harness). The socket,
    // graph and promotion paths are unaffected - only the `~/.timeline` view
    // goes away. Unset falls back to the default mount path as before.
    let timeline_mount = match std::env::var("ARLEN_TIMELINE_MOUNT") {
        Ok(v) if v.is_empty() || v == "off" => None,
        Ok(v) => Some(v),
        Err(_) => {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
            Some(format!("{home}/{DEFAULT_TIMELINE_MOUNT}"))
        }
    };
    info!(%daemon_socket, "daemon socket path resolved");

    // Open SQLite write store
    let pool = db::open(&db_path).await?;
    info!(path = db_path, "sqlite write store ready");

    // Spawn the dedicated Ladybug thread
    let graph = graph::spawn(&graph_path)?;
    info!(path = graph_path, "ladybug query store ready");

    // FUSE runs on a dedicated OS thread (blocking mount).
    //
    // Before attempting to mount, check if `timeline_mount` is already
    // a (possibly stale) mount point. If a previous daemon was
    // SIGKILL'd without its FUSE exit handler firing, the kernel
    // keeps the mount registered while the userspace process is gone
    // — calling `fuse::mount` on that path then returns `File exists
    // (os error 17)`. Skip the mount-attempt entirely in that case
    // and point the operator at the launcher script which handles
    // cleanup.
    if let Some(fuse_mount_path) = timeline_mount.clone() {
        let fuse_graph = graph.clone();
        std::thread::Builder::new()
            .name("fuse-timeline".into())
            .spawn(move || {
                if is_mountpoint(&fuse_mount_path) {
                    warn!(
                        path = %fuse_mount_path,
                        "FUSE: path already mounted — skipping remount. \
                         Stale mount from a previous run? Fix with \
                         `fusermount -u {fuse_mount_path}` or use \
                         `just dev` which handles this automatically",
                    );
                    return;
                }
                if let Err(e) = fuse::mount(&fuse_mount_path, fuse_graph) {
                    tracing::error!("FUSE mount failed: {e}");
                }
            })?;
    } else {
        info!("timeline FUSE mount disabled (ARLEN_TIMELINE_MOUNT empty/off)");
    }

    // Validate-on-startup pass: any project whose root_path vanished
    // since the last run gets pruned (inferred) or archived (explicit).
    // Per docs/architecture/project-system.md §Validation on Access we
    // do not poll periodically; daemon startup is one of the access
    // points the spec calls out. Failures on individual projects do
    // not abort the sweep — they are logged and counted.
    {
        let store = project::ProjectStore::new(graph.clone());
        match store.prune_dead_projects().await {
            Ok(stats) => info!(
                alive = stats.alive,
                pruned = stats.pruned,
                archived = stats.archived,
                errors = stats.errors,
                "startup project validation complete"
            ),
            Err(e) => warn!(
                error = %e,
                "startup project validation failed; continuing without prune"
            ),
        }
    }

    // Project watcher: scans configured directories and watches for changes.
    let project_graph = graph.clone();
    tokio::spawn(async move {
        if let Err(e) = project::watcher::run(project_graph).await {
            tracing::error!("project watcher error: {e}");
        }
    });

    // Run all four components concurrently. `tokio::select!` — not
    // `try_join!` — so a failing task is attributed by name instead
    // of leaving the operator with an anonymous "Error: Permission
    // denied (os error 13)" and no way to tell which task emitted it.
    tokio::select! {
        r = writer::run(&consumer_socket, pool.clone()) => match r {
            Ok(()) => bail!("writer task exited unexpectedly"),
            Err(e) => bail!("writer ({consumer_socket}): {e}"),
        },
        r = promotion::run(pool.clone(), graph.clone()) => match r {
            Ok(()) => bail!("promotion task exited unexpectedly"),
            Err(e) => bail!("promotion: {e}"),
        },
        r = retention::run(pool.clone(), graph.clone()) => match r {
            Ok(()) => bail!("retention task exited unexpectedly"),
            Err(e) => bail!("retention: {e}"),
        },
        r = project::cooccurrence::run(graph.clone()) => match r {
            Ok(()) => bail!("project inference task exited unexpectedly"),
            Err(e) => bail!("project inference: {e}"),
        },
        r = daemon::listen(&daemon_socket, graph, pool) => match r {
            Ok(()) => bail!("daemon listener exited unexpectedly"),
            Err(e) => bail!("daemon listen ({daemon_socket}): {e}"),
        },
    }
}

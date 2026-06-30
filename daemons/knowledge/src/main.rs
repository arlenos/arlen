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
mod derivation;
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

fn main() -> Result<()> {
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
    info!(%daemon_socket, "daemon socket path resolved");

    // The timeline FUSE mount now lives in the separate `arlen-timeline` helper
    // (same-uid-isolation-plan.md option b): the mount needs the SUID
    // `fusermount3`, which the Landlock + no_new_privs fence below would block.
    // The helper reads the graph over this daemon's read socket; nothing here
    // mounts FUSE.

    // Self-confine BEFORE building the async runtime, so the ladybug thread and
    // every tokio worker spawned afterward inherit the Landlock write-domain.
    // Read everywhere; write only under the daemon's own dirs. Best-effort: a
    // kernel that cannot enforce leaves the daemon exactly as safe as no fence.
    apply_fence(&db_path, &graph_path, &daemon_socket);

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(run(consumer_socket, db_path, graph_path, daemon_socket))
}

/// The async daemon body. Run inside the runtime that is built AFTER the fence,
/// so the ladybug thread and the tokio workers inherit the Landlock domain.
async fn run(
    consumer_socket: String,
    db_path: String,
    graph_path: String,
    daemon_socket: String,
) -> Result<()> {
    // Open SQLite write store
    let pool = db::open(&db_path).await?;
    info!(path = db_path, "sqlite write store ready");

    // Spawn the dedicated Ladybug thread
    let graph = graph::spawn(&graph_path)?;
    info!(path = graph_path, "ladybug query store ready");

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

/// Install the Landlock write-fence over the daemon's own dirs: the SQLite store
/// (events.db + WAL/SHM live in its parent dir), the ladybug graph dir, the
/// socket dir it binds, and the system temp dir SQLite/Kuzu may spill to. The
/// dirs are created first so each grant is expressible (the fence skips a path it
/// cannot open). Defense-in-depth and best-effort: a kernel that cannot enforce
/// leaves the daemon exactly as safe as no fence, so a non-enforcing kernel or a
/// ruleset error is logged and the daemon continues - unless
/// `ARLEN_KNOWLEDGE_REQUIRE_FENCE=1`, which makes that fatal for a hardened
/// deployment. Read access stays granted everywhere (config, permission profiles,
/// `/proc/<pid>/exe` for caller identity); only writes are confined.
fn apply_fence(db_path: &str, graph_path: &str, daemon_socket: &str) {
    use arlen_landlock_fence::{fence_writes, FenceOutcome};
    use std::path::{Path, PathBuf};

    let require = std::env::var_os("ARLEN_KNOWLEDGE_REQUIRE_FENCE").is_some_and(|v| v == "1");

    // Grant each path's PARENT dir, never the leaf itself: the ladybug/Kuzu
    // database at `graph_path` is created and managed by Kuzu, which refuses a
    // pre-existing directory at that path ("Database path cannot be a
    // directory"), and `db_path` is the SQLite file. Granting the parent lets
    // the daemon create + write both under it; pre-creating the graph leaf as a
    // dir breaks Kuzu's open. The db and graph parents are normally the same dir
    // (the StateDirectory); overlapping grants are harmless.
    let mut writable: Vec<PathBuf> = Vec::new();
    for leaf in [db_path, graph_path, daemon_socket] {
        if let Some(p) = Path::new(leaf).parent() {
            writable.push(p.to_path_buf());
        }
    }
    writable.push(std::env::temp_dir());

    for dir in &writable {
        let _ = std::fs::create_dir_all(dir);
    }

    let degraded = match fence_writes(&writable) {
        Ok(FenceOutcome::Enforced) => {
            info!("landlock write-fence enforced (db + graph + socket + temp dirs)");
            None
        }
        Ok(FenceOutcome::NotEnforced) => Some("landlock not enforced by this kernel".to_string()),
        Err(e) => Some(format!("landlock fence not applied: {e}")),
    };
    if let Some(reason) = degraded {
        if require {
            tracing::error!(
                "ARLEN_KNOWLEDGE_REQUIRE_FENCE=1 but the fence is not active ({reason}); refusing to run unconfined"
            );
            std::process::exit(1);
        }
        warn!("{reason}; running unconfined (no worse than no fence)");
    }
}

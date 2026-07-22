/// Arlen Install Daemon -- user-level D-Bus service.
///
/// Provides `org.arlen.InstallDaemon1` on the session bus. Handles
/// `.lunpkg` installation, uninstallation, and app listing. Delegates
/// privileged operations (system-wide installs) to `install-helper`
/// via the system bus.
///
/// See `docs/architecture/install-daemon.md`.

mod audit;
mod consent;
mod dbus;
mod event_emit;
mod flatpak;
mod install;
mod jobs;
mod module_permissions;
mod permission_helper;
mod signature;
mod transaction;
mod trash;

/// Serialises the tests that override the install/trash/key directories via
/// environment variables.
///
/// Those overrides are PROCESS-global, so under `cargo test`'s default parallel
/// runner one test's `remove_var` tears down the directory another test is
/// mid-way through using: it then resolves the real path, finds nothing, and
/// fails. The suite passed only with `--test-threads=1`, which meant plain
/// `cargo test` - what CI runs - was flaky across `install`, `trash` and
/// `signature` (6-8 failures per run, a different set each time).
///
/// One crate-wide lock, not one per module: `install` and `trash` both override
/// `ARLEN_USER_APPS_DIR`, so per-module locks would still race. Poisoning is
/// ignored - a panicking test has already failed, and blocking every later test
/// behind its poison would turn one failure into a cascade.
#[cfg(test)]
pub(crate) fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

use std::sync::Arc;

use zbus::connection;

use jobs::JobQueue;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("arlen_installd=info".parse()?),
        )
        .init();

    tracing::info!("starting installd");

    // Run trash cleanup on startup.
    let cleaned = trash::cleanup_trash();
    if cleaned > 0 {
        tracing::info!("startup: cleaned {cleaned} expired trash entries");
    }

    let job_queue = Arc::new(JobQueue::new());
    let daemon = dbus::InstallDaemon::new(job_queue.clone());

    let conn = connection::Builder::session()?
        .name("org.arlen.InstallDaemon1")?
        .serve_at("/org/arlen/InstallDaemon1", daemon)?
        .build()
        .await?;

    // Start the job worker.
    let worker_queue = job_queue.clone();
    let worker_conn = conn.clone();
    tokio::spawn(async move {
        jobs::run_worker(worker_queue, worker_conn).await;
    });

    tracing::info!("D-Bus service ready on org.arlen.InstallDaemon1");

    tokio::signal::ctrl_c().await?;
    tracing::info!("shutting down");

    Ok(())
}

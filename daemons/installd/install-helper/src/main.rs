/// Arlen Install Helper -- root D-Bus service.
///
/// Provides `org.arlen.InstallHelper1` for privileged install operations
/// that require root access: copying apps to `/usr/lib/arlen/apps/`,
/// writing desktop entries to `/usr/share/applications/`, and removing
/// system-wide installations.
///
/// Only authorized callers (arlen-installd) may invoke methods.
/// Caller identity is verified via `/proc/{pid}/exe`.
///
/// See `docs/architecture/install-daemon.md`.

/// Serialises tests that override the `ARLEN_*_DIR` path env vars.
///
/// Those overrides are PROCESS-global, so under `cargo test`'s parallel runner
/// one test's `remove_var` tears down the directory another test is mid-way
/// through using. `test_create_desktop_entry` and
/// `test_create_desktop_entry_invalid` both drive `ARLEN_SYSTEM_DESKTOP_DIR`,
/// and either could fail depending on interleaving - which is exactly what a
/// full-workspace run showed, a different test failing each time. Mirrors the
/// same lock in installd.
#[cfg(test)]
pub(crate) fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

mod dbus;
mod install;

use zbus::connection;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("arlen_install_helper=info".parse()?),
        )
        .init();

    tracing::info!("starting install helper");

    let helper = dbus::InstallHelper;

    let _conn = connection::Builder::system()?
        .name("org.arlen.InstallHelper1")?
        .serve_at("/org/arlen/InstallHelper1", helper)?
        .build()
        .await?;

    tracing::info!("D-Bus service ready on org.arlen.InstallHelper1");

    // Run until SIGTERM.
    tokio::signal::ctrl_c().await?;
    tracing::info!("shutting down");

    Ok(())
}

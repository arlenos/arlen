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

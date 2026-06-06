/// Arlen Permission Helper -- root D-Bus service.
///
/// Provides `org.arlen.PermissionHelper1` for writing permission profiles
/// to `/var/lib/arlen/permissions/`. Only authorized callers (installd,
/// settings) may invoke methods.
///
/// See `docs/architecture/permission-system.md`.

mod dbus;
mod profile;

use zbus::connection;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("arlen_permission_helper=info".parse()?),
        )
        .init();

    tracing::info!("starting permission helper");

    let helper = dbus::PermissionHelper;

    let _conn = connection::Builder::system()?
        .name("org.arlen.PermissionHelper1")?
        .serve_at("/org/arlen/PermissionHelper1", helper)?
        .build()
        .await?;

    tracing::info!("D-Bus service ready on org.arlen.PermissionHelper1");

    // Run until SIGTERM.
    tokio::signal::ctrl_c().await?;
    tracing::info!("shutting down");

    Ok(())
}

/// `lunaris-modulesd` entry point.
///
/// Bring-up sequence:
///   1. Init tracing.
///   2. Create the manager (Tier 1 runtime + Tier 2 broker).
///   3. Run discovery against the system + user module directories.
///   4. Bind the Unix socket.
///   5. Accept connections forever, broadcast lifecycle events.

use std::sync::Arc;

use tokio::sync::broadcast;
use tracing::info;

use lunaris_modulesd::manager::Manager;
use lunaris_modulesd::socket::server::{default_socket_path, SocketServer};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("lunaris_modulesd=info".parse()?),
        )
        .init();

    info!("modulesd: starting v{}", env!("CARGO_PKG_VERSION"));

    let (events_tx, _events_rx) = broadcast::channel(256);
    let manager: Arc<Manager> = Manager::new(events_tx.clone())?;
    manager.discover().await;

    let socket_path = default_socket_path();
    let server = SocketServer::bind(&socket_path, Arc::clone(&manager), events_tx)?;

    // Cooperative shutdown: SIGINT / SIGTERM trigger a clean exit so
    // the socket file is removed.
    // Cooperative shutdown: SIGINT / SIGTERM trigger a clean exit so
    // the socket file is removed and every loaded Tier 1 module gets
    // a `Guest::shutdown` politeness call (capped at 1 s per module
    // by `Manager::shutdown_all_tier1`).
    let socket_path_clone = socket_path.clone();
    let manager_for_shutdown = Arc::clone(&manager);
    let shutdown = async move {
        let mut sigint = tokio::signal::unix::signal(
            tokio::signal::unix::SignalKind::interrupt(),
        )?;
        let mut sigterm = tokio::signal::unix::signal(
            tokio::signal::unix::SignalKind::terminate(),
        )?;
        tokio::select! {
            _ = sigint.recv() => info!("modulesd: SIGINT received"),
            _ = sigterm.recv() => info!("modulesd: SIGTERM received"),
        }
        manager_for_shutdown.shutdown_all_tier1().await;
        let _ = std::fs::remove_file(&socket_path_clone);
        Ok::<_, std::io::Error>(())
    };

    tokio::select! {
        res = server.run() => {
            if let Err(err) = res {
                tracing::error!("modulesd: server ended with error: {err}");
            }
        }
        res = shutdown => {
            if let Err(err) = res {
                tracing::error!("modulesd: shutdown handler error: {err}");
            }
        }
    }

    info!("modulesd: bye");
    Ok(())
}

//! System Monitor read-only MCP server daemon.
//!
//! Binds the well-known `system.monitor` MCP socket and serves the read-only
//! process-list and resource-usage tools to the AI daemon. See the crate docs.

use arlen_system_monitor_mcp::{SystemMonitorMcp, SERVER_ID};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    tracing::info!(server_id = SERVER_ID, "system monitor mcp server starting");

    let make_handler = SystemMonitorMcp::new;

    tokio::select! {
        result = os_sdk::mcp::serve_mcp(SERVER_ID, make_handler) => {
            if let Err(err) = result {
                tracing::error!(error = %err, "system monitor mcp server exited with error");
                std::process::exit(1);
            }
        }
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("system monitor mcp server: shutdown signal received");
        }
    }
}

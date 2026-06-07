//! File Manager read-only MCP server daemon.
//!
//! Binds the well-known `system.file-manager` MCP socket and serves the
//! read-only directory and metadata tools to the AI daemon, scoped fail-closed
//! to the configured allowlist. See the crate docs for the design.

use arlen_file_manager_mcp::{load_scope, FileManagerMcp, SERVER_ID};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    // Informational startup check; the scope is (re)loaded per call, so this
    // only reports the posture at boot.
    if load_scope().is_empty() {
        tracing::warn!(
            server_id = SERVER_ID,
            "no scope roots configured; the server will refuse every path until \
             ~/.config/arlen/file-manager-mcp.toml sets [scope] roots"
        );
    }
    tracing::info!(server_id = SERVER_ID, "file manager mcp server starting");

    let make_handler = FileManagerMcp::new;

    tokio::select! {
        result = os_sdk::mcp::serve_mcp(SERVER_ID, make_handler) => {
            if let Err(err) = result {
                tracing::error!(error = %err, "file manager mcp server exited with error");
                std::process::exit(1);
            }
        }
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("file manager mcp server: shutdown signal received");
        }
    }
}

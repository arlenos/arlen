//! The Terminal-run MCP server daemon.
//!
//! Binds the well-known `system.terminal-run` MCP socket and serves the
//! `run_command` tool to the AI daemon (peer-authenticated via `SO_PEERCRED`).
//! The tool is FAIL-CLOSED at the per-action consent boundary: it executes nothing
//! until the biscuit-verify tie-in lands (see the crate docs). run_command is the
//! sharp edge - always Confirm-gated, confined, output-captured, never autonomous.

use arlen_terminal_run_mcp::server::{TerminalRunMcp, SERVER_ID};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    tracing::info!(
        server_id = SERVER_ID,
        "terminal-run mcp server starting (run_command is fail-closed on consent until the \
         biscuit-at-the-boundary tie-in lands)"
    );

    let make_handler = TerminalRunMcp::new;

    tokio::select! {
        result = os_sdk::mcp::serve_mcp(SERVER_ID, make_handler) => {
            if let Err(err) = result {
                tracing::error!(error = %err, "terminal-run mcp server exited with error");
                std::process::exit(1);
            }
        }
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("terminal-run mcp server: shutdown signal received");
        }
    }
}

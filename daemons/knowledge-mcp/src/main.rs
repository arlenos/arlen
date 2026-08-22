//! Knowledge Graph read-only MCP server daemon.
//!
//! Binds the well-known `system.knowledge` MCP socket and serves the
//! read-only query tool to the AI daemon. See the crate docs for the design.

use arlen_knowledge_mcp::{knowledge_socket_path, KnowledgeMcp, SERVER_ID};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        // `warn` for everything, `info` for this server. It answers graph questions
        // for a model; a blanket level puts the transport's frames - and with them
        // the rows - beside its own lines.
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                tracing_subscriber::EnvFilter::new("warn,arlen_knowledge_mcp=info")
            }),
        )
        .init();

    let knowledge_socket = knowledge_socket_path();
    tracing::info!(
        graph_socket = %knowledge_socket,
        server_id = SERVER_ID,
        "knowledge mcp server starting"
    );

    let make_handler = move || KnowledgeMcp::new(knowledge_socket.clone());

    tokio::select! {
        result = os_sdk::mcp::serve_mcp(SERVER_ID, make_handler) => {
            if let Err(err) = result {
                tracing::error!(error = %err, "knowledge mcp server exited with error");
                std::process::exit(1);
            }
        }
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("knowledge mcp server: shutdown signal received");
        }
    }
}

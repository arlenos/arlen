//! `arlen-ai-undo-signer` entry point.
//!
//! The separate-uid helper that seals the AI agent's durable undo-log
//! (reversible-receipts-and-the-effect-model.md EM-R1). It opens the sealed
//! store under the private state directory, then serves submit/lookup requests to
//! the agent over a peer-authenticated socket.
//!
//! Fail-closed posture: if the store cannot be opened safely (a missing key over
//! a non-empty log, a broken HMAC chain, or a head-checkpoint integrity failure),
//! the daemon logs the reason and exits non-zero rather than serving from a
//! suspect log. Undo is then unavailable until the user recovers, which is the
//! correct posture for an integrity helper; a supervisor's restart back-off
//! surfaces the persistent fault.

use std::sync::Arc;

use arlen_ai_undo_signer::server;
use arlen_ai_undo_signer::SignerStore;
use tokio::sync::Mutex;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    tracing::info!("arlen-ai-undo-signer starting");

    // Open the sealed store fail-closed: a tampered or unverifiable log refuses
    // to open, and the daemon exits rather than serving a suspect undo history.
    let store = match SignerStore::open_default() {
        Ok(store) => store,
        Err(e) => {
            tracing::error!("undo-log unavailable, refusing to serve: {e}");
            return Err(e.into());
        }
    };
    let store = Arc::new(Mutex::new(store));
    let socket = server::socket_path();

    tokio::select! {
        result = server::run(&socket, store) => {
            // The accept loop only returns on a bind/accept error.
            if let Err(e) = result {
                tracing::error!("undo-signer serve loop ended: {e}");
                let _ = std::fs::remove_file(&socket);
                return Err(e.into());
            }
        }
        _ = shutdown_signal() => {
            tracing::info!("arlen-ai-undo-signer shutting down");
        }
    }

    // Best-effort: remove the socket so a restart binds cleanly.
    let _ = std::fs::remove_file(&socket);
    Ok(())
}

/// Resolve when a Ctrl-C (SIGINT) or SIGTERM arrives, for graceful shutdown.
async fn shutdown_signal() {
    use tokio::signal::unix::{signal, SignalKind};
    let mut term = match signal(SignalKind::terminate()) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("cannot install SIGTERM handler: {e}");
            // Fall back to Ctrl-C only.
            let _ = tokio::signal::ctrl_c().await;
            return;
        }
    };
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = term.recv() => {}
    }
}

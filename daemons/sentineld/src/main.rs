//! `arlen-sentineld` - the physical-world privacy sentinel.
//!
//! It answers what this machine broadcasts about itself and holds the switches
//! for the detectors that watch for it (`privacy-sentinel-plan.md`). Settings has
//! been asking for this since 26 August and getting nothing, and the page above
//! it was written to tell somebody they are protected, so an unanswered ask there
//! is not a blank card - it is a protection page with no protection behind it.

use std::sync::Arc;

use arlen_sentineld::config;
use arlen_sentineld::server::{bind, run, socket_path, Context};

#[tokio::main]
async fn main() -> std::process::ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                // Both crates this daemon is made of: a target roots at the crate
                // the line was compiled into, so naming one leaves the other mute.
                tracing_subscriber::EnvFilter::new("warn,arlen_sentineld=info")
            }),
        )
        .init();

    let Some(config_path) = config::config_path() else {
        tracing::error!("neither XDG_CONFIG_HOME nor HOME is set, so there is nowhere to keep the detector switches");
        return std::process::ExitCode::FAILURE;
    };
    let socket = socket_path();
    let listener = match bind(&socket) {
        Ok(l) => l,
        Err(e) => {
            tracing::error!("cannot bind {}: {e}", socket.display());
            return std::process::ExitCode::FAILURE;
        }
    };
    let mut term = match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("cannot listen for SIGTERM: {e}");
            return std::process::ExitCode::FAILURE;
        }
    };
    // Lazy by construction: an audit daemon that is not up yet is a failed submit
    // at the moment somebody switches a detector off, which is the fail-closed
    // answer rather than a reason to refuse to start.
    let ctx = Arc::new(Context {
        config_path,
        audit: Arc::new(audit_proto::sink::LedgerAuditSink::at_default_socket()),
    });

    tracing::info!(socket = %socket.display(), "privacy sentinel listening");
    tokio::select! {
        () = run(listener, Arc::clone(&ctx)) => {}
        _ = tokio::signal::ctrl_c() => tracing::info!("interrupted, shutting down"),
        _ = term.recv() => tracing::info!("asked to stop, shutting down"),
    }
    let _ = std::fs::remove_file(&socket);
    std::process::ExitCode::SUCCESS
}

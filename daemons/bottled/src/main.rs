//! `arlen-bottled` - the Windows-app runtime.
//!
//! It holds the bottles: a capability-scoped Wine prefix per app, the drive table
//! derived from that app's grants, and the launch that puts a Windows program
//! inside one. `windows-apps-plan.md` is why this is a daemon and not a library
//! the Settings backend links: a launched Windows program has to outlive the
//! window that started it, and there is no window here to close.

use std::path::PathBuf;

use arlen_wine_core::registry::bottles_dir;
use arlen_wine_core::server::{run, socket_path};

/// `$XDG_DATA_HOME`, else `$HOME/.local/share`.
///
/// Fail rather than guess: a daemon that cannot tell where the person's data
/// lives would create bottles somewhere nobody looks, and a bottle in the wrong
/// place is worse than a daemon that says it could not start.
fn data_home() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("XDG_DATA_HOME") {
        if !d.is_empty() {
            return Some(PathBuf::from(d));
        }
    }
    std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .map(|h| PathBuf::from(h).join(".local/share"))
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    let Some(data) = data_home() else {
        tracing::error!(
            "neither XDG_DATA_HOME nor HOME is set, so there is nowhere to keep bottles"
        );
        return std::process::ExitCode::FAILURE;
    };
    let dir = bottles_dir(&data);
    let socket = socket_path();

    // SIGTERM as well as ctrl-c: a user service is stopped with the first, and a
    // daemon that only listens for the second leaves its socket behind on every
    // logout.
    let mut term = match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("cannot listen for SIGTERM: {e}");
            return std::process::ExitCode::FAILURE;
        }
    };
    let serving = run(&socket, dir);
    tokio::select! {
        result = serving => {
            if let Err(e) = result {
                tracing::error!("stopped serving: {e}");
                let _ = std::fs::remove_file(&socket);
                return std::process::ExitCode::FAILURE;
            }
        }
        _ = tokio::signal::ctrl_c() => tracing::info!("interrupted, shutting down"),
        _ = term.recv() => tracing::info!("asked to stop, shutting down"),
    }
    let _ = std::fs::remove_file(&socket);
    std::process::ExitCode::SUCCESS
}

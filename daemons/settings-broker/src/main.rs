//! The settings broker daemon.
//!
//! Owns writes to apps' `config.toml` files: validates each against the app's
//! declared schema, applies it atomically, and answers with exactly the keys
//! that changed. Reads do not come here - an app reads its own config directly.

use std::path::PathBuf;
use std::sync::Arc;

use arlen_settings_broker::registry::DirectoryRegistry;
use arlen_settings_broker::serve::AppRegistry;
use arlen_settings_broker::server::{run, socket_path};

/// Where installed schemas are looked for, highest precedence first: a
/// user-installed app shadows a system one, matching how the rest of Arlen
/// layers configuration.
fn schema_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(data) = dirs_next_data() {
        dirs.push(data.join("arlen/settings-schemas"));
    }
    dirs.push(PathBuf::from("/usr/share/arlen/settings-schemas"));
    dirs
}

/// `$XDG_DATA_HOME`, else `~/.local/share`.
fn dirs_next_data() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("XDG_DATA_HOME") {
        let path = PathBuf::from(dir);
        if path.is_absolute() {
            return Some(path);
        }
    }
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share"))
}

/// The directory holding apps' config files.
///
/// Derived from the SDK's own `config_path`, not written out again here: the
/// broker must write exactly where `Config::load` reads, and two independent
/// copies of that layout would eventually disagree. Asking the SDK for a known
/// app-id and taking its parent keeps them the same by construction.
fn config_dir() -> PathBuf {
    os_sdk::config::config_path("probe")
        .parent()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/etc/arlen"))
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() {
    let Some(socket) = socket_path() else {
        eprintln!("settings-broker: no XDG_RUNTIME_DIR; cannot bind the socket");
        std::process::exit(1);
    };

    let registry: Arc<dyn AppRegistry> =
        Arc::new(DirectoryRegistry::new(schema_dirs(), config_dir()));

    eprintln!("settings-broker: serving on {}", socket.display());

    tokio::select! {
        result = run(registry, &socket) => {
            if let Err(e) = result {
                eprintln!("settings-broker: {e}");
            }
        }
        _ = shutdown_signal() => {
            eprintln!("settings-broker: shutting down");
        }
    }

    // Best effort: leave no stale socket for the next run to trip over.
    let _ = std::fs::remove_file(&socket);
}

/// Resolve on SIGINT or SIGTERM.
async fn shutdown_signal() {
    use tokio::signal::unix::{signal, SignalKind};
    let mut term = match signal(SignalKind::terminate()) {
        Ok(s) => s,
        Err(_) => return,
    };
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = term.recv() => {}
    }
}

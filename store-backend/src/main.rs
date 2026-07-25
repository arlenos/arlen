//! The `arlen-store-backend` daemon: compose the merged app catalog from the local
//! metadata the image ships, then serve it over `org.arlen.Store1` on the session
//! socket (store-app.md section 9).
//!
//! This is the thin runnable shell over the tested library. Source metadata is read
//! from configured paths (all optional, tolerating absence so a fresh image with no
//! catalog yet serves an empty store rather than failing); the live catalog refresh +
//! the canonical on-image metadata paths are the section 9.6 deployment detail. The
//! backend performs NO network I/O today (the compose reads local files), so the unit
//! denies egress outright; the per-host allowlist lands with the live fetchers.

use std::path::PathBuf;
use std::sync::Arc;

use arlen_store_backend::{compose_catalog, serve, SourceInputs, SourceLayer};

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() {
    let Some(socket) = serve::socket_path() else {
        eprintln!("store-backend: no XDG_RUNTIME_DIR; cannot bind the store socket");
        std::process::exit(1);
    };

    let catalog = Arc::new(compose_catalog(load_source_inputs()));
    eprintln!("store-backend: serving {} on {}", catalog_summary(&catalog), socket.display());

    tokio::select! {
        result = serve::run(Arc::clone(&catalog), &socket) => {
            if let Err(e) = result {
                eprintln!("store-backend: serve loop ended: {e:?}");
            }
        }
        _ = shutdown_signal() => {
            eprintln!("store-backend: shutting down");
        }
    }
    // Best-effort: leave no stale socket behind.
    let _ = std::fs::remove_file(&socket);
}

/// Read the source metadata from configured paths, all optional. A path that is unset
/// or missing contributes nothing (the compose is best-effort), so the daemon runs on
/// an image that ships only some catalogs, or none.
fn load_source_inputs() -> SourceInputs {
    let read_env = |var: &str| {
        std::env::var_os(var)
            .map(PathBuf::from)
            .filter(|p| p.is_file())
            .and_then(|p| std::fs::read_to_string(p).ok())
    };
    // A single forage recipe path (the multi-cookbook tiering is the resolver's job,
    // wired with the refresh step); default it to the Official tier for the skeleton.
    let forage = read_env("ARLEN_STORE_FORAGE_RECIPE")
        .map(|toml| vec![(toml, SourceLayer::Official)])
        .unwrap_or_default();
    SourceInputs {
        forage,
        flathub_xml: read_env("ARLEN_STORE_FLATHUB_XML"),
        dep11_yaml: read_env("ARLEN_STORE_DEP11_YAML"),
    }
}

/// A one-line summary of the composed catalog for the startup log.
fn catalog_summary(catalog: &arlen_store_backend::Catalog) -> String {
    // The catalog has no public len; a search with an empty query returns every card.
    let n = catalog.search("", &[]).len();
    format!("{n} app(s)")
}

/// Resolve on SIGTERM (systemd stop) or SIGINT (Ctrl-C).
async fn shutdown_signal() {
    use tokio::signal::unix::{signal, SignalKind};
    let mut term = match signal(SignalKind::terminate()) {
        Ok(s) => s,
        Err(_) => {
            let _ = tokio::signal::ctrl_c().await;
            return;
        }
    };
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = term.recv() => {}
    }
}

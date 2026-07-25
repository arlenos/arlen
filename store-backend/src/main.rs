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
            .and_then(|p| read_catalog(&p))
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

/// Read a catalog file to a UTF-8 string, transparently decompressing gzip. The
/// deployed Flathub AppStream and Debian DEP-11 catalogs ship gzipped
/// (`.xml.gz`/`.yml.gz`), so a plain read would hand the parser garbage and silently
/// drop the whole source; detection is by the gzip magic bytes (robust to a
/// `.gz`-less or mislabelled path), not the extension. `None` on an unreadable file
/// or invalid UTF-8.
fn read_catalog(path: &std::path::Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    if bytes.starts_with(&[0x1f, 0x8b]) {
        use std::io::Read;
        let mut out = String::new();
        flate2::read::GzDecoder::new(&bytes[..]).read_to_string(&mut out).ok()?;
        Some(out)
    } else {
        String::from_utf8(bytes).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::read_catalog;

    fn tmp_file(name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("store-read-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(name);
        std::fs::write(&p, bytes).unwrap();
        p
    }

    #[test]
    fn reads_a_plain_utf8_catalog() {
        let p = tmp_file("plain.xml", b"<components/>");
        assert_eq!(read_catalog(&p).as_deref(), Some("<components/>"));
    }

    #[test]
    fn transparently_decompresses_a_gzipped_catalog() {
        use flate2::{write::GzEncoder, Compression};
        use std::io::Write;
        let mut enc = GzEncoder::new(Vec::new(), Compression::default());
        enc.write_all(b"File: DEP-11\n").unwrap();
        let gz = enc.finish().unwrap();
        // Named without a .gz suffix on purpose: detection is by magic bytes.
        let p = tmp_file("catalog.yml", &gz);
        assert_eq!(read_catalog(&p).as_deref(), Some("File: DEP-11\n"));
    }

    #[test]
    fn a_missing_file_is_none() {
        assert!(read_catalog(std::path::Path::new("/nonexistent/store-x.xml")).is_none());
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

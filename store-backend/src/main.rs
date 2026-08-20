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

use arlen_store_backend::{compose_catalog, serve, CatalogInput, SourceInputs, SourceLayer};

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() {
    let Some(socket) = serve::socket_path() else {
        eprintln!("store-backend: no XDG_RUNTIME_DIR; cannot bind the store socket");
        std::process::exit(1);
    };

    let catalog = serve::shared(compose_catalog(load_source_inputs()));
    let count = catalog.lock().map(|c| c.search("", &[]).len()).unwrap_or(0);
    eprintln!("store-backend: serving {count} app(s) on {}", socket.display());

    // Periodically re-compose the catalog so it tracks the on-disk metadata without a
    // restart (store-app.md section 9.3). `ARLEN_STORE_REFRESH_SECS=0` disables it.
    if let Some(interval) = refresh_interval() {
        let catalog = catalog.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(interval).await;
                // The compose reads + parses files (blocking), off the runtime.
                if let Ok(fresh) = tokio::task::spawn_blocking(|| {
                    compose_catalog(load_source_inputs())
                })
                .await
                {
                    serve::swap(&catalog, fresh);
                }
            }
        });
    }

    tokio::select! {
        result = serve::run(catalog, &socket) => {
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

/// The catalog-refresh interval from `ARLEN_STORE_REFRESH_SECS` (default 3600s = 1h).
/// `0` disables the periodic refresh (the catalog is then composed once at startup).
fn refresh_interval() -> Option<std::time::Duration> {
    let secs = std::env::var("ARLEN_STORE_REFRESH_SECS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(3600);
    (secs > 0).then(|| std::time::Duration::from_secs(secs))
}

/// Read the source metadata from configured paths, all optional. A path that is unset
/// or missing contributes nothing (the compose is best-effort), so the daemon runs on
/// an image that ships only some catalogs, or none.
fn load_source_inputs() -> SourceInputs {
    // Where the metadata actually is. Env vars still override every source, so
    // discovery is the default rather than a replacement.
    let found = arlen_store_backend::discover::discover(&Default::default());
    let read_env = |var: &str| {
        std::env::var_os(var)
            .map(PathBuf::from)
            .filter(|p| p.is_file())
            .and_then(|p| arlen_store_backend::discover::read_catalog(&p))
    };
    // A single forage recipe path (the multi-cookbook tiering is the resolver's job,
    // wired with the refresh step); default it to the Official tier for the skeleton.
    //
    // No cookbook origin, and that is the truthful answer for this input rather
    // than a gap in the plumbing: a recipe named by a path came from a path. It
    // is in no tracked cookbook, so there is no publisher to name and no TUF
    // chain to point at. The catalog rows that CAN carry an origin are the ones
    // enumerated from the registry's own cookbooks, which is the next step and
    // needs recipe discovery inside a cookbook clone.
    let forage = read_env("ARLEN_STORE_FORAGE_RECIPE")
        .map(|toml| vec![(toml, SourceLayer::Official, None)])
        .unwrap_or_default();
    SourceInputs {
        forage,
        // An env override replaces discovery for that source rather than adding to
        // it, so a test pointing at one fixture catalog gets exactly that catalog
        // and not the build machine's installed apps mixed in.
        flathub_xml: read_env("ARLEN_STORE_FLATHUB_XML")
            .map(|xml| vec![xml])
            .unwrap_or_else(|| read_all(&found.flathub_xml)),
        dep11_yaml: read_env("ARLEN_STORE_DEP11_YAML")
            .map(|yaml| vec![yaml.into()])
            .unwrap_or_else(|| read_catalogs(&found.dep11_yaml)),
        // No env override: this one is discovered by origin name, and a single
        // file with no name to carry could not say which layer it belongs to.
        catalog_xml: found
            .catalog_xml
            .iter()
            .filter_map(|(origin, path)| {
                Some(CatalogInput {
                    text: arlen_store_backend::discover::read_catalog(path)?,
                    root: swcatalog_root(path),
                    origin: Some(origin.clone()),
                })
            })
            .collect(),
        // `ARLEN_STORE_METAINFO_DIR` REPLACES the discovered roots, like the other
        // source overrides, and exists for the same reason they do: without it
        // there is no way to run this daemon against a known catalog. The other
        // sources can be pointed at a fixture file; this one would always read
        // whatever the host has installed, so a harness bringing the store up
        // hermetically would still get the machine's own apps mixed into its
        // catalog. I first wrote "a test sets the roots, not a file" here, which
        // is true of the library and useless to anything that runs the binary.
        metainfo_xml: match std::env::var_os("ARLEN_STORE_METAINFO_DIR") {
            Some(dir) => {
                let roots = arlen_store_backend::discover::SourceRoots {
                    metainfo_dirs: vec![PathBuf::from(dir)],
                    ..Default::default()
                };
                read_all(&arlen_store_backend::discover::discover(&roots).metainfo_xml)
            }
            None => read_all(&found.metainfo_xml),
        },
        // SC-3 + SC-5: the sandbox permissions and enrolled profiles, located on
        // the machine rather than configured. An env override still wins above,
        // so a test or a bespoke deployment can point at its own tree.
        flatpak_metadata: read_pairs(&found.flatpak_metadata),
        apt_profiles: read_pairs(&found.apt_profiles),
        // What other people made of an app, from a document somebody else
        // fetched. THE DAEMON DOES NOT FETCH IT, and that is a posture rather
        // than an omission: the header of this file says the backend performs no
        // network I/O and its unit denies egress outright, so the first request
        // out of here is a change to what this daemon may reach - and it lands
        // with the allowlist that permits it, not ahead of it. Until then a
        // cached document at `ARLEN_STORE_ODRS_JSON`, or under the state dir a
        // future refresher writes to, is read if present and nothing is claimed
        // if it is absent.
        odrs: odrs_ratings(),
    }
}

/// The ODRS ratings document, if this machine has one.
///
/// Absent is the ordinary case and not an error: an app with no score shows no
/// row, which is the honest rendering of "nobody asked" and of "nobody rated
/// it" alike. A document that will not parse is also absent rather than fatal -
/// a corrupt cache must cost the ratings row, not the catalogue.
fn odrs_ratings() -> Option<arlen_store_backend::odrs::Ratings> {
    // The user's own copy first, then the one the image was built with. The order
    // is what lets a refresher land later without touching this: whatever writes
    // the state file wins over the shipped document, and until something does, the
    // shipped one is what a fresh install reads.
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(p) = std::env::var_os("ARLEN_STORE_ODRS_JSON") {
        candidates.push(PathBuf::from(p));
    } else {
        if let Some(base) = std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))
        {
            candidates.push(base.join("arlen/store/odrs-ratings.json"));
        }
        candidates.push(PathBuf::from("/usr/share/arlen/store/odrs-ratings.json"));
    }
    let (path, text) = candidates
        .into_iter()
        .find_map(|p| Some((p.clone(), std::fs::read_to_string(&p).ok()?)))?;
    match arlen_store_backend::odrs::Ratings::parse(&text) {
        Ok(r) => {
            eprintln!("store-backend: odrs ratings for {} app(s)", r.len());
            Some(r)
        }
        Err(e) => {
            eprintln!("store-backend: ignoring the odrs cache at {}: {e}", path.display());
            None
        }
    }
}

/// Read every discovered catalog, dropping any that cannot be read.
/// Read each catalogue, remembering the `swcatalog` directory it sits in so the
/// icon names inside it can be resolved to files. Unreadable files are skipped, the
/// same best-effort rule the rest of discovery follows.
fn read_catalogs(paths: &[std::path::PathBuf]) -> Vec<CatalogInput> {
    paths
        .iter()
        .filter_map(|p| {
            Some(CatalogInput {
                text: arlen_store_backend::discover::read_catalog(p)?,
                root: swcatalog_root(p),
                origin: None,
            })
        })
        .collect()
}

/// The `swcatalog` directory a catalogue file sits under: both forms live one
/// directory down (`<root>/yaml/x.yml.gz`, `<root>/xml/x.xml.gz`).
fn swcatalog_root(path: &std::path::Path) -> Option<std::path::PathBuf> {
    path.parent().and_then(|d| d.parent()).map(|d| d.to_path_buf())
}

fn read_all(paths: &[std::path::PathBuf]) -> Vec<String> {
    paths.iter().filter_map(|p| arlen_store_backend::discover::read_catalog(p)).collect()
}

/// Read each discovered `(id, path)` pair, dropping any that cannot be read.
///
/// A file that vanished between discovery and read, or is not UTF-8, is skipped
/// rather than failing the whole compose - one unreadable app's metadata must
/// not cost the user their entire catalog.
fn read_pairs(pairs: &[(String, std::path::PathBuf)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .filter_map(|(id, path)| Some((id.clone(), arlen_store_backend::discover::read_catalog(path)?)))
        .collect()
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

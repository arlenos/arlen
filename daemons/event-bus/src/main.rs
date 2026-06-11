#![warn(clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]
#![allow(clippy::must_use_candidate)]

mod proto {
    #![allow(dead_code)]
    #![allow(clippy::doc_markdown)]
    include!(concat!(env!("OUT_DIR"), "/arlen.eventbus.rs"));
}

mod registry;
mod socket;
mod validation;

use anyhow::Result;
use registry::ConsumerRegistry;
use std::path::PathBuf;
use tracing::info;

/// Resolve a daemon socket path per the standard Arlen 3-tier
/// convention: the `env_var` override (non-empty) wins, else
/// `$XDG_RUNTIME_DIR/arlen/<file_name>` (the per-user path, i.e.
/// `/run/user/{uid}/arlen/<file_name>`), else `/run/arlen/<file_name>`.
///
/// event-bus is a leaf daemon that does not depend on `os-sdk`, so the
/// shared `os_sdk::runtime::socket_path` resolver is reproduced here by
/// hand rather than pulling in that crate's surface. The precedence
/// must match it exactly: the `ARLEN_*_SOCKET` env override stays
/// tier 1 — it is the contract the dev stack and the integration
/// harness pin sockets through.
fn socket_path(env_var: &str, file_name: &str) -> PathBuf {
    let pinned = std::env::var(env_var).ok().filter(|s| !s.is_empty());
    let xdg = std::env::var("XDG_RUNTIME_DIR").ok();
    let path = resolve_socket(pinned.as_deref(), xdg.as_deref(), file_name);
    // Best-effort: ensure the per-user `arlen/` parent exists so the
    // daemon binds cleanly in a dev session. Skip when env-pinned (the
    // launcher owns that parent).
    if pinned.is_none() {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
    }
    path
}

/// Pure precedence backing [`socket_path`], shared with its tests so
/// the contract is checked without mutating process env.
fn resolve_socket(pinned: Option<&str>, xdg: Option<&str>, file_name: &str) -> PathBuf {
    if let Some(p) = pinned {
        return PathBuf::from(p);
    }
    if let Some(dir) = xdg.filter(|s| !s.is_empty()) {
        return PathBuf::from(dir).join("arlen").join(file_name);
    }
    PathBuf::from("/run").join("arlen").join(file_name)
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("event_bus=debug".parse()?),
        )
        .init();

    // Resolve socket paths per-user, overridable via the env vars the
    // dev stack and integration tests pin without modifying the binary.
    let producer_socket = socket_path("ARLEN_PRODUCER_SOCKET", "event-bus-producer.sock");
    let consumer_socket = socket_path("ARLEN_CONSUMER_SOCKET", "event-bus-consumer.sock");

    info!("starting event bus daemon");

    let registry = ConsumerRegistry::new();
    socket::listen(
        &producer_socket.to_string_lossy(),
        &consumer_socket.to_string_lossy(),
        registry,
    )
    .await?;

    Ok(())
}

#[cfg(test)]
mod socket_path_tests {
    use super::resolve_socket;
    use std::path::PathBuf;

    #[test]
    fn env_override_wins() {
        // `pinned` is pre-filtered for non-empty by `socket_path`, so a
        // present override is honoured outright here.
        let p = resolve_socket(Some("/pinned.sock"), Some("/run/user/1000"), "x.sock");
        assert_eq!(p, PathBuf::from("/pinned.sock"));
    }

    #[test]
    fn no_pin_falls_through_to_xdg() {
        // `socket_path` maps an empty `ARLEN_*_SOCKET` to `None`, so the
        // resolver sees no pin and derives the per-user path.
        let p = resolve_socket(None, Some("/run/user/1000"), "event-bus-producer.sock");
        assert_eq!(
            p,
            PathBuf::from("/run/user/1000/arlen/event-bus-producer.sock")
        );
    }

    #[test]
    fn xdg_is_per_user() {
        let p = resolve_socket(None, Some("/run/user/1000"), "event-bus-consumer.sock");
        assert_eq!(
            p,
            PathBuf::from("/run/user/1000/arlen/event-bus-consumer.sock")
        );
    }

    #[test]
    fn empty_xdg_falls_to_run_arlen() {
        let p = resolve_socket(None, Some(""), "event-bus-producer.sock");
        assert_eq!(p, PathBuf::from("/run/arlen/event-bus-producer.sock"));
    }

    #[test]
    fn run_arlen_last_resort() {
        let p = resolve_socket(None, None, "event-bus-producer.sock");
        assert_eq!(p, PathBuf::from("/run/arlen/event-bus-producer.sock"));
    }
}

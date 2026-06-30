//! The config-broker daemon: the separate-uid owner of the AI master
//! switches. Run as a dedicated uid (the systemd unit's `User=`), it
//! holds the canonical state in a directory the user's normal uid
//! cannot write and mutates it only over a SO_PEERPIDFD-authenticated
//! socket - so a same-uid process can no longer silently flip
//! `executor_live`, widen `access_level`, repoint `provider`, or
//! grant itself autonomy. (`same-uid-isolation-plan.md` Tier-A #1.)

use std::sync::Arc;

use arlen_config_broker::server;
use arlen_config_broker::state::{AiMasterSwitches, StateStore};

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    // Fail closed: if the canonical store cannot be opened (no state
    // dir, an un-tightenable directory), refuse to run rather than
    // serve from a guessed location.
    let store = match StateStore::open_default() {
        Ok(s) => Arc::new(s),
        Err(e) => {
            tracing::error!("cannot open config-broker store: {e}");
            std::process::exit(1);
        }
    };

    // Seed the generous shipped defaults on first run (a fresh store
    // only); never clobber an existing one. A seed failure is
    // non-fatal: a read then resolves to the fail-closed floor, which
    // is safe.
    match store.seed_if_absent(&AiMasterSwitches::shipped_default()) {
        Ok(true) => tracing::info!("seeded the shipped AI defaults into a fresh store"),
        Ok(false) => {}
        Err(e) => tracing::warn!("could not seed defaults: {e}"),
    }

    let socket = server::socket_path();

    // All synchronous filesystem setup that the fence does NOT permit
    // (creating the socket parent dir) happens here, BEFORE the fence:
    // the socket dir must exist so its write grant is expressible (an
    // unopenable grant is skipped fail-safe) and so the post-fence
    // `bind_socket` only ever creates the socket node inside it.
    if let Some(parent) = socket.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            tracing::warn!("could not pre-create the socket dir {}: {e}", parent.display());
        }
    }

    // Self-confine: read everywhere, write only under the store dir and
    // the socket dir - the daemon's entire legitimate footprint. This is
    // applied on the main thread BEFORE the runtime starts so every tokio
    // worker inherits the Landlock domain (a domain is inherited only by
    // threads created after `restrict_self`).
    apply_fence(store.dir(), socket.parent());

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            tracing::error!("cannot build the tokio runtime: {e}");
            std::process::exit(1);
        }
    };

    runtime.block_on(async {
        tokio::select! {
            r = server::run(Arc::clone(&store), &socket) => {
                if let Err(e) = r {
                    tracing::error!("serve loop ended: {e}");
                }
            }
            _ = shutdown_signal() => {
                tracing::info!("shutting down");
            }
        }
    });
    let _ = std::fs::remove_file(&socket);
}

/// Install the Landlock write-fence over the store and socket dirs. The
/// fence is defense-in-depth: a kernel that cannot enforce it (Linux <
/// 5.13) leaves the broker exactly as safe as no fence, so by default a
/// non-enforcing kernel or a ruleset error is logged and the daemon
/// continues rather than refusing to serve the switches over a missing
/// hardening add-on.
///
/// A hardened deployment that wants the confinement *guaranteed* (not
/// best-effort) sets `ARLEN_CONFIG_BROKER_REQUIRE_FENCE=1`: then a
/// non-enforcing kernel or a ruleset error is fatal, so "the broker is
/// running" implies "the broker is write-confined" - an assertable
/// property instead of a log line that scrolls away.
#[cfg(target_os = "linux")]
fn apply_fence(store_dir: &std::path::Path, socket_dir: Option<&std::path::Path>) {
    use arlen_config_broker::landlock_fence::{fence_writes, FenceOutcome};
    let require = std::env::var_os("ARLEN_CONFIG_BROKER_REQUIRE_FENCE")
        .is_some_and(|v| v == "1");
    let mut writable: Vec<&std::path::Path> = vec![store_dir];
    if let Some(d) = socket_dir {
        writable.push(d);
    }
    let degraded = match fence_writes(&writable) {
        Ok(FenceOutcome::Enforced) => {
            tracing::info!("landlock write-fence enforced (write-confined to store + socket dirs)");
            None
        }
        Ok(FenceOutcome::NotEnforced) => {
            Some("landlock not enforced by this kernel".to_string())
        }
        Err(e) => Some(format!("landlock fence not applied: {e}")),
    };
    if let Some(reason) = degraded {
        if require {
            tracing::error!(
                "ARLEN_CONFIG_BROKER_REQUIRE_FENCE=1 but the fence is not active ({reason}); refusing to run unconfined"
            );
            std::process::exit(1);
        }
        tracing::warn!("{reason}; running unconfined (no worse than no fence)");
    }
}

/// On a non-Linux target there is no Landlock; the fence is a no-op.
#[cfg(not(target_os = "linux"))]
fn apply_fence(_store_dir: &std::path::Path, _socket_dir: Option<&std::path::Path>) {}

/// Resolve on SIGTERM (systemd stop) or SIGINT (Ctrl-C).
async fn shutdown_signal() {
    use tokio::signal::unix::{signal, SignalKind};
    let mut term = signal(SignalKind::terminate()).expect("install SIGTERM handler");
    let mut intr = signal(SignalKind::interrupt()).expect("install SIGINT handler");
    tokio::select! {
        _ = term.recv() => {}
        _ = intr.recv() => {}
    }
}

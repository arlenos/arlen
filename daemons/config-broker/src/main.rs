//! The config-broker daemon: the separate-uid owner of the AI master
//! switches. Run as a dedicated uid (the systemd unit's `User=`), it
//! holds the canonical state in a directory the user's normal uid
//! cannot write and mutates it only over a SO_PEERPIDFD-authenticated
//! socket - so a same-uid process can no longer silently flip
//! `executor_live`, widen `access_level`, repoint `provider`, or
//! grant itself autonomy. (`same-uid-isolation-plan.md` Tier-A #1.)

use std::sync::Arc;

use arlen_config_broker::server;
use arlen_config_broker::state::{seed_from_ai_toml, StateStore};

#[tokio::main]
async fn main() {
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

    // Seed on first run only (a fresh store); never clobber an existing one. The
    // seed MIGRATES the user's current `ai.toml` master switches so the cutover to
    // the broker preserves their settings (a customized `enabled`/`provider`/
    // `executor_live` carries over) rather than resetting them to the shipped
    // defaults; a missing/unreadable ai.toml falls back to the shipped default. A
    // seed failure is non-fatal: a read then resolves to the fail-closed floor.
    match store.seed_if_absent(&seed_from_ai_toml()) {
        Ok(true) => tracing::info!("seeded the AI master switches from ai.toml (or shipped defaults) into a fresh store"),
        Ok(false) => {}
        Err(e) => tracing::warn!("could not seed the store: {e}"),
    }

    let socket = server::socket_path();

    // The audit ledger sink: a change to a security-relevant AI master
    // switch is recorded to the OWNER's user auditd (there is ONE ledger,
    // the owner's - the broker runs as a distinct uid, so the default
    // resolver's own-runtime-dir / `/run/arlen` path is bound by nothing;
    // `owner_audit_socket` targets `/run/user/<owner_uid>/arlen`). The
    // audit policy is asymmetric fail-closed ([`server::apply_set_audited`]):
    // an authority-adding flip is refused if it cannot be recorded, while
    // the off-switch direction always applies. Connects lazily per submit.
    let audit_socket = server::owner_audit_socket();
    tracing::info!(audit_socket = %audit_socket.display(), "audit sink target");
    let sink: Arc<dyn audit_proto::sink::AuditSink> = Arc::new(
        audit_proto::sink::LedgerAuditSink::new(audit_proto::client::AuditClient::new(audit_socket)),
    );

    tokio::select! {
        r = server::run(Arc::clone(&store), &socket, Arc::clone(&sink)) => {
            if let Err(e) = r {
                tracing::error!("serve loop ended: {e}");
            }
        }
        _ = shutdown_signal() => {
            tracing::info!("shutting down");
        }
    }
    let _ = std::fs::remove_file(&socket);
}

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

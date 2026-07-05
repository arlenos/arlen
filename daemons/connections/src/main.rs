//! The Arlen Connections daemon (`org.arlen.Connections1`): the single
//! capability-gated credential authority.
//!
//! It opens the master-key-sealed credential store, owns the well-known bus name,
//! and serves the [`ConnectionsDaemon`] interface. Every request is authorized
//! against the declarative grant config and audited (CONN-R1); the downscoped
//! token delivery lands in CONN-R2. Fail-closed startup: if the state home or the
//! master key cannot be resolved, the daemon refuses to start rather than serve
//! without a store.

use audit_proto::LedgerAuditSink;
use connections::dbus::ConnectionsDaemon;
use connections::master::{self, MasterSecret};
use connections::store::CredentialStore;
use zbus::connection;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let dir = master::state_dir().ok_or("no state home for the credential store")?;
    let key_path = master::master_key_path().ok_or("no state home for the master secret")?;
    let master = MasterSecret::load_or_create(&key_path)?;
    let store = CredentialStore::new(*master.bytes(), &dir);
    tracing::info!(dir = %dir.display(), "credential store opened");

    let audit = LedgerAuditSink::at_default_socket();
    let daemon = ConnectionsDaemon::new(store, audit);

    let _conn = connection::Builder::session()?
        .name("org.arlen.Connections1")?
        .serve_at("/org/arlen/Connections1", daemon)?
        .build()
        .await?;
    tracing::info!("org.arlen.Connections1 serving; every handout is capability-gated + audited");

    // Serve until asked to stop.
    shutdown_signal().await;
    tracing::info!("shutting down");
    Ok(())
}

/// Resolve on the first of SIGINT or SIGTERM.
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

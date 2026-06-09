//! The `capsuled` daemon: serve same-machine Context Capsule reads
//! (context-capsule.md §6).
//!
//! Opens the persisted capsule signing key, the frozen-slice store and the durable
//! revoke/op-count ledger, then serves the SO_PEERCRED Unix socket. A reader
//! presents a signed grant; a valid, unrevoked, unexpired, in-budget grant gets
//! the frozen slice, every read audited fail-closed. Minting (which materializes a
//! slice and registers a grant) is the human-gated surface (CC-R6); this daemon is
//! the serve + revoke-enforcement half.

use std::sync::Arc;

use arlen_forage_store::Store;
use audit_proto::LedgerAuditSink;
use capsuled::key::{capsule_key_path, CapsuleSigningKey};
use capsuled::revocation::RevocationFile;
use capsuled::server::{run, socket_path, ServeContext};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let key_path = capsule_key_path().ok_or("no XDG_STATE_HOME or HOME for the capsule key")?;
    let key = CapsuleSigningKey::load_or_create(&key_path)?;
    let verifying_key = key.verifying_key();

    // The capsule state dir is the signing key's parent (arlen/capsule/): the slice
    // store and the revoke ledger live alongside it.
    let state_dir = key_path
        .parent()
        .ok_or("capsule key path has no parent")?
        .to_path_buf();
    let store = Store::open(state_dir.join("store"))?;
    let ledger = RevocationFile::open(&state_dir)?;
    let audit = LedgerAuditSink::at_default_socket();

    let sock = socket_path().ok_or("no XDG_RUNTIME_DIR for the capsule socket")?;
    let ctx = ServeContext {
        verifying_key,
        ledger: Arc::new(ledger),
        store: Arc::new(store),
        audit: Arc::new(audit),
    };

    tracing::info!(socket = %sock.display(), "capsule daemon listening");
    tokio::select! {
        r = run(&sock, ctx) => { r?; }
        _ = shutdown_signal() => { tracing::info!("capsule daemon shutting down"); }
    }
    // Best-effort socket cleanup on a clean exit.
    let _ = std::fs::remove_file(&sock);
    Ok(())
}

/// Resolve when the daemon should shut down: SIGINT (ctrl-c) or SIGTERM.
async fn shutdown_signal() {
    let ctrl_c = tokio::signal::ctrl_c();
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .expect("install SIGTERM handler");
    tokio::select! {
        _ = ctrl_c => {}
        _ = term.recv() => {}
    }
}

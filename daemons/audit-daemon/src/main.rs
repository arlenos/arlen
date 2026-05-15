//! `lunaris-auditd` entry point.
//!
//! S13.1 ships the ledger core only (`lunaris_auditd::ledger`). The
//! ingest socket (S13.3), the read API (S13.4), and the startup
//! chain-verification + `audit.tampered` handling (S13.6) are wired
//! in over the following sub-sprints. Until then this binary opens
//! the ledger and verifies the existing chain as a smoke check.

use std::path::PathBuf;

use lunaris_auditd::ledger::Ledger;

/// Resolve the ledger path: `$XDG_DATA_HOME/lunaris/audit/ledger.db`,
/// falling back to `~/.local/share/...`. The audit log is per-user
/// (foundation §8.4.12).
fn ledger_path() -> PathBuf {
    let base = std::env::var("XDG_DATA_HOME")
        .ok()
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .map(|h| PathBuf::from(h).join(".local/share"))
        })
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    base.join("lunaris/audit/ledger.db")
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    tracing::info!("lunaris-auditd starting (S13.1: ledger core only)");

    // S13.2 replaces this placeholder key with the keyring-backed,
    // persistent HMAC key. A throwaway key here would make every
    // restart's verification fail, so the smoke check uses a fixed
    // placeholder; it is not a security boundary yet.
    let key = b"s13.1-placeholder-hmac-key".to_vec();
    let path = ledger_path();
    let ledger = Ledger::open(&path, key).await?;
    match ledger.verify().await {
        Ok(count) => tracing::info!(entries = count, "audit chain verified"),
        Err(err) => tracing::error!("audit chain verification failed: {err}"),
    }

    tracing::info!(
        "lunaris-auditd: socket layer not yet wired (lands in S13.3); exiting"
    );
    Ok(())
}

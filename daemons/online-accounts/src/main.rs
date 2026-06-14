//! The Arlen online-accounts daemon (`org.arlen.Accounts1`).
//!
//! Loads the account configs (surfacing malformed ones rather than granting
//! them) and serves the capability-gated D-Bus surface. Every method resolves the
//! caller's app id and consults the gate, so an app reaches only its granted
//! accounts. The per-account ObjectManager objects, the typed per-service
//! interfaces and the Secret Service token handout build on this skeleton.

use online_accounts::dbus::AccountsDaemon;
use online_accounts::vault::{vault_dir, Vault};
use online_accounts::{config, master};
use zbus::connection;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let dir = config::accounts_dir().ok_or("no config home for account configs")?;
    let (accounts, errors) = config::load_accounts(&dir);
    for (path, err) in &errors {
        // A malformed config is reported and skipped, never granted.
        tracing::warn!(path = %path.display(), %err, "skipping malformed account config");
    }
    tracing::info!(count = accounts.len(), dir = %dir.display(), "accounts loaded");

    // The token vault: AEAD-encrypted tokens under the persistent master secret.
    // A missing vault dir or an unloadable master is fatal (fail-closed: the
    // daemon must not serve token handouts it cannot decrypt or persist).
    let vdir = vault_dir().ok_or("no state home for the token vault")?;
    let key_path = master::master_key_path().ok_or("no state home for the master secret")?;
    let master = master::MasterSecret::load_or_create(&key_path)?;
    let vault = Vault::new(*master.bytes(), &vdir);
    tracing::info!(dir = %vdir.display(), "token vault opened");

    let _conn = connection::Builder::session()?
        .name("org.arlen.Accounts1")?
        .serve_at("/org/arlen/Accounts1", AccountsDaemon::new(accounts, vault))?
        .build()
        .await?;
    tracing::info!("org.arlen.Accounts1 serving; the per-app gate mediates every method");

    // Serve until terminated.
    tokio::signal::ctrl_c().await?;
    tracing::info!("shutting down");
    Ok(())
}

//! The Arlen online-accounts daemon (`org.arlen.Accounts1`).
//!
//! Loads the account configs (surfacing malformed ones rather than granting
//! them) and serves the capability-gated D-Bus surface. Every method resolves the
//! caller's app id and consults the gate, so an app reaches only its granted
//! accounts, and the token handout reads the encrypted vault only after the gate
//! admits the caller. The per-account ObjectManager + typed per-service
//! interfaces are deferred for a security reason (a naive same-uid enumeration
//! would regress per-caller visibility); see the crate docs.

use std::path::Path;

use online_accounts::dbus::AccountsDaemon;
use online_accounts::vault::{vault_dir, Vault};
use online_accounts::{config, master};
use zbus::connection;

fn main() -> Result<(), Box<dyn std::error::Error>> {
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
    // Startup diagnostic only; the daemon re-reads the configs per call so a
    // grant change (add or revoke) takes effect without a restart.
    tracing::info!(count = accounts.len(), dir = %dir.display(), "accounts loaded");

    // The token vault: AEAD-encrypted tokens under the persistent master secret.
    // A missing vault dir or an unloadable master is fatal (fail-closed: the
    // daemon must not serve token handouts it cannot decrypt or persist).
    let vdir = vault_dir().ok_or("no state home for the token vault")?;
    let key_path = master::master_key_path().ok_or("no state home for the master secret")?;
    // Create the vault dir 0700 before the fence: the master.key write below and
    // every later token write land here, and the fence's write grant is only
    // expressible if the dir exists (an absent grant path is skipped fail-safe).
    {
        use std::os::unix::fs::DirBuilderExt;
        let _ = std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&vdir);
    }
    let master = master::MasterSecret::load_or_create(&key_path)?;
    let vault = Vault::new(*master.bytes(), &vdir);
    tracing::info!(dir = %vdir.display(), "token vault opened");

    // Self-confine: read everywhere, write only under the vault dir - the
    // daemon's entire filesystem footprint (master.key + the per-account .vault
    // records; the account configs are read-only and the session bus is a
    // connect, not a write). Applied on the main thread BEFORE the runtime is
    // built so every tokio worker (and zbus task) inherits the Landlock domain.
    apply_fence(&vdir);

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async move {
        let _conn = connection::Builder::session()?
            .name("org.arlen.Accounts1")?
            .serve_at("/org/arlen/Accounts1", AccountsDaemon::new(dir, vault))?
            .build()
            .await?;
        tracing::info!("org.arlen.Accounts1 serving; the per-app gate mediates every method");

        // Serve until terminated.
        tokio::signal::ctrl_c().await?;
        tracing::info!("shutting down");
        Ok::<(), Box<dyn std::error::Error>>(())
    })
}

/// Install the Landlock write-fence over the vault dir. Defense-in-depth: a
/// kernel that cannot enforce it leaves the daemon exactly as safe as no fence,
/// so by default a non-enforcing kernel or a ruleset error is logged and the
/// daemon continues. A hardened deployment that wants the confinement
/// guaranteed sets `ARLEN_ONLINE_ACCOUNTS_REQUIRE_FENCE=1`, making a
/// non-enforcing kernel a fatal startup error (assertable confinement).
fn apply_fence(vault_dir: &Path) {
    use arlen_landlock_fence::{fence_writes, FenceOutcome};
    let require =
        std::env::var_os("ARLEN_ONLINE_ACCOUNTS_REQUIRE_FENCE").is_some_and(|v| v == "1");
    let degraded = match fence_writes(&[vault_dir]) {
        Ok(FenceOutcome::Enforced) => {
            tracing::info!("landlock write-fence enforced (write-confined to the vault dir)");
            None
        }
        Ok(FenceOutcome::NotEnforced) => Some("landlock not enforced by this kernel".to_string()),
        Err(e) => Some(format!("landlock fence not applied: {e}")),
    };
    if let Some(reason) = degraded {
        if require {
            tracing::error!(
                "ARLEN_ONLINE_ACCOUNTS_REQUIRE_FENCE=1 but the fence is not active ({reason}); refusing to run unconfined"
            );
            std::process::exit(1);
        }
        tracing::warn!("{reason}; running unconfined (no worse than no fence)");
    }
}

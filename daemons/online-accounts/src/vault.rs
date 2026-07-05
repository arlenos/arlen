//! The account token vault (online-accounts-plan.md, "the per-app master-secret
//! token vault - the daemon is the only key-holder").
//!
//! The AEAD record store itself is the shared [`arlen_secret_vault::Vault`],
//! re-exported here; this module only resolves the account daemon's own state
//! dir. Each account's tokens are sealed under a per-account subkey of the daemon
//! master, so a record sealed for one account cannot be decrypted as another and
//! the refresh token never leaves the daemon.

use std::path::PathBuf;

pub use arlen_secret_vault::{Vault, VaultError};

/// The vault directory: `$XDG_STATE_HOME/arlen/accounts`, else
/// `$HOME/.local/state/arlen/accounts`. `None` when neither is set.
pub fn vault_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local").join("state")))?;
    Some(base.join("arlen").join("accounts"))
}

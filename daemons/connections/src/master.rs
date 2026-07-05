//! The connections master-secret custody (mirrors the account daemon's).
//!
//! The credential store seals every connection credential under one 32-byte
//! master. That master is a persisted CSPRNG seed the daemon holds as the sole
//! key-holder, via the reviewed `arlen-forage-signing::BuilderKey` primitive:
//! generate-or-load, fail-closed on a symlinked, group/world-readable, or
//! wrong-length key file, never a silent re-key (a re-key would orphan every
//! stored credential). The TPM/PCR sealing the plan calls for is a deploy-time
//! wrapping of this file; the custody here is the same-uid software floor.

use std::path::{Path, PathBuf};

use arlen_forage_signing::{BuilderKey, KeyError};
use zeroize::Zeroizing;

/// The connections state dir: `$XDG_STATE_HOME/arlen/connections`, else
/// `$HOME/.local/state/arlen/connections`. The sealed credential records and the
/// master key live here. `None` when neither is set.
pub fn state_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local").join("state")))?;
    Some(base.join("arlen").join("connections"))
}

/// The master-key file path: `master.key` in the state dir, or `None` when the
/// state dir cannot be resolved (then the daemon refuses handouts, fail-closed).
pub fn master_key_path() -> Option<PathBuf> {
    state_dir().map(|d| d.join("master.key"))
}

/// The persistent credential-store master secret.
pub struct MasterSecret {
    inner: BuilderKey,
}

impl MasterSecret {
    /// Load the master, generating it on first run. Fail-closed on a symlinked,
    /// group/world-readable, or wrong-length key file (the reviewed `BuilderKey`
    /// custody), never a silent re-key.
    pub fn load_or_create(path: &Path) -> Result<Self, KeyError> {
        Ok(Self {
            inner: BuilderKey::load_or_create(path)?,
        })
    }

    /// The 32-byte symmetric master for the store AEAD. Zeroized on drop; stable
    /// across restarts (the persisted seed), so stored credentials stay
    /// decryptable.
    pub fn bytes(&self) -> Zeroizing<[u8; 32]> {
        Zeroizing::new(self.inner.signing_key().to_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn master_is_stable_across_reload() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("master.key");
        let first = MasterSecret::load_or_create(&path).unwrap();
        let a = *first.bytes();
        // A second load reads the persisted secret, not a fresh one, so stored
        // credentials stay decryptable across a restart.
        let second = MasterSecret::load_or_create(&path).unwrap();
        let b = *second.bytes();
        assert_eq!(a, b, "the master must be stable so sealed credentials stay decryptable");
    }
}

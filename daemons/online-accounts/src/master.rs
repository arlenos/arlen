//! The vault master-secret custody (online-accounts-plan.md).
//!
//! The token [`crate::vault::Vault`] AEAD-encrypts each account's tokens under a
//! per-account subkey HKDF-derived from one daemon **master secret**. That
//! master must persist (so stored tokens survive a restart) and never leak, so
//! it is held with the same hardened custody every Arlen daemon key uses: a
//! 32-byte CSPRNG secret at mode `0600`, generate-or-load, fail-closed against a
//! symlinked / group- or world-readable / wrong-length file, zeroized in memory.
//!
//! Rather than re-implement that custody, this reuses the reviewed
//! `arlen-forage-signing::BuilderKey` primitive (forage was its first user, the
//! capsule daemon its second): it is exactly "a 32-byte secret seed at 0600,
//! generate-or-load". Its Ed25519 wrapper is incidental here - the daemon never
//! signs; it takes the raw 32-byte seed as the ChaCha20-Poly1305 master.
//!
//! The plan names Secret Service as the eventual store; a root-owned-equivalent
//! 0600 file is the as-built custody (the same trust as the audit / undo-signer
//! / capsule daemon keys), and avoids the keyring-locked-at-daemon-start
//! chicken-and-egg. Moving the master into Secret Service is a deploy-time
//! refinement that does not change this interface.

use std::path::{Path, PathBuf};

use arlen_forage_signing::{BuilderKey, KeyError};
use zeroize::Zeroizing;

use crate::vault::vault_dir;

/// The master-secret file path: `master.key` beside the vault records, or
/// `None` when the vault directory cannot be resolved (then the daemon serves
/// metadata only and refuses token handouts, fail-closed).
pub fn master_key_path() -> Option<PathBuf> {
    vault_dir().map(|d| d.join("master.key"))
}

/// The persistent vault master secret.
pub struct MasterSecret {
    inner: BuilderKey,
}

impl MasterSecret {
    /// Load the master secret, generating it on first run. Fail-closed: a
    /// symlinked, group/world-readable, or wrong-length key file is an error
    /// (inherited from the reviewed `BuilderKey` custody), never a silent
    /// re-key.
    pub fn load_or_create(path: &Path) -> Result<Self, KeyError> {
        Ok(Self {
            inner: BuilderKey::load_or_create(path)?,
        })
    }

    /// The 32-byte symmetric master for the vault AEAD. Zeroized on drop. The
    /// bytes are the persisted CSPRNG seed, so they are stable across restarts
    /// (stored tokens stay decryptable).
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
        // A second load reads the persisted secret, not a fresh one.
        let second = MasterSecret::load_or_create(&path).unwrap();
        let b = *second.bytes();
        assert_eq!(a, b, "the master must be stable so stored tokens stay decryptable");
    }

    #[test]
    fn the_key_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("master.key");
        let _ = MasterSecret::load_or_create(&path).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "the master secret must never be group/world readable");
    }
}

//! The capability-token ROOT keypair custody (connections-plan.md §7b).
//!
//! The daemon mints and verifies destination-scoped Biscuit capability tokens (see
//! [`crate::capability`]) under a persistent Ed25519 ROOT keypair. Biscuit is
//! public-key-verifiable, so any net-guard/checkpoint validates a token with only
//! this keypair's PUBLIC half; the private half never leaves the daemon.
//!
//! The root keypair is a SEPARATE key file from the credential-sealing master
//! ([`crate::master`]): key separation, so the token-signing authority and the
//! at-rest credential encryption never share a secret. Both use the same reviewed
//! `arlen-forage-signing::BuilderKey` custody (generate-or-load, fail-closed on a
//! symlinked, group/world-readable, or wrong-length key file, never a silent
//! re-key). A re-key here would invalidate every already-minted token, so the
//! persisted seed is stable across restarts.

use std::path::{Path, PathBuf};

use arlen_forage_signing::{BuilderKey, KeyError};
use biscuit_auth::{Algorithm, KeyPair, PrivateKey, PublicKey};

/// The root-key file path: `root.key` in the connections state dir, or `None` when
/// the state dir cannot be resolved (then the daemon refuses to mint/verify tokens,
/// fail-closed).
pub fn root_key_path() -> Option<PathBuf> {
    crate::master::state_dir().map(|d| d.join("root.key"))
}

/// A failure building the root keypair.
#[derive(Debug, thiserror::Error)]
pub enum RootKeyError {
    /// The persisted seed could not be loaded or created (custody failure).
    #[error("key custody: {0}")]
    Custody(#[from] KeyError),
    /// The persisted seed did not reconstruct a valid Ed25519 key (corrupt).
    #[error("key reconstruct: {0}")]
    Reconstruct(String),
}

/// The persistent capability-token root keypair.
pub struct RootKeypair {
    keypair: KeyPair,
}

impl RootKeypair {
    /// Load the root keypair, generating it on first run. The 32-byte Ed25519 seed
    /// is persisted via the reviewed [`BuilderKey`] custody (fail-closed on a
    /// symlinked, group/world-readable, or wrong-length key file), then
    /// reconstructed into a Biscuit [`KeyPair`]. Stable across restarts, so tokens
    /// minted before a restart still verify after.
    pub fn load_or_create(path: &Path) -> Result<Self, RootKeyError> {
        let builder = BuilderKey::load_or_create(path)?;
        // The BuilderKey's Ed25519 signing key is a 32-byte seed; Biscuit rebuilds
        // its own Ed25519 keypair from exactly that seed, so the same file yields
        // the same signing identity every load.
        let seed = builder.signing_key().to_bytes();
        let private = PrivateKey::from_bytes(&seed, Algorithm::Ed25519)
            .map_err(|e| RootKeyError::Reconstruct(e.to_string()))?;
        Ok(Self {
            keypair: KeyPair::from(&private),
        })
    }

    /// The full keypair, for minting/attenuating tokens (private half stays in the
    /// daemon).
    pub fn keypair(&self) -> &KeyPair {
        &self.keypair
    }

    /// The public half, for verifying tokens. This is the only part a net-guard or
    /// checkpoint needs to validate a presented capability token.
    pub fn public(&self) -> PublicKey {
        self.keypair.public()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_identity_is_stable_across_reload() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("root.key");
        let first = RootKeypair::load_or_create(&path).unwrap();
        let a = first.public();
        // A second load reads the persisted seed, not a fresh one, so tokens minted
        // under the first keypair still verify under the second (same public key).
        let second = RootKeypair::load_or_create(&path).unwrap();
        let b = second.public();
        assert_eq!(
            a.to_bytes(),
            b.to_bytes(),
            "the root public key must be stable so already-minted tokens keep verifying"
        );
    }

    #[test]
    fn a_minted_token_verifies_under_a_reloaded_root() {
        // The end-to-end custody property: mint under one load, verify under a
        // fresh load of the same file.
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("root.key");
        let minted = {
            let root = RootKeypair::load_or_create(&path).unwrap();
            crate::capability::mint_token(
                root.keypair(),
                "github",
                &["api.github.com".to_string()],
                4_102_444_800, // far future
                "nonce-abc",
            )
            .unwrap()
        };
        let reloaded = RootKeypair::load_or_create(&path).unwrap();
        let ok = crate::capability::verify_token(
            &minted,
            &reloaded.public(),
            "api.github.com",
            1_000_000_000,
        )
        .unwrap();
        assert!(ok, "a token minted under the persisted root must verify after reload");
    }
}

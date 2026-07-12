//! The `run_command` consent-biscuit ROOT keypair custody (ai-act-layer-plan.md,
//! the biscuit per-action tie-in).
//!
//! `run_command` runs in the SEPARATE terminal-run MCP server, so the daemon's
//! in-memory execution proof cannot be verified there. Instead the daemon mints a
//! public-key-verifiable Biscuit (see [`arlen_run_consent_token`]) when a
//! `run_command` Confirm is approved, and the MCP server verifies it with only the
//! PUBLIC half of this keypair. The private half never leaves the daemon.
//!
//! Custody reuses the reviewed `arlen-forage-signing::BuilderKey` (generate-or-load,
//! fail-closed on a symlinked, group/world-readable or wrong-length key file, never
//! a silent re-key), exactly as the Connections daemon does. A re-key would
//! invalidate every already-minted token, so the persisted seed is stable across
//! restarts. The daemon also publishes the public half to a sibling file the MCP
//! server reads; a public key is not a secret, so that file is world-readable.

use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use arlen_forage_signing::{BuilderKey, KeyError};
use biscuit_auth::{Algorithm, KeyPair, PrivateKey, PublicKey};

/// The state directory for the AI-engine daemon's own key material:
/// `$XDG_STATE_HOME|$HOME/.local/state` + `arlen/ai-engine`, or `None` when neither
/// env var is set (then the daemon refuses to mint consent tokens, fail-closed).
pub fn state_dir() -> Option<PathBuf> {
    let base = std::env::var("XDG_STATE_HOME")
        .ok()
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .filter(|s| !s.is_empty())
                .map(|h| PathBuf::from(h).join(".local/state"))
        })?;
    Some(base.join("arlen/ai-engine"))
}

/// The root-key file: `run-consent-root.key` in the state dir.
pub fn root_key_path() -> Option<PathBuf> {
    state_dir().map(|d| d.join("run-consent-root.key"))
}

/// The published public-key file the terminal-run MCP server reads to verify a
/// consent biscuit: `run-consent-root.pub` (hex) in the state dir.
pub fn public_key_path() -> Option<PathBuf> {
    state_dir().map(|d| d.join("run-consent-root.pub"))
}

/// A failure building or publishing the consent-root keypair.
#[derive(Debug, thiserror::Error)]
pub enum ConsentRootError {
    /// The persisted seed could not be loaded or created (custody failure).
    #[error("key custody: {0}")]
    Custody(#[from] KeyError),
    /// The persisted seed did not reconstruct a valid Ed25519 key (corrupt).
    #[error("key reconstruct: {0}")]
    Reconstruct(String),
    /// The public key could not be published for the MCP server.
    #[error("publish public key: {0}")]
    Publish(String),
}

/// The persistent `run_command` consent-biscuit root keypair.
pub struct ConsentRoot {
    keypair: KeyPair,
}

impl ConsentRoot {
    /// Load the root keypair, generating it on first run. The 32-byte Ed25519 seed
    /// is persisted via the reviewed [`BuilderKey`] custody, then reconstructed into
    /// a Biscuit [`KeyPair`]. Stable across restarts, so a token minted before a
    /// restart still verifies after (the daemon and the MCP server outlive one
    /// approval, but a supervised restart must not orphan an in-flight token).
    pub fn load_or_create(path: &Path) -> Result<Self, ConsentRootError> {
        let builder = BuilderKey::load_or_create(path)?;
        // The BuilderKey's Ed25519 signing key is a 32-byte seed; Biscuit rebuilds
        // its own Ed25519 keypair from exactly that seed, so the same file yields the
        // same signing identity every load.
        let seed = builder.signing_key().to_bytes();
        let private = PrivateKey::from_bytes(&seed, Algorithm::Ed25519)
            .map_err(|e| ConsentRootError::Reconstruct(e.to_string()))?;
        Ok(Self {
            keypair: KeyPair::from(&private),
        })
    }

    /// The full keypair, for minting consent tokens (private half stays in the
    /// daemon).
    pub fn keypair(&self) -> &KeyPair {
        &self.keypair
    }

    /// The public half, for verifying tokens (what the MCP server needs).
    pub fn public(&self) -> PublicKey {
        self.keypair.public()
    }

    /// The public key as lowercase hex, the on-disk publication form.
    pub fn public_key_hex(&self) -> String {
        let bytes = self.keypair.public().to_bytes();
        let mut hex = String::with_capacity(bytes.len() * 2);
        for b in bytes {
            hex.push_str(&format!("{b:02x}"));
        }
        hex
    }

    /// Publish the public key (hex) to `path` so the terminal-run MCP server can
    /// read it. Written atomically (temp + rename) and world-readable (a public key
    /// is not a secret; the MCP server is a distinct, confined process). The parent
    /// directory is created 0700 (it holds the private key too).
    pub fn publish_public_key(&self, path: &Path) -> Result<(), ConsentRootError> {
        if let Some(parent) = path.parent() {
            std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(parent)
                .map_err(|e| ConsentRootError::Publish(e.to_string()))?;
        }
        let hex = self.public_key_hex();
        let tmp = path.with_extension("pub.tmp");
        {
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o644)
                .open(&tmp)
                .map_err(|e| ConsentRootError::Publish(e.to_string()))?;
            f.write_all(hex.as_bytes())
                .map_err(|e| ConsentRootError::Publish(e.to_string()))?;
            f.sync_all()
                .map_err(|e| ConsentRootError::Publish(e.to_string()))?;
        }
        std::fs::rename(&tmp, path).map_err(|e| ConsentRootError::Publish(e.to_string()))?;
        Ok(())
    }
}

/// Parse a published hex public key back into a Biscuit [`PublicKey`]. This is the
/// verify-side reader the MCP server uses; it lives here so the mint side and the
/// verify side share one encoding. A malformed hex string or a non-Ed25519 key is a
/// hard error (the verifier must refuse, never fall back to an unverified run).
pub fn public_key_from_hex(hex: &str) -> Result<PublicKey, ConsentRootError> {
    let hex = hex.trim();
    if !hex.len().is_multiple_of(2) {
        return Err(ConsentRootError::Reconstruct("odd-length hex".to_string()));
    }
    let mut bytes = Vec::with_capacity(hex.len() / 2);
    let raw = hex.as_bytes();
    let mut i = 0;
    while i < raw.len() {
        let hi = (raw[i] as char)
            .to_digit(16)
            .ok_or_else(|| ConsentRootError::Reconstruct("non-hex byte".to_string()))?;
        let lo = (raw[i + 1] as char)
            .to_digit(16)
            .ok_or_else(|| ConsentRootError::Reconstruct("non-hex byte".to_string()))?;
        bytes.push(((hi << 4) | lo) as u8);
        i += 2;
    }
    PublicKey::from_bytes(&bytes, Algorithm::Ed25519)
        .map_err(|e| ConsentRootError::Reconstruct(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_identity_is_stable_across_reload() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("root.key");
        let a = ConsentRoot::load_or_create(&path).unwrap().public().to_bytes();
        // A second load reads the persisted seed, not a fresh one, so an in-flight
        // token minted before a restart still verifies after (same public key).
        let b = ConsentRoot::load_or_create(&path).unwrap().public().to_bytes();
        assert_eq!(a, b, "the consent-root public key must be stable across reload");
    }

    #[test]
    fn a_minted_token_verifies_under_the_published_public_key() {
        // The end-to-end custody + distribution property: mint under the daemon's
        // keypair, publish the public key, read it back as the MCP server would, and
        // verify the token succeeds for its command.
        let tmp = tempfile::tempdir().unwrap();
        let root = ConsentRoot::load_or_create(&tmp.path().join("root.key")).unwrap();
        let pub_path = tmp.path().join("run-consent-root.pub");
        root.publish_public_key(&pub_path).unwrap();

        let token = arlen_run_consent_token::mint_run_consent(
            root.keypair(),
            "ls",
            &["-la".to_string()],
            4_102_444_800,
        )
        .unwrap();

        let published = std::fs::read_to_string(&pub_path).unwrap();
        let verify_key = public_key_from_hex(&published).unwrap();
        assert!(
            arlen_run_consent_token::verify_run_consent(
                &token,
                &verify_key,
                "ls",
                &["-la".to_string()],
                1_000_000_000,
            )
            .unwrap(),
            "a token minted under the daemon's root must verify under the published public key"
        );
    }

    #[test]
    fn the_published_public_key_matches_the_hex_accessor() {
        let tmp = tempfile::tempdir().unwrap();
        let root = ConsentRoot::load_or_create(&tmp.path().join("root.key")).unwrap();
        let pub_path = tmp.path().join("run-consent-root.pub");
        root.publish_public_key(&pub_path).unwrap();
        let on_disk = std::fs::read_to_string(&pub_path).unwrap();
        assert_eq!(on_disk, root.public_key_hex());
        // And the round-trip reconstructs the exact same key bytes.
        assert_eq!(
            public_key_from_hex(&on_disk).unwrap().to_bytes(),
            root.public().to_bytes()
        );
    }

    #[test]
    fn malformed_hex_is_a_hard_error() {
        assert!(public_key_from_hex("not-hex").is_err());
        assert!(public_key_from_hex("abc").is_err()); // odd length
    }
}

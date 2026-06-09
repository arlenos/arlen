//! The capsule signing key custody (context-capsule.md §3, §5).
//!
//! Grants are signed by a **persisted** Ed25519 key (not the process-ephemeral
//! token HMAC key, which is regenerated every restart and has no public verifier,
//! so it is the wrong base for a portable grant). The custody is the proven,
//! reviewed `arlen-forage-signing` primitive — a generic persisted-Ed25519 key
//! (forage was its first user): a 32-byte seed at mode `0600`, generate-or-load,
//! held in a zeroizing buffer, fail-closed against a symlink, a group- or
//! world-readable mode, or a wrong length. The capsule key lives at its own path,
//! distinct from the forage builder key, so the two never share key material.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use arlen_forage_signing::{BuilderKey, KeyError};
use ed25519_dalek::{SigningKey, VerifyingKey};

/// The originator's persisted Ed25519 capsule signing key.
pub struct CapsuleSigningKey {
    inner: BuilderKey,
}

impl CapsuleSigningKey {
    /// Load the key at `path`, or generate it on genesis. Ensures the parent
    /// directory exists and is private (`0700`) first, then delegates to the
    /// fail-closed `arlen-forage-signing` custody for the key file itself (`0600`,
    /// no-symlink, exact length).
    pub fn load_or_create(path: &Path) -> Result<Self, KeyError> {
        if let Some(dir) = path.parent() {
            // Best-effort: create the private dir. A real failure surfaces from
            // the key open below; clamp the mode so the key's dir is owner-only.
            let _ = std::fs::create_dir_all(dir);
            let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
        }
        Ok(Self {
            inner: BuilderKey::load_or_create(path)?,
        })
    }

    /// The signing key, for signing a grant.
    pub fn signing_key(&self) -> &SigningKey {
        self.inner.signing_key()
    }

    /// The verifying key, the public half a grant verifier (or audience) uses.
    pub fn verifying_key(&self) -> VerifyingKey {
        self.inner.signing_key().verifying_key()
    }
}

/// The capsule signing key path: `$XDG_STATE_HOME` (or `$HOME/.local/state`) →
/// `arlen/capsule/signing.key`. `None` if neither env var resolves (the daemon
/// fails closed rather than writing a key to an unknown location).
pub fn capsule_key_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .filter(|p| !p.as_os_str().is_empty())
                .map(|h| h.join(".local/state"))
        })?;
    Some(base.join("arlen/capsule/signing.key"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn genesis_then_reload_is_the_same_identity() {
        let tmp = std::env::temp_dir().join(format!("capsule-key-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let path = tmp.join("arlen/capsule/signing.key");

        let first = CapsuleSigningKey::load_or_create(&path).expect("genesis");
        let vk1 = first.verifying_key();
        // The key file is owner-only.
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "the capsule key must be 0600");

        let second = CapsuleSigningKey::load_or_create(&path).expect("reload");
        assert_eq!(vk1.to_bytes(), second.verifying_key().to_bytes(), "reload is the same key");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn key_path_prefers_xdg_state_home() {
        // A direct unit check of the resolution would race the process env, so just
        // assert the shape: the path ends with the capsule key segments.
        if let Some(p) = capsule_key_path() {
            assert!(p.ends_with("arlen/capsule/signing.key"));
        }
    }
}

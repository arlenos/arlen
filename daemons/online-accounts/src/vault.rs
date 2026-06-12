//! The per-account token vault (online-accounts-plan.md, "the per-app
//! master-secret token vault - the daemon is the only key-holder").
//!
//! Raw Secret Service is ambient (any app can read any item), so per-account
//! OAuth tokens are NOT stored as keyring items. Instead the daemon holds a
//! single master secret (retrieved via the Secret-portal per-app flow, keyed to
//! the daemon's own identity - that retrieval is the daemon-startup wiring, not
//! this module) and stores each account's tokens encrypted at rest in its own
//! state dir. Each record is sealed under a per-account subkey HKDF-derived from
//! the master with the account id as the info string, with an AEAD (ChaCha20-
//! Poly1305) per record and the account id bound in as associated data. So a
//! record sealed for one account cannot be decrypted as another (different
//! subkey AND different AAD), other apps cannot read the tokens (they are not
//! keyring-readable items), and the refresh token never leaves the daemon.

use std::path::{Path, PathBuf};

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use hkdf::Hkdf;
use sha2::Sha256;
use zeroize::Zeroizing;

/// The ChaCha20-Poly1305 nonce length (96-bit).
const NONCE_LEN: usize = 12;

/// A vault failure. Every variant means no plaintext was produced or persisted;
/// callers fail closed (a decrypt failure must never be treated as "no record").
#[derive(Debug, thiserror::Error)]
pub enum VaultError {
    /// The account id is not a safe single path component.
    #[error("invalid account id")]
    InvalidAccount,
    /// The vault state directory could not be resolved (no `HOME`/`XDG_STATE_HOME`).
    #[error("no vault directory")]
    NoVaultDir,
    /// A filesystem error reading or writing a record.
    #[error("vault io: {0}")]
    Io(String),
    /// A stored record is too short to contain a nonce (truncated/corrupt).
    #[error("vault record is corrupt")]
    Corrupt,
    /// AEAD decryption or authentication failed (wrong key, tamper, or the record
    /// does not belong to this account).
    #[error("vault record could not be decrypted")]
    Decrypt,
    /// AEAD encryption failed.
    #[error("vault record could not be encrypted")]
    Encrypt,
}

/// The per-account token vault: the daemon's master secret plus the state dir the
/// encrypted records live in.
pub struct Vault {
    master: Zeroizing<[u8; 32]>,
    dir: PathBuf,
}

/// Whether `id` is a safe single path component (it becomes a filename): non-empty,
/// no separators, no `.`/`..`, ordinary id characters only.
fn is_valid_account(id: &str) -> bool {
    !id.is_empty()
        && id != "."
        && id != ".."
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// The vault directory: `$XDG_STATE_HOME/arlen/accounts`, else
/// `$HOME/.local/state/arlen/accounts`. `None` when neither is set.
pub fn vault_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local").join("state")))?;
    Some(base.join("arlen").join("accounts"))
}

impl Vault {
    /// A vault keyed by `master` (the daemon's master secret) storing records
    /// under `dir`. The directory is created (0700) on first write.
    pub fn new(master: [u8; 32], dir: impl Into<PathBuf>) -> Self {
        Self {
            master: Zeroizing::new(master),
            dir: dir.into(),
        }
    }

    /// Derive the per-account subkey: HKDF-SHA256 over the master secret with the
    /// account id as the info string, so each account gets a distinct key.
    fn subkey(&self, account_id: &str) -> Zeroizing<[u8; 32]> {
        let hk = Hkdf::<Sha256>::new(None, &*self.master);
        let mut okm = Zeroizing::new([0u8; 32]);
        // expand only fails for an absurd output length; 32 bytes is always valid.
        hk.expand(account_id.as_bytes(), &mut *okm)
            .expect("32-byte HKDF output is valid");
        okm
    }

    /// The on-disk path for an account's record.
    fn record_path(&self, account_id: &str) -> Result<PathBuf, VaultError> {
        if !is_valid_account(account_id) {
            return Err(VaultError::InvalidAccount);
        }
        Ok(self.dir.join(format!("{account_id}.vault")))
    }

    /// Encrypt and persist `plaintext` (the account's token blob) at rest. The
    /// stored bytes are `nonce || ciphertext`, written 0600 via a temp file +
    /// rename so a crash never leaves a half-written record.
    pub fn store(&self, account_id: &str, plaintext: &[u8]) -> Result<(), VaultError> {
        let path = self.record_path(account_id)?;
        let key = self.subkey(account_id);
        let cipher = ChaCha20Poly1305::new(Key::from_slice(&*key));

        let mut nonce = [0u8; NONCE_LEN];
        getrandom::getrandom(&mut nonce).map_err(|e| VaultError::Io(e.to_string()))?;
        let ciphertext = cipher
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: plaintext,
                    aad: account_id.as_bytes(),
                },
            )
            .map_err(|_| VaultError::Encrypt)?;

        let mut out = Vec::with_capacity(NONCE_LEN + ciphertext.len());
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&ciphertext);

        ensure_private_dir(&self.dir)?;
        atomic_write_private(&path, &out)
    }

    /// Load and decrypt an account's token blob. `Ok(None)` when no record exists;
    /// any decrypt/authentication failure is an error (fail closed), never `None`.
    pub fn load(&self, account_id: &str) -> Result<Option<Vec<u8>>, VaultError> {
        let path = self.record_path(account_id)?;
        let data = match std::fs::read(&path) {
            Ok(d) => d,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(VaultError::Io(e.to_string())),
        };
        if data.len() < NONCE_LEN {
            return Err(VaultError::Corrupt);
        }
        let (nonce, ciphertext) = data.split_at(NONCE_LEN);
        let key = self.subkey(account_id);
        let cipher = ChaCha20Poly1305::new(Key::from_slice(&*key));
        let plaintext = cipher
            .decrypt(
                Nonce::from_slice(nonce),
                Payload {
                    msg: ciphertext,
                    aad: account_id.as_bytes(),
                },
            )
            .map_err(|_| VaultError::Decrypt)?;
        Ok(Some(plaintext))
    }

    /// Remove an account's record (on disconnect). Absent is success (idempotent).
    pub fn remove(&self, account_id: &str) -> Result<(), VaultError> {
        let path = self.record_path(account_id)?;
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(VaultError::Io(e.to_string())),
        }
    }
}

/// Create `dir` (and parents) with owner-only permissions.
fn ensure_private_dir(dir: &Path) -> Result<(), VaultError> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .map_err(|e| VaultError::Io(e.to_string()))
}

/// Write `bytes` to `path` 0600 via a sibling temp file + rename (atomic replace).
fn atomic_write_private(path: &Path, bytes: &[u8]) -> Result<(), VaultError> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let tmp = path.with_extension("vault.tmp");
    {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)
            .map_err(|e| VaultError::Io(e.to_string()))?;
        f.write_all(bytes).map_err(|e| VaultError::Io(e.to_string()))?;
        f.sync_all().map_err(|e| VaultError::Io(e.to_string()))?;
    }
    std::fs::rename(&tmp, path).map_err(|e| VaultError::Io(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vault() -> (tempfile::TempDir, Vault) {
        let tmp = tempfile::TempDir::new().unwrap();
        let v = Vault::new([7u8; 32], tmp.path().join("accounts"));
        (tmp, v)
    }

    #[test]
    fn store_then_load_round_trips() {
        let (_tmp, v) = vault();
        v.store("com.example.alice", b"refresh-token-xyz").unwrap();
        let got = v.load("com.example.alice").unwrap();
        assert_eq!(got.as_deref(), Some(&b"refresh-token-xyz"[..]));
    }

    #[test]
    fn a_missing_record_is_none() {
        let (_tmp, v) = vault();
        assert_eq!(v.load("com.example.nobody").unwrap(), None);
    }

    #[test]
    fn another_account_cannot_decrypt_a_record() {
        // Two accounts' records use different subkeys; reading one record's bytes
        // under the other account id fails AEAD authentication.
        let (tmp, v) = vault();
        v.store("acct.a", b"secret-a").unwrap();
        // Copy a's record bytes onto b's path, then try to load as b.
        let a = tmp.path().join("accounts/acct.a.vault");
        let b = tmp.path().join("accounts/acct.b.vault");
        std::fs::copy(&a, &b).unwrap();
        assert!(matches!(v.load("acct.b"), Err(VaultError::Decrypt)));
    }

    #[test]
    fn a_different_master_cannot_decrypt() {
        let (tmp, v) = vault();
        v.store("acct.a", b"secret-a").unwrap();
        let other = Vault::new([9u8; 32], tmp.path().join("accounts"));
        assert!(matches!(other.load("acct.a"), Err(VaultError::Decrypt)));
    }

    #[test]
    fn a_tampered_record_fails_authentication() {
        let (tmp, v) = vault();
        v.store("acct.a", b"secret-a").unwrap();
        let path = tmp.path().join("accounts/acct.a.vault");
        let mut data = std::fs::read(&path).unwrap();
        let last = data.len() - 1;
        data[last] ^= 0xff;
        std::fs::write(&path, &data).unwrap();
        assert!(matches!(v.load("acct.a"), Err(VaultError::Decrypt)));
    }

    #[test]
    fn a_truncated_record_is_corrupt() {
        let (tmp, v) = vault();
        v.store("acct.a", b"secret-a").unwrap();
        let path = tmp.path().join("accounts/acct.a.vault");
        std::fs::write(&path, b"short").unwrap();
        assert!(matches!(v.load("acct.a"), Err(VaultError::Corrupt)));
    }

    #[test]
    fn an_invalid_account_id_is_rejected() {
        let (_tmp, v) = vault();
        assert!(matches!(v.store("../escape", b"x"), Err(VaultError::InvalidAccount)));
        assert!(matches!(v.load("a/b"), Err(VaultError::InvalidAccount)));
    }

    #[test]
    fn remove_is_idempotent() {
        let (_tmp, v) = vault();
        v.store("acct.a", b"x").unwrap();
        v.remove("acct.a").unwrap();
        assert_eq!(v.load("acct.a").unwrap(), None);
        v.remove("acct.a").unwrap(); // absent is success
    }
}

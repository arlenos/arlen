//! HMAC key management for the audit ledger.
//!
//! The hash chain is signed with a 32-byte HMAC key. S13.2 provides
//! the software-key baseline: a persistent key file under the audit
//! data directory, mode 0600, generated once with the OS CSPRNG.
//!
//! The key file is deliberately the durable store. Foundation §8.4.7
//! names the Kernel Keyring as the no-TPM fallback, but the keyring
//! does not survive a reboot while the audit chain must still verify
//! afterwards — so the file is the persistence layer. A 0600 file
//! already delivers the property the foundation attributes to that
//! fallback: an attacker without key access cannot recompute an
//! entry's HMAC, so post-hoc tampering stays detectable; an attacker
//! who reads the key can still forge (early-access forgery). TPM PCR
//! sealing closes that remaining gap and, together with a runtime
//! Kernel Keyring handle, is layered hardening left as a follow-up.

use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use zeroize::Zeroizing;

use crate::error::{AuditError, Result};

/// HMAC key length in bytes — 256-bit, SHA-256-block-friendly.
const KEY_LEN: usize = 32;

/// Path of the audit HMAC key file. Fails closed if no per-user
/// audit data directory can be resolved (see [`crate::audit_data_dir`]).
pub fn key_path() -> Result<PathBuf> {
    Ok(crate::audit_data_dir()?.join("hmac.key"))
}

/// Load the HMAC key, creating it on genesis.
///
/// `ledger_has_entries` is whether the ledger already holds at least
/// one entry. If the key file is absent *and* the ledger is
/// non-empty the call fails closed: a fresh key would make every
/// existing chain entry fail verification, so the daemon must stop
/// and the user recover explicitly. There is no silent re-key.
pub fn load_or_create(path: &Path, ledger_has_entries: bool) -> Result<Zeroizing<Vec<u8>>> {
    if path.exists() {
        return read_key(path);
    }
    if ledger_has_entries {
        return Err(AuditError::KeyUnavailable(format!(
            "key file {} is missing but the ledger already holds entries; \
             refusing to generate a new key, which would invalidate the chain",
            path.display()
        )));
    }
    create_key(path)
}

/// Read and validate an existing key file with no TOCTOU window.
///
/// The file is opened `O_NOFOLLOW` (a symlink at the final component
/// fails the open with `ELOOP`, so a symlink planted between any check
/// and the read cannot redirect the daemon), then validated by `fstat`
/// on the *open fd* (not a re-resolved path): a regular file, not
/// reachable by group or others (mode `0600`), and read to exactly
/// [`KEY_LEN`] bytes from that same fd. The key is held in `Zeroizing`.
fn read_key(path: &Path) -> Result<Zeroizing<Vec<u8>>> {
    use std::io::Read;
    use std::os::unix::fs::PermissionsExt;

    // O_NOFOLLOW: the open fails if the final path component is a
    // symlink, so the path is resolved once and every later check plus
    // the read operate on that single fd, never a re-resolved path.
    let mut file = match std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
    {
        Ok(f) => f,
        Err(e) if e.raw_os_error() == Some(libc::ELOOP) => {
            return Err(AuditError::KeyUnavailable(format!(
                "key file {} is a symlink; refusing to follow it",
                path.display()
            )))
        }
        Err(e) => return Err(e.into()),
    };

    let meta = file.metadata()?; // fstat on the open fd
    if !meta.is_file() {
        return Err(AuditError::KeyUnavailable(format!(
            "key path {} is not a regular file",
            path.display()
        )));
    }
    if meta.permissions().mode() & 0o077 != 0 {
        return Err(AuditError::KeyUnavailable(format!(
            "key file {} is reachable by group or others; expected mode 0600",
            path.display()
        )));
    }

    let mut bytes = Zeroizing::new(Vec::new());
    file.read_to_end(&mut bytes)?;
    if bytes.len() != KEY_LEN {
        return Err(AuditError::KeyUnavailable(format!(
            "key file {} holds {} bytes, expected {KEY_LEN}",
            path.display(),
            bytes.len()
        )));
    }
    Ok(bytes)
}

/// Generate a fresh key with the OS CSPRNG and persist it durably at
/// mode 0600.
fn create_key(path: &Path) -> Result<Zeroizing<Vec<u8>>> {
    let parent = path.parent().ok_or_else(|| {
        AuditError::KeyUnavailable(format!(
            "key path {} has no parent directory",
            path.display()
        ))
    })?;
    crate::ensure_private_dir(parent)?;

    let mut key = Zeroizing::new(vec![0u8; KEY_LEN]);
    getrandom::getrandom(&mut key)
        .map_err(|e| AuditError::KeyUnavailable(format!("CSPRNG failed: {e}")))?;

    // `create_new` makes this start the race winner if two starts
    // collide; `mode(0o600)` keeps the key from ever existing
    // world-readable, not even for the moment between create and
    // chmod.
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
    {
        Ok(mut f) => {
            f.write_all(&key)?;
            // The key must be on disk before any entry is signed with
            // it: a crash that loses the key while ledger rows survive
            // leaves the chain unverifiable forever. fsync the file,
            // then the directory so the new directory entry is durable
            // too.
            f.sync_all()?;
            fsync_dir(parent)?;
            Ok(key)
        }
        // Lost the race — another start created the key first; use it.
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => read_key(path),
        Err(e) => Err(e.into()),
    }
}

/// fsync a directory so a newly-created entry within it survives a
/// crash or power loss.
fn fsync_dir(dir: &Path) -> Result<()> {
    std::fs::File::open(dir)?.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn creates_a_0600_key_on_genesis() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hmac.key");
        let key = load_or_create(&path, false).expect("genesis create");
        assert_eq!(key.len(), KEY_LEN);
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "key file must be mode 0600");
    }

    #[test]
    fn load_returns_the_same_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hmac.key");
        let first = load_or_create(&path, false).unwrap();
        // The file now exists; a second call must read it back, not
        // generate a new one.
        let second = load_or_create(&path, true).unwrap();
        assert_eq!(first.as_slice(), second.as_slice());
    }

    #[test]
    fn missing_key_with_a_nonempty_ledger_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hmac.key");
        match load_or_create(&path, true) {
            Err(AuditError::KeyUnavailable(_)) => {}
            other => panic!("expected KeyUnavailable, got {other:?}"),
        }
        assert!(!path.exists(), "no key file may be written in this case");
    }

    /// Write `content` to `path` with an exact mode (umask-independent).
    fn write_file(path: &Path, content: &[u8], mode: u32) {
        std::fs::write(path, content).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
            .unwrap();
    }

    #[test]
    fn a_wrong_length_key_file_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hmac.key");
        // 0600 so the mode check passes and the length check is what
        // actually rejects this file.
        write_file(&path, b"too short", 0o600);
        assert!(matches!(
            load_or_create(&path, false),
            Err(AuditError::KeyUnavailable(_))
        ));
    }

    #[test]
    fn a_symlinked_key_file_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real.key");
        write_file(&real, &[7u8; KEY_LEN], 0o600);
        let link = dir.path().join("hmac.key");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        // `link` exists (its target does), so `load_or_create` reaches
        // `read_key`, which lstat-rejects the symlink.
        assert!(matches!(
            load_or_create(&link, false),
            Err(AuditError::KeyUnavailable(_))
        ));
    }

    #[test]
    fn a_group_readable_key_file_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hmac.key");
        write_file(&path, &[7u8; KEY_LEN], 0o640);
        assert!(matches!(
            load_or_create(&path, false),
            Err(AuditError::KeyUnavailable(_))
        ));
    }
}

//! HMAC key custody for the undo-log signer
//! (reversible-receipts-and-the-effect-model.md §6).
//!
//! The undo-log's HMAC chain is keyed by a 32-byte key the *agent never sees*:
//! integrity against a same-uid compromised agent (the F3 concern) requires the
//! key to live with the separate-uid signer, not the agent. This module is the
//! signer's key custody: a persistent 0600 key file under the signer's private
//! state directory, generated once with the OS CSPRNG, fail-closed against a
//! symlink, a group/other-reachable mode, or a wrong length. It mirrors the
//! audit-daemon's key custody exactly; the undo-log key is more sensitive (it
//! seals PII-adjacent prior state), so the loaded key is held in `Zeroizing`.
//!
//! As with the audit ledger, the key file is the durable store and a 0600 file
//! delivers the post-hoc-tamper-detection property; an attacker who *reads* the
//! key can still forge (early-access forgery), closed only by the separate-uid
//! boundary plus future TPM sealing. Confidentiality of the log at rest (a
//! same-uid agent reading the history) is closed by the 0700 directory and the
//! signer serving lookups, not by this key.

use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use zeroize::Zeroizing;

use crate::error::{Result, SignerError};

/// HMAC key length in bytes: 256-bit, SHA-256-block-friendly (matches the chain
/// in `arlen-ai-agent::undo_log`).
pub const KEY_LEN: usize = 32;

/// Load the signing key, creating it on genesis.
///
/// `log_has_records` is whether the undo-log already holds at least one record.
/// If the key file is absent *and* the log is non-empty the call fails closed: a
/// fresh key would make every existing chained record fail verification, so the
/// signer must stop and the user recover explicitly. There is no silent re-key
/// while records remain.
///
/// Honest limit: this guards against a *lost* key, not against an attacker who
/// can also erase the log. Removing the key AND truncating the log to empty makes
/// `log_has_records` false and re-keys over a blank log, erasing history with no
/// error. That is the same class as the documented tail-truncation gap (the chain
/// proves no in-place tamper, but a prefix or whole-log erasure leaves a
/// self-consistent shorter chain), and it is closed only by an external head
/// checkpoint (a signed record-count + head-hash sidecar, the deferred next
/// increment), not by key custody alone. In the intended separate-uid deployment
/// the 0700 signer-owned directory keeps any non-signer process from doing this;
/// in the as-built same-uid deployment it collapses into the acknowledged F3
/// residual (see `auth.rs`).
pub fn load_or_create(path: &Path, log_has_records: bool) -> Result<Zeroizing<Vec<u8>>> {
    if path.exists() {
        return read_key(path);
    }
    if log_has_records {
        return Err(SignerError::KeyUnavailable(format!(
            "key file {} is missing but the undo-log already holds records; \
             refusing to generate a new key, which would invalidate the chain",
            path.display()
        )));
    }
    create_key(path)
}

/// Read and validate an existing key file with no TOCTOU window: the file is
/// opened `O_NOFOLLOW` (a symlink at the final component fails the open with
/// `ELOOP`), then validated by `fstat` on the *open fd* (not a re-resolved path)
/// for being a regular file, not reachable by group or others (mode 0600), and
/// read to exactly [`KEY_LEN`] bytes from that same fd.
fn read_key(path: &Path) -> Result<Zeroizing<Vec<u8>>> {
    use std::io::Read;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    // O_NOFOLLOW: if the final path component is a symlink the open fails, so a
    // symlink planted between any check and the read cannot redirect us. The fd
    // is then the single object we fstat and read; the path is never re-resolved.
    let mut file = match std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
    {
        Ok(f) => f,
        Err(e) if e.raw_os_error() == Some(libc::ELOOP) => {
            return Err(SignerError::KeyUnavailable(format!(
                "key file {} is a symlink; refusing to follow it",
                path.display()
            )))
        }
        Err(e) => return Err(e.into()),
    };

    let meta = file.metadata()?; // fstat on the open fd
    if !meta.is_file() {
        return Err(SignerError::KeyUnavailable(format!(
            "key path {} is not a regular file",
            path.display()
        )));
    }
    if meta.permissions().mode() & 0o077 != 0 {
        return Err(SignerError::KeyUnavailable(format!(
            "key file {} is reachable by group or others; expected mode 0600",
            path.display()
        )));
    }

    let mut bytes = Zeroizing::new(Vec::new());
    file.read_to_end(&mut bytes)?;
    if bytes.len() != KEY_LEN {
        return Err(SignerError::KeyUnavailable(format!(
            "key file {} holds {} bytes, expected {KEY_LEN}",
            path.display(),
            bytes.len()
        )));
    }
    Ok(bytes)
}

/// Generate a fresh key with the OS CSPRNG and persist it durably at mode 0600.
fn create_key(path: &Path) -> Result<Zeroizing<Vec<u8>>> {
    let parent = path.parent().ok_or_else(|| {
        SignerError::KeyUnavailable(format!("key path {} has no parent directory", path.display()))
    })?;
    crate::paths::ensure_private_dir(parent)?;

    let mut key = Zeroizing::new(vec![0u8; KEY_LEN]);
    getrandom::getrandom(&mut key)
        .map_err(|e| SignerError::KeyUnavailable(format!("CSPRNG failed: {e}")))?;

    // `create_new` makes this start the race winner if two starts collide;
    // `mode(0o600)` keeps the key from ever existing world-readable, not even for
    // the moment between create and a later chmod.
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
    {
        Ok(mut f) => {
            f.write_all(&key)?;
            // The key must be on disk before any record is chained with it: a
            // crash that loses the key while log records survive leaves the chain
            // unverifiable forever. fsync the file, then the directory so the new
            // directory entry is durable too.
            f.sync_all()?;
            fsync_dir(parent)?;
            Ok(key)
        }
        // Lost the race: another start created the key first; read it back.
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => read_key(path),
        Err(e) => Err(e.into()),
    }
}

/// fsync a directory so a newly-created entry within it survives a crash.
fn fsync_dir(dir: &Path) -> Result<()> {
    std::fs::File::open(dir)?.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::key_path;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;

    #[test]
    fn creates_a_0600_key_on_genesis() {
        let dir = tempfile::tempdir().unwrap();
        let path = key_path(dir.path());
        let key = load_or_create(&path, false).expect("genesis create");
        assert_eq!(key.len(), KEY_LEN);
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "key file must be mode 0600");
    }

    #[test]
    fn load_returns_the_same_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = key_path(dir.path());
        let first = load_or_create(&path, false).unwrap();
        // The file now exists; a second call (log now non-empty) must read it
        // back, never generate a new one.
        let second = load_or_create(&path, true).unwrap();
        assert_eq!(first.as_slice(), second.as_slice());
    }

    #[test]
    fn missing_key_with_a_nonempty_log_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        let path = key_path(dir.path());
        match load_or_create(&path, true) {
            Err(SignerError::KeyUnavailable(_)) => {}
            other => panic!("expected KeyUnavailable, got {other:?}"),
        }
        assert!(!path.exists(), "no key file may be written in this case");
    }

    fn write_file(path: &Path, content: &[u8], mode: u32) {
        std::fs::write(path, content).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
    }

    #[test]
    fn a_wrong_length_key_file_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = key_path(dir.path());
        write_file(&path, b"too short", 0o600);
        assert!(matches!(
            load_or_create(&path, false),
            Err(SignerError::KeyUnavailable(_))
        ));
    }

    #[test]
    fn a_symlinked_key_file_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real.key");
        write_file(&real, &[7u8; KEY_LEN], 0o600);
        let link = key_path(dir.path());
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert!(matches!(
            load_or_create(&link, false),
            Err(SignerError::KeyUnavailable(_))
        ));
    }

    #[test]
    fn a_group_readable_key_file_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = key_path(dir.path());
        write_file(&path, &[7u8; KEY_LEN], 0o640);
        assert!(matches!(
            load_or_create(&path, false),
            Err(SignerError::KeyUnavailable(_))
        ));
    }
}

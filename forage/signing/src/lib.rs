//! Persistent builder signing key for forage.
//!
//! A forage builder signs every `.lunpkg` it produces with a persistent
//! Ed25519 keypair (forage-recipes.md R4, "a persistent 0600 keypair"). This
//! crate owns that key's lifecycle: generate one with the OS CSPRNG on first
//! use, load it on subsequent runs, and refuse to touch a key file whose
//! permissions or shape expose or corrupt the secret. The signing scheme itself
//! (what bytes are signed, how the signature is framed in the package) lives in
//! `arlen-forage-package`; this only manages the key material the package
//! writer consumes through [`BuilderKey::signing_key`].
//!
//! The file handling mirrors the audit daemon's HMAC key: `lstat` so a planted
//! symlink cannot redirect the builder to key material it does not own, a
//! regular-file check, a strict `0600` mode check, and `create_new` +
//! `mode(0o600)` on genesis so the secret never exists group- or
//! world-readable, not even for the instant between create and chmod. Genesis
//! is fsynced (file then directory) so a crash cannot leave a half-written key.

use std::io::Write;
use std::path::{Path, PathBuf};

use ed25519_dalek::SigningKey;
use thiserror::Error;
use zeroize::Zeroizing;

/// The on-disk length of an Ed25519 secret seed.
const SEED_LEN: usize = 32;

/// A failure loading or creating the builder key. Every variant is terminal:
/// the caller must not sign without a key it fully controls.
#[derive(Debug, Error)]
pub enum KeyError {
    /// The key file or its directory could not be read or written.
    #[error("builder key io at {path}: {source}")]
    Io {
        /// The path being read or written.
        path: PathBuf,
        /// The underlying io error.
        source: std::io::Error,
    },
    /// The key path is a symlink; following it could redirect signing to key
    /// material the builder does not control.
    #[error("builder key {path} is a symlink; refusing to follow it")]
    Symlink {
        /// The offending path.
        path: PathBuf,
    },
    /// The key path exists but is not a regular file.
    #[error("builder key {path} is not a regular file")]
    NotRegular {
        /// The offending path.
        path: PathBuf,
    },
    /// The key file is reachable by group or other; a private key must be 0600.
    #[error("builder key {path} has insecure mode {mode:o}; a private key must be readable only by its owner (0600)")]
    InsecureMode {
        /// The offending path.
        path: PathBuf,
        /// The observed permission bits.
        mode: u32,
    },
    /// The key file exists but is not exactly [`SEED_LEN`] bytes.
    #[error("builder key {path} is malformed: expected {SEED_LEN} bytes, found {found}")]
    Malformed {
        /// The offending path.
        path: PathBuf,
        /// The byte length found.
        found: usize,
    },
    /// The OS CSPRNG could not produce a fresh seed.
    #[error("CSPRNG failed: {0}")]
    Csprng(String),
}

/// A loaded builder signing key. The inner [`SigningKey`] zeroizes its secret
/// scalar on drop (ed25519-dalek `zeroize` feature), and the crate never derives
/// `Debug` on it, so the secret neither lingers in freed memory nor leaks into
/// logs.
pub struct BuilderKey {
    key: SigningKey,
}

impl BuilderKey {
    /// Load the builder key at `path`, generating and persisting a fresh one if
    /// it does not exist.
    ///
    /// A new file is written `0600` with its parent directory created `0700`.
    /// An existing file is rejected fail-closed if it is a symlink, not a
    /// regular file, reachable by group or other, or not exactly 32 bytes. If
    /// two builders race genesis, `create_new` picks one winner and the loser
    /// loads the winner's persisted key, so the builder identity is stable.
    pub fn load_or_create(path: &Path) -> Result<Self, KeyError> {
        // `exists()` follows symlinks, so a dangling symlink routes here to
        // `create_seed`; there `create_new` (O_EXCL) fails `AlreadyExists` on a
        // symlink regardless of target, falling back to `read_seed`, whose
        // `lstat` then rejects it. So the symlink guard does not depend on this
        // pre-check, which is only an optimisation.
        let seed = if path.exists() {
            read_seed(path)?
        } else {
            create_seed(path)?
        };
        // `seed` zeroizes when this scope ends; the key keeps its own copy and
        // zeroizes that on drop.
        Ok(BuilderKey {
            key: SigningKey::from_bytes(&seed),
        })
    }

    /// The Ed25519 signing key, for the package writer.
    pub fn signing_key(&self) -> &SigningKey {
        &self.key
    }

    /// The lowercase-hex public key: a stable builder identity fingerprint.
    pub fn public_key_hex(&self) -> String {
        self.key
            .verifying_key()
            .to_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }
}

/// Read and validate an existing key file into a 32-byte seed. The returned
/// seed and the intermediate file buffer both zeroize on drop.
fn read_seed(path: &Path) -> Result<Zeroizing<[u8; SEED_LEN]>, KeyError> {
    // `symlink_metadata` is `lstat`: it does not follow a symlink, so a planted
    // link cannot point the builder at a key it does not own.
    let meta = std::fs::symlink_metadata(path).map_err(|source| KeyError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    if meta.file_type().is_symlink() {
        return Err(KeyError::Symlink {
            path: path.to_path_buf(),
        });
    }
    if !meta.is_file() {
        return Err(KeyError::NotRegular {
            path: path.to_path_buf(),
        });
    }
    check_mode(path, &meta)?;

    let bytes = Zeroizing::new(std::fs::read(path).map_err(|source| KeyError::Io {
        path: path.to_path_buf(),
        source,
    })?);
    let found = bytes.len();
    let seed: [u8; SEED_LEN] = bytes.as_slice().try_into().map_err(|_| KeyError::Malformed {
        path: path.to_path_buf(),
        found,
    })?;
    Ok(Zeroizing::new(seed))
}

/// Reject a key file reachable by group or other (any of the low 6 mode bits).
#[cfg(unix)]
fn check_mode(path: &Path, meta: &std::fs::Metadata) -> Result<(), KeyError> {
    use std::os::unix::fs::PermissionsExt;
    let mode = meta.permissions().mode();
    if mode & 0o077 != 0 {
        return Err(KeyError::InsecureMode {
            path: path.to_path_buf(),
            mode: mode & 0o777,
        });
    }
    Ok(())
}

#[cfg(not(unix))]
fn check_mode(_path: &Path, _meta: &std::fs::Metadata) -> Result<(), KeyError> {
    Ok(())
}

/// Generate a fresh seed with the OS CSPRNG and persist it durably at mode 0600.
/// The returned seed zeroizes on drop.
fn create_seed(path: &Path) -> Result<Zeroizing<[u8; SEED_LEN]>, KeyError> {
    let parent = path.parent().filter(|p| !p.as_os_str().is_empty());
    if let Some(parent) = parent {
        ensure_private_dir(parent)?;
    }

    let mut seed = Zeroizing::new([0u8; SEED_LEN]);
    getrandom::getrandom(seed.as_mut()).map_err(|e| KeyError::Csprng(e.to_string()))?;

    match open_new_0600(path) {
        Ok(mut f) => {
            f.write_all(seed.as_ref()).map_err(|source| KeyError::Io {
                path: path.to_path_buf(),
                source,
            })?;
            // The key must be durable before anything is signed with it: a
            // crash that loses the key while signed packages survive leaves
            // those signatures unverifiable. fsync the file, then its directory
            // so the new entry is durable too.
            f.sync_all().map_err(|source| KeyError::Io {
                path: path.to_path_buf(),
                source,
            })?;
            if let Some(parent) = parent {
                fsync_dir(parent)?;
            }
            Ok(seed)
        }
        // Lost the genesis race: another builder created it first; load that.
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => read_seed(path),
        Err(source) => Err(KeyError::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
}

/// Open `path` for writing, creating it new at mode 0600 so it never exists
/// group- or world-readable. `create_new` fails with `AlreadyExists` rather
/// than clobber a racing creator's key.
#[cfg(unix)]
fn open_new_0600(path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
}

#[cfg(not(unix))]
fn open_new_0600(path: &Path) -> std::io::Result<std::fs::File> {
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
}

/// Ensure `dir` exists, pinning it to 0700 only when this call creates it.
///
/// A pre-existing directory is left untouched: the caller may pass a path under
/// a shared directory (for example `~/.config`), and silently tightening that
/// to 0700 would be an out-of-bounds mutation of a directory this crate does not
/// own. The key file itself is always 0600, so the secret stays private even
/// when its directory is group-traversable.
#[cfg(unix)]
fn ensure_private_dir(dir: &Path) -> Result<(), KeyError> {
    use std::os::unix::fs::PermissionsExt;
    // Create ancestors with their default mode; only the leaf this call creates
    // is forced to 0700.
    if let Some(parent) = dir.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|source| KeyError::Io {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    match std::fs::create_dir(dir) {
        Ok(()) => std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).map_err(
            |source| KeyError::Io {
                path: dir.to_path_buf(),
                source,
            },
        ),
        // Already there: do not touch a directory we did not just create.
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(source) => Err(KeyError::Io {
            path: dir.to_path_buf(),
            source,
        }),
    }
}

#[cfg(not(unix))]
fn ensure_private_dir(dir: &Path) -> Result<(), KeyError> {
    std::fs::create_dir_all(dir).map_err(|source| KeyError::Io {
        path: dir.to_path_buf(),
        source,
    })
}

/// fsync a directory so a newly-created entry within it survives a crash.
fn fsync_dir(dir: &Path) -> Result<(), KeyError> {
    std::fs::File::open(dir)
        .and_then(|f| f.sync_all())
        .map_err(|source| KeyError::Io {
            path: dir.to_path_buf(),
            source,
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, Verifier};

    #[cfg(unix)]
    fn mode_of(path: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn genesis_creates_a_0600_key_that_signs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("forage/builder.key");
        let key = BuilderKey::load_or_create(&path).expect("genesis");
        assert!(path.exists());
        #[cfg(unix)]
        assert_eq!(mode_of(&path), 0o600, "the key file must be 0600");

        // The key actually works: sign then verify.
        let msg = b"a package digest";
        let sig = key.signing_key().sign(msg);
        assert!(key.signing_key().verifying_key().verify(msg, &sig).is_ok());
    }

    #[test]
    fn load_returns_the_same_identity() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("builder.key");
        let first = BuilderKey::load_or_create(&path).unwrap();
        // A second call must read the persisted key back, not mint a new one.
        let second = BuilderKey::load_or_create(&path).unwrap();
        assert_eq!(first.public_key_hex(), second.public_key_hex());
    }

    #[test]
    fn parent_directory_is_created_private() {
        let dir = tempfile::tempdir().unwrap();
        let keydir = dir.path().join("nested/forage");
        let path = keydir.join("builder.key");
        BuilderKey::load_or_create(&path).unwrap();
        #[cfg(unix)]
        assert_eq!(mode_of(&keydir), 0o700, "the key directory must be 0700");
    }

    #[cfg(unix)]
    #[test]
    fn existing_directory_permissions_are_left_alone() {
        use std::os::unix::fs::PermissionsExt;
        // A caller may put the key under a pre-existing shared directory; that
        // directory's mode must not be silently tightened to 0700.
        let dir = tempfile::tempdir().unwrap();
        let shared = dir.path().join("config");
        std::fs::create_dir(&shared).unwrap();
        std::fs::set_permissions(&shared, std::fs::Permissions::from_mode(0o755)).unwrap();
        let path = shared.join("builder.key");
        BuilderKey::load_or_create(&path).unwrap();
        assert_eq!(mode_of(&shared), 0o755, "the pre-existing dir keeps its mode");
        assert_eq!(mode_of(&path), 0o600, "but the key file is still 0600");
    }

    #[cfg(unix)]
    #[test]
    fn group_readable_key_is_rejected() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("builder.key");
        std::fs::write(&path, [0u8; SEED_LEN]).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
        match BuilderKey::load_or_create(&path) {
            Err(KeyError::InsecureMode { mode, .. }) => assert_eq!(mode, 0o640),
            Ok(_) => panic!("expected InsecureMode, got a valid key"),
            Err(e) => panic!("expected InsecureMode, got {e:?}"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_key_is_refused() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real.key");
        std::fs::write(&real, [1u8; SEED_LEN]).unwrap();
        std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o600)).unwrap();
        let link = dir.path().join("builder.key");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        match BuilderKey::load_or_create(&link) {
            Err(KeyError::Symlink { .. }) => {}
            Ok(_) => panic!("expected Symlink, got a valid key"),
            Err(e) => panic!("expected Symlink, got {e:?}"),
        }
    }

    #[test]
    fn malformed_length_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("builder.key");
        std::fs::write(&path, b"too short").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        match BuilderKey::load_or_create(&path) {
            Err(KeyError::Malformed { found, .. }) => assert_eq!(found, 9),
            Ok(_) => panic!("expected Malformed, got a valid key"),
            Err(e) => panic!("expected Malformed, got {e:?}"),
        }
    }
}

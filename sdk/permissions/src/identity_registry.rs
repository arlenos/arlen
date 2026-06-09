//! The broker-owned app-identity registry (F3 Rung B).
//!
//! `~/.local/share/arlen/apps/` is user-writable by design, so a PATH rule can
//! never be unforgeable there. But the INODE of the binary installd actually wrote
//! is unforgeable by a same-uid copy-to-a-different-path: a copy gets a new inode,
//! while a hardlink to the original is the same file (harmless). installd records
//! `(app_id -> install_path, ino, dev)` at install time into this registry, written
//! ONLY through the root `permission-helper` to `/var/lib/arlen/identity/{uid}/`, so
//! a same-uid attacker cannot rewrite the mapping to a malicious binary; the
//! resolver then rejects any inode/device mismatch (a spoof copy) and trusts an
//! inode match (the real binary, even via a benign hardlink).
//!
//! This module is the read side and the verifier (consumed by the daemon/signer
//! resolvers). The write side is the root `permission-helper`'s `RecordIdentity`.

use std::collections::HashMap;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Why the identity registry could not be loaded.
#[derive(Debug, Error)]
pub enum IdentityError {
    /// The registry file could not be read.
    #[error("identity registry IO: {0}")]
    Io(#[from] std::io::Error),
    /// The registry file was present but malformed (fail closed: a tampered or
    /// corrupt registry is an error, never silently treated as empty-and-permissive).
    #[error("identity registry parse: {0}")]
    Parse(String),
}

/// One app's recorded binary identity: where installd wrote the binary and the
/// inode/device of that file. The `(ino, dev)` pair is the unforgeable gate; the
/// path is advisory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdentityRecord {
    /// The path installd wrote the binary to (advisory; the inode is the gate).
    pub install_path: PathBuf,
    /// The inode number of the written binary.
    pub ino: u64,
    /// The device id of the binary's filesystem.
    pub dev: u64,
}

impl IdentityRecord {
    /// Build a record by stat-ing `path` (following symlinks, so a `/proc/pid/exe`
    /// or an install path resolves to the real binary). This is what the root helper
    /// uses to record the truth rather than trusting a caller-supplied inode.
    pub fn for_path(path: &Path) -> std::io::Result<Self> {
        let meta = std::fs::metadata(path)?;
        Ok(IdentityRecord {
            install_path: path.to_path_buf(),
            ino: meta.ino(),
            dev: meta.dev(),
        })
    }
}

/// The broker-owned app-identity registry: `app_id -> ` the recorded binary
/// identity. Loaded from a root-owned file, so a same-uid process cannot forge a
/// mapping.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IdentityRegistry {
    apps: HashMap<String, IdentityRecord>,
}

impl IdentityRegistry {
    /// Load the registry for `uid` from `/var/lib/arlen/identity/{uid}/registry.json`
    /// (or `<ARLEN_IDENTITY_DIR>/registry.json` when the test/dev override is set). A
    /// missing file yields an empty registry (no app is inode-attested yet, the
    /// pre-Rung-B state); a present-but-corrupt file is an error, never silently
    /// empty (fail closed).
    pub fn load(uid: u32) -> Result<Self, IdentityError> {
        let path = registry_path(uid);
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                serde_json::from_str(&text).map_err(|e| IdentityError::Parse(e.to_string()))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(IdentityError::Io(e)),
        }
    }

    /// The recorded identity for `app_id`, if any.
    pub fn lookup(&self, app_id: &str) -> Option<&IdentityRecord> {
        self.apps.get(app_id)
    }

    /// Insert or replace `app_id`'s record (used by the writer side and tests).
    pub fn record(&mut self, app_id: String, record: IdentityRecord) {
        self.apps.insert(app_id, record);
    }

    /// The number of recorded apps.
    pub fn len(&self) -> usize {
        self.apps.len()
    }

    /// Whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.apps.is_empty()
    }

    /// Serialize to the canonical JSON the writer persists.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("identity registry serialization is infallible")
    }
}

/// The registry file path for `uid`. The `ARLEN_IDENTITY_DIR` override resolves
/// directly to `<dir>/registry.json` (no uid subdir), but ONLY in a debug build:
/// the registry is the trust root for the inode gate, so a release build must never
/// let an attacker-controlled environment variable redirect the resolver to a
/// forged registry. In release the path is always the root-owned `/var/lib`
/// location.
fn registry_path(uid: u32) -> PathBuf {
    #[cfg(debug_assertions)]
    if let Ok(dir) = std::env::var("ARLEN_IDENTITY_DIR") {
        return PathBuf::from(dir).join("registry.json");
    }
    PathBuf::from("/var/lib/arlen/identity")
        .join(uid.to_string())
        .join("registry.json")
}

/// Whether `exe_path` is the binary `record` attests: its inode AND device must
/// match the recorded ones. Path equality is advisory - an inode match at a
/// DIFFERENT path is a hardlink to the same file (benign, passes), while a path
/// match with an inode mismatch is a copy/spoof (fails). Stats `exe_path` (following
/// symlinks, so `/proc/pid/exe` resolves to the real binary); an unstattable path
/// fails closed.
pub fn verify_binary(record: &IdentityRecord, exe_path: &Path) -> bool {
    match std::fs::metadata(exe_path) {
        Ok(meta) => meta.ino() == record.ino && meta.dev() == record.dev,
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_file(dir: &Path, name: &str, content: &[u8]) -> PathBuf {
        let p = dir.join(name);
        let mut f = std::fs::File::create(&p).unwrap();
        f.write_all(content).unwrap();
        p
    }

    #[test]
    fn verify_passes_for_the_recorded_binary() {
        let tmp = tempfile::tempdir().unwrap();
        let bin = write_file(tmp.path(), "app", b"binary");
        let record = IdentityRecord::for_path(&bin).unwrap();
        assert!(verify_binary(&record, &bin));
    }

    #[test]
    fn verify_fails_for_a_copy_at_a_different_path() {
        let tmp = tempfile::tempdir().unwrap();
        let bin = write_file(tmp.path(), "app", b"binary");
        let record = IdentityRecord::for_path(&bin).unwrap();
        // A copy gets a new inode: the spoof the registry exists to catch.
        let copy = tmp.path().join("evil-copy");
        std::fs::copy(&bin, &copy).unwrap();
        assert!(!verify_binary(&record, &copy), "a copy has a different inode");
    }

    #[test]
    fn verify_passes_for_a_hardlink_to_the_same_file() {
        let tmp = tempfile::tempdir().unwrap();
        let bin = write_file(tmp.path(), "app", b"binary");
        let record = IdentityRecord::for_path(&bin).unwrap();
        // A hardlink is the same file (same inode), so it is the same binary - benign.
        let link = tmp.path().join("hardlink");
        std::fs::hard_link(&bin, &link).unwrap();
        assert!(verify_binary(&record, &link), "a hardlink shares the inode");
    }

    #[test]
    fn verify_fails_closed_on_a_missing_path() {
        let record = IdentityRecord {
            install_path: PathBuf::from("/nope"),
            ino: 1,
            dev: 1,
        };
        assert!(!verify_binary(&record, Path::new("/no/such/binary")));
    }

    #[test]
    fn registry_round_trips_and_looks_up() {
        let mut reg = IdentityRegistry::default();
        reg.record(
            "com.example.app".into(),
            IdentityRecord {
                install_path: PathBuf::from("/home/u/.local/share/arlen/apps/com.example.app/bin"),
                ino: 42,
                dev: 7,
            },
        );
        let json = reg.to_json();
        let back: IdentityRegistry = serde_json::from_str(&json).unwrap();
        assert_eq!(back.len(), 1);
        assert_eq!(back.lookup("com.example.app").unwrap().ino, 42);
        assert!(back.lookup("other").is_none());
    }

    #[test]
    fn load_missing_is_empty_and_corrupt_is_error() {
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("ARLEN_IDENTITY_DIR", tmp.path());
        // Missing file -> empty registry (the pre-Rung-B state).
        assert!(IdentityRegistry::load(1000).unwrap().is_empty());
        // Corrupt file -> error (fail closed, never empty-and-permissive).
        write_file(tmp.path(), "registry.json", b"{ not json");
        assert!(IdentityRegistry::load(1000).is_err());
        std::env::remove_var("ARLEN_IDENTITY_DIR");
    }
}

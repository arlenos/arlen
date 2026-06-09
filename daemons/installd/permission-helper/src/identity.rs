//! Recording app binary identity into the broker-owned registry (F3 Rung B).
//!
//! installd asks the root helper to record `(app_id -> install_path, ino, dev)` for
//! a freshly installed app. The helper RE-STATS the install path itself rather than
//! trusting a caller-supplied inode (a compromised installd must not be able to
//! record a lie), validates the app_id, rejects an install path outside the
//! expected roots (`/usr/lib/arlen/apps/` for system apps, the uid's
//! `~/.local/share/arlen/apps/` for user apps), and atomically merges the record
//! into `/var/lib/arlen/identity/{uid}/registry.json` - root-owned, so a same-uid
//! process cannot rewrite the mapping. The reader (`arlen-permissions`
//! `identity_registry`) then gates a runtime binary by its `(ino, dev)`.

use std::ffi::CStr;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Component, Path, PathBuf};

use arlen_permissions::identity_registry::{IdentityRecord, IdentityRegistry};
use thiserror::Error;

use crate::profile::validate_app_id;

const DEFAULT_BASE: &str = "/var/lib/arlen/identity";
const SYSTEM_APPS_ROOT: &str = "/usr/lib/arlen/apps";

/// Errors from recording an identity.
#[derive(Debug, Error)]
pub enum IdentityError {
    /// The app_id was not a safe identifier.
    #[error("invalid app_id: {0}")]
    InvalidAppId(String),
    /// The install path is outside the roots an app may be installed to.
    #[error("install path outside the allowed roots: {0}")]
    ForbiddenPath(String),
    /// The registry file was present but malformed (fail closed).
    #[error("registry parse: {0}")]
    Parse(String),
    /// IO error.
    #[error("IO: {0}")]
    Io(#[from] std::io::Error),
}

/// Get the registry base directory (overridable for tests via
/// `ARLEN_IDENTITY_DIR`, matching the reader's override).
fn base_dir() -> PathBuf {
    std::env::var("ARLEN_IDENTITY_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_BASE))
}

/// The registry file path for a uid under `base`.
fn registry_path_in(base: &Path, uid: u32) -> PathBuf {
    base.join(uid.to_string()).join("registry.json")
}

/// The home directory of `uid`, resolved through `getpwuid_r`. Used to locate the
/// per-user app root the install path must be under.
fn home_for_uid(uid: u32) -> Option<PathBuf> {
    let mut buf = vec![0u8; 4096];
    let mut pwd: libc::passwd = unsafe { std::mem::zeroed() };
    let mut result: *mut libc::passwd = std::ptr::null_mut();
    // SAFETY: standard getpwuid_r call with a sufficiently large buffer; `result`
    // is null on not-found or error, checked below before any deref.
    let rc = unsafe {
        libc::getpwuid_r(
            uid as libc::uid_t,
            &mut pwd,
            buf.as_mut_ptr() as *mut libc::c_char,
            buf.len(),
            &mut result,
        )
    };
    if rc != 0 || result.is_null() || pwd.pw_dir.is_null() {
        return None;
    }
    // SAFETY: pw_dir is a valid C string into `buf` for a found entry.
    let dir = unsafe { CStr::from_ptr(pwd.pw_dir) };
    Some(PathBuf::from(dir.to_string_lossy().into_owned()))
}

/// The roots a `uid`'s app binary may be installed under: the system apps tree and
/// the uid's own app tree (resolved through the uid's home).
fn allowed_roots(uid: u32) -> Vec<PathBuf> {
    let mut roots = vec![PathBuf::from(SYSTEM_APPS_ROOT)];
    if let Some(home) = home_for_uid(uid) {
        roots.push(home.join(".local/share/arlen/apps"));
    }
    roots
}

/// Whether `install_path`'s real location is under one of `roots`. The path is
/// canonicalised first (resolving symlinks and `..`), so neither a `..` escape nor
/// a symlink can place a binary outside its app root yet pass the check; a path
/// that cannot be canonicalised (missing) is rejected.
fn is_under_allowed_root(install_path: &Path, roots: &[PathBuf]) -> bool {
    let Ok(canon) = install_path.canonicalize() else {
        return false;
    };
    // canonicalize removes `..`, but double-check defensively.
    if canon.components().any(|c| matches!(c, Component::ParentDir)) {
        return false;
    }
    roots.iter().any(|root| {
        let canon_root = root.canonicalize().unwrap_or_else(|_| root.clone());
        canon.starts_with(&canon_root)
    })
}

/// Record `app_id`'s binary identity into the registry under `base`. Validates the
/// app_id and the install path, RE-STATS the install path (never trusts a
/// caller-supplied inode), and atomically merges the record into the uid's
/// registry file. The directory is root-owned and owner-traversable (`0o711`,
/// file `0o644`), like the profile tree, so the user-side resolver can read it but
/// a same-uid process cannot rewrite it.
pub fn record_identity_in(
    base: &Path,
    uid: u32,
    app_id: &str,
    install_path: &Path,
) -> Result<PathBuf, IdentityError> {
    record_with_roots(base, uid, app_id, install_path, &allowed_roots(uid))
}

/// The core of [`record_identity_in`] with explicit allowed roots (so the path-root
/// gate is unit-testable without depending on the host's `/usr/lib` or the test
/// user's real home).
fn record_with_roots(
    base: &Path,
    uid: u32,
    app_id: &str,
    install_path: &Path,
    roots: &[PathBuf],
) -> Result<PathBuf, IdentityError> {
    validate_app_id(app_id).map_err(|_| IdentityError::InvalidAppId(app_id.into()))?;
    if !is_under_allowed_root(install_path, roots) {
        return Err(IdentityError::ForbiddenPath(
            install_path.display().to_string(),
        ));
    }
    // The helper stats the path itself: the recorded (ino, dev) is the truth of the
    // file installd wrote, not a value the caller could have lied about.
    let record = IdentityRecord::for_path(install_path)?;

    let path = registry_path_in(base, uid);
    let dir = path.parent().unwrap();
    if !dir.exists() {
        fs::create_dir_all(dir)?;
    }
    if let Some(parent) = dir.parent() {
        let _ = fs::set_permissions(parent, fs::Permissions::from_mode(0o755));
    }
    let _ = fs::set_permissions(dir, fs::Permissions::from_mode(0o711));

    // Load-merge-write so a second app's record does not clobber the first.
    let mut registry = match fs::read_to_string(&path) {
        Ok(text) => {
            serde_json::from_str::<IdentityRegistry>(&text).map_err(|e| IdentityError::Parse(e.to_string()))?
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => IdentityRegistry::default(),
        Err(e) => return Err(IdentityError::Io(e)),
    };
    registry.record(app_id.to_string(), record);

    let tmp = path.with_extension("tmp");
    fs::write(&tmp, registry.to_json())?;
    let _ = fs::set_permissions(&tmp, fs::Permissions::from_mode(0o644));
    fs::rename(&tmp, &path)?;
    Ok(path)
}

/// Record an identity at the default base directory.
pub fn record_identity(uid: u32, app_id: &str, install_path: &Path) -> Result<PathBuf, IdentityError> {
    record_identity_in(&base_dir(), uid, app_id, install_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::os::unix::fs::MetadataExt;

    fn write_bin(dir: &Path, name: &str) -> PathBuf {
        let p = dir.join(name);
        let mut f = fs::File::create(&p).unwrap();
        f.write_all(b"binary").unwrap();
        p
    }

    fn roots(p: &Path) -> Vec<PathBuf> {
        vec![p.to_path_buf()]
    }

    #[test]
    fn records_and_round_trips_under_an_allowed_root() {
        let apps = tempfile::tempdir().unwrap();
        let base = tempfile::tempdir().unwrap();
        let bin = write_bin(apps.path(), "app-bin");

        let path =
            record_with_roots(base.path(), 1000, "com.example.app", &bin, &roots(apps.path()))
                .unwrap();
        let reg: IdentityRegistry =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        let rec = reg.lookup("com.example.app").unwrap();
        // The recorded inode is the real file's, not a caller claim.
        let real = fs::metadata(&bin).unwrap();
        assert_eq!(rec.ino, real.ino());
    }

    #[test]
    fn refuses_a_path_outside_the_allowed_roots() {
        let outside = tempfile::tempdir().unwrap();
        let apps = tempfile::tempdir().unwrap();
        let base = tempfile::tempdir().unwrap();
        let bin = write_bin(outside.path(), "evil");
        let err = record_with_roots(base.path(), 1000, "com.example.app", &bin, &roots(apps.path()))
            .unwrap_err();
        assert!(matches!(err, IdentityError::ForbiddenPath(_)));
    }

    #[test]
    fn refuses_an_invalid_app_id() {
        let apps = tempfile::tempdir().unwrap();
        let base = tempfile::tempdir().unwrap();
        let bin = write_bin(apps.path(), "app-bin");
        assert!(matches!(
            record_with_roots(base.path(), 1000, "../evil", &bin, &roots(apps.path())),
            Err(IdentityError::InvalidAppId(_))
        ));
    }

    #[test]
    fn a_second_record_does_not_clobber_the_first() {
        let apps = tempfile::tempdir().unwrap();
        let base = tempfile::tempdir().unwrap();
        let a = write_bin(apps.path(), "a-bin");
        let b = write_bin(apps.path(), "b-bin");
        record_with_roots(base.path(), 1000, "com.a", &a, &roots(apps.path())).unwrap();
        let path =
            record_with_roots(base.path(), 1000, "com.b", &b, &roots(apps.path())).unwrap();
        let reg: IdentityRegistry =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(reg.len(), 2);
        assert!(reg.lookup("com.a").is_some() && reg.lookup("com.b").is_some());
    }
}

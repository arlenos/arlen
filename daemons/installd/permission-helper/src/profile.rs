//! Profile writing and validation logic.
//!
//! Writes permission profiles to `/var/lib/arlen/permissions/{uid}/{app_id}.toml`.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use arlen_permissions::revoke::{RestoreOutcome, RevokeOutcome, RevokedReach};
use arlen_permissions::revoke_apply::{restore_at, revoke_at, RemovalLedger};
use thiserror::Error;

const DEFAULT_BASE: &str = "/var/lib/arlen/permissions";

/// Errors from profile operations.
#[derive(Debug, Error)]
pub enum ProfileError {
    #[error("invalid app_id: {0}")]
    InvalidAppId(String),
    #[error("invalid TOML: {0}")]
    InvalidToml(String),
    #[error("invalid removal record: {0}")]
    InvalidRecord(String),
    #[error("IO: {0}")]
    Io(#[from] std::io::Error),
}

/// Get the base directory. The `ARLEN_PERMISSIONS_DIR` override is honoured only
/// in a debug build (tests/dev); a release helper always writes the root-owned
/// `/var/lib` location, so the two root writers (this and the identity writer)
/// share one release-pins-the-path rule and a release env-misconfiguration cannot
/// relocate the authoritative profiles (review F3-3).
pub(crate) fn base_dir() -> PathBuf {
    #[cfg(debug_assertions)]
    if let Ok(dir) = std::env::var("ARLEN_PERMISSIONS_DIR") {
        return PathBuf::from(dir);
    }
    PathBuf::from(DEFAULT_BASE)
}

/// Get the profile file path for an app.
pub fn profile_path(uid: u32, app_id: &str) -> PathBuf {
    profile_path_in(&base_dir(), uid, app_id)
}

/// Profile path with explicit base directory.
pub fn profile_path_in(base: &Path, uid: u32, app_id: &str) -> PathBuf {
    base.join(uid.to_string()).join(format!("{app_id}.toml"))
}

/// Validate an app_id: `[A-Za-z0-9._-]`, no path
/// traversal. Deferred to the profile loaders' `arlen_permissions::is_valid_app_id`
/// rather than restated, because the two MUST agree in both directions and a
/// restatement is what let them drift apart.
///
/// Accepting an id the loader rejects would write a root-owned profile the loader
/// cannot resolve, so it would silently fall back to the spoofable `~/.config`
/// tier - the F3 hole. Rejecting an id the loader accepts is safer but not
/// harmless: it makes system-tier enrollment impossible for that app, and since
/// real ids capitalise their last component (`org.gnome.Calculator`) that was most
/// of them. Sharing one function removes both failure modes at once.
///
/// Case is significant and never folded here or in `path_to_app_id`, so two ids
/// differing only in case address two separate profiles on the case-sensitive
/// filesystems this runs on, and neither can shadow the other.
pub fn validate_app_id(app_id: &str) -> Result<(), ProfileError> {
    if arlen_permissions::is_valid_app_id(app_id) {
        Ok(())
    } else {
        Err(ProfileError::InvalidAppId(app_id.into()))
    }
}

/// Validate that the content is parseable TOML with an [info] section.
pub fn validate_toml(content: &str) -> Result<(), ProfileError> {
    let value: toml::Value =
        toml::from_str(content).map_err(|e| ProfileError::InvalidToml(e.to_string()))?;
    let table = value
        .as_table()
        .ok_or_else(|| ProfileError::InvalidToml("expected table at root".into()))?;
    if !table.contains_key("info") {
        return Err(ProfileError::InvalidToml("missing [info] section".into()));
    }
    Ok(())
}

/// Write a permission profile to the default location.
pub fn write_profile(uid: u32, app_id: &str, content: &str) -> Result<PathBuf, ProfileError> {
    write_profile_in(&base_dir(), uid, app_id, content)
}

/// Write a permission profile to an explicit base directory.
pub fn write_profile_in(
    base: &Path,
    uid: u32,
    app_id: &str,
    content: &str,
) -> Result<PathBuf, ProfileError> {
    validate_app_id(app_id)?;
    validate_toml(content)?;

    let path = profile_path_in(base, uid, app_id);
    let dir = path.parent().unwrap();

    if !dir.exists() {
        fs::create_dir_all(dir)?;
    }
    // The directory tree stays root-owned (so a same-uid process cannot swap a
    // profile for a symlink or rewrite it) but must be TRAVERSABLE by the owning
    // user, whose desktop-shell brokers and `--user` knowledge daemon read the 0644
    // profile by name. `0o700 root` would deny the user `--x`, so `Path::exists()`
    // on the reader side returns false on EACCES and the loader silently falls back
    // to the spoofable `~/.config` tier (F3 no-op). The base is `0o755` and the
    // per-uid dir `0o711`: root-write-only, owner-traversable, not listable by
    // others. Profiles are capability declarations, not secrets, so cross-user read
    // of a known id is acceptable; integrity is the property. Set unconditionally so
    // a directory created at the old `0o700` is corrected on the next write.
    if let Some(parent) = dir.parent() {
        let _ = fs::set_permissions(parent, fs::Permissions::from_mode(0o755));
    }
    let _ = fs::set_permissions(dir, fs::Permissions::from_mode(0o711));

    // Atomic write: temp file then rename.
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, content)?;
    let _ = fs::set_permissions(&tmp, fs::Permissions::from_mode(0o644));
    fs::rename(&tmp, &path)?;

    Ok(path)
}

/// Where the helper keeps what a person revoked from a system-tier profile, so
/// a later restore has a ceiling this process can check itself.
///
/// The knowledge daemon bounds a restore by its audit ledger, and the helper
/// cannot: the ledger is keyed and read by another process, and a root writer
/// that widens a profile on a caller's word is the hole the system tier exists
/// to close. So the helper records each reach it removed beside the profile, in
/// the same root-owned directory, and re-adds only what that record names.
///
/// Not `.toml`, so nothing that lists profiles reads it as one.
pub fn removed_path_in(base: &Path, uid: u32, app_id: &str) -> PathBuf {
    base.join(uid.to_string()).join(format!("{app_id}.removed.json"))
}

/// The reaches recorded as removed for one app.
///
/// Absent is nothing removed. A record that is there and cannot be read or
/// parsed is an error: the callers write this file back, and answering an empty
/// record for a damaged one would overwrite every removal it held, after which
/// nothing a person revoked could be given back.
fn removed_in(base: &Path, uid: u32, app_id: &str) -> Result<RemovalLedger, ProfileError> {
    match fs::read(removed_path_in(base, uid, app_id)) {
        Ok(bytes) => {
            serde_json::from_slice(&bytes).map_err(|e| ProfileError::InvalidRecord(e.to_string()))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(RemovalLedger::default()),
        Err(e) => Err(ProfileError::Io(e)),
    }
}

/// Replace the removal record, through a sibling temp file and a rename.
fn write_removed_in(
    base: &Path,
    uid: u32,
    app_id: &str,
    ledger: &RemovalLedger,
) -> Result<(), ProfileError> {
    let path = removed_path_in(base, uid, app_id);
    let tmp = path.with_extension("json.tmp");
    let bytes =
        serde_json::to_vec(ledger).map_err(|e| ProfileError::InvalidRecord(e.to_string()))?;
    fs::write(&tmp, bytes)?;
    let _ = fs::set_permissions(&tmp, fs::Permissions::from_mode(0o644));
    fs::rename(&tmp, &path)?;
    Ok(())
}

/// Remove one reach from a system-tier profile at the default location.
pub fn narrow_profile(
    uid: u32,
    app_id: &str,
    reach: &RevokedReach,
) -> Result<RevokeOutcome, ProfileError> {
    narrow_profile_in(&base_dir(), uid, app_id, reach)
}

/// Remove one reach from the system-tier profile under `base`.
///
/// The edit and its proof are `arlen_permissions::revoke_apply::revoke_at`, the
/// same function the knowledge daemon runs on a user-tier profile: nothing is
/// written unless the re-derived scopes strictly shrank. The removal is recorded
/// only after the profile is written. The other order would let a revoke that
/// changed nothing leave a record behind, and a restore could then add a reach
/// the app never held. The record is READ first, though: one that cannot be read
/// refuses the revoke before the profile changes, so a removal is never made
/// that the record could not then hold.
pub fn narrow_profile_in(
    base: &Path,
    uid: u32,
    app_id: &str,
    reach: &RevokedReach,
) -> Result<RevokeOutcome, ProfileError> {
    validate_app_id(app_id)?;
    let path = profile_path_in(base, uid, app_id);
    let mut ledger = removed_in(base, uid, app_id)?;
    let outcome = revoke_at(&path, reach)?;
    if outcome == RevokeOutcome::Revoked {
        let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o644));
        ledger.record(reach.clone());
        write_removed_in(base, uid, app_id, &ledger)?;
    }
    Ok(outcome)
}

/// Give back one reach to a system-tier profile at the default location.
pub fn restore_reach(
    uid: u32,
    app_id: &str,
    reach: &RevokedReach,
) -> Result<RestoreOutcome, ProfileError> {
    restore_reach_in(&base_dir(), uid, app_id, reach)
}

/// Give back one reach to the system-tier profile under `base`, and only a reach
/// this helper removed: the ceiling is the record [`narrow_profile_in`] keeps.
pub fn restore_reach_in(
    base: &Path,
    uid: u32,
    app_id: &str,
    reach: &RevokedReach,
) -> Result<RestoreOutcome, ProfileError> {
    validate_app_id(app_id)?;
    let path = profile_path_in(base, uid, app_id);
    let mut ledger = removed_in(base, uid, app_id)?;
    let outcome = restore_at(&path, reach, &ledger)?;
    if outcome == RestoreOutcome::Restored {
        let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o644));
        ledger.clear(reach);
        write_removed_in(base, uid, app_id, &ledger)?;
    }
    Ok(outcome)
}

/// Delete a permission profile from the default location.
pub fn delete_profile(uid: u32, app_id: &str) -> Result<(), ProfileError> {
    delete_profile_in(&base_dir(), uid, app_id)
}

/// Delete a permission profile from an explicit base directory.
pub fn delete_profile_in(base: &Path, uid: u32, app_id: &str) -> Result<(), ProfileError> {
    validate_app_id(app_id)?;
    let path = profile_path_in(base, uid, app_id);
    if path.exists() {
        fs::remove_file(&path)?;
    }
    // The removal record goes with the profile it bounds. Left behind, it would
    // let a reinstall be widened by what a person revoked from the old install.
    let removed = removed_path_in(base, uid, app_id);
    if removed.exists() {
        fs::remove_file(&removed)?;
    }
    Ok(())
}

/// Check if a profile exists at the default location.
pub fn profile_exists(uid: u32, app_id: &str) -> bool {
    profile_path(uid, app_id).exists()
}

/// Check if a profile exists at an explicit base directory.
// Nothing in the helper asks this yet: the write path overwrites rather than
// checking first, and the read path is the SDK's. Its tests below are the only
// callers, which is what makes it read as dead.
#[allow(dead_code)]
pub fn profile_exists_in(base: &Path, uid: u32, app_id: &str) -> bool {
    profile_path_in(base, uid, app_id).exists()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_PROFILE: &str = r#"
[info]
app_id = "com.test"
tier = "third-party"

[graph]
read = ["com.test.*"]

[filesystem]
documents = true
"#;

    #[test]
    fn test_validate_app_id_valid() {
        assert!(validate_app_id("com.example.app").is_ok());
        assert!(validate_app_id("org.arlen.contacts").is_ok());
        assert!(validate_app_id("my-app_v2").is_ok());
    }

    #[test]
    fn test_validate_app_id_invalid() {
        assert!(validate_app_id("").is_err());
        assert!(validate_app_id("../evil").is_err());
        assert!(validate_app_id("path/traversal").is_err());
        assert!(validate_app_id("has spaces").is_err());
    }

    #[test]
    fn test_validate_app_id_agrees_with_the_loaders() {
        // Both directions, since either disagreement bites. An id the loader
        // rejects must be rejected here too, or the written profile is
        // unresolvable and the loader falls back to the spoofable user tier (the
        // F3 hole). An id the loader accepts must be accepted here, or the app can
        // never hold a system-tier profile at all.
        for bad in ["café.app", ".hidden", "trail.", "..", "a/b", ""] {
            assert!(validate_app_id(bad).is_err(), "{bad:?} must be rejected");
            assert!(!arlen_permissions::is_valid_app_id(bad));
        }
        for good in ["com.example.notes", "org.gnome.Calculator", "app.drey.Biblioteca"] {
            assert!(validate_app_id(good).is_ok(), "{good:?} must be accepted");
            assert!(arlen_permissions::is_valid_app_id(good));
        }
    }

    #[test]
    fn test_validate_toml_valid() {
        assert!(validate_toml(VALID_PROFILE).is_ok());
    }

    #[test]
    fn test_validate_toml_invalid() {
        assert!(validate_toml("not valid toml {{{{").is_err());
    }

    #[test]
    fn test_validate_toml_missing_info() {
        assert!(validate_toml("[graph]\nread = []").is_err());
    }

    #[test]
    fn test_write_and_read_profile() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = write_profile_in(dir.path(), 1000, "com.test", VALID_PROFILE).unwrap();
        assert!(path.exists());
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("com.test"));
    }

    #[test]
    fn test_delete_profile() {
        let dir = tempfile::TempDir::new().unwrap();
        write_profile_in(dir.path(), 1000, "com.test", VALID_PROFILE).unwrap();
        assert!(profile_exists_in(dir.path(), 1000, "com.test"));

        delete_profile_in(dir.path(), 1000, "com.test").unwrap();
        assert!(!profile_exists_in(dir.path(), 1000, "com.test"));

        // Deleting non-existent is OK.
        delete_profile_in(dir.path(), 1000, "com.test").unwrap();
    }

    #[test]
    fn test_profile_path_format() {
        let base = Path::new("/var/lib/arlen/permissions");
        let p = profile_path_in(base, 1000, "com.app");
        assert_eq!(
            p,
            PathBuf::from("/var/lib/arlen/permissions/1000/com.app.toml")
        );
    }

    #[test]
    fn test_write_rejects_invalid_app_id() {
        let dir = tempfile::TempDir::new().unwrap();
        assert!(write_profile_in(dir.path(), 1000, "../evil", VALID_PROFILE).is_err());
        assert!(write_profile_in(dir.path(), 1000, "", VALID_PROFILE).is_err());
    }

    #[test]
    fn test_per_uid_dir_is_owner_traversable() {
        // The owning user's loaders must be able to traverse to their 0644 profile.
        // A 0700 per-uid dir would deny `--x` and silently break the F3 fix.
        let dir = tempfile::TempDir::new().unwrap();
        let path = write_profile_in(dir.path(), 1000, "com.test", VALID_PROFILE).unwrap();
        let uid_dir = path.parent().unwrap();
        let dir_mode = fs::metadata(uid_dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(dir_mode, 0o711, "per-uid dir must be owner-traversable");
        let file_mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(file_mode, 0o644, "profile must be world-readable, root-write-only");
    }
    /// The defect this exists for: the loader lets the system tier win outright,
    /// so a revoke written to the user's own overlay changed nothing for an app
    /// the image ships. The overlay here still grants the reach after the
    /// revoke, and the app must read without it.
    /// A record that is there and does not parse refuses both directions, and
    /// neither the profile nor the record is touched: reading it as empty would
    /// have the next write replace every removal it held.
    #[test]
    fn a_damaged_removal_record_refuses_and_is_left_alone() {
        use arlen_permissions::revoke::RevokedReach;
        let base = tempfile::TempDir::new().unwrap();
        let shipped = "[info]\napp_id = \"dev.arlen.files\"\ntier = \"first-party\"\n\n[graph]\nread = [\"system.File\", \"system.Project\"]\n";
        write_profile_in(base.path(), 1000, "dev.arlen.files", shipped).unwrap();
        let profile = profile_path_in(base.path(), 1000, "dev.arlen.files");
        let before = std::fs::read_to_string(&profile).unwrap();
        let record = removed_path_in(base.path(), 1000, "dev.arlen.files");
        std::fs::write(&record, b"{ not json").unwrap();

        let reach = RevokedReach::Read { entity_pattern: "system.Project".into() };
        assert!(matches!(
            narrow_profile_in(base.path(), 1000, "dev.arlen.files", &reach),
            Err(ProfileError::InvalidRecord(_))
        ));
        assert!(matches!(
            restore_reach_in(base.path(), 1000, "dev.arlen.files", &reach),
            Err(ProfileError::InvalidRecord(_))
        ));
        assert_eq!(std::fs::read_to_string(&profile).unwrap(), before);
        assert_eq!(std::fs::read(&record).unwrap(), b"{ not json");
    }

    #[test]
    fn a_revoke_on_a_system_tier_profile_is_what_the_app_reads() {
        use arlen_permissions::revoke::{RestoreOutcome, RevokeOutcome, RevokedReach};
        let base = tempfile::TempDir::new().unwrap();
        let user = tempfile::TempDir::new().unwrap();
        let shipped = r#"
[info]
app_id = "dev.arlen.files"
tier = "first-party"

[graph]
read = ["system.File", "system.Project"]
"#;
        write_profile_in(base.path(), 1000, "dev.arlen.files", shipped).unwrap();
        std::fs::write(user.path().join("dev.arlen.files.toml"), shipped).unwrap();
        std::env::set_var("ARLEN_SYSTEM_PERMISSIONS_DIR", base.path().join("1000"));
        std::env::set_var("ARLEN_PERMISSIONS_DIR", user.path());
        let reads = || arlen_permissions::load_profile("dev.arlen.files").unwrap().graph.read;
        assert_eq!(reads(), ["system.File", "system.Project"]);

        let reach = RevokedReach::Read { entity_pattern: "system.Project".into() };
        let never_held = RevokedReach::Read { entity_pattern: "system.Mail".into() };
        assert_eq!(
            narrow_profile_in(base.path(), 1000, "dev.arlen.files", &reach).unwrap(),
            RevokeOutcome::Revoked
        );
        assert_eq!(reads(), ["system.File"], "the app reads the narrowed system tier");

        // A second revoke of the same reach changes nothing, and a revoke that
        // changed nothing leaves no record a restore could lean on.
        assert_eq!(
            narrow_profile_in(base.path(), 1000, "dev.arlen.files", &never_held).unwrap(),
            RevokeOutcome::NoChange
        );
        assert_eq!(
            restore_reach_in(base.path(), 1000, "dev.arlen.files", &never_held).unwrap(),
            RestoreOutcome::NotPermitted,
            "a reach this helper never removed is not one it gives back"
        );
        assert_eq!(reads(), ["system.File"]);

        assert_eq!(
            restore_reach_in(base.path(), 1000, "dev.arlen.files", &reach).unwrap(),
            RestoreOutcome::Restored
        );
        assert!(reads().contains(&"system.Project".to_string()));
        assert_eq!(
            restore_reach_in(base.path(), 1000, "dev.arlen.files", &reach).unwrap(),
            RestoreOutcome::NotPermitted,
            "given back once, the record is spent"
        );
        std::env::remove_var("ARLEN_SYSTEM_PERMISSIONS_DIR");
        std::env::remove_var("ARLEN_PERMISSIONS_DIR");
    }

    #[test]
    fn deleting_a_profile_takes_its_removal_record_along() {
        use arlen_permissions::revoke::RevokedReach;
        let base = tempfile::TempDir::new().unwrap();
        write_profile_in(base.path(), 1000, "com.test", VALID_PROFILE).unwrap();
        let reach = RevokedReach::Read { entity_pattern: "com.test.*".into() };
        narrow_profile_in(base.path(), 1000, "com.test", &reach).unwrap();
        assert!(removed_path_in(base.path(), 1000, "com.test").exists());
        delete_profile_in(base.path(), 1000, "com.test").unwrap();
        assert!(!removed_path_in(base.path(), 1000, "com.test").exists());
    }

    #[test]
    fn narrowing_refuses_an_id_that_is_not_a_path_component() {
        use arlen_permissions::revoke::RevokedReach;
        let base = tempfile::TempDir::new().unwrap();
        let reach = RevokedReach::InstanceAll;
        assert!(narrow_profile_in(base.path(), 1000, "../evil", &reach).is_err());
        assert!(restore_reach_in(base.path(), 1000, "a/b", &reach).is_err());
    }

}

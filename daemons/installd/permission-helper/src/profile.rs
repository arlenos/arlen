/// Profile writing and validation logic.
///
/// Writes permission profiles to `/var/lib/arlen/permissions/{uid}/{app_id}.toml`.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use thiserror::Error;

const DEFAULT_BASE: &str = "/var/lib/arlen/permissions";

/// Errors from profile operations.
#[derive(Debug, Error)]
pub enum ProfileError {
    #[error("invalid app_id: {0}")]
    InvalidAppId(String),
    #[error("invalid TOML: {0}")]
    InvalidToml(String),
    #[error("IO: {0}")]
    Io(#[from] std::io::Error),
}

/// Get the base directory.
fn base_dir() -> PathBuf {
    std::env::var("ARLEN_PERMISSIONS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_BASE))
}

/// Get the profile file path for an app.
pub fn profile_path(uid: u32, app_id: &str) -> PathBuf {
    profile_path_in(&base_dir(), uid, app_id)
}

/// Profile path with explicit base directory.
pub fn profile_path_in(base: &Path, uid: u32, app_id: &str) -> PathBuf {
    base.join(uid.to_string()).join(format!("{app_id}.toml"))
}

/// Validate an app_id: lowercase reverse-domain notation over `[a-z0-9._-]`, no
/// path traversal. This MUST agree with the profile loaders' `is_valid_app_id`
/// (`sdk/permissions` and `daemons/knowledge`): if the helper accepted an id the
/// loader rejects (e.g. an uppercase or non-ASCII one), the helper would write a
/// root-owned profile the loader cannot resolve, and the loader would silently fall
/// back to the spoofable `~/.config` tier (the F3 hole reopened). `is_alphanumeric`
/// is Unicode- and uppercase-accepting, so the charset is pinned to ASCII lowercase
/// here, matching the loaders. Leading/trailing dots are rejected too.
pub fn validate_app_id(app_id: &str) -> Result<(), ProfileError> {
    let ok = !app_id.is_empty()
        && app_id != ".."
        && !app_id.starts_with('.')
        && !app_id.ends_with('.')
        && !app_id.contains("..")
        && app_id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'));
    if ok {
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
    Ok(())
}

/// Check if a profile exists at the default location.
pub fn profile_exists(uid: u32, app_id: &str) -> bool {
    profile_path(uid, app_id).exists()
}

/// Check if a profile exists at an explicit base directory.
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
        // Forms the loaders' is_valid_app_id rejects must be rejected here too, or a
        // written profile would be unresolvable and the loader would fall back to the
        // spoofable user tier (F3 hole). Uppercase, non-ASCII, leading/trailing dot.
        for bad in ["Com.Victim", "com.VICTIM", "café.app", ".hidden", "trail."] {
            assert!(validate_app_id(bad).is_err(), "{bad:?} must be rejected");
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
}

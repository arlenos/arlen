//! Resolution of the signer's private state directory and key path.
//!
//! The undo-log lives under `~/.local/state/arlen/agent/undo-log/`
//! (reversible-receipts-and-the-effect-model.md §6), the XDG *state* home (not
//! data home): it is durable machine-written state, not user documents. The
//! directory is created mode 0700 so only the owner can reach it.

use std::path::{Path, PathBuf};

use crate::error::{Result, SignerError};

/// The signer's state directory: `$XDG_STATE_HOME/arlen/agent/undo-log` or, when
/// `XDG_STATE_HOME` is unset, `$HOME/.local/state/arlen/agent/undo-log`. Fails
/// closed if neither variable resolves an absolute base.
pub fn undo_log_dir() -> Result<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .filter(|s| !s.is_empty())
                .map(|h| PathBuf::from(h).join(".local/state"))
        })
        .ok_or_else(|| {
            SignerError::Storage(
                "cannot resolve a per-user state directory: \
                 neither XDG_STATE_HOME nor HOME is set"
                    .to_string(),
            )
        })?;
    if !base.is_absolute() {
        return Err(SignerError::Storage(format!(
            "resolved state base {} is not an absolute path",
            base.display()
        )));
    }
    Ok(base.join("arlen/agent/undo-log"))
}

/// The HMAC key file path inside `dir`.
pub fn key_path(dir: &Path) -> PathBuf {
    dir.join("hmac.key")
}

/// Create `dir` (and parents) and clamp it to mode 0700 so the undo-log and its
/// key are reachable only by the owner (the signer uid).
pub fn ensure_private_dir(dir: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(dir)?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_path_is_under_the_dir() {
        let p = key_path(Path::new("/x/y"));
        assert_eq!(p, PathBuf::from("/x/y/hmac.key"));
    }

    #[test]
    fn ensure_private_dir_creates_a_0700_dir() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("nested/undo-log");
        ensure_private_dir(&dir).unwrap();
        let mode = std::fs::metadata(&dir).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700);
    }
}

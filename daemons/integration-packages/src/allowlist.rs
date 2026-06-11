//! The user-config allowlist: where an adapter's source paths may live.
//!
//! A Settings adapter is UNTRUSTED community data interpreted by the privileged
//! Settings app (integration-packages-plan.md Decided 3). Its declared source
//! paths are therefore confined here to the user's OWN config directories
//! (`~/.config`, `~/.mozilla`, `~/.local`, `~/.var`); a system path or anything
//! outside this set is refused, so an adapter can never name a file outside the
//! user's config to read or write.
//!
//! This is the DECLARED-PATH gate (lexical): it expands `~`, refuses a `..`
//! traversal, and requires the path to sit under an allowlist root. It is the
//! first of two layers; the second is the ACCESS-TIME cap-std confinement (when a
//! glob is resolved and a file opened, the open is relative to a cap-std `Dir`
//! capability rooted at the allowlist dir, which the kernel refuses to let a
//! symlink escape). A symlink UNDER the allowlist that points outside is caught by
//! that second layer, not this one; together they bound the adapter to user config.

use std::path::{Component, Path, PathBuf};

/// The user-owned config subdirectories (relative to `$HOME`) an adapter source
/// may live under. System paths are never reachable through an adapter.
pub const ALLOWED_SUBDIRS: &[&str] = &[".config", ".mozilla", ".local", ".var"];

/// Why a source path was refused. Fail-closed: an out-of-allowlist or traversal
/// path is an error, never silently clamped.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AllowlistError {
    /// The source path was empty.
    #[error("source path is empty")]
    Empty,
    /// The source path carried a NUL (a path that reaches a syscall must not).
    #[error("source path carries a NUL")]
    Nul,
    /// The path was not absolute after `~` expansion (a bare relative path has no
    /// anchored allowlist root).
    #[error("source path is not absolute or ~-rooted: {0}")]
    NotRooted(String),
    /// The path carried a `..` component (a lexical traversal escape).
    #[error("source path has a `..` traversal component")]
    Traversal,
    /// The path resolved outside every user-config allowlist root.
    #[error("source path is outside the user-config allowlist: {0}")]
    OutsideAllowlist(String),
}

/// Resolve a raw adapter source path against the user-config allowlist under
/// `home`, returning the `~`-expanded absolute path on success.
///
/// - `~` / `~/...` expands to `home`.
/// - The result must be absolute, carry no `..` component, and no NUL.
/// - It must sit under `home/<sub>` for one of [`ALLOWED_SUBDIRS`].
///
/// A glob (`*`) in the path is fine: it is just a path component for this gate
/// (the allowlist is about the rooted prefix, not the glob match), and the actual
/// file access at glob-resolve time is additionally cap-std-confined.
pub fn resolve_under_allowlist(raw: &str, home: &Path) -> Result<PathBuf, AllowlistError> {
    if raw.is_empty() {
        return Err(AllowlistError::Empty);
    }
    if raw.contains('\0') {
        return Err(AllowlistError::Nul);
    }

    let expanded: PathBuf = if raw == "~" {
        home.to_path_buf()
    } else if let Some(rest) = raw.strip_prefix("~/") {
        home.join(rest)
    } else {
        PathBuf::from(raw)
    };

    if !expanded.is_absolute() {
        return Err(AllowlistError::NotRooted(raw.to_string()));
    }
    // Reject ANY parent-dir component: a lexical `..` can escape the allowlist root
    // even when the string prefix looks contained, so refuse it outright rather
    // than try to normalize it away.
    if expanded
        .components()
        .any(|c| matches!(c, Component::ParentDir))
    {
        return Err(AllowlistError::Traversal);
    }
    // `starts_with` is component-wise, so `~/.config-evil` does not match
    // `~/.config` (no string-prefix bypass).
    let under_allowed = ALLOWED_SUBDIRS
        .iter()
        .any(|sub| expanded.starts_with(home.join(sub)));
    if !under_allowed {
        return Err(AllowlistError::OutsideAllowlist(expanded.display().to_string()));
    }
    Ok(expanded)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home() -> PathBuf {
        PathBuf::from("/home/u")
    }

    #[test]
    fn accepts_a_tilde_rooted_user_config_path() {
        assert_eq!(
            resolve_under_allowlist("~/.mozilla/firefox/*/prefs.js", &home()).unwrap(),
            PathBuf::from("/home/u/.mozilla/firefox/*/prefs.js")
        );
        assert_eq!(
            resolve_under_allowlist("~/.config/app/config.toml", &home()).unwrap(),
            PathBuf::from("/home/u/.config/app/config.toml")
        );
        // An absolute path already under the allowlist is fine too.
        assert!(resolve_under_allowlist("/home/u/.local/share/x.conf", &home()).is_ok());
    }

    #[test]
    fn refuses_system_paths_and_outside_home() {
        assert!(matches!(
            resolve_under_allowlist("/etc/passwd", &home()),
            Err(AllowlistError::OutsideAllowlist(_))
        ));
        assert!(matches!(
            resolve_under_allowlist("/home/u/.ssh/id_ed25519", &home()),
            Err(AllowlistError::OutsideAllowlist(_))
        ));
        // The home itself, not under an allowed subdir, is refused.
        assert!(matches!(
            resolve_under_allowlist("~/.bashrc", &home()),
            Err(AllowlistError::OutsideAllowlist(_))
        ));
    }

    #[test]
    fn refuses_traversal_even_when_the_prefix_looks_contained() {
        assert!(matches!(
            resolve_under_allowlist("~/.config/../.ssh/key", &home()),
            Err(AllowlistError::Traversal)
        ));
        assert!(matches!(
            resolve_under_allowlist("/home/u/.config/../../etc/shadow", &home()),
            Err(AllowlistError::Traversal)
        ));
    }

    #[test]
    fn refuses_a_string_prefix_sibling_of_an_allowed_root() {
        // `.config-evil` shares the string prefix `.config` but is a different
        // component, so it must be refused (no string-prefix bypass).
        assert!(matches!(
            resolve_under_allowlist("~/.config-evil/x", &home()),
            Err(AllowlistError::OutsideAllowlist(_))
        ));
    }

    #[test]
    fn refuses_empty_relative_and_nul() {
        assert_eq!(resolve_under_allowlist("", &home()), Err(AllowlistError::Empty));
        assert!(matches!(
            resolve_under_allowlist("relative/path.conf", &home()),
            Err(AllowlistError::NotRooted(_))
        ));
        assert_eq!(
            resolve_under_allowlist("~/.config/a\0b", &home()),
            Err(AllowlistError::Nul)
        );
    }
}

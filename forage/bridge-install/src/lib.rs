//! Install the two halves of a foreign-app bridge from a fetched recipe source
//! (`foreign-app-bridges.md` §4). A bridge recipe's `[install]` manifest names an
//! Arlen side (the `entities.toml` schema + `bridge.toml` mapping that register the
//! bridge with the ingest daemon) and a foreign side (the plugin files dropped into
//! the foreign app's own config dir). This crate does the path-safe copy of both
//! out of the fetched, verified recipe source into their destinations.
//!
//! The security floor: every declared file is confined to the source tree on read
//! (declared paths are re-validated safe-relative and symlinks are refused, so a
//! recipe cannot exfiltrate `/etc/passwd` by naming a symlink) and to its
//! destination on write (safe-relative + destination-rooted, so it cannot escape
//! the bridge dir or the foreign plugin dir). A pre-flight pass validates every
//! path and source file before anything is written, so a bad manifest fails without
//! leaving a partial install; whatever a late I/O error does write is reported for
//! the caller to roll back.

use arlen_forage_recipe::Install;
use std::path::{Component, Path, PathBuf};

/// The canonical Arlen-side directory for an installed bridge's registration files,
/// `$XDG_DATA_HOME/arlen/bridges/<recipe_id>/` (else `$HOME/.local/share/...`). One
/// bridge-ingest daemon instance is pointed at the `bridge.toml` under it. Returns
/// `None` when neither env var is set (the caller fails closed rather than guessing).
pub fn arlen_bridge_dir(recipe_id: &str) -> Option<PathBuf> {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local").join("share")))?;
    Some(base.join("arlen").join("bridges").join(recipe_id))
}

/// What a bridge install placed, so the caller can roll it back (on a later step's
/// failure) and record it for the revocable grant.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InstalledBridge {
    /// Absolute paths written under the Arlen bridge dir.
    pub arlen_files: Vec<PathBuf>,
    /// Absolute paths written under the foreign app's plugin dir.
    pub foreign_files: Vec<PathBuf>,
}

/// A bridge install failure. Every variant leaves the destinations unchanged (a
/// pre-flight failure) or names what was written (a write failure) so the caller
/// can roll back.
#[derive(Debug, thiserror::Error)]
pub enum BridgeInstallError {
    /// A declared file is not a safe relative path (absolute or `..`).
    #[error("unsafe install path: {0}")]
    UnsafePath(String),
    /// A declared source file is missing or is not a regular file (e.g. a symlink,
    /// which is refused so a recipe cannot read outside its own source tree).
    #[error("source file is missing or not a regular file: {0}")]
    BadSource(String),
    /// An I/O error during the copy; `wrote` is what had already been placed.
    #[error("install I/O error at {path}: {source}")]
    Io {
        /// The path being written when the error occurred.
        path: String,
        /// The underlying error.
        source: std::io::Error,
        /// What was written before the error, for rollback.
        wrote: InstalledBridge,
    },
}

/// Whether `p` is a safe relative path: non-empty, not absolute, no `..` component.
fn is_safe_relative(p: &Path) -> bool {
    let mut any = false;
    for c in p.components() {
        any = true;
        match c {
            Component::Normal(_) | Component::CurDir => {}
            _ => return false,
        }
    }
    any
}

/// Verify each declared file is safe-relative and its source is a real regular file
/// (not a symlink), before any write happens.
fn preflight(source_dir: &Path, files: &[PathBuf]) -> Result<(), BridgeInstallError> {
    for f in files {
        if !is_safe_relative(f) {
            return Err(BridgeInstallError::UnsafePath(f.display().to_string()));
        }
        let src = source_dir.join(f);
        // symlink_metadata does NOT follow the final symlink, so a symlink source
        // reports its own (symlink) type and is refused by the is_file() check.
        match std::fs::symlink_metadata(&src) {
            Ok(m) if m.file_type().is_file() => {}
            _ => return Err(BridgeInstallError::BadSource(src.display().to_string())),
        }
    }
    Ok(())
}

/// Copy `files` from `source_dir` to `dest_dir`, recording each written path into
/// `into`. Paths were pre-flighted, so a failure here is a real I/O error.
fn copy_into(
    source_dir: &Path,
    dest_dir: &Path,
    files: &[PathBuf],
    record: &mut dyn FnMut(&mut InstalledBridge, PathBuf),
    installed: &mut InstalledBridge,
) -> Result<(), (String, std::io::Error)> {
    for f in files {
        let src = source_dir.join(f);
        let dst = dest_dir.join(f);
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent).map_err(|e| (parent.display().to_string(), e))?;
        }
        let bytes = std::fs::read(&src).map_err(|e| (src.display().to_string(), e))?;
        std::fs::write(&dst, &bytes).map_err(|e| (dst.display().to_string(), e))?;
        record(installed, dst);
    }
    Ok(())
}

/// Install both halves of a bridge from a fetched recipe source.
///
/// `source_dir` is the verified recipe checkout, `install` its `[install]` manifest,
/// `arlen_bridge_dir` the destination for the Arlen-side registration files, and
/// `foreign_dest` the ALREADY-RESOLVED foreign plugin directory (template
/// resolution of `foreign_side.into`, e.g. `$VAULT`, is the caller's job; this
/// function receives a concrete path). Returns what was placed.
pub fn install_bridge_halves(
    source_dir: &Path,
    install: &Install,
    arlen_bridge_dir: &Path,
    foreign_dest: &Path,
) -> Result<InstalledBridge, BridgeInstallError> {
    // Pre-flight BOTH halves before writing anything, so a bad manifest cannot leave
    // a half-installed bridge.
    preflight(source_dir, &install.arlen_side)?;
    preflight(source_dir, &install.foreign_side.files)?;

    let mut installed = InstalledBridge::default();
    if let Err((path, source)) = copy_into(
        source_dir,
        arlen_bridge_dir,
        &install.arlen_side,
        &mut |i, p| i.arlen_files.push(p),
        &mut installed,
    ) {
        return Err(BridgeInstallError::Io { path, source, wrote: installed });
    }
    if let Err((path, source)) = copy_into(
        source_dir,
        foreign_dest,
        &install.foreign_side.files,
        &mut |i, p| i.foreign_files.push(p),
        &mut installed,
    ) {
        return Err(BridgeInstallError::Io { path, source, wrote: installed });
    }
    Ok(installed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use arlen_forage_recipe::{ForeignSide, Install};
    use std::fs;

    fn install_manifest(arlen: &[&str], foreign: &[&str]) -> Install {
        Install {
            arlen_side: arlen.iter().map(PathBuf::from).collect(),
            foreign_side: ForeignSide {
                into: "$VAULT/.obsidian/plugins/x/".to_string(),
                files: foreign.iter().map(PathBuf::from).collect(),
            },
        }
    }

    fn seed(dir: &Path, rel: &str, contents: &str) {
        let p = dir.join(rel);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(p, contents).unwrap();
    }

    #[test]
    fn installs_both_halves() {
        let src = tempfile::tempdir().unwrap();
        seed(src.path(), "entities.toml", "e");
        seed(src.path(), "bridge.toml", "b");
        seed(src.path(), "main.js", "js");
        seed(src.path(), "manifest.json", "{}");

        let arlen = tempfile::tempdir().unwrap();
        let foreign = tempfile::tempdir().unwrap();
        let manifest = install_manifest(&["entities.toml", "bridge.toml"], &["main.js", "manifest.json"]);

        let got = install_bridge_halves(src.path(), &manifest, arlen.path(), foreign.path()).unwrap();
        assert_eq!(got.arlen_files.len(), 2);
        assert_eq!(got.foreign_files.len(), 2);
        assert_eq!(fs::read_to_string(arlen.path().join("bridge.toml")).unwrap(), "b");
        assert_eq!(fs::read_to_string(foreign.path().join("main.js")).unwrap(), "js");
    }

    #[test]
    fn a_missing_source_file_fails_before_writing() {
        let src = tempfile::tempdir().unwrap();
        seed(src.path(), "entities.toml", "e");
        // bridge.toml is NOT seeded.
        let arlen = tempfile::tempdir().unwrap();
        let foreign = tempfile::tempdir().unwrap();
        let manifest = install_manifest(&["entities.toml", "bridge.toml"], &["main.js"]);

        let err = install_bridge_halves(src.path(), &manifest, arlen.path(), foreign.path()).unwrap_err();
        assert!(matches!(err, BridgeInstallError::BadSource(_)));
        // Pre-flight failed, so nothing was written (not even the present entities.toml).
        assert!(!arlen.path().join("entities.toml").exists());
    }

    #[test]
    fn a_symlink_source_is_refused() {
        let src = tempfile::tempdir().unwrap();
        // A recipe naming a symlink that points outside its own tree must not be
        // able to copy the target's contents out.
        let secret = src.path().join("secret.txt");
        fs::write(&secret, "top secret").unwrap();
        std::os::unix::fs::symlink(&secret, src.path().join("entities.toml")).unwrap();
        seed(src.path(), "bridge.toml", "b");
        seed(src.path(), "main.js", "js");

        let arlen = tempfile::tempdir().unwrap();
        let foreign = tempfile::tempdir().unwrap();
        let manifest = install_manifest(&["entities.toml", "bridge.toml"], &["main.js"]);

        let err = install_bridge_halves(src.path(), &manifest, arlen.path(), foreign.path()).unwrap_err();
        assert!(matches!(err, BridgeInstallError::BadSource(_)));
    }

    #[test]
    fn a_nested_relative_file_is_placed_under_its_dir() {
        let src = tempfile::tempdir().unwrap();
        seed(src.path(), "dist/main.js", "nested");
        let arlen = tempfile::tempdir().unwrap();
        let foreign = tempfile::tempdir().unwrap();
        let manifest = install_manifest(&[], &["dist/main.js"]);

        install_bridge_halves(src.path(), &manifest, arlen.path(), foreign.path()).unwrap();
        assert_eq!(fs::read_to_string(foreign.path().join("dist/main.js")).unwrap(), "nested");
    }

    #[test]
    fn the_arlen_bridge_dir_is_recipe_scoped_under_xdg() {
        // With XDG_DATA_HOME set, the dir is <xdg>/arlen/bridges/<id>.
        std::env::set_var("XDG_DATA_HOME", "/tmp/xdg-test-data");
        let dir = arlen_bridge_dir("md.obsidian.bridge").unwrap();
        assert_eq!(dir, PathBuf::from("/tmp/xdg-test-data/arlen/bridges/md.obsidian.bridge"));
        std::env::remove_var("XDG_DATA_HOME");
    }
}

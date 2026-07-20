//! Locate and load the active wallpaper manifest for the renderer client.
//!
//! Precedence: the user's `$XDG_CONFIG_HOME/arlen/wallpaper.toml` (else
//! `$HOME/.config/arlen/wallpaper.toml`) if present and valid, otherwise the
//! system default [`SYSTEM_MANIFEST_PATH`]. When neither is present or valid the
//! renderer paints nothing and the compositor's flat clear colour shows - a
//! missing or malformed wallpaper config must never crash the background.

use crate::manifest::{ManifestError, WallpaperManifest};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// The distro-provided default wallpaper manifest, used when the user has not set
/// one.
pub const SYSTEM_MANIFEST_PATH: &str = "/usr/share/arlen/wallpaper/default.toml";

/// Why a manifest could not be loaded from a path.
#[derive(Debug, Error)]
pub enum LoadError {
    /// The file could not be read.
    #[error("could not read wallpaper manifest: {0}")]
    Io(#[from] std::io::Error),
    /// The file parsed but was not a valid manifest.
    #[error(transparent)]
    Manifest(#[from] ManifestError),
}

/// The user manifest path from the given env values (pure, so it is unit-tested
/// without mutating process env): `$XDG_CONFIG_HOME/arlen/wallpaper.toml`, else
/// `$HOME/.config/arlen/wallpaper.toml`, else `None`.
pub fn user_manifest_path_from(xdg: Option<&OsStr>, home: Option<&OsStr>) -> Option<PathBuf> {
    if let Some(x) = xdg.filter(|x| !x.is_empty()) {
        return Some(Path::new(x).join("arlen").join("wallpaper.toml"));
    }
    home.filter(|h| !h.is_empty())
        .map(|h| Path::new(h).join(".config").join("arlen").join("wallpaper.toml"))
}

/// The user manifest path from the process environment.
pub fn user_manifest_path() -> Option<PathBuf> {
    user_manifest_path_from(
        std::env::var_os("XDG_CONFIG_HOME").as_deref(),
        std::env::var_os("HOME").as_deref(),
    )
}

/// Read and validate a manifest from `path`.
pub fn load_manifest(path: &Path) -> Result<WallpaperManifest, LoadError> {
    let text = std::fs::read_to_string(path)?;
    Ok(WallpaperManifest::parse(&text)?)
}

/// The active manifest: the user's if present and valid, else the system default,
/// else `None` (the renderer then paints nothing). A malformed USER manifest does
/// NOT silently fall through to the system default here - that decision is the
/// caller's; this returns the first that loads, user first.
pub fn active_manifest() -> Option<WallpaperManifest> {
    if let Some(p) = user_manifest_path() {
        if let Ok(m) = load_manifest(&p) {
            return Some(m);
        }
    }
    load_manifest(Path::new(SYSTEM_MANIFEST_PATH)).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_prefers_xdg_then_home_then_none() {
        assert_eq!(
            user_manifest_path_from(Some(OsStr::new("/x/cfg")), Some(OsStr::new("/home/u"))),
            Some(PathBuf::from("/x/cfg/arlen/wallpaper.toml"))
        );
        assert_eq!(
            user_manifest_path_from(None, Some(OsStr::new("/home/u"))),
            Some(PathBuf::from("/home/u/.config/arlen/wallpaper.toml"))
        );
        // Empty XDG falls through to HOME (an unset-but-empty env var is not a path).
        assert_eq!(
            user_manifest_path_from(Some(OsStr::new("")), Some(OsStr::new("/home/u"))),
            Some(PathBuf::from("/home/u/.config/arlen/wallpaper.toml"))
        );
        assert_eq!(user_manifest_path_from(None, None), None);
    }

    #[test]
    fn loads_a_valid_manifest_and_errors_on_a_bad_one() {
        let dir = tempfile::tempdir().unwrap();
        let good = dir.path().join("wallpaper.toml");
        std::fs::write(
            &good,
            "kind = \"image\"\n[default]\nasset = \"/usr/share/backgrounds/a.png\"\nscale = \"fill\"\n",
        )
        .unwrap();
        let m = load_manifest(&good).unwrap();
        assert_eq!(m.default.asset, "/usr/share/backgrounds/a.png");

        let bad = dir.path().join("bad.toml");
        std::fs::write(&bad, "kind = \"image\"\n").unwrap(); // no [default] source
        assert!(load_manifest(&bad).is_err());

        // Missing file -> Io error, not a panic.
        assert!(matches!(load_manifest(&dir.path().join("nope.toml")), Err(LoadError::Io(_))));
    }
}

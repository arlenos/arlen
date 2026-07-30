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

/// The active manifest: the user's if it loads, else the distro default, else
/// `None` (the renderer then paints nothing).
///
/// `override_path` is the `ARLEN_WALLPAPER_MANIFEST` test hook. When set it is
/// the only path tried, so a test cannot accidentally pick up a real manifest
/// from the machine it runs on.
///
/// A user manifest that exists but does not load falls through to the default
/// rather than leaving the desktop bare. The fault is reported through
/// `on_user_error` rather than swallowed, because the two halves want different
/// answers: the user should hear that their file is broken, and should still get
/// a background while they fix it. Silently falling through would hide the fault;
/// refusing to fall through would punish it with an empty screen.
pub fn active_manifest(
    override_path: Option<PathBuf>,
    on_user_error: impl FnOnce(&Path, LoadError),
) -> Option<WallpaperManifest> {
    active_manifest_from(
        override_path,
        user_manifest_path(),
        Path::new(SYSTEM_MANIFEST_PATH),
        on_user_error,
    )
}

/// [`active_manifest`] over explicit paths, so the precedence is unit-tested
/// against fixtures rather than against whatever the running machine happens to
/// have configured. Same split as [`user_manifest_path_from`].
pub fn active_manifest_from(
    override_path: Option<PathBuf>,
    user_path: Option<PathBuf>,
    system_path: &Path,
    on_user_error: impl FnOnce(&Path, LoadError),
) -> Option<WallpaperManifest> {
    if let Some(p) = override_path {
        return load_manifest(&p).ok();
    }
    if let Some(p) = user_path {
        match load_manifest(&p) {
            Ok(m) => return Some(m),
            // Absent is the ordinary case for a user who has not set one, and is
            // not worth reporting; unreadable or malformed is.
            Err(LoadError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => on_user_error(&p, e),
        }
    }
    load_manifest(system_path).ok()
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

#[cfg(test)]
mod resolver_tests {
    use super::*;

    fn write(dir: &Path, name: &str, text: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, text).unwrap();
        p
    }

    const VALID: &str = r#"
kind = "image"
[default]
asset = "/usr/share/backgrounds/arlen.png"
scale = "fill"
"#;

    #[test]
    fn an_override_is_the_only_path_tried() {
        // So a test never picks up a manifest from the machine it runs on.
        let dir = tempfile::tempdir().unwrap();
        let p = write(dir.path(), "w.toml", VALID);
        assert!(active_manifest(Some(p), |_, _| panic!("not a user path")).is_some());
        assert!(active_manifest(Some(dir.path().join("absent.toml")), |_, _| {}).is_none());
    }

    #[test]
    fn a_broken_user_manifest_is_reported_and_still_falls_back() {
        // Both halves matter: the user hears their file is broken, and does not
        // get an empty desktop while they fix it.
        let dir = tempfile::tempdir().unwrap();
        let user = write(dir.path(), "user.toml", "this is not a manifest {{{");
        let system = write(dir.path(), "system.toml", VALID);
        let mut reported = None;
        let got = active_manifest_from(None, Some(user.clone()), &system, |p, _| {
            reported = Some(p.to_path_buf())
        });
        assert_eq!(reported.as_deref(), Some(user.as_path()), "the fault is reported");
        assert!(got.is_some(), "and the default still paints something");
    }

    #[test]
    fn a_valid_user_manifest_wins_over_the_default() {
        let dir = tempfile::tempdir().unwrap();
        let user = write(dir.path(), "user.toml", VALID);
        let system = write(dir.path(), "system.toml", "broken {{{");
        assert!(
            active_manifest_from(None, Some(user), &system, |_, _| panic!("no fault")).is_some()
        );
    }

    #[test]
    fn with_no_user_manifest_the_distro_default_is_used() {
        // The case that was unreachable: the resolver existed and nothing called
        // it, so a machine carrying only the shipped default rendered nothing.
        let dir = tempfile::tempdir().unwrap();
        let system = write(dir.path(), "system.toml", VALID);
        let absent = dir.path().join("nope.toml");
        assert!(
            active_manifest_from(None, Some(absent), &system, |_, _| panic!("absent is no fault"))
                .is_some()
        );
    }
}

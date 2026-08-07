// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

//! The wallpaper catalogue (WP-R1): which wallpapers this machine offers.
//!
//! The daemon's manifest answers "what IS the wallpaper" - one asset, per-monitor
//! overrides, time variants. It has no notion of a choosable set, which is what a
//! picker needs, so the catalogue is an enumeration of two directories rather
//! than a read of the manifest.
//!
//! **User shadows system, by stem.** `~/.local/share/arlen/wallpapers/sunset.jpg`
//! and `/usr/share/arlen/wallpapers/sunset.png` are both `sunset`, and the user's
//! wins - the same precedence as themes, and as `PATH`.
//!
//! **An id is a stem, never a path.** It comes back from the surface as the thing
//! to apply, so it is validated on the way out and on the way back in: no
//! separator, no `..`, nothing that could be spelled to escape the two roots. A
//! picker that can name any path is a file-read oracle wearing a grid.

use serde::Serialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Shipped wallpapers.
const SYSTEM_DIR: &str = "/usr/share/arlen/wallpapers";

/// The image types the picker offers. An allowlist: the grid renders these as
/// images, so a type it cannot name is one it should not list.
const EXTENSIONS: [&str; 4] = ["png", "jpg", "jpeg", "webp"];

/// One choosable wallpaper, in the shape the picker renders.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct WallpaperEntry {
    /// The file stem, and the handle `set_wallpaper` takes back.
    pub id: String,
    /// What the tile says. The stem until a manifest gives a better name.
    pub name: String,
    /// The absolute asset path. The surface turns it into an asset-protocol URL;
    /// the scope in `tauri.conf.json` is what keeps that from reaching further
    /// than these two directories.
    pub thumb: String,
    /// `static` today; `live` (video, shader) is WP-R2.
    pub kind: String,
}

/// The per-user wallpaper directory, or `None` without a home.
fn user_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_DATA_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .filter(|v| !v.is_empty())
                .map(|h| PathBuf::from(h).join(".local").join("share"))
        })?;
    Some(base.join("arlen").join("wallpapers"))
}

/// Whether `id` is a plain stem this will accept back from a surface.
///
/// Deliberately stricter than "has no slash": a stem is what the enumeration
/// produced, so anything that is not one is a caller inventing an id.
pub fn is_valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id != "."
        && id != ".."
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        && !id.contains("..")
}

/// The entries in one directory, keyed by stem. Unreadable directory yields an
/// empty map: a machine without a shipped set still offers the user's.
fn entries_in(dir: &Path) -> BTreeMap<String, WallpaperEntry> {
    let mut out = BTreeMap::new();
    let Ok(read) = std::fs::read_dir(dir) else {
        return out;
    };
    for e in read.flatten() {
        let path = e.path();
        if !path.is_file() {
            continue;
        }
        let ext = path
            .extension()
            .and_then(|x| x.to_str())
            .map(str::to_ascii_lowercase);
        if !ext.is_some_and(|x| EXTENSIONS.contains(&x.as_str())) {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        if !is_valid_id(stem) {
            continue;
        }
        out.insert(
            stem.to_string(),
            WallpaperEntry {
                id: stem.to_string(),
                name: stem.to_string(),
                thumb: path.to_string_lossy().into_owned(),
                kind: "static".to_string(),
            },
        );
    }
    out
}

/// The catalogue over explicit roots, so the shadowing is tested against
/// fixtures rather than against whatever the running machine has installed.
pub fn catalogue_from(system: &Path, user: Option<&Path>) -> Vec<WallpaperEntry> {
    let mut merged = entries_in(system);
    if let Some(u) = user {
        // The user's copy replaces the system's under the same stem.
        merged.extend(entries_in(u));
    }
    merged.into_values().collect()
}

/// Every wallpaper this machine offers, system then user, user winning ties.
#[tauri::command]
pub async fn list_wallpapers() -> Result<Vec<WallpaperEntry>, String> {
    let user = user_dir();
    Ok(catalogue_from(Path::new(SYSTEM_DIR), user.as_deref()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(dir: &Path, name: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join(name), b"x").unwrap();
    }

    #[test]
    fn the_users_copy_shadows_the_shipped_one_under_the_same_stem() {
        let tmp = tempfile::tempdir().unwrap();
        let sys = tmp.path().join("sys");
        let usr = tmp.path().join("usr");
        touch(&sys, "sunset.png");
        touch(&sys, "only-system.png");
        touch(&usr, "sunset.jpg");

        let c = catalogue_from(&sys, Some(&usr));
        let ids: Vec<&str> = c.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, vec!["only-system", "sunset"], "one entry per stem");
        let sunset = c.iter().find(|e| e.id == "sunset").unwrap();
        assert!(
            sunset.thumb.ends_with("usr/sunset.jpg"),
            "the user's file wins, got {}",
            sunset.thumb
        );
    }

    #[test]
    fn only_listed_image_types_are_offered() {
        let tmp = tempfile::tempdir().unwrap();
        let sys = tmp.path().join("sys");
        for f in ["a.png", "b.JPG", "c.webp", "d.mp4", "e.toml", "f"] {
            touch(&sys, f);
        }
        let ids: Vec<String> = catalogue_from(&sys, None).into_iter().map(|e| e.id).collect();
        assert_eq!(ids, vec!["a", "b", "c"], "the video, the config and the extensionless file are not wallpapers");
    }

    #[test]
    fn an_id_is_a_stem_and_nothing_that_could_leave_the_roots() {
        assert!(is_valid_id("sunset"));
        assert!(is_valid_id("blue-ridge_2"));
        for bad in ["", ".", "..", "../etc/passwd", "a/b", "a\\b", "a..b", "\u{0}"] {
            assert!(!is_valid_id(bad), "{bad:?} must be refused");
        }
        assert!(!is_valid_id(&"x".repeat(129)), "an unbounded id is not a stem");
    }

    #[test]
    fn a_missing_directory_is_an_empty_set_not_an_error() {
        // A machine with no shipped wallpapers still offers the user's, and one
        // with neither shows an empty picker rather than a failure.
        let tmp = tempfile::tempdir().unwrap();
        assert!(catalogue_from(&tmp.path().join("absent"), None).is_empty());
    }
}

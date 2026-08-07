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

/// Resolve an id to the file it names, user root first.
///
/// Only ever returns a path INSIDE one of the two roots: the id is validated as
/// a stem and joined to a root, so there is no input that reaches elsewhere.
pub fn resolve_from(id: &str, system: &Path, user: Option<&Path>) -> Option<PathBuf> {
    if !is_valid_id(id) {
        return None;
    }
    let roots = user.into_iter().chain(std::iter::once(system));
    for root in roots {
        for ext in EXTENSIONS {
            let p = root.join(format!("{id}.{ext}"));
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

/// The daemon's scale for the picker's fit mode.
///
/// The daemon models two: Fill and Zoom. The picker's vocabulary carries three
/// more, and `stretch` in particular the daemon refuses on purpose - it breaks
/// aspect. Mapping them onto Fill would silently overrule that refusal and show
/// the user something other than what they picked, so an unmodelled fit is an
/// error naming itself.
fn daemon_scale(scale: &str) -> Result<&'static str, String> {
    match scale {
        "fill" => Ok("fill"),
        "fit" => Ok("zoom"),
        other => Err(format!(
            "the wallpaper daemon renders fill or fit only; it does not model {other}"
        )),
    }
}

/// The manifest text for a static image wallpaper.
///
/// Written by hand because `WallpaperManifest` derives `Deserialize` only - the
/// daemon reads manifests, nothing wrote one until now. The daemon validates on
/// read, so a shape error surfaces there rather than being assumed here.
fn manifest_toml(asset: &Path, scale: &str) -> String {
    format!(
        "# Written by Settings. The wallpaper daemon reads this at startup.\n\
         kind = \"image\"\n\n[default]\nasset = \"{}\"\nscale = \"{scale}\"\n",
        asset.display()
    )
}

/// Make `id` the wallpaper.
///
/// **Takes effect when the daemon next starts.** `arlen-wallpaperd` loads the
/// manifest once and then dispatches Wayland events; there is no watch and no
/// reload signal. Saying so is better than a picker that looks applied and is
/// not - the reload is a daemon change, not something this can paper over.
#[tauri::command]
pub async fn set_wallpaper(id: String, scale: String) -> Result<(), String> {
    let daemon_scale = daemon_scale(&scale)?;
    let user = user_dir();
    let asset = resolve_from(&id, Path::new(SYSTEM_DIR), user.as_deref())
        .ok_or_else(|| format!("no wallpaper named {id}"))?;
    let path = arlen_wallpaper::config::user_manifest_path()
        .ok_or_else(|| "no home directory to write the wallpaper manifest into".to_string())?;
    let parent = path
        .parent()
        .ok_or_else(|| "the wallpaper manifest path has no directory".to_string())?;
    std::fs::create_dir_all(parent).map_err(|e| format!("create {}: {e}", parent.display()))?;
    // Temp-and-rename: a half-written manifest is one the daemon would refuse at
    // next start, which would read as "my wallpaper reset itself".
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, manifest_toml(&asset, daemon_scale))
        .map_err(|e| format!("write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("rename into {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(dir: &Path, name: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join(name), b"x").unwrap();
    }

    #[test]
    fn an_id_resolves_to_the_users_file_before_the_shipped_one() {
        let tmp = tempfile::tempdir().unwrap();
        let sys = tmp.path().join("sys");
        let usr = tmp.path().join("usr");
        touch(&sys, "sunset.png");
        touch(&usr, "sunset.jpg");
        let got = resolve_from("sunset", &sys, Some(&usr)).unwrap();
        assert!(got.ends_with("usr/sunset.jpg"), "got {}", got.display());
        // Falls back to the shipped file when the user has none.
        let only_sys = resolve_from("sunset", &sys, None).unwrap();
        assert!(only_sys.ends_with("sys/sunset.png"));
        assert!(resolve_from("absent", &sys, Some(&usr)).is_none());
    }

    #[test]
    fn a_crafted_id_resolves_to_nothing_at_all() {
        let tmp = tempfile::tempdir().unwrap();
        let sys = tmp.path().join("sys");
        touch(&sys, "ok.png");
        for bad in ["../../etc/passwd", "..", "a/b", ""] {
            assert!(resolve_from(bad, &sys, None).is_none(), "{bad:?} must resolve to nothing");
        }
    }

    #[test]
    fn a_fit_the_daemon_does_not_model_is_an_error_not_a_substitution() {
        assert_eq!(daemon_scale("fill").unwrap(), "fill");
        assert_eq!(daemon_scale("fit").unwrap(), "zoom");
        // The daemon refuses Stretch on purpose. Quietly showing Fill instead
        // would overrule that and show something other than what was picked.
        for other in ["stretch", "center", "tile", ""] {
            assert!(daemon_scale(other).is_err(), "{other:?} must be refused");
        }
    }

    #[test]
    fn the_written_manifest_is_one_the_daemon_accepts() {
        // Round-trips through the daemon's own parser rather than asserting the
        // text: the shape that matters is the one it will read back.
        let text = manifest_toml(Path::new("/usr/share/arlen/wallpapers/sunset.png"), "zoom");
        let m = arlen_wallpaper::manifest::WallpaperManifest::parse(&text)
            .expect("the daemon parses what Settings writes");
        assert_eq!(m.default.asset, "/usr/share/arlen/wallpapers/sunset.png");
        m.validate().expect("and validates it");
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

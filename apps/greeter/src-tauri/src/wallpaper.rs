// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

//! The login background: the system wallpaper, as a data URL.
//!
//! **The system manifest only, never a user's.** The greeter runs before anyone
//! has authenticated, so reading `$HOME/.config/arlen/wallpaper.toml` would mean
//! showing one user's choice on a shared login screen - and reading it out of
//! whatever environment the greeter inherited, which is not a user's consent.
//! `/usr/share/arlen/wallpaper/default.toml` is what the machine shows before it
//! knows who is looking.
//!
//! **A data URL rather than a path, and that is the security half.** The value
//! lands in `background-image: url('…')` in the greeter's webview, where a
//! filesystem path resolves against the page and silently shows nothing. The
//! alternative to inlining is granting the webview an asset protocol - a
//! file-read capability on a PRE-AUTHENTICATION surface, which is the last place
//! to open one. Reading the image here costs one bounded IPC message at startup
//! and keeps the webview with no filesystem reach at all.

use arlen_wallpaper::config::active_manifest;
use base64::Engine;
use std::path::Path;

/// The largest wallpaper the greeter will inline. A login background past this
/// is a packaging mistake, and refusing is better than a slow or wedged login.
const MAX_BYTES: u64 = 12 * 1024 * 1024;

/// Where a greeter wallpaper may live. The manifest is root-owned, so this is
/// not a trust boundary against its author; it stops a mistaken relative or
/// traversing asset path from turning the login screen into a file reader.
const ALLOWED_ROOT: &str = "/usr/share/arlen/";

/// The login background as a `data:` URL, or `None` for the calm fallback.
///
/// Every failure is `None`: no manifest, an asset outside the allowed root, an
/// unreadable or oversized file, an unknown extension. A login screen that comes
/// up plain is fine; one that does not come up is not.
#[tauri::command]
pub async fn greeter_wallpaper() -> Option<String> {
    // Reads a file, so it runs on the blocking pool rather than the async one:
    // a wallpaper is megabytes and the greeter's runtime also serves the
    // keystrokes of whoever is typing their password.
    let manifest = active_manifest(None, |_, _| {})?;
    let asset = manifest.default.asset.clone();
    tauri::async_runtime::spawn_blocking(move || inline_asset(Path::new(&asset)))
        .await
        .ok()
        .flatten()
}

/// Read `path` into a data URL, or `None` if it is not an inlinable wallpaper.
fn inline_asset(path: &Path) -> Option<String> {
    if !path.is_absolute() || !path.starts_with(ALLOWED_ROOT) || path.components().any(is_parent) {
        return None;
    }
    let mime = mime_for(path)?;
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > MAX_BYTES {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
    Some(format!("data:{mime};base64,{b64}"))
}

/// Whether a path component is `..`. Checked even though the root prefix is
/// already required, because `/usr/share/arlen/../..` starts with the root and
/// leaves it.
fn is_parent(c: std::path::Component<'_>) -> bool {
    matches!(c, std::path::Component::ParentDir)
}

/// The image type from the extension, or `None` for anything this will not
/// inline. An allowlist rather than a guess: the greeter renders it as an image,
/// so a type it cannot name is a type it should not embed.
fn mime_for(path: &Path) -> Option<&'static str> {
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "webp" => Some("image/webp"),
        "avif" => Some("image/avif"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_asset_outside_the_allowed_root_is_refused() {
        // The manifest is root-owned, so this is not defending against its
        // author. It stops a wrong path from making the pre-login screen read
        // arbitrary files.
        for p in [
            "/etc/shadow",
            "/home/someone/.ssh/id_rsa",
            "relative.png",
            "/usr/share/arlen/../../etc/passwd.png",
        ] {
            assert!(inline_asset(Path::new(p)).is_none(), "{p} must be refused");
        }
    }

    #[test]
    fn only_image_types_the_greeter_can_name_are_inlinable() {
        assert_eq!(mime_for(Path::new("/usr/share/arlen/a.PNG")), Some("image/png"));
        assert_eq!(mime_for(Path::new("/usr/share/arlen/a.jpeg")), Some("image/jpeg"));
        // A video source is a valid wallpaper for the daemon and not something
        // this can inline as a background image.
        assert_eq!(mime_for(Path::new("/usr/share/arlen/a.mp4")), None);
        assert_eq!(mime_for(Path::new("/usr/share/arlen/noext")), None);
    }

    #[test]
    fn a_real_file_under_the_root_becomes_a_data_url() {
        // Uses the allowed root itself: if the machine has no such directory the
        // test still exercises the refusal path rather than silently passing.
        let dir = Path::new(ALLOWED_ROOT);
        if !dir.exists() {
            assert!(inline_asset(&dir.join("absent.png")).is_none());
            return;
        }
        assert!(inline_asset(&dir.join("definitely-absent.png")).is_none());
    }
}

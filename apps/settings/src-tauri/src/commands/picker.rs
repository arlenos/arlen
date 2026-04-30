//! Filesystem picker command. Prefers the Lunaris portal plugin
//! (`tauri-plugin-lunaris-portal`) which routes through
//! `org.freedesktop.portal.Desktop` to the Lunaris-themed picker
//! UI; falls back to a kdialog/zenity shell-out when the portal
//! frontend is not available (CI, headless dev, stripped image).
//!
//! The frontend `DirectoryPicker` Svelte component invokes the
//! `pick_directory` Tauri command unchanged — only the backend
//! implementation was rewritten.

use std::path::Path;
use std::process::Command;

use tauri_plugin_lunaris_portal::{api, PickFileOptions, PickerError, PickerResult};

/// Open a directory picker and return the selected absolute path,
/// or `None` if the user cancelled or no chooser is installed.
#[tauri::command]
pub async fn pick_directory(
    start_path: Option<String>,
) -> Result<Option<String>, String> {
    // Plugin-first path. The portal returns a URI on success;
    // for unconfined callers (Settings is unconfined) the URI is
    // `file:///absolute/path` which we convert back to a string
    // path the rest of Settings expects.
    let options = PickFileOptions {
        title: Some("Choose folder".to_string()),
        current_folder: start_path
            .clone()
            .filter(|s| !s.is_empty())
            .map(Into::into),
        ..PickFileOptions::default()
    };

    match api::pick_directory(options).await {
        Ok(PickerResult::Picked { uris }) => {
            if let Some(path) = uris.first().and_then(|u| uri_to_path(u)) {
                return Ok(Some(path));
            }
            log::warn!("portal returned empty/invalid uri list");
            // Fall through to kdialog/zenity as a defensive
            // fallback. The portal succeeded structurally but
            // gave us a useless answer; the legacy chooser is
            // better than no answer at all.
            return Ok(legacy_pick(start_path.as_deref()));
        }
        Ok(PickerResult::Cancelled) => {
            return Ok(None);
        }
        Err(PickerError::PortalUnavailable { .. })
        | Err(PickerError::ConnectionLost { .. }) => {
            // No portal frontend running. Fall through.
            log::info!("portal unavailable, falling back to kdialog/zenity");
        }
        Err(e) => {
            return Err(format!("portal pick_directory failed: {e}"));
        }
    }

    Ok(legacy_pick(start_path.as_deref()))
}

/// Decode a `file://...` URI back to an absolute filesystem path.
/// `None` for non-file URIs and parse failures — the caller falls
/// back to kdialog/zenity in that case.
fn uri_to_path(uri: &str) -> Option<String> {
    let suffix = uri.strip_prefix("file://")?;
    // Drop optional host segment per RFC 8089. `file:///path` has
    // empty host; `file://localhost/path` is also valid.
    let path_part = if let Some(stripped) = suffix.strip_prefix('/') {
        format!("/{stripped}")
    } else {
        // file://host/path — path begins after host's first slash.
        let idx = suffix.find('/')?;
        suffix[idx..].to_string()
    };
    let decoded = percent_decode(&path_part)?;
    if !decoded.starts_with('/') {
        return None;
    }
    Some(decoded)
}

/// Minimal RFC 3986 percent-decoder. Returns `None` on invalid
/// hex sequences. We avoid pulling the `percent-encoding` crate
/// because Settings already has it transitively but not as a
/// direct dep, and the decode here is straight-line.
fn percent_decode(input: &str) -> Option<String> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if i + 2 >= bytes.len() {
                return None;
            }
            let hi = (bytes[i + 1] as char).to_digit(16)?;
            let lo = (bytes[i + 2] as char).to_digit(16)?;
            out.push((hi * 16 + lo) as u8);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// kdialog → zenity fallback. Same shape as the pre-portal
/// implementation, kept intact so a missing portal frontend does
/// not block Settings from picking a directory.
fn legacy_pick(start_path: Option<&str>) -> Option<String> {
    let start = start_path
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| std::env::var("HOME").unwrap_or_else(|_| "/".to_string()));

    if let Some(path) = try_kdialog(&start) {
        return Some(path);
    }
    if let Some(path) = try_zenity(&start) {
        return Some(path);
    }
    log::warn!(
        "pick_directory: no file chooser found (portal unavailable, kdialog and zenity not \
         installed)"
    );
    None
}

fn try_kdialog(start: &str) -> Option<String> {
    let output = Command::new("kdialog")
        .args(["--getexistingdirectory", start])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let path = String::from_utf8(output.stdout).ok()?;
    valid_directory(path.trim())
}

fn try_zenity(start: &str) -> Option<String> {
    let seed = if start.ends_with('/') {
        start.to_string()
    } else {
        format!("{start}/")
    };
    let output = Command::new("zenity")
        .args([
            "--file-selection",
            "--directory",
            "--title=Choose folder",
            &format!("--filename={seed}"),
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let path = String::from_utf8(output.stdout).ok()?;
    valid_directory(path.trim())
}

fn valid_directory(path: &str) -> Option<String> {
    let p = Path::new(path);
    if p.is_dir() {
        Some(path.to_string())
    } else {
        log::warn!("pick_directory: chooser returned non-directory: {path}");
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Plain ASCII URIs round-trip back to their path.
    #[test]
    fn uri_to_path_basic() {
        assert_eq!(
            uri_to_path("file:///home/user/Documents"),
            Some("/home/user/Documents".to_string())
        );
    }

    /// Percent-encoded characters decode back into the path.
    #[test]
    fn uri_to_path_percent_encoded() {
        assert_eq!(
            uri_to_path("file:///home/user/My%20Documents"),
            Some("/home/user/My Documents".to_string())
        );
        assert_eq!(
            uri_to_path("file:///home/user/%C3%9Cber"),
            Some("/home/user/Über".to_string())
        );
    }

    /// `file://localhost/path` form drops the host segment.
    #[test]
    fn uri_to_path_with_host() {
        assert_eq!(
            uri_to_path("file://localhost/etc/hostname"),
            Some("/etc/hostname".to_string())
        );
    }

    /// Non-file URIs return None so the caller falls back.
    #[test]
    fn uri_to_path_rejects_other_schemes() {
        assert_eq!(uri_to_path("https://example.com"), None);
        assert_eq!(uri_to_path(""), None);
    }

    /// Malformed percent sequences return None rather than
    /// silently producing garbage.
    #[test]
    fn percent_decode_handles_malformed() {
        assert_eq!(percent_decode("a%2Z"), None);
        assert_eq!(percent_decode("trailing%2"), None);
    }
}

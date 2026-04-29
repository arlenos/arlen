//! Filesystem picker command. Wraps a system file-chooser
//! (zenity / kdialog) so the frontend `DirectoryPicker` component
//! can request a folder without bundling a Tauri-plugin-dialog dep.
//!
//! Try order: KDE first (kdialog), then GNOME/Fallback (zenity).
//! Returns `None` when the user cancelled or no chooser is
//! installed — the frontend treats both as "no-op", which is the
//! right UX for a cancelled dialog.

use std::path::Path;
use std::process::Command;

/// Open a directory picker and return the selected absolute path,
/// or `None` if the user cancelled or no chooser is installed.
#[tauri::command]
pub fn pick_directory(start_path: Option<String>) -> Result<Option<String>, String> {
    let start = start_path
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            std::env::var("HOME").unwrap_or_else(|_| "/".to_string())
        });

    if let Some(path) = try_kdialog(&start) {
        return Ok(Some(path));
    }
    if let Some(path) = try_zenity(&start) {
        return Ok(Some(path));
    }
    // No chooser found and not cancelled — log so a missing
    // dependency on a fresh install is visible in the daemon log
    // rather than silently failing.
    log::warn!(
        "pick_directory: no file chooser found (tried kdialog, zenity); \
         install one of them to enable the directory picker"
    );
    Ok(None)
}

/// `kdialog --getexistingdirectory <start>` returns the path on
/// stdout and exit code 0 on confirm, exit code 1 on cancel.
fn try_kdialog(start: &str) -> Option<String> {
    let output = Command::new("kdialog")
        .args(["--getexistingdirectory", start])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let path = String::from_utf8(output.stdout).ok()?;
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return None;
    }
    valid_directory(trimmed)
}

/// `zenity --file-selection --directory --filename=<start>` does the
/// same. `--filename=` (with trailing slash) seeds the starting dir.
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
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return None;
    }
    valid_directory(trimmed)
}

/// Defensive — only accept paths that exist and are directories.
/// A picker that returns a non-existent path would be a security
/// or DX failure; better to drop than to feed a bad path back into
/// the config.
fn valid_directory(path: &str) -> Option<String> {
    let p = Path::new(path);
    if p.is_dir() {
        Some(path.to_string())
    } else {
        log::warn!("pick_directory: chooser returned non-directory: {path}");
        None
    }
}

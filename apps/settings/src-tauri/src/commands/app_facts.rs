// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: GPL-3.0-only

//! The facts about an installed app that the per-app page states above its
//! settings: who published it, what it opens, what it is storing.
//!
//! Separate from `app_settings` because the source is different in kind. That
//! module hands over the app's DECLARED schema and the values in force; this one
//! reports what the system observes about the app from the outside - its desktop
//! entry, its data directories, the default-handler table. An app cannot declare
//! how much cache it has accumulated.
//!
//! Every field is `Option` or a possibly-empty list on purpose. The page's rule
//! is that a section it cannot state is simply absent, so a missing desktop entry
//! or an unreadable directory must degrade to "not known" rather than to a zero
//! that reads as a measurement.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Identity facts shown in the page head.
#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppMeta {
    /// The installed version, when the entry declares one.
    pub version: Option<String>,
    /// Who publishes it.
    pub publisher: Option<String>,
    /// The AppStream component id of the store entry, null when the app did not
    /// come from the store.
    pub store_component: Option<String>,
}

/// What the app opens and what it is storing.
#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppGeneral {
    /// File types and link kinds the app registered handlers for.
    pub opens: Vec<String>,
    /// App data size in bytes, null when not yet measured.
    pub app_bytes: Option<u64>,
    /// Cache size in bytes, null when not yet measured.
    pub cache_bytes: Option<u64>,
    /// Roles this app is the system default for.
    pub default_for: Vec<String>,
}

/// Reject anything that is not a plain app id before it reaches a path.
///
/// The id arrives from the page's route parameter, so it is caller-controlled,
/// and every use below joins it onto a directory. The same charset the identity
/// resolver uses, minus any separator.
fn is_safe_app_id(id: &str) -> bool {
    !id.is_empty()
        && id != "."
        && id != ".."
        && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from).filter(|p| p.is_absolute())
}

fn xdg_dir(var: &str, fallback: &str) -> Option<PathBuf> {
    std::env::var_os(var)
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| home().map(|h| h.join(fallback)))
}

/// Where the app's own state and cache live, matching the directories the
/// launcher creates for a confined app.
fn app_data_dir(app_id: &str) -> Option<PathBuf> {
    xdg_dir("XDG_DATA_HOME", ".local/share").map(|d| d.join("arlen/apps").join(app_id))
}

fn app_cache_dir(app_id: &str) -> Option<PathBuf> {
    xdg_dir("XDG_CACHE_HOME", ".cache").map(|d| d.join("arlen/apps").join(app_id))
}

/// The desktop entries an app id could own, user-installed first.
fn desktop_entry_paths(app_id: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(data) = xdg_dir("XDG_DATA_HOME", ".local/share") {
        out.push(data.join("applications").join(format!("{app_id}.desktop")));
    }
    out.push(PathBuf::from("/usr/share/applications").join(format!("{app_id}.desktop")));
    out
}

/// One key from a desktop entry's `[Desktop Entry]` group.
///
/// Deliberately not a full parser: the entry format allows localised keys
/// (`Name[de]`) and other groups, and taking the first match of a bare key in the
/// main group is what every consumer here wants. A localised variant is a
/// different key and correctly does not match.
fn entry_key(text: &str, key: &str) -> Option<String> {
    let mut in_main = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_main = line == "[Desktop Entry]";
            continue;
        }
        if !in_main {
            continue;
        }
        if let Some(rest) = line.strip_prefix(key) {
            if let Some(value) = rest.strip_prefix('=') {
                let value = value.trim();
                if !value.is_empty() {
                    return Some(value.to_owned());
                }
            }
        }
    }
    None
}

fn read_desktop_entry(app_id: &str) -> Option<String> {
    desktop_entry_paths(app_id).into_iter().find_map(|p| std::fs::read_to_string(p).ok())
}

/// Sum the regular files under `dir`, following no symlinks.
///
/// Returns `None` when the directory does not exist, which the page renders as
/// "not measured"; an existing but empty directory is `Some(0)`, which is a
/// measurement and reads differently.
fn dir_bytes(dir: &Path) -> Option<u64> {
    if !dir.is_dir() {
        return None;
    }
    let mut total = 0u64;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(next) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&next) else { continue };
        for entry in entries.flatten() {
            // `symlink_metadata`, so a link into a large tree is counted as the
            // link it is rather than as the tree it points at.
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_dir() {
                stack.push(entry.path());
            } else if meta.is_file() {
                total = total.saturating_add(meta.len());
            }
        }
    }
    Some(total)
}

/// The MIME types the entry registers, as the types themselves.
///
/// Human names for them belong to the page, which has the catalog; handing over
/// `text/markdown` unchanged keeps this side free of a second copy of that table.
fn declared_mime_types(entry: &str) -> Vec<String> {
    entry_key(entry, "MimeType")
        .map(|v| v.split(';').filter(|s| !s.is_empty()).map(str::to_owned).collect())
        .unwrap_or_default()
}

/// The MIME types this app is the registered default handler for.
fn default_handler_types(app_id: &str) -> Vec<String> {
    let desktop_file = format!("{app_id}.desktop");
    let Some(config) = xdg_dir("XDG_CONFIG_HOME", ".config") else { return Vec::new() };
    let Ok(text) = std::fs::read_to_string(config.join("mimeapps.list")) else {
        return Vec::new();
    };
    let mut out = BTreeSet::new();
    let mut in_default = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_default = line == "[Default Applications]";
            continue;
        }
        if !in_default {
            continue;
        }
        if let Some((mime, handlers)) = line.split_once('=') {
            if handlers.split(';').any(|h| h.trim() == desktop_file) {
                out.insert(mime.trim().to_owned());
            }
        }
    }
    out.into_iter().collect()
}

/// One app's identity facts, or `None` when nothing on the system knows it.
#[tauri::command]
pub async fn settings_app_meta(app_id: String) -> Result<Option<AppMeta>, String> {
    if !is_safe_app_id(&app_id) {
        return Err("not an app id".to_owned());
    }
    let Some(entry) = read_desktop_entry(&app_id) else { return Ok(None) };
    Ok(Some(AppMeta {
        version: entry_key(&entry, "X-Arlen-Version"),
        publisher: entry_key(&entry, "X-Arlen-Publisher"),
        store_component: entry_key(&entry, "X-AppStream-Component"),
    }))
}

/// What one app opens and what it is storing.
///
/// Unlike the metadata this is answerable for an app with no desktop entry - the
/// directories exist either way - so it returns a value whenever any part is
/// known, and `None` only when none of it is.
#[tauri::command]
pub async fn settings_app_general(app_id: String) -> Result<Option<AppGeneral>, String> {
    if !is_safe_app_id(&app_id) {
        return Err("not an app id".to_owned());
    }
    let entry = read_desktop_entry(&app_id);
    let general = AppGeneral {
        opens: entry.as_deref().map(declared_mime_types).unwrap_or_default(),
        app_bytes: app_data_dir(&app_id).as_deref().and_then(dir_bytes),
        cache_bytes: app_cache_dir(&app_id).as_deref().and_then(dir_bytes),
        default_for: default_handler_types(&app_id),
    };
    let known = !general.opens.is_empty()
        || general.app_bytes.is_some()
        || general.cache_bytes.is_some()
        || !general.default_for.is_empty();
    Ok(known.then_some(general))
}

/// Empty one app's cache directory, leaving the directory itself.
///
/// Idempotent, including when the directory does not exist, because the button
/// offers no confirm and a second press must not become an error.
#[tauri::command]
pub async fn settings_app_clear_cache(app_id: String) -> Result<(), String> {
    if !is_safe_app_id(&app_id) {
        return Err("not an app id".to_owned());
    }
    let Some(dir) = app_cache_dir(&app_id) else { return Ok(()) };
    if !dir.is_dir() {
        return Ok(());
    }
    let entries = std::fs::read_dir(&dir).map_err(|e| e.to_string())?;
    for entry in entries.flatten() {
        let path = entry.path();
        // Metadata of the entry itself: a symlink is removed as a link, never
        // followed into whatever it points at.
        let Ok(meta) = entry.metadata() else { continue };
        let removed = if meta.is_dir() {
            std::fs::remove_dir_all(&path)
        } else {
            std::fs::remove_file(&path)
        };
        removed.map_err(|e| format!("{}: {e}", path.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_app_id_that_could_leave_its_directory_is_refused() {
        assert!(is_safe_app_id("org.arlen.files"));
        assert!(is_safe_app_id("files"));
        assert!(!is_safe_app_id(""));
        assert!(!is_safe_app_id(".."));
        assert!(!is_safe_app_id("../../etc/passwd"));
        assert!(!is_safe_app_id("a/b"));
        assert!(!is_safe_app_id("a\\b"));
    }

    #[test]
    fn a_key_is_read_from_the_main_group_only() {
        let entry = "[Desktop Entry]\nName=Files\nX-Arlen-Version=1.2\n\n[Desktop Action New]\nName=New\n";
        assert_eq!(entry_key(entry, "Name").as_deref(), Some("Files"));
        assert_eq!(entry_key(entry, "X-Arlen-Version").as_deref(), Some("1.2"));
        assert_eq!(entry_key(entry, "Publisher"), None);
    }

    #[test]
    fn a_localised_key_is_a_different_key() {
        // `Name[de]` must not answer for `Name`, or a German entry would hand the
        // German string to a caller that asked for the unlocalised one.
        let entry = "[Desktop Entry]\nName[de]=Dateien\nName=Files\n";
        assert_eq!(entry_key(entry, "Name").as_deref(), Some("Files"));
    }

    #[test]
    fn an_empty_value_reads_as_absent() {
        assert_eq!(entry_key("[Desktop Entry]\nX-Arlen-Publisher=\n", "X-Arlen-Publisher"), None);
    }

    #[test]
    fn mime_types_are_handed_over_unchanged() {
        let entry = "[Desktop Entry]\nMimeType=text/markdown;text/plain;\n";
        assert_eq!(declared_mime_types(entry), vec!["text/markdown", "text/plain"]);
        assert!(declared_mime_types("[Desktop Entry]\nName=x\n").is_empty());
    }

    #[test]
    fn a_missing_directory_is_unmeasured_and_an_empty_one_is_zero() {
        let tmp = std::env::temp_dir().join(format!("arlen-appfacts-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        assert_eq!(dir_bytes(&tmp), None, "absent must not read as measured zero");
        std::fs::create_dir_all(&tmp).unwrap();
        assert_eq!(dir_bytes(&tmp), Some(0));
        std::fs::write(tmp.join("a"), b"1234").unwrap();
        std::fs::create_dir_all(tmp.join("sub")).unwrap();
        std::fs::write(tmp.join("sub/b"), b"12").unwrap();
        assert_eq!(dir_bytes(&tmp), Some(6), "nested files count");
        let _ = std::fs::remove_dir_all(&tmp);
    }
}

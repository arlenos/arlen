//! App metadata for the autonomy-dial per-app grant rows (arlen-ui-unblock #3).
//!
//! The harness can read + change the autonomous-app grant ids (`action_state` /
//! `ai_set_autonomous_app`), but an id like `org.telegram.desktop` is not a name
//! a person recognises. This resolves each id to its freedesktop `.desktop`
//! `Name=` + `Icon=` so the dial renders named (and iconned) rows instead of a
//! raw-id chip list.
//!
//! Best-effort + read-only: an id with no matching `.desktop` (a first-party
//! arlen app id, or an app without a launcher entry) returns `None` fields, which
//! the frontend renders as the id itself (its existing `principalLabel`
//! fallback). The `Icon=` value is returned verbatim (a theme name or an absolute
//! path); resolving a theme name to image bytes is the desktop-shell icon-theme
//! traversal, a follow-up - the named rows are the win here, and the frontend
//! keeps its initial-tile fallback for an unresolved icon.

use std::path::PathBuf;

/// One app's display metadata, keyed by the grant's app id.
#[derive(Debug, Clone, serde::Serialize)]
pub struct AppMeta {
    /// The app id the caller asked about (the autonomy-grant key).
    pub app_id: String,
    /// The `.desktop` `Name=`, if a matching entry was found.
    pub name: Option<String>,
    /// The `.desktop` `Icon=` value verbatim (a theme name or an absolute path),
    /// if found. Not resolved to image bytes here.
    pub icon: Option<String>,
}

/// Parse the `Name=` and `Icon=` of the `[Desktop Entry]` group from a `.desktop`
/// file's contents. Only the main group is read (a trailing `[Desktop Action ...]`
/// group's own `Name=` is ignored), and only the bare keys (a localised
/// `Name[de]=` does not start with `Name=`, so it is skipped); the first value of
/// each wins. Pure, so it is unit-tested without the filesystem.
fn parse_desktop_entry(contents: &str) -> (Option<String>, Option<String>) {
    let mut in_entry = false;
    let mut name = None;
    let mut icon = None;
    for line in contents.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_entry {
            continue;
        }
        if name.is_none() {
            if let Some(v) = line.strip_prefix("Name=") {
                name = Some(v.trim().to_string());
            }
        }
        if icon.is_none() {
            if let Some(v) = line.strip_prefix("Icon=") {
                icon = Some(v.trim().to_string());
            }
        }
    }
    (name, icon)
}

/// The freedesktop application directories searched for `<app_id>.desktop`, in
/// precedence order (user data dir + flatpak exports first, then the system
/// dirs). A fixed set covering the common layout; an exotic `XDG_DATA_DIRS` is a
/// follow-up - a missing dir simply does not match.
fn application_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(data_home) = dirs::data_dir() {
        dirs.push(data_home.join("applications"));
        dirs.push(data_home.join("flatpak/exports/share/applications"));
    }
    for base in [
        "/usr/local/share",
        "/usr/share",
        "/var/lib/flatpak/exports/share",
    ] {
        dirs.push(PathBuf::from(base).join("applications"));
    }
    dirs
}

/// Resolve one app id to its `.desktop` metadata. The id is the freedesktop
/// desktop-id, so the file is `<app_id>.desktop`. Path-traversal-safe: an id with
/// a separator or `..` is refused (returns empty), so a hostile grant id can
/// never read outside the application dirs. Returns empty fields when no entry
/// matches (best-effort).
fn read_app_desktop(app_id: &str) -> AppMeta {
    let empty = AppMeta {
        app_id: app_id.to_string(),
        name: None,
        icon: None,
    };
    if app_id.is_empty()
        || app_id.contains('/')
        || app_id.contains('\\')
        || app_id.contains("..")
    {
        return empty;
    }
    let file = format!("{app_id}.desktop");
    for dir in application_dirs() {
        if let Ok(contents) = std::fs::read_to_string(dir.join(&file)) {
            let (name, icon) = parse_desktop_entry(&contents);
            return AppMeta {
                app_id: app_id.to_string(),
                name,
                icon,
            };
        }
    }
    empty
}

/// Resolve display metadata for the given app ids (the autonomy-dial's
/// `autonomous_apps`), so the per-app grant rows render named/iconned instead of
/// as raw ids. One entry per input id, in order; an id with no `.desktop` entry
/// carries `None` fields (the frontend falls back to the id). Read-only +
/// best-effort.
#[tauri::command]
pub fn app_metadata(app_ids: Vec<String>) -> Vec<AppMeta> {
    app_ids.iter().map(|id| read_app_desktop(id)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_name_and_icon_from_the_main_group_only() {
        let contents = "[Desktop Entry]\n\
                        Type=Application\n\
                        Name=Telegram Desktop\n\
                        Name[de]=Telegramm\n\
                        Icon=org.telegram.desktop\n\
                        Exec=telegram-desktop\n\
                        \n\
                        [Desktop Action new-window]\n\
                        Name=New Window\n";
        let (name, icon) = parse_desktop_entry(contents);
        // The main-group bare Name wins; the localised key and the action's Name
        // are both ignored.
        assert_eq!(name.as_deref(), Some("Telegram Desktop"));
        assert_eq!(icon.as_deref(), Some("org.telegram.desktop"));
    }

    #[test]
    fn missing_keys_or_no_entry_group_yield_none() {
        assert_eq!(parse_desktop_entry(""), (None, None));
        let (name, icon) = parse_desktop_entry("[Desktop Action x]\nName=Nope\n");
        assert_eq!(name, None);
        assert_eq!(icon, None);
    }

    #[test]
    fn a_traversing_app_id_is_refused_without_a_read() {
        for hostile in ["../../etc/passwd", "a/b", "..", ""] {
            let meta = read_app_desktop(hostile);
            assert_eq!(meta.name, None);
            assert_eq!(meta.icon, None);
        }
    }
}

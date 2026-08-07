//! Reading a desktop entry, for launching rather than for listing.
//!
//! **The launcher's filter is not this filter, and that is the point of a second
//! reader rather than a shared one.** The app index drops an entry with
//! `NoDisplay=true`, `Hidden=true` or an `OnlyShowIn` that excludes us, because
//! those keys say "do not show this in a launcher". They do not say "this cannot
//! open a file", and `mimeapps.list` is entitled to name such an entry as the
//! handler for a type - a helper application that opens a format without wanting
//! a tile in the grid is exactly the normal case. Inheriting the visibility
//! filter here would turn that into "unknown application", which is a lie about
//! why nothing happened.
//!
//! What launching needs and listing does not: the `Exec` **verbatim**, field
//! codes intact. The index stores a placeholder-stripped copy because it starts
//! applications without documents; a service that opens one has to fill `%f`,
//! and by the time the index is done there is nothing left to fill.

use super::request::Entry;

/// Parse the `[Desktop Entry]` group of a `.desktop` file for launching.
///
/// `None` when it is not a launchable application: a `Type` other than
/// `Application`, or a missing `Name` or `Exec`. Only the first group is read -
/// a later `[Desktop Action ...]` describes a different thing to run and is not
/// what a handler lookup asked for.
///
/// A locale-suffixed key (`Name[de]`) is skipped, so the unlocalised value wins.
/// That is the wrong answer for a display name and the right one here: this
/// `Name` only ever reaches `%c`, and a launch that varies with the locale is
/// harder to reason about than one that does not.
pub fn parse_entry(desktop_id: &str, contents: &str, path: Option<&str>) -> Option<Entry> {
    let mut in_entry = false;
    let mut seen_entry = false;
    let mut name = None;
    let mut exec = None;
    let mut icon = None;
    let mut declared_id = None;
    let mut typ = None;

    for raw in contents.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            if seen_entry {
                break;
            }
            in_entry = line == "[Desktop Entry]";
            seen_entry = in_entry;
            continue;
        }
        if !in_entry {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let (key, value) = (key.trim(), value.trim());
        match key {
            // First wins, so a locale-suffixed key later in the group cannot
            // overwrite the unlocalised one it follows.
            "Name" if name.is_none() => name = Some(value.to_string()),
            "Exec" if exec.is_none() => exec = Some(value.to_string()),
            "Icon" if icon.is_none() => icon = Some(value.to_string()),
            "X-Arlen-AppId" if declared_id.is_none() => declared_id = Some(value.to_string()),
            "Type" if typ.is_none() => typ = Some(value.to_string()),
            _ => {}
        }
    }

    if matches!(&typ, Some(t) if t != "Application") {
        return None;
    }
    Some(Entry {
        app_id: app_id_of(declared_id.as_deref(), desktop_id),
        exec: exec.filter(|s| !s.is_empty())?,
        icon: icon.filter(|s| !s.is_empty()),
        name: name.filter(|s| !s.is_empty()),
        desktop_file: path.map(str::to_string),
    })
}

/// The application id `arlen-run` keys a permission profile on.
///
/// `X-Arlen-AppId` when the entry declares one, otherwise the desktop id without
/// its `.desktop` suffix, which is the freedesktop application identifier and
/// what every packaged app already uses.
///
/// The shell's app index derives the same thing its own way, from the file path.
/// Same rule, and the index's copy should come here once the service is wired -
/// two derivations of an application's identity is exactly the class of drift
/// this strand is removing everywhere else.
pub fn app_id_of(declared: Option<&str>, desktop_id: &str) -> String {
    match declared.map(str::trim).filter(|s| !s.is_empty()) {
        Some(id) => id.to_string(),
        None => desktop_id
            .strip_suffix(".desktop")
            .unwrap_or(desktop_id)
            .to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VIEWER: &str = "[Desktop Entry]\n\
        Type=Application\n\
        Name=Viewer\n\
        Icon=image-viewer\n\
        Exec=viewer %U\n\
        MimeType=image/png;\n";

    #[test]
    fn an_entry_yields_what_launching_needs() {
        let e = parse_entry("org.arlen.Viewer.desktop", VIEWER, Some("/a/x.desktop")).unwrap();
        assert_eq!(e.app_id, "org.arlen.Viewer");
        // Verbatim: the field code has to survive for the service to fill it.
        assert_eq!(e.exec, "viewer %U");
        assert_eq!(e.icon.as_deref(), Some("image-viewer"));
        assert_eq!(e.name.as_deref(), Some("Viewer"));
        assert_eq!(e.desktop_file.as_deref(), Some("/a/x.desktop"));
    }

    /// The whole reason this is not the index's reader.
    #[test]
    fn a_hidden_entry_still_launches_when_a_handler_names_it() {
        let hidden = format!("{VIEWER}NoDisplay=true\nHidden=true\nOnlyShowIn=GNOME;\n");
        let e = parse_entry("x.desktop", &hidden, None).unwrap();
        assert_eq!(e.exec, "viewer %U");
    }

    #[test]
    fn a_declared_app_id_wins_over_the_file_name() {
        let with = format!("{VIEWER}X-Arlen-AppId=org.example.Other\n");
        let e = parse_entry("x.desktop", &with, None).unwrap();
        assert_eq!(e.app_id, "org.example.Other");
    }

    #[test]
    fn an_empty_declared_id_falls_back_rather_than_becoming_empty() {
        let blank = format!("{VIEWER}X-Arlen-AppId=\n");
        let e = parse_entry("org.x.App.desktop", &blank, None).unwrap();
        assert_eq!(e.app_id, "org.x.App");
    }

    #[test]
    fn a_link_or_directory_entry_is_not_launchable() {
        let link = "[Desktop Entry]\nType=Link\nName=A\nExec=x\nURL=http://e.org\n";
        assert!(parse_entry("x.desktop", link, None).is_none());
    }

    #[test]
    fn an_entry_without_an_exec_is_not_launchable() {
        let no_exec = "[Desktop Entry]\nType=Application\nName=A\n";
        assert!(parse_entry("x.desktop", no_exec, None).is_none());
        let empty = "[Desktop Entry]\nType=Application\nName=A\nExec=\n";
        assert!(parse_entry("x.desktop", empty, None).is_none());
    }

    /// A `[Desktop Action ...]` describes a different thing to run, and a
    /// handler lookup did not ask for it.
    #[test]
    fn a_later_action_group_does_not_replace_the_entry() {
        let with_action = format!("{VIEWER}\n[Desktop Action New]\nName=New\nExec=viewer --new\n");
        let e = parse_entry("x.desktop", &with_action, None).unwrap();
        assert_eq!(e.exec, "viewer %U");
    }

    #[test]
    fn a_locale_suffixed_key_does_not_replace_the_plain_one() {
        let localised = "[Desktop Entry]\nType=Application\nName=Viewer\nName[de]=Betrachter\n\
                         Exec=viewer\n";
        let e = parse_entry("x.desktop", localised, None).unwrap();
        assert_eq!(e.name.as_deref(), Some("Viewer"));
    }

    /// An entry with no `Type` at all is still an application by the spec's
    /// default, and refusing it would drop working handlers.
    #[test]
    fn a_missing_type_is_treated_as_an_application() {
        let no_type = "[Desktop Entry]\nName=A\nExec=a\n";
        assert!(parse_entry("x.desktop", no_type, None).is_some());
    }

    #[test]
    fn comments_and_stray_lines_do_not_stop_the_read() {
        let messy = "# a comment\n[Desktop Entry]\nnonsense\nType=Application\nName=A\nExec=a\n";
        assert_eq!(parse_entry("x.desktop", messy, None).unwrap().exec, "a");
    }
}

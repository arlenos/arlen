//! The picker's own recently-used folders.
//!
//! WHAT THIS IS, and the distinction is the whole design. A sidebar place is a
//! FOLDER somebody navigates to, so "Recent" here is the folders they last
//! picked something from - the picker's own history. It is deliberately NOT the
//! system's recent FILES, which the file manager reads from the knowledge graph
//! for its own Zuletzt section. Files are not places, and a list of documents in
//! a folder sidebar would be a category error dressed as a feature.
//!
//! It also means this needs no capability the picker does not already have. A
//! graph-backed recent-files list would hand a dialog that untrusted apps
//! trigger a read over everything the person has opened, which is a grant worth
//! asking about rather than one worth assuming.
//!
//! Recorded on a successful PICK, not on navigation: the folder somebody chose
//! from is a place they will want again, while the folders they passed through
//! on the way are noise that would push it out.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// How many folders are kept.
///
/// Short on purpose. The group sits above Places in a dialog somebody wants to
/// leave quickly, and a list long enough to need reading is one that costs more
/// than it saves.
pub const KEEP: usize = 6;

/// One remembered folder, as the sidebar renders it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecentPlace {
    /// The folder's own name, which is what a person recognises.
    pub label: String,
    /// The absolute path.
    pub path: String,
}

/// The remembered list, newest first.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Recent {
    #[serde(default)]
    pub places: Vec<RecentPlace>,
}

/// Where the list lives: `$XDG_STATE_HOME/arlen/picker/recent.toml`, else
/// `~/.local/state/...`.
///
/// STATE rather than config, and the difference is not pedantic: this is
/// accumulated by using the thing, nobody edits it, and losing it costs a
/// convenience rather than a setting. A config directory is for what somebody
/// chose.
#[must_use]
pub fn state_path() -> Option<PathBuf> {
    let base = match std::env::var_os("XDG_STATE_HOME") {
        Some(d) if !d.is_empty() => PathBuf::from(d),
        _ => PathBuf::from(std::env::var_os("HOME")?).join(".local/state"),
    };
    Some(base.join("arlen/picker/recent.toml"))
}

/// The folder a picked path sits in, as a remembered place.
///
/// The path itself is a file; its parent is the place. A picked DIRECTORY is
/// remembered as itself, which is what somebody choosing a folder meant.
#[must_use]
pub fn place_for(picked: &Path, is_dir: bool) -> Option<RecentPlace> {
    let folder = if is_dir { picked } else { picked.parent()? };
    // A pick at the filesystem root has no folder worth a row.
    let label = folder.file_name()?.to_str()?.to_string();
    Some(RecentPlace {
        label,
        path: folder.to_str()?.to_string(),
    })
}

/// Put a folder at the front, keeping the list to [`KEEP`] and free of
/// duplicates.
///
/// Moving an existing entry rather than adding a second one: a folder somebody
/// picks from repeatedly is the one they want first, and a list that grew a row
/// per visit would push everything else out with one directory.
pub fn remember(recent: &mut Recent, place: RecentPlace) {
    recent.places.retain(|p| p.path != place.path);
    recent.places.insert(0, place);
    recent.places.truncate(KEEP);
}

/// Why a read did not produce a list.
///
/// ABSENT IS NOT UNREADABLE, and keeping them apart is what stops this feature
/// destroying its own file. A first run has no list and writing one is correct;
/// a file that would not parse is one somebody could still fix by hand, and
/// treating it as empty means the next pick overwrites it with a single entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unread {
    /// No file yet. An empty list, and safe to write.
    Absent,
    /// It is there and did not read. NOT safe to write over.
    Unreadable(String),
}

/// Read the list.
pub fn load(path: &Path) -> Result<Recent, Unread> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Recent::default()),
        Err(e) => return Err(Unread::Unreadable(e.to_string())),
    };
    toml::from_str(&text).map_err(|e| Unread::Unreadable(e.to_string()))
}

/// Write the list, creating its directory on first use.
///
/// Written to a sibling temp file and renamed, so a picker reading while another
/// writes sees one list or the other and never half of one.
pub fn save(path: &Path, recent: &Recent) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = toml::to_string_pretty(recent)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}

/// Record that something was picked from `picked`.
///
/// Best-effort throughout: a pick that succeeded must not fail because a
/// convenience list could not be written.
pub fn record(picked: &Path, is_dir: bool) {
    let Some(path) = state_path() else {
        return;
    };
    let Some(place) = place_for(picked, is_dir) else {
        return;
    };
    // A file that did not read is left exactly as it is. Writing here would turn
    // a list somebody could repair into a list with one entry, and the thing
    // being protected is not important enough to destroy anything for.
    let Ok(mut recent) = load(&path) else {
        return;
    };
    remember(&mut recent, place);
    let _ = save(&path, &recent);
}

/// The remembered folders, newest first, skipping any that no longer exist.
///
/// Checked rather than trusted: a folder that was deleted or unmounted since it
/// was remembered is a row that leads nowhere, and a sidebar entry that fails
/// when clicked is worse than one that is not there.
#[tauri::command]
pub fn picker_recent() -> Vec<RecentPlace> {
    let Some(path) = state_path() else {
        return Vec::new();
    };
    // For DISPLAY an unreadable list and an empty one look the same: the group
    // does not render. That is the right end for a sidebar convenience, and it
    // is safe here precisely because this path writes nothing.
    load(&path)
        .unwrap_or_default()
        .places
        .into_iter()
        .filter(|p| Path::new(&p.path).is_dir())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picked_file_remembers_the_folder_it_was_in() {
        let p = place_for(Path::new("/home/tim/Pictures/holiday.png"), false).unwrap();
        assert_eq!(p.label, "Pictures");
        assert_eq!(p.path, "/home/tim/Pictures");
    }

    #[test]
    fn a_picked_folder_is_remembered_as_itself() {
        let p = place_for(Path::new("/home/tim/Projects"), true).unwrap();
        assert_eq!(p.label, "Projects");
        assert_eq!(p.path, "/home/tim/Projects");
    }

    #[test]
    fn a_pick_at_the_root_remembers_nothing() {
        assert!(place_for(Path::new("/"), true).is_none());
        assert!(place_for(Path::new("/x"), false).is_none(), "its folder is the root");
    }

    #[test]
    fn picking_from_the_same_folder_twice_moves_it_rather_than_doubling_it() {
        let mut r = Recent::default();
        let a = RecentPlace { label: "A".into(), path: "/a".into() };
        let b = RecentPlace { label: "B".into(), path: "/b".into() };
        remember(&mut r, a.clone());
        remember(&mut r, b);
        remember(&mut r, a.clone());
        assert_eq!(r.places.len(), 2);
        assert_eq!(r.places[0], a, "the one just used is first");
    }

    #[test]
    fn the_list_stays_short() {
        let mut r = Recent::default();
        for n in 0..20 {
            remember(&mut r, RecentPlace { label: n.to_string(), path: format!("/p{n}") });
        }
        assert_eq!(r.places.len(), KEEP);
        assert_eq!(r.places[0].path, "/p19", "newest first");
    }

    #[test]
    fn a_missing_file_is_an_empty_list_rather_than_a_failure() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load(&dir.path().join("nothing.toml")), Ok(Recent::default()));
    }

    #[test]
    fn a_file_nobody_can_parse_is_not_reported_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("recent.toml");
        std::fs::write(&p, "this is not toml {{{").unwrap();
        assert!(matches!(load(&p), Err(Unread::Unreadable(_))));
    }

    #[test]
    fn a_pick_never_overwrites_a_list_it_could_not_read() {
        // The fault a check caught rather than a reviewer: a read that failed
        // became an empty list and the next pick wrote it back, turning a file
        // somebody could still repair into one with a single entry.
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("recent.toml");
        let broken = "this is not toml {{{";
        std::fs::write(&p, broken).unwrap();

        let mut recent = match load(&p) {
            Ok(r) => r,
            Err(_) => {
                assert_eq!(std::fs::read_to_string(&p).unwrap(), broken, "left alone");
                return;
            }
        };
        remember(&mut recent, RecentPlace { label: "X".into(), path: "/x".into() });
        panic!("an unreadable list was treated as empty");
    }

    #[test]
    fn the_list_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("arlen/picker/recent.toml");
        let mut r = Recent::default();
        remember(&mut r, RecentPlace { label: "Pictures".into(), path: "/home/tim/Pictures".into() });
        save(&p, &r).unwrap();
        assert_eq!(load(&p), Ok(r));
    }
}

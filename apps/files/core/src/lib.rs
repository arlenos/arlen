//! The Arlen file-browser core: the embeddable model + listing + sort + path
//! decomposition the file manager and the confined xdg picker both host
//! (`file-manager-plan.md` FM-R1, "one component, many hosts").
//!
//! Filesystem access is **capability-scoped**: the host opens the allowed root(s)
//! as cap-std [`Dir`] capabilities and the core only ever reads through one, with
//! a path relative to it. cap-std resolves every component against the capability
//! and refuses to escape it, so the confined picker can host this exact core
//! without an ambient-authority hole. The UI view modes (list/grid/Miller/
//! dual-pane), the icons and the chrome are the host's; this crate is the
//! Tauri-agnostic data core (entries, scoped listing, sorting, the breadcrumb
//! model, keyboard-selection helpers) shared across hosts.

use std::cmp::Ordering;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::time::UNIX_EPOCH;

use cap_std::fs::{Dir, FileType, Metadata};
use serde::{Deserialize, Serialize};

/// What a directory entry is, reported WITHOUT following symlinks (a symlink is a
/// symlink, never silently its target's kind).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EntryKind {
    /// A directory.
    Directory,
    /// A regular file.
    File,
    /// A symbolic link (its target may be anything, or missing).
    Symlink,
    /// A device, socket, fifo or other special node.
    Other,
}

/// One entry in a directory listing, the unit a `FileRow`/`FileTile` renders.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
    /// The entry's file name (no path).
    pub name: String,
    /// What the entry is (symlinks are not followed for this).
    pub kind: EntryKind,
    /// Size in bytes for non-directories; `None` for directories or when the
    /// metadata is unavailable.
    pub size: Option<u64>,
    /// Modification time, seconds since the Unix epoch, when available.
    pub modified_unix: Option<u64>,
    /// Whether the name begins with `.` (the dotfile convention).
    pub is_hidden: bool,
    /// Whether the entry is read-only (the owner write bit is clear).
    pub readonly: bool,
    /// For a symlink, the raw link target (not resolved); `None` otherwise.
    pub symlink_target: Option<String>,
}

/// List the directory at `rel` (relative to the capability `dir`), one
/// [`FileEntry`] per child, without following symlinks.
///
/// cap-std confines every read to `dir`, so `rel` cannot escape the host's
/// granted root (a `..` or an absolute path is refused at the syscall, not by a
/// string check here). An entry whose type cannot be read is skipped rather than
/// guessed; an entry whose metadata is unavailable is still listed with the
/// unknown fields left `None`, so a stat failure hides nothing from the user.
pub fn list_dir(dir: &Dir, rel: impl AsRef<Path>) -> io::Result<Vec<FileEntry>> {
    let rel = rel.as_ref();
    let mut out = Vec::new();
    for entry in dir.read_dir(rel)? {
        let Ok(entry) = entry else { continue };
        let name = entry.file_name().to_string_lossy().into_owned();
        let Ok(file_type) = entry.file_type() else { continue };
        let kind = kind_of(&file_type);
        // `DirEntry::metadata` reports the entry's OWN metadata (a symlink is not
        // followed), which is what a listing wants.
        let meta = entry.metadata().ok();
        let size = meta
            .as_ref()
            .and_then(|m| if m.is_dir() { None } else { Some(m.len()) });
        let modified_unix = meta.as_ref().and_then(modified_unix);
        let readonly = meta
            .as_ref()
            .map(|m| m.permissions().readonly())
            .unwrap_or(false);
        let symlink_target = if kind == EntryKind::Symlink {
            dir.read_link(rel.join(&name))
                .ok()
                .map(|p| p.to_string_lossy().into_owned())
        } else {
            None
        };
        out.push(FileEntry {
            is_hidden: name.starts_with('.'),
            name,
            kind,
            size,
            modified_unix,
            readonly,
            symlink_target,
        });
    }
    Ok(out)
}

/// Classify an entry's `FileType` without following symlinks.
fn kind_of(file_type: &FileType) -> EntryKind {
    if file_type.is_symlink() {
        EntryKind::Symlink
    } else if file_type.is_dir() {
        EntryKind::Directory
    } else if file_type.is_file() {
        EntryKind::File
    } else {
        EntryKind::Other
    }
}

/// Seconds since the Unix epoch for a metadata mtime, or `None` if unavailable.
fn modified_unix(meta: &Metadata) -> Option<u64> {
    meta.modified()
        .ok()
        .and_then(|t| t.into_std().duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
}

/// The column a listing is sorted by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SortKey {
    /// Case-insensitive by name.
    Name,
    /// By size (directories compare as 0).
    Size,
    /// By modification time.
    Modified,
}

/// Sort a listing in place. When `folders_first` (the file-manager convention),
/// directories always precede non-directories regardless of `key`/`ascending`;
/// the `key` and direction order the entries WITHIN the folder and file groups.
/// The name comparison is case-insensitive; missing size/mtime compare as 0.
pub fn sort_entries(entries: &mut [FileEntry], key: SortKey, folders_first: bool, ascending: bool) {
    entries.sort_by(|a, b| {
        if folders_first {
            let ad = a.kind == EntryKind::Directory;
            let bd = b.kind == EntryKind::Directory;
            if ad != bd {
                return if ad { Ordering::Less } else { Ordering::Greater };
            }
        }
        let ord = match key {
            SortKey::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            SortKey::Size => a.size.unwrap_or(0).cmp(&b.size.unwrap_or(0)),
            SortKey::Modified => a.modified_unix.unwrap_or(0).cmp(&b.modified_unix.unwrap_or(0)),
        };
        // A stable tiebreak by name keeps the order deterministic for equal keys.
        let ord = ord.then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        if ascending {
            ord
        } else {
            ord.reverse()
        }
    });
}

/// One clickable segment of the path bar: the display name and the absolute path
/// that navigating to it produces.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Crumb {
    /// The segment's display name (`/` for the root).
    pub name: String,
    /// The absolute path this crumb navigates to.
    pub path: String,
}

/// Decompose an absolute path into breadcrumb segments, each carrying the
/// absolute path that navigating to it produces (`/home/x/p` ->
/// `/`, `/home`, `/home/x`, `/home/x/p`). `.`/`..` components are ignored (the
/// path bar shows canonical locations, not relative steps).
pub fn breadcrumb(path: &Path) -> Vec<Crumb> {
    let mut crumbs = Vec::new();
    let mut acc = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::RootDir => {
                acc.push("/");
                crumbs.push(Crumb {
                    name: "/".to_string(),
                    path: "/".to_string(),
                });
            }
            Component::Normal(seg) => {
                acc.push(seg);
                crumbs.push(Crumb {
                    name: seg.to_string_lossy().into_owned(),
                    path: acc.to_string_lossy().into_owned(),
                });
            }
            // Prefix (Windows), CurDir, ParentDir: not part of a canonical Unix
            // location bar.
            _ => {}
        }
    }
    crumbs
}

#[cfg(test)]
mod tests {
    use super::*;
    use cap_std::ambient_authority;

    fn entry(name: &str, kind: EntryKind, size: Option<u64>, mtime: Option<u64>) -> FileEntry {
        FileEntry {
            name: name.to_string(),
            kind,
            size,
            modified_unix: mtime,
            is_hidden: name.starts_with('.'),
            readonly: false,
            symlink_target: None,
        }
    }

    #[test]
    fn folders_sort_before_files_regardless_of_key_and_direction() {
        let mut v = vec![
            entry("zeta.txt", EntryKind::File, Some(10), Some(5)),
            entry("alpha", EntryKind::Directory, None, Some(1)),
            entry("beta.txt", EntryKind::File, Some(3), Some(9)),
            entry("gamma", EntryKind::Directory, None, Some(2)),
        ];
        sort_entries(&mut v, SortKey::Name, true, true);
        let names: Vec<&str> = v.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["alpha", "gamma", "beta.txt", "zeta.txt"]);

        // Descending still keeps folders first, only reverses within the groups.
        sort_entries(&mut v, SortKey::Name, true, false);
        let names: Vec<&str> = v.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["gamma", "alpha", "zeta.txt", "beta.txt"]);
    }

    #[test]
    fn name_sort_is_case_insensitive_and_size_sort_orders_files() {
        let mut v = vec![
            entry("Banana", EntryKind::File, Some(2), None),
            entry("apple", EntryKind::File, Some(9), None),
            entry("Cherry", EntryKind::File, Some(1), None),
        ];
        sort_entries(&mut v, SortKey::Name, false, true);
        assert_eq!(
            v.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
            vec!["apple", "Banana", "Cherry"]
        );
        sort_entries(&mut v, SortKey::Size, false, true);
        assert_eq!(
            v.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
            vec!["Cherry", "Banana", "apple"]
        );
    }

    #[test]
    fn breadcrumb_decomposes_an_absolute_path() {
        let crumbs = breadcrumb(Path::new("/home/x/proj"));
        assert_eq!(
            crumbs,
            vec![
                Crumb { name: "/".into(), path: "/".into() },
                Crumb { name: "home".into(), path: "/home".into() },
                Crumb { name: "x".into(), path: "/home/x".into() },
                Crumb { name: "proj".into(), path: "/home/x/proj".into() },
            ]
        );
        assert!(breadcrumb(Path::new("/")).len() == 1);
        // `.`/`..` are dropped, not turned into segments.
        assert_eq!(breadcrumb(Path::new("/a/../b")).last().unwrap().name, "b");
    }

    #[test]
    fn list_dir_reports_kinds_hidden_and_a_symlink_without_following_it() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("subdir")).unwrap();
        std::fs::write(tmp.path().join("file.txt"), b"hello").unwrap();
        std::fs::write(tmp.path().join(".hidden"), b"x").unwrap();
        std::os::unix::fs::symlink("file.txt", tmp.path().join("link")).unwrap();

        let dir = Dir::open_ambient_dir(tmp.path(), ambient_authority()).unwrap();
        let mut entries = list_dir(&dir, ".").unwrap();
        sort_entries(&mut entries, SortKey::Name, true, true);

        let by = |n: &str| entries.iter().find(|e| e.name == n).cloned().unwrap();
        assert_eq!(by("subdir").kind, EntryKind::Directory);
        assert_eq!(by("subdir").size, None, "directories report no size");
        let f = by("file.txt");
        assert_eq!(f.kind, EntryKind::File);
        assert_eq!(f.size, Some(5));
        assert!(!f.is_hidden);
        assert!(by(".hidden").is_hidden, "a dotfile is hidden");
        let link = by("link");
        assert_eq!(link.kind, EntryKind::Symlink, "a symlink is reported as a symlink");
        assert_eq!(
            link.symlink_target.as_deref(),
            Some("file.txt"),
            "the raw link target is reported, not followed"
        );
        // Folders-first ordering put the directory at the front.
        assert_eq!(entries[0].name, "subdir");
    }

    #[test]
    fn list_dir_cannot_escape_the_capability() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = Dir::open_ambient_dir(tmp.path(), ambient_authority()).unwrap();
        // An absolute path or a parent traversal is refused by cap-std, not served.
        assert!(list_dir(&dir, "/etc").is_err());
        assert!(list_dir(&dir, "../..").is_err());
    }
}

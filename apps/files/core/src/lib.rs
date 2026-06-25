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

pub mod bulk_rename;
/// The media-metadata-edit write-back (editable EXIF), behind the off-by-default
/// `metadata-edit` feature.
#[cfg(feature = "metadata-edit")]
pub mod metadata;
pub mod openwith;
pub mod ops;
pub mod search;
pub mod undo;
pub mod selection;
pub mod thumbnail_cache;

use std::cmp::Ordering;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::time::UNIX_EPOCH;

use cap_std::fs::{Dir, FileType, Metadata, MetadataExt};
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
    /// The entry's own absolute path, set ONLY for a virtual-location listing
    /// (Recent / Trash) where each entry lives in a different directory, not under
    /// one browsed folder - the FM Recent/Trash navigation-location re-plan (item 12).
    /// `None` for a normal folder listing (the controller knows the containing dir),
    /// so this is additive + backward-compat. For Trash this is the ORIGINAL path.
    #[serde(default)]
    pub full_path: Option<String>,
    /// An opaque per-entry token a location-specific action needs that the path
    /// alone cannot supply - currently the Trash `trashed_name`, which Restore /
    /// Delete-forever pass back. `None` for any normal entry. Additive.
    #[serde(default)]
    pub restore_token: Option<String>,
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
            full_path: None,
            restore_token: None,
        });
    }
    Ok(out)
}

/// Classify an entry's `FileType` without following symlinks.
pub(crate) fn kind_of(file_type: &FileType) -> EntryKind {
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
    /// By type, i.e. the lowercased file extension (all `.rs` together, all
    /// `.png` together). Extension-less files group under an empty key; the name
    /// tiebreak orders within each extension group. Folders-first is unaffected
    /// (directories already group ahead via `folders_first`), so the extension
    /// key only discriminates the file group, which is what "sort by type" means.
    Type,
}

/// The lowercased extension of `name` (without the dot), used as the
/// [`SortKey::Type`] comparison key. Follows the same convention as the ops
/// module's name split: the part after the LAST `.`, except a leading-dot
/// dotfile (whose only `.` is at index 0, e.g. `.bashrc`) and a name with no `.`
/// have no extension and yield an empty key.
pub(crate) fn ext_key(name: &str) -> String {
    match name.rfind('.') {
        Some(i) if i > 0 => name[i + 1..].to_lowercase(),
        _ => String::new(),
    }
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
            SortKey::Type => ext_key(&a.name).cmp(&ext_key(&b.name)),
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

/// How to partition a listing into labeled groups for the "group by" view. The
/// default is [`GroupKey::None`] (a single unlabeled group, the ungrouped
/// listing). Grouping is applied AFTER [`sort_entries`], so each group keeps the
/// sort order within it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GroupKey {
    /// One group: the listing as-is, ungrouped.
    None,
    /// By entry kind: Folders, Files, Links, Other.
    Kind,
    /// By lowercased extension (all `.rs` together), with folders in their own
    /// group; mirrors [`SortKey::Type`].
    Type,
    /// By modification recency, bucketed by ELAPSED time from a reference `now`
    /// (Today / Yesterday / Earlier this week / Earlier this month / Older /
    /// Unknown), not calendar days, so it needs no timezone.
    Modified,
    /// By size bucket (Folders / Empty / Small / Medium / Large / Unknown).
    Size,
}

/// One labeled section of a grouped listing. The entries keep the order they had
/// in the input slice, so a caller that sorts first and groups second gets each
/// group internally sorted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntryGroup {
    /// The section heading (e.g. "Folders", "RS", "Today").
    pub label: String,
    /// The entries in this group, in input order.
    pub entries: Vec<FileEntry>,
}

/// Partition a listing into labeled [`EntryGroup`]s by `key`, preserving the
/// input order within each group and ordering the groups by first appearance
/// (so a listing pre-sorted by the matching axis yields groups in a sensible
/// order, e.g. Folders first under a folders-first sort). `now_unix` is the
/// reference for [`GroupKey::Modified`] recency buckets and is ignored by the
/// other keys. An empty input yields an empty Vec; [`GroupKey::None`] yields a
/// single group with an empty label.
pub fn group_entries(entries: &[FileEntry], key: GroupKey, now_unix: u64) -> Vec<EntryGroup> {
    let mut groups: Vec<EntryGroup> = Vec::new();
    // Map a label to its group index for O(1) appends; group ORDER comes from
    // `groups` (first appearance), never the map, so the result is deterministic.
    let mut index: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for entry in entries {
        let label = group_label(entry, key, now_unix);
        let gi = match index.get(&label) {
            Some(&i) => i,
            None => {
                index.insert(label.clone(), groups.len());
                groups.push(EntryGroup { label, entries: Vec::new() });
                groups.len() - 1
            }
        };
        groups[gi].entries.push(entry.clone());
    }
    groups
}

/// The group label `entry` falls under for `key`.
fn group_label(entry: &FileEntry, key: GroupKey, now_unix: u64) -> String {
    match key {
        GroupKey::None => String::new(),
        GroupKey::Kind => match entry.kind {
            EntryKind::Directory => "Folders",
            EntryKind::File => "Files",
            EntryKind::Symlink => "Links",
            EntryKind::Other => "Other",
        }
        .to_string(),
        GroupKey::Type => {
            if entry.kind == EntryKind::Directory {
                "Folders".to_string()
            } else {
                let ext = ext_key(&entry.name);
                if ext.is_empty() {
                    "No extension".to_string()
                } else {
                    ext.to_uppercase()
                }
            }
        }
        GroupKey::Modified => match entry.modified_unix {
            None => "Unknown".to_string(),
            Some(t) => modified_bucket(t, now_unix).to_string(),
        },
        GroupKey::Size => {
            if entry.kind == EntryKind::Directory {
                "Folders".to_string()
            } else {
                match entry.size {
                    None => "Unknown".to_string(),
                    Some(s) => size_bucket(s).to_string(),
                }
            }
        }
    }
}

/// Bucket a modification time into a recency label by ELAPSED seconds from
/// `now` (not calendar days, so no timezone is needed). A timestamp ahead of
/// `now` (clock skew) counts as "Today".
fn modified_bucket(t: u64, now: u64) -> &'static str {
    const DAY: u64 = 86_400;
    let elapsed = now.saturating_sub(t);
    if elapsed < DAY {
        "Today"
    } else if elapsed < 2 * DAY {
        "Yesterday"
    } else if elapsed < 7 * DAY {
        "Earlier this week"
    } else if elapsed < 30 * DAY {
        "Earlier this month"
    } else {
        "Older"
    }
}

/// Bucket a file size (bytes) into a coarse label: 0 is "Empty", under 1 MiB
/// "Small", under 1 GiB "Medium", else "Large".
fn size_bucket(bytes: u64) -> &'static str {
    const MIB: u64 = 1 << 20;
    const GIB: u64 = 1 << 30;
    if bytes == 0 {
        "Empty"
    } else if bytes < MIB {
        "Small"
    } else if bytes < GIB {
        "Medium"
    } else {
        "Large"
    }
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

/// The full-fidelity detail of a single entry, the data behind a Properties
/// panel (FM-R3): kind, size, the Unix mode, the four timestamps, ownership and
/// link count. Read WITHOUT following a final symlink, so a symlink reports the
/// LINK's own properties (its size/mode/times), never silently the target's.
///
/// Times are `i64` seconds, the native form of the `MetadataExt` time accessors
/// and wide enough for pre-1970 mtimes; this is the detail surface, so it keeps
/// the full fidelity that [`FileEntry::modified_unix`] (`u64`, for listing-
/// display economy) trims. `created_unix` is an `Option` because birth time is
/// often unsupported by the filesystem/kernel and then reads as `None`, not an
/// error.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Properties {
    /// What the entry is (a symlink is reported as a symlink, not followed).
    pub kind: EntryKind,
    /// Size in bytes of the entry itself (for a symlink, the link's own size).
    pub size: u64,
    /// The full Unix `st_mode` (the file-type bits plus the permission bits,
    /// including suid/sgid/sticky); the host formats the `rwxr-xr-x` display.
    pub mode: u32,
    /// Modification time (`mtime`), seconds since the Unix epoch.
    pub modified_unix: i64,
    /// Inode change time (`ctime`), seconds since the Unix epoch.
    pub changed_unix: i64,
    /// Last access time (`atime`), seconds since the Unix epoch.
    pub accessed_unix: i64,
    /// Birth time (`btime`), seconds since the Unix epoch, IF the filesystem and
    /// platform report it; `None` otherwise (most Linux setups).
    pub created_unix: Option<i64>,
    /// The owning user id.
    pub uid: u32,
    /// The owning group id.
    pub gid: u32,
    /// The number of hard links to the entry.
    pub nlink: u64,
    /// For a symlink, its raw (unresolved) target; `None` otherwise. Mirrors
    /// [`FileEntry::symlink_target`].
    pub symlink_target: Option<String>,
}

/// Read the full [`Properties`] of the entry at `rel` (relative to the capability
/// `dir`), WITHOUT following a final symlink.
///
/// Like [`list_dir`], cap-std confines the read to `dir`, so an absolute or
/// `..`-bearing `rel` is refused at the syscall (an `io::Error`), not by a string
/// check. The metadata is read with `symlink_metadata`, so a symlink's properties
/// are the LINK's own (its mode/size/times), never the target's; a host that
/// wants the target's properties resolves the link and calls this on the target.
///
/// Unlike [`search`](crate::search::search), this is a single fail-closed
/// `io::Result`: properties is asked for one selected entry, and a stat failure
/// there (the file vanished, permission denied) is a real error the host should
/// surface, not swallow.
pub fn properties(dir: &Dir, rel: impl AsRef<Path>) -> io::Result<Properties> {
    let rel = rel.as_ref();
    let meta = dir.symlink_metadata(rel)?;
    let kind = kind_of(&meta.file_type());
    // Birth time is often unsupported; map the io::Result to an Option so a
    // missing btime degrades to None rather than failing the whole read.
    let created_unix = meta
        .created()
        .ok()
        .and_then(|t| t.into_std().duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64);
    let symlink_target = if kind == EntryKind::Symlink {
        dir.read_link(rel)
            .ok()
            .map(|p| p.to_string_lossy().into_owned())
    } else {
        None
    };
    Ok(Properties {
        kind,
        size: meta.size(),
        mode: meta.mode(),
        modified_unix: meta.mtime(),
        changed_unix: meta.ctime(),
        accessed_unix: meta.atime(),
        created_unix,
        uid: meta.uid(),
        gid: meta.gid(),
        nlink: meta.nlink(),
        symlink_target,
    })
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
            full_path: None,
            restore_token: None,
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
    fn type_sort_groups_by_extension_with_folders_first_and_a_name_tiebreak() {
        let mut v = vec![
            entry("b.txt", EntryKind::File, Some(1), None),
            entry("photo.png", EntryKind::File, Some(1), None),
            entry("dir", EntryKind::Directory, None, None),
            entry("a.txt", EntryKind::File, Some(1), None),
            entry("code.rs", EntryKind::File, Some(1), None),
        ];
        sort_entries(&mut v, SortKey::Type, true, true);
        let names: Vec<&str> = v.iter().map(|e| e.name.as_str()).collect();
        // Folder first, then by extension (png < rs < txt), and a.txt before
        // b.txt within the .txt group via the name tiebreak.
        assert_eq!(names, vec!["dir", "photo.png", "code.rs", "a.txt", "b.txt"]);
    }

    #[test]
    fn ext_key_follows_the_dotfile_convention() {
        assert_eq!(ext_key("photo.PNG"), "png", "extension is lowercased");
        assert_eq!(ext_key("archive.tar.gz"), "gz", "last dot wins");
        assert_eq!(ext_key("README"), "", "no dot is no extension");
        assert_eq!(ext_key(".bashrc"), "", "a leading-dot dotfile has no extension");
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

    #[test]
    fn properties_reads_a_files_mode_size_and_timestamps() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("doc.txt");
        std::fs::write(&path, b"hello").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();

        let dir = Dir::open_ambient_dir(tmp.path(), ambient_authority()).unwrap();
        let p = properties(&dir, "doc.txt").unwrap();
        assert_eq!(p.kind, EntryKind::File);
        assert_eq!(p.size, 5);
        // The low 12 bits are the chmod'd permission bits.
        assert_eq!(p.mode & 0o7777, 0o644);
        assert!(p.modified_unix > 0, "mtime is a sane epoch value");
        assert!(p.changed_unix > 0);
        assert!(p.accessed_unix > 0);
        assert!(p.nlink >= 1);
        assert!(p.symlink_target.is_none());
        // btime support is platform-dependent: assert the call succeeded, not a
        // particular value.
        let _ = p.created_unix;
    }

    #[test]
    fn properties_reports_a_directory_kind() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("sub")).unwrap();
        let dir = Dir::open_ambient_dir(tmp.path(), ambient_authority()).unwrap();
        let p = properties(&dir, "sub").unwrap();
        assert_eq!(p.kind, EntryKind::Directory);
    }

    #[test]
    fn properties_does_not_follow_a_symlink() {
        let tmp = tempfile::tempdir().unwrap();
        // A small link pointing at a much larger file: no-follow must report the
        // LINK's own (small) size and symlink kind, not the target's size.
        std::fs::write(tmp.path().join("big.bin"), vec![0u8; 4096]).unwrap();
        std::os::unix::fs::symlink("big.bin", tmp.path().join("link")).unwrap();

        let dir = Dir::open_ambient_dir(tmp.path(), ambient_authority()).unwrap();
        let p = properties(&dir, "link").unwrap();
        assert_eq!(p.kind, EntryKind::Symlink, "a symlink is reported as a symlink");
        assert_eq!(
            p.symlink_target.as_deref(),
            Some("big.bin"),
            "the raw link target is reported"
        );
        assert!(
            p.size < 4096,
            "no-follow reports the link's own size, not the target's"
        );
    }

    #[test]
    fn properties_cannot_escape_the_capability() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = Dir::open_ambient_dir(tmp.path(), ambient_authority()).unwrap();
        assert!(properties(&dir, "/etc/passwd").is_err());
        assert!(properties(&dir, "../../etc").is_err());
    }

    fn labels(groups: &[EntryGroup]) -> Vec<&str> {
        groups.iter().map(|g| g.label.as_str()).collect()
    }

    #[test]
    fn grouping_none_yields_a_single_unlabeled_group() {
        let v = vec![
            entry("a.txt", EntryKind::File, Some(1), Some(1)),
            entry("dir", EntryKind::Directory, None, Some(1)),
        ];
        let groups = group_entries(&v, GroupKey::None, 0);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].label, "");
        assert_eq!(groups[0].entries.len(), 2);
    }

    #[test]
    fn empty_input_yields_no_groups() {
        assert!(group_entries(&[], GroupKey::Kind, 0).is_empty());
    }

    #[test]
    fn grouping_by_kind_partitions_folders_files_and_links() {
        // Pre-sorted folders-first, so the groups appear Folders then Files.
        let v = vec![
            entry("dir", EntryKind::Directory, None, Some(1)),
            entry("a.txt", EntryKind::File, Some(1), Some(1)),
            entry("link", EntryKind::Symlink, None, Some(1)),
            entry("b.txt", EntryKind::File, Some(1), Some(1)),
        ];
        let groups = group_entries(&v, GroupKey::Kind, 0);
        assert_eq!(labels(&groups), vec!["Folders", "Files", "Links"]);
        // Files keep their input order within the group.
        assert_eq!(
            groups[1].entries.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
            vec!["a.txt", "b.txt"],
        );
    }

    #[test]
    fn grouping_by_type_uses_the_extension_with_folders_apart() {
        let v = vec![
            entry("dir", EntryKind::Directory, None, Some(1)),
            entry("main.rs", EntryKind::File, Some(1), Some(1)),
            entry("lib.rs", EntryKind::File, Some(1), Some(1)),
            entry("README", EntryKind::File, Some(1), Some(1)),
        ];
        let groups = group_entries(&v, GroupKey::Type, 0);
        assert_eq!(labels(&groups), vec!["Folders", "RS", "No extension"]);
        assert_eq!(groups[1].entries.len(), 2, "both .rs files share the RS group");
    }

    #[test]
    fn grouping_by_modified_buckets_by_elapsed_time() {
        const DAY: u64 = 86_400;
        let now = 100 * DAY;
        let v = vec![
            entry("today", EntryKind::File, Some(1), Some(now)),
            entry("yesterday", EntryKind::File, Some(1), Some(now - DAY - 1)),
            entry("thisweek", EntryKind::File, Some(1), Some(now - 3 * DAY)),
            entry("old", EntryKind::File, Some(1), Some(now - 60 * DAY)),
            entry("nostamp", EntryKind::File, Some(1), None),
        ];
        let groups = group_entries(&v, GroupKey::Modified, now);
        assert_eq!(
            labels(&groups),
            vec!["Today", "Yesterday", "Earlier this week", "Older", "Unknown"],
        );
    }

    #[test]
    fn grouping_by_size_buckets_files_and_keeps_folders_apart() {
        const MIB: u64 = 1 << 20;
        let v = vec![
            entry("dir", EntryKind::Directory, None, Some(1)),
            entry("empty", EntryKind::File, Some(0), Some(1)),
            entry("small", EntryKind::File, Some(1024), Some(1)),
            entry("medium", EntryKind::File, Some(50 * MIB), Some(1)),
            entry("nostat", EntryKind::File, None, Some(1)),
        ];
        let groups = group_entries(&v, GroupKey::Size, 0);
        assert_eq!(labels(&groups), vec!["Folders", "Empty", "Small", "Medium", "Unknown"]);
    }
}

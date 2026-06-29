//! Duplicate-file detection (the find-duplicates seam): a capability-confined,
//! symlink-cycle-safe walk that groups byte-identical regular files by content
//! hash, so the file manager can surface redundant copies.
//!
//! Like [`crate::search`], the scan root is a cap-std [`Dir`] capability the host
//! opens on the browsed folder; cap-std resolves every component against it and
//! refuses to escape it at the syscall, so the scan cannot leave the granted root
//! even through an escaping symlink (the descent is simply refused). The walk
//! classifies children WITHOUT following symlinks (a symlink is a leaf, never
//! descended), so a symlink to an ancestor cannot loop it; a `(dev, ino)` visited
//! set and a depth cap close hardlinked-directory and bind-mount cycles besides;
//! and the hash pass reads in bounded 64 KiB chunks, never `read_to_end` on an
//! untrusted file. The scan is read-only: it never mutates.
//!
//! Two passes keep the common case cheap. Pass one groups candidate files by SIZE
//! (a free stat): a file whose size is unique in the tree cannot have a byte
//! duplicate, so its content is never read. Pass two BLAKE3-hashes only the files
//! that share a size with another and groups them by digest; a digest shared by
//! more than one file is a set of byte-identical duplicates. Zero-byte files are
//! skipped (every empty file is trivially "identical", which is noise, not a
//! finding). The walk is bounded by a depth cap, an examined-entries cap, and a
//! total-bytes-hashed budget so a pathological tree cannot hang or exhaust memory;
//! the bounds are generous (a normal home never reaches them) and a scan that does
//! hit one reports it on the [`DupReport`] flags rather than silently truncating.

use std::collections::HashMap;
use std::collections::HashSet;
use std::io::Read as _;
use std::path::Path;

use cap_std::fs::{Dir, MetadataExt};
use serde::{Deserialize, Serialize};

use crate::{kind_of, EntryKind};

/// The maximum directory depth below the scan root (0 = the root only).
pub const MAX_DEPTH: usize = 16;

/// The cap on entries EXAMINED (statted) across the whole walk, so the scan's cost
/// is bounded by this and not only by the granted tree size. Once reached the walk
/// ends with [`DupReport::examined_capped`] set.
pub const MAX_ENTRIES_EXAMINED: usize = 1_000_000;

/// The total number of bytes the hash pass may read across the whole scan (8 GiB).
/// Once spent, no further candidate is hashed (so some duplicate groups may be
/// missed) and [`DupReport::hash_budget_exhausted`] is set. BLAKE3 is fast, so this
/// is generous; it exists to bound a pathological tree of many large same-size
/// files, not to limit a normal scan.
pub const MAX_TOTAL_HASH_BYTES: u64 = 8 * 1024 * 1024 * 1024;

/// The chunk size the hash pass reads in (64 KiB); the working buffer is this, the
/// file size never enters memory.
const HASH_CHUNK_BYTES: usize = 64 * 1024;

/// One file in a duplicate group: its capability-relative path plus the listing
/// fields the UI renders. `rel_path` is relative to the scan-root capability (e.g.
/// `"photos/a.jpg"`), the same form [`crate::search::SearchHit`] uses; the host
/// joins it to the browsed folder to produce the absolute path it acts on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DupFile {
    /// The file's path relative to the scan-root capability. Always within it.
    pub rel_path: String,
    /// The file's own name (no path).
    pub name: String,
    /// The file's size in bytes (every member of a group shares it).
    pub size: u64,
    /// Modification time, seconds since the Unix epoch, when available.
    pub modified_unix: Option<u64>,
}

/// A set of byte-identical files sharing one content hash. A group always has more
/// than one member (a lone file is not a duplicate).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DupGroup {
    /// The shared content digest, hex BLAKE3 of the file bytes.
    pub hash: String,
    /// The files with this digest, sorted by `rel_path` for a stable display.
    pub files: Vec<DupFile>,
}

/// The result of [`find_duplicates`]: the duplicate groups plus honest status
/// flags. Never an `io::Result`; a recursive walk over a real tree always meets
/// some unreadable entry, so per-entry errors are skipped (mirroring
/// [`crate::list_dir`]) and the reason the scan stopped short is reported here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct DupReport {
    /// The duplicate sets, sorted by each group's first file's `rel_path`.
    pub groups: Vec<DupGroup>,
    /// Set when the walk stopped because [`MAX_ENTRIES_EXAMINED`] was reached, so
    /// parts of the tree were not visited and more duplicates may exist.
    pub examined_capped: bool,
    /// Set when the hash budget [`MAX_TOTAL_HASH_BYTES`] was spent, so some
    /// same-size candidates were not hashed and their duplicates (if any) are not
    /// reported. Honest reporting, never a silent partial.
    pub hash_budget_exhausted: bool,
}

/// Find byte-identical regular files in the tree rooted at the capability `dir`,
/// grouped by content hash.
///
/// Pass one walks the tree (symlink-cycle-safe, depth- and work-capped) and buckets
/// regular non-empty files by size; pass two BLAKE3-hashes only the files that
/// share a size with another and groups them by digest. Groups with more than one
/// member are returned, sorted for a stable display. Per-entry read failures are
/// skipped, never fatal; see [`DupReport`] for the status flags.
pub fn find_duplicates(dir: &Dir) -> DupReport {
    let mut report = DupReport::default();

    // Pass one: bucket candidate files by size. A unique size cannot be a
    // duplicate, so those files are never read in pass two.
    let mut by_size: HashMap<u64, Vec<DupFile>> = HashMap::new();
    let mut visited: HashSet<(u64, u64)> = HashSet::new();
    if let Ok(meta) = dir.metadata(".") {
        // Record the root so a symlink back to it is detected as a cycle.
        visited.insert((meta.dev(), meta.ino()));
    }
    let mut examined: usize = 0;
    // The walk reads with `"."` for the root (cap-std rejects an empty path) while
    // keeping the rel-path prefix empty, so paths read `a/b/x` not `./a/b/x`.
    collect(
        dir,
        Path::new("."),
        Path::new(""),
        0,
        &mut visited,
        &mut examined,
        &mut by_size,
        &mut report,
    );

    // Pass two: hash only the size-buckets that hold more than one file, grouping
    // by digest. A bucket of one cannot contain a duplicate.
    let mut by_hash: HashMap<String, Vec<DupFile>> = HashMap::new();
    let mut hashed_total: u64 = 0;
    for (_size, mut bucket) in by_size {
        if bucket.len() < 2 {
            continue;
        }
        // Walk-order within a bucket is non-deterministic (HashMap); sort so the
        // final output is stable regardless of directory iteration order.
        bucket.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
        for file in bucket {
            if hashed_total.saturating_add(file.size) > MAX_TOTAL_HASH_BYTES {
                // The budget would be exceeded by this file: stop hashing. Already
                // accumulated groups still stand; the flag tells the user the scan
                // is not exhaustive.
                report.hash_budget_exhausted = true;
                break;
            }
            match hash_file(dir, &file.rel_path) {
                Some(hash) => {
                    hashed_total = hashed_total.saturating_add(file.size);
                    by_hash.entry(hash).or_default().push(file);
                }
                // An unreadable file (vanished, permission denied) is skipped, like
                // list_dir skips an unreadable entry. It costs no budget.
                None => {}
            }
        }
        if report.hash_budget_exhausted {
            break;
        }
    }

    // Emit only digests shared by more than one file, each group's files sorted by
    // path and the groups sorted by their first file's path for a stable display.
    let mut groups: Vec<DupGroup> = by_hash
        .into_iter()
        .filter(|(_, files)| files.len() > 1)
        .map(|(hash, mut files)| {
            files.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
            DupGroup { hash, files }
        })
        .collect();
    groups.sort_by(|a, b| a.files[0].rel_path.cmp(&b.files[0].rel_path));
    report.groups = groups;
    report
}

/// Walk one directory, bucketing regular non-empty files by size and descending
/// real subdirectories within the depth and cycle bounds. `read_path` is the path
/// to read RELATIVE to the root capability (`"."` at the root); `rel_prefix` is the
/// clean prefix used to build each file's `rel_path` (empty at the root).
#[allow(clippy::too_many_arguments)]
fn collect(
    root: &Dir,
    read_path: &Path,
    rel_prefix: &Path,
    depth: usize,
    visited: &mut HashSet<(u64, u64)>,
    examined: &mut usize,
    by_size: &mut HashMap<u64, Vec<DupFile>>,
    report: &mut DupReport,
) {
    if report.examined_capped {
        return;
    }
    let Ok(read_dir) = root.read_dir(read_path) else {
        // An unreadable directory (permission denied, vanished) is skipped.
        return;
    };
    for entry in read_dir {
        // Bound the walk's cost: count every entry examined and stop at the cap so
        // a huge tree cannot stat without end.
        *examined += 1;
        if *examined > MAX_ENTRIES_EXAMINED {
            report.examined_capped = true;
            return;
        }
        let Ok(entry) = entry else { continue };
        let name = entry.file_name().to_string_lossy().into_owned();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let kind = kind_of(&file_type);
        let child_read = read_path.join(&name);
        let child_rel = rel_prefix.join(&name);

        match kind {
            EntryKind::File => {
                // The entry's own metadata (no-follow), as list_dir reads it.
                let Ok(meta) = root.symlink_metadata(&child_read) else {
                    continue;
                };
                let size = meta.len();
                // Skip zero-byte files: every empty file is trivially identical,
                // which is noise rather than a useful finding.
                if size == 0 {
                    continue;
                }
                let modified_unix = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.into_std().duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs());
                by_size.entry(size).or_default().push(DupFile {
                    rel_path: child_rel.to_string_lossy().into_owned(),
                    name,
                    size,
                    modified_unix,
                });
            }
            EntryKind::Directory if depth < MAX_DEPTH => {
                // Descend only real directories within the depth bound. A symlink
                // (even to a directory) is a leaf, so a symlink-to-ancestor cannot
                // loop the walk; the `(dev, ino)` guard refuses re-descending a
                // directory already seen (a hardlink/bind-mount loop), and a
                // directory whose metadata we cannot read is not descended (fail
                // closed: never risk a cycle we cannot track).
                let Ok(meta) = root.symlink_metadata(&child_read) else {
                    continue;
                };
                if !visited.insert((meta.dev(), meta.ino())) {
                    continue;
                }
                collect(
                    root, &child_read, &child_rel, depth + 1, visited, examined, by_size, report,
                );
                if report.examined_capped {
                    return;
                }
            }
            // Symlinks, deeper-than-cap directories, and special files are leaves.
            _ => {}
        }
    }
}

/// BLAKE3-hash a regular file at `rel`, reading it through the capability in
/// bounded chunks. Returns the hex digest, or `None` if the file cannot be opened
/// or a read fails mid-file (the file is then simply not grouped).
fn hash_file(root: &Dir, rel: &str) -> Option<String> {
    let mut file = root.open(rel).ok()?;
    let mut hasher = blake3::Hasher::new();
    let mut buf = vec![0u8; HASH_CHUNK_BYTES];
    loop {
        match file.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                hasher.update(&buf[..n]);
            }
            // A mid-file read error abandons this file (it is not grouped); never
            // returns a partial hash, which would false-match.
            Err(_) => return None,
        }
    }
    Some(hasher.finalize().to_hex().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use cap_std::ambient_authority;
    use std::fs;
    use std::path::Path as StdPath;

    fn cap(path: &StdPath) -> Dir {
        Dir::open_ambient_dir(path, ambient_authority()).unwrap()
    }

    /// The set of rel_paths in a report's groups, flattened and sorted.
    fn paths(report: &DupReport) -> Vec<String> {
        let mut v: Vec<String> = report
            .groups
            .iter()
            .flat_map(|g| g.files.iter().map(|f| f.rel_path.clone()))
            .collect();
        v.sort();
        v
    }

    #[test]
    fn groups_byte_identical_files_across_subdirectories() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("a/b")).unwrap();
        fs::write(tmp.path().join("a/one.txt"), b"identical body").unwrap();
        fs::write(tmp.path().join("a/b/two.txt"), b"identical body").unwrap();
        fs::write(tmp.path().join("unique.txt"), b"a different body").unwrap();

        let out = find_duplicates(&cap(tmp.path()));
        assert_eq!(out.groups.len(), 1, "one duplicate set");
        assert_eq!(out.groups[0].files.len(), 2);
        assert_eq!(paths(&out), vec!["a/b/two.txt", "a/one.txt"]);
        // The hash is a real BLAKE3 hex digest (64 hex chars).
        assert_eq!(out.groups[0].hash.len(), 64);
        assert!(out.groups[0].hash.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn same_size_but_different_content_is_not_a_duplicate() {
        let tmp = tempfile::tempdir().unwrap();
        // Two files of equal length but differing bytes: same size bucket, distinct
        // hashes, so NOT a group (the size pre-filter must not over-report).
        fs::write(tmp.path().join("x.bin"), b"AAAAAAAA").unwrap();
        fs::write(tmp.path().join("y.bin"), b"BBBBBBBB").unwrap();

        let out = find_duplicates(&cap(tmp.path()));
        assert!(out.groups.is_empty());
    }

    #[test]
    fn three_copies_form_one_group_with_three_members() {
        let tmp = tempfile::tempdir().unwrap();
        for name in ["p.dat", "q.dat", "r.dat"] {
            fs::write(tmp.path().join(name), b"triplicate").unwrap();
        }
        fs::write(tmp.path().join("lone.dat"), b"alone here").unwrap();

        let out = find_duplicates(&cap(tmp.path()));
        assert_eq!(out.groups.len(), 1);
        assert_eq!(out.groups[0].files.len(), 3);
        assert_eq!(
            out.groups[0]
                .files
                .iter()
                .map(|f| f.name.clone())
                .collect::<Vec<_>>(),
            vec!["p.dat", "q.dat", "r.dat"],
            "files sorted by path for a stable display",
        );
    }

    #[test]
    fn zero_byte_files_are_not_grouped() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("empty1"), b"").unwrap();
        fs::write(tmp.path().join("empty2"), b"").unwrap();
        let out = find_duplicates(&cap(tmp.path()));
        assert!(out.groups.is_empty(), "empty files are noise, not a finding");
    }

    #[test]
    fn a_unique_file_is_never_a_group() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("only.txt"), b"singleton").unwrap();
        let out = find_duplicates(&cap(tmp.path()));
        assert!(out.groups.is_empty());
    }

    #[test]
    fn carries_size_and_mtime_on_each_member() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("a"), b"twelve bytes").unwrap();
        fs::write(tmp.path().join("b"), b"twelve bytes").unwrap();
        let out = find_duplicates(&cap(tmp.path()));
        assert_eq!(out.groups.len(), 1);
        for f in &out.groups[0].files {
            assert_eq!(f.size, 12);
            assert!(f.modified_unix.is_some(), "mtime is reported when available");
        }
    }

    #[test]
    fn a_symlink_cycle_to_an_ancestor_does_not_hang() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("d")).unwrap();
        fs::write(tmp.path().join("d/copy1"), b"looped body").unwrap();
        fs::write(tmp.path().join("copy2"), b"looped body").unwrap();
        // A link inside `d` pointing back at the root: a classic cycle. It is a
        // leaf in the walk, so the scan terminates with a bounded result.
        std::os::unix::fs::symlink(tmp.path(), tmp.path().join("d/loop")).unwrap();

        let out = find_duplicates(&cap(tmp.path()));
        // The two real copies still group; the cycle did not hang and the symlink
        // was never followed (its target's files are not double-counted).
        assert_eq!(out.groups.len(), 1);
        assert_eq!(paths(&out), vec!["copy2", "d/copy1"]);
    }

    #[test]
    fn an_escaping_symlink_is_not_followed() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("inside1"), b"shared").unwrap();
        fs::write(tmp.path().join("inside2"), b"shared").unwrap();
        // A symlink to an absolute outside path is a leaf, never descended.
        std::os::unix::fs::symlink("/etc", tmp.path().join("escape")).unwrap();

        let out = find_duplicates(&cap(tmp.path()));
        assert_eq!(out.groups.len(), 1);
        assert_eq!(paths(&out), vec!["inside1", "inside2"]);
    }
}

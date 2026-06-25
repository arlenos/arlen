//! Recursive file search (FM-R3): a bounded, symlink-cycle-safe, capability-
//! confined walk that the file manager's search bar drives
//! (`file-manager-plan.md` FM-R3).
//!
//! Like [`crate::list_dir`], the search root is a cap-std [`Dir`] capability the
//! host opens on the browsed folder; cap-std resolves every component against it
//! and refuses to escape it at the syscall, so the walk cannot leave the granted
//! root even if the tree contains an escaping symlink (the descent is simply
//! refused, surfacing as a skipped entry). The walk classifies every child
//! WITHOUT following symlinks (a symlink is a leaf, never descended), so a
//! symlink pointing at an ancestor cannot loop it; a `(dev, ino)` visited set and
//! a depth cap close hardlinked-directory and bind-mount cycles besides; and a
//! result cap plus per-file and total content-byte budgets keep a hostile wide
//! tree or a huge file from hanging or exhausting memory.
//!
//! The search is read-only: it never mutates, and the optional content grep reads
//! through the capability in bounded chunks, never `read_to_end` on an untrusted
//! file. It returns a [`SearchOutcome`] (hits plus honest status flags), never an
//! `io::Result` at the top level: a recursive walk over a real tree always meets
//! some unreadable entry, and aborting the whole search on the first one is wrong
//! for a file manager, so per-entry errors are skipped (mirroring `list_dir`) and
//! the reason the walk stopped is reported as a flag.

use std::collections::HashSet;
use std::io::Read as _;
use std::path::Path;

use cap_std::fs::{Dir, MetadataExt};
use serde::{Deserialize, Serialize};

use crate::{kind_of, EntryKind, FileEntry};

/// The default maximum directory depth below the search root (0 = the root only).
pub const DEFAULT_MAX_DEPTH: usize = 16;

/// The default cap on results before the walk ends [`SearchOutcome::truncated`].
pub const DEFAULT_MAX_RESULTS: usize = 1000;

/// The default cap on entries EXAMINED (statted) across the whole walk. The
/// result cap bounds hits, not work: a query that matches nothing would
/// otherwise stat every entry in the granted tree up to the depth cap. This
/// bounds the walk's cost by the options, not only by the on-disk tree size;
/// once reached the walk ends with [`SearchOutcome::examined_capped`] set. The
/// default is generous (a large home rarely reaches it); a host wanting a
/// snappier interactive search lowers it and pairs it with a cancel flag.
pub const DEFAULT_MAX_ENTRIES_EXAMINED: usize = 1_000_000;

/// The default per-file byte cap for the content grep (8 MiB): a larger file is
/// name-matched but its content is not read.
pub const DEFAULT_MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;

/// The default total byte budget the content grep may read across the whole walk
/// (256 MiB): once spent, later files are name-matched only.
pub const DEFAULT_MAX_TOTAL_CONTENT_BYTES: u64 = 256 * 1024 * 1024;

/// The chunk size the content grep reads in (64 KiB). The working buffer is
/// bounded by `query.len() + CONTENT_CHUNK_BYTES` (the file size never enters it):
/// a sliding window keeps only the query-length overlap between chunks so a match
/// straddling a chunk boundary is still found. The query is host-supplied (the
/// search box), so in practice the buffer is essentially the chunk size.
const CONTENT_CHUNK_BYTES: usize = 64 * 1024;

/// How a search query is matched against an entry. The host wires the search
/// bar's "name / content / both" choice straight onto these independent flags.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchOptions {
    /// The substring to find, matched case-insensitively. An empty query matches
    /// nothing (a name-only search yields no hits; a content-only search greps
    /// for nothing), so a both-empty search returns an empty outcome.
    pub query: String,
    /// Match entry NAMES by case-insensitive substring (the default layer).
    pub match_names: bool,
    /// ALSO match file CONTENT by a bounded byte-substring scan. Off by default:
    /// content matching reads file bytes, while the conventional file-manager
    /// default is a name search.
    pub match_content: bool,
    /// Maximum directory depth below the root. 0 lists the root only and never
    /// descends; the primary, always-effective descent bound.
    pub max_depth: usize,
    /// Stop after this many results; the walk then ends with
    /// [`SearchOutcome::truncated`] set (more may exist).
    pub max_results: usize,
    /// Stop after examining (statting) this many entries, so the walk's cost is
    /// bounded by the options and not only by the granted tree size. A no-match
    /// query would otherwise stat the whole tree; once this is reached the walk
    /// ends with [`SearchOutcome::examined_capped`] set.
    pub max_entries_examined: usize,
    /// Per-file byte cap for the content grep. A file larger than this is
    /// name-matched but its content is NOT read, and
    /// [`SearchOutcome::content_budget_exhausted`] is set.
    pub max_file_bytes: u64,
    /// Total bytes the content grep may read across the whole walk. Once spent,
    /// no further content is read (names still match) and
    /// [`SearchOutcome::content_budget_exhausted`] is set.
    pub max_total_content_bytes: u64,
    /// Skip dotfiles and dot-directories (and do not descend into them).
    pub skip_hidden: bool,
}

impl Default for SearchOptions {
    /// Name-only search with the sane default bounds. The host overrides any
    /// field per call; these are just safe fallbacks.
    fn default() -> Self {
        Self {
            query: String::new(),
            match_names: true,
            match_content: false,
            max_depth: DEFAULT_MAX_DEPTH,
            max_results: DEFAULT_MAX_RESULTS,
            max_entries_examined: DEFAULT_MAX_ENTRIES_EXAMINED,
            max_file_bytes: DEFAULT_MAX_FILE_BYTES,
            max_total_content_bytes: DEFAULT_MAX_TOTAL_CONTENT_BYTES,
            skip_hidden: false,
        }
    }
}

/// Why an entry matched the query, so the host can badge a content hit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MatchKind {
    /// The entry's name matched.
    Name,
    /// The file's content matched (its name did not).
    Content,
    /// Both the name and the content matched.
    Both,
}

/// One search result: where it is, what it is, and why it matched.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchHit {
    /// The hit's path RELATIVE to the search-root capability (e.g.
    /// `"src/lib.rs"`), the same relative form `list_dir`/`ops` take; the host
    /// joins it to the displayed root to navigate. Always within the capability.
    pub rel_path: String,
    /// The matched entry as a [`FileEntry`] (kind, size, mtime, hidden, readonly,
    /// symlink target), so a hit renders as an ordinary file row.
    pub entry: FileEntry,
    /// Whether the name, the content, or both matched.
    pub matched: MatchKind,
}

/// The result of a [`search`]: the hits plus honest status flags. Never an
/// `io::Result`; per-entry errors are skipped, and the reason the walk stopped is
/// reported here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SearchOutcome {
    /// The matched entries, in walk order (the host sorts for display).
    pub hits: Vec<SearchHit>,
    /// Set when the walk stopped because `max_results` was reached, so more
    /// matches may exist beyond what is reported.
    pub truncated: bool,
    /// Set when at least one file's content was not read because it exceeded
    /// `max_file_bytes` or the total budget was spent, so the content results are
    /// not exhaustive. Honest reporting, never a silent partial.
    pub content_budget_exhausted: bool,
    /// Set when the walk stopped because `max_entries_examined` was reached, so
    /// parts of the tree were not visited and more matches may exist. Distinct
    /// from `truncated` (which is about the result cap): this is the work cap.
    pub examined_capped: bool,
}

/// Search the tree rooted at the capability `dir`, returning every entry whose
/// name and/or content matches `opts.query` (case-insensitively), bounded by the
/// depth, result and content-byte limits in `opts`.
///
/// The walk is symlink-cycle-safe (symlinks are leaves, never descended; a
/// `(dev, ino)` visited set and the depth cap close hardlink/bind-mount cycles),
/// capability-confined (an escaping symlink's descent is refused by cap-std, the
/// entry skipped), and bounded (a result cap plus per-file and total content-byte
/// budgets). Per-entry read failures are skipped, never fatal; see
/// [`SearchOutcome`] for the status flags.
pub fn search(dir: &Dir, opts: &SearchOptions) -> SearchOutcome {
    let query_lower = opts.query.to_lowercase();
    let mut state = WalkState {
        opts,
        query_lower: &query_lower,
        // The query bytes for the content grep are ASCII-lowercased (a full
        // Unicode-case content scan is out of scope; documented limit).
        query_content: query_lower.as_bytes().to_ascii_lowercase(),
        visited: HashSet::new(),
        content_spent: 0,
        examined: 0,
        outcome: SearchOutcome::default(),
    };
    // The root itself is recorded as visited so a symlink back to it is detected.
    if let Ok(meta) = dir.metadata(".") {
        state.visited.insert((meta.dev(), meta.ino()));
    }
    // The walk reads with `"."` for the root (cap-std does not accept an empty
    // path) while keeping the rel-path prefix empty, so hits read `a/b/x` not
    // `./a/b/x`.
    walk(dir, Path::new("."), Path::new(""), 0, &mut state);
    state.outcome
}

/// The mutable state threaded through the recursion: the options, the lowered
/// query forms, the visited `(dev, ino)` set, the content bytes spent so far, and
/// the accumulating outcome.
struct WalkState<'a> {
    opts: &'a SearchOptions,
    query_lower: &'a str,
    query_content: Vec<u8>,
    visited: HashSet<(u64, u64)>,
    content_spent: u64,
    examined: usize,
    outcome: SearchOutcome,
}

/// Whether the walk should stop and unwind: the result cap was reached (sets
/// [`SearchOutcome::truncated`]) or the work cap was reached (the loop set
/// [`SearchOutcome::examined_capped`] before returning). Checked at every
/// early-return point so a stop propagates up through the recursion.
fn should_stop(state: &mut WalkState) -> bool {
    if state.outcome.hits.len() >= state.opts.max_results {
        state.outcome.truncated = true;
        return true;
    }
    state.outcome.examined_capped
}

/// Walk one directory at `depth`, matching each child and descending real
/// subdirectories within the depth and cycle bounds. `read_path` is the path to
/// read RELATIVE to the root capability (`"."` at the root, since cap-std rejects
/// an empty path); `rel_prefix` is the clean prefix used to build each hit's
/// `rel_path` (empty at the root, so hits read `a/b/x` not `./a/b/x`). Returns
/// early (and the caller stops) once the result cap is hit.
fn walk(root: &Dir, read_path: &Path, rel_prefix: &Path, depth: usize, state: &mut WalkState) {
    let Ok(read_dir) = root.read_dir(read_path) else {
        // An unreadable directory (permission denied, vanished) is skipped, like
        // list_dir skips an unreadable entry.
        return;
    };
    for entry in read_dir {
        if should_stop(state) {
            return;
        }
        // Count every entry the walk examines and stop at the work cap, so a
        // query that matches no name cannot stat the whole granted tree.
        state.examined += 1;
        if state.examined > state.opts.max_entries_examined {
            state.outcome.examined_capped = true;
            return;
        }
        let Ok(entry) = entry else { continue };
        let name = entry.file_name().to_string_lossy().into_owned();
        if state.opts.skip_hidden && name.starts_with('.') {
            continue;
        }
        let Ok(file_type) = entry.file_type() else { continue };
        let kind = kind_of(&file_type);
        // The capability-relative path to read this child by, and the clean
        // display path. They differ only by the root's `"."`-vs-empty start.
        let child_read = read_path.join(&name);
        let child_rel = rel_prefix.join(&name);

        // Match this entry (name and/or content) and record a hit if it matches.
        match_entry(root, &child_read, &child_rel, &name, kind, state);
        if should_stop(state) {
            return;
        }

        // Descend only real directories, and only within the depth bound. A
        // symlink (even to a directory) is a leaf, so a symlink-to-ancestor
        // cannot loop the walk. cap-std refuses an escaping descent at the
        // syscall, surfacing as the skipped open_dir below.
        if kind == EntryKind::Directory && depth < state.opts.max_depth {
            // The `(dev, ino)` guard refuses re-descending a directory already
            // seen (a hardlinked directory or a bind-mount loop), and a directory
            // whose metadata we cannot read is not descended (fail closed: never
            // risk a cycle we cannot track).
            let Ok(meta) = root.symlink_metadata(&child_read) else {
                continue;
            };
            if !state.visited.insert((meta.dev(), meta.ino())) {
                continue;
            }
            walk(root, &child_read, &child_rel, depth + 1, state);
            if should_stop(state) {
                return;
            }
        }
    }
}

/// Test one entry against the query and push a [`SearchHit`] if it matches.
/// `read_path` is the capability-relative path to stat/grep the entry by;
/// `rel_path` is the clean path recorded on the hit. The content grep runs only
/// for regular files, only when `match_content` is on, and only within the
/// per-file and total byte budgets.
fn match_entry(
    root: &Dir,
    read_path: &Path,
    rel_path: &Path,
    name: &str,
    kind: EntryKind,
    state: &mut WalkState,
) {
    let name_matched = state.opts.match_names
        && !state.query_lower.is_empty()
        && name.to_lowercase().contains(state.query_lower);

    // Read the entry's own metadata (no-follow), as list_dir does, to build the
    // FileEntry. A stat failure leaves the fields None rather than dropping a hit.
    let meta = root.symlink_metadata(read_path).ok();

    let content_matched = if state.opts.match_content
        && kind == EntryKind::File
        && !state.query_content.is_empty()
    {
        let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
        grep_file(root, read_path, size, state)
    } else {
        false
    };

    let matched = match (name_matched, content_matched) {
        (true, true) => Some(MatchKind::Both),
        (true, false) => Some(MatchKind::Name),
        (false, true) => Some(MatchKind::Content),
        (false, false) => None,
    };
    let Some(matched) = matched else { return };

    let size = meta
        .as_ref()
        .and_then(|m| if m.is_dir() { None } else { Some(m.len()) });
    let modified_unix = meta
        .as_ref()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.into_std().duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs());
    let readonly = meta
        .as_ref()
        .map(|m| m.permissions().readonly())
        .unwrap_or(false);
    let symlink_target = if kind == EntryKind::Symlink {
        root.read_link(read_path)
            .ok()
            .map(|p| p.to_string_lossy().into_owned())
    } else {
        None
    };

    state.outcome.hits.push(SearchHit {
        rel_path: rel_path.to_string_lossy().into_owned(),
        entry: FileEntry {
            is_hidden: name.starts_with('.'),
            name: name.to_string(),
            kind,
            size,
            modified_unix,
            readonly,
            symlink_target,
            full_path: None,
        },
        matched,
    });
}

/// Scan a regular file at `rel` for the (ASCII-lowercased) query bytes, reading
/// at most `max_file_bytes` of it in bounded chunks and respecting the running
/// total budget. Returns whether the substring was found. A file too large for
/// the per-file cap, or one reached after the total budget is spent, is not read
/// and sets [`SearchOutcome::content_budget_exhausted`].
fn grep_file(root: &Dir, rel: &Path, size: u64, state: &mut WalkState) -> bool {
    // A file larger than the per-file cap is not read: name-matched only.
    if size > state.opts.max_file_bytes {
        state.outcome.content_budget_exhausted = true;
        return false;
    }
    // The running total budget is spent: read no further content.
    if state.content_spent >= state.opts.max_total_content_bytes {
        state.outcome.content_budget_exhausted = true;
        return false;
    }
    let remaining_total = state.opts.max_total_content_bytes - state.content_spent;
    // Read at most the per-file cap, and no more than the total budget leaves.
    let allowance = state.opts.max_file_bytes.min(remaining_total);

    let Ok(file) = root.open(rel) else {
        return false;
    };
    let mut reader = file.take(allowance);
    let needle = &state.query_content;
    let overlap = needle.len().saturating_sub(1);

    // A sliding window: keep the last `needle.len() - 1` bytes of the previous
    // chunk so a match straddling a chunk boundary is still found. The scan is
    // ASCII-case-insensitive (the conventional content-grep fold).
    let mut window: Vec<u8> = Vec::with_capacity(CONTENT_CHUNK_BYTES + overlap);
    let mut chunk = vec![0u8; CONTENT_CHUNK_BYTES];
    let mut read_total: u64 = 0;
    loop {
        let n = match reader.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => n,
            // A mid-file read error ends this file's scan; the entry is simply
            // not a content match (it may still be a name match).
            Err(_) => break,
        };
        read_total += n as u64;
        window.extend(chunk[..n].iter().map(|b| b.to_ascii_lowercase()));
        if contains_subslice(&window, needle) {
            state.content_spent += read_total;
            return true;
        }
        // Retain only the boundary overlap so the buffer stays bounded.
        if window.len() > overlap {
            let drop = window.len() - overlap;
            window.drain(..drop);
        }
    }
    state.content_spent += read_total;
    // If the file was truncated at the allowance, the content view is partial.
    if read_total >= allowance && size > read_total {
        state.outcome.content_budget_exhausted = true;
    }
    false
}

/// Whether `haystack` contains `needle` as a contiguous subslice. `needle` is
/// never empty here (the caller guards it).
fn contains_subslice(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.len() > haystack.len() {
        return false;
    }
    haystack
        .windows(needle.len())
        .any(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cap_std::ambient_authority;
    use std::fs;
    use std::path::Path as StdPath;

    /// Open `path` as a cap-std capability rooted there.
    fn cap(path: &StdPath) -> Dir {
        Dir::open_ambient_dir(path, ambient_authority()).unwrap()
    }

    /// A name-only search options with the given query.
    fn name_search(query: &str) -> SearchOptions {
        SearchOptions {
            query: query.to_string(),
            ..Default::default()
        }
    }

    fn hit_paths(outcome: &SearchOutcome) -> Vec<String> {
        outcome.hits.iter().map(|h| h.rel_path.clone()).collect()
    }

    #[test]
    fn finds_a_name_match_recursively_case_insensitively() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("a/b")).unwrap();
        fs::write(tmp.path().join("a/b/Match.txt"), b"irrelevant").unwrap();
        fs::write(tmp.path().join("a/other.txt"), b"irrelevant").unwrap();

        let dir = cap(tmp.path());
        let out = search(&dir, &name_search("match"));
        assert_eq!(hit_paths(&out), vec!["a/b/Match.txt".to_string()]);
        assert_eq!(out.hits[0].matched, MatchKind::Name);
        assert!(!out.truncated);
    }

    #[test]
    fn finds_a_content_match_and_reports_both_when_the_name_also_matches() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("src")).unwrap();
        fs::write(tmp.path().join("src/notes.txt"), b"the needle is here").unwrap();
        fs::write(tmp.path().join("src/needle.rs"), b"unrelated body").unwrap();

        let dir = cap(tmp.path());
        let opts = SearchOptions {
            query: "needle".to_string(),
            match_names: true,
            match_content: true,
            ..Default::default()
        };
        let out = search(&dir, &opts);

        let by = |p: &str| out.hits.iter().find(|h| h.rel_path == p).cloned().unwrap();
        // notes.txt: content match only.
        assert_eq!(by("src/notes.txt").matched, MatchKind::Content);
        // needle.rs: name match only (body does not contain the query).
        assert_eq!(by("src/needle.rs").matched, MatchKind::Name);
        assert_eq!(out.hits.len(), 2);
    }

    #[test]
    fn content_only_search_ignores_name_matches() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("needle.txt"), b"nothing").unwrap();
        fs::write(tmp.path().join("plain.txt"), b"has needle inside").unwrap();

        let dir = cap(tmp.path());
        let opts = SearchOptions {
            query: "needle".to_string(),
            match_names: false,
            match_content: true,
            ..Default::default()
        };
        let out = search(&dir, &opts);
        assert_eq!(hit_paths(&out), vec!["plain.txt".to_string()]);
        assert_eq!(out.hits[0].matched, MatchKind::Content);
    }

    #[test]
    fn a_symlink_cycle_to_an_ancestor_does_not_hang() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("d")).unwrap();
        fs::write(tmp.path().join("d/target.txt"), b"x").unwrap();
        // A link inside `d` pointing back at the root: a classic cycle. It is a
        // leaf in the walk, so the search terminates with a bounded result.
        std::os::unix::fs::symlink(tmp.path(), tmp.path().join("d/loop")).unwrap();

        let dir = cap(tmp.path());
        let out = search(&dir, &name_search("target"));
        // The walk completes (no hang) and the link was never descended.
        assert_eq!(hit_paths(&out), vec!["d/target.txt".to_string()]);
        // The loop link itself is not a name match for "target".
        assert!(out.hits.iter().all(|h| h.entry.kind != EntryKind::Symlink));
        // This exercises the symlink-leaf protection (a symlink is never
        // descended). The `(dev, ino)` visited set is a second, defense-in-depth
        // guard against a hardlinked-directory / bind-mount loop, which a
        // privilege-free unit test cannot create, so it is not reached here.
    }

    #[test]
    fn max_entries_examined_bounds_the_walk_and_sets_examined_capped() {
        let tmp = tempfile::tempdir().unwrap();
        for i in 0..50 {
            fs::write(tmp.path().join(format!("f{i}.dat")), b"x").unwrap();
        }
        let dir = cap(tmp.path());
        // A query that matches NOTHING, so the result cap never trips; only the
        // work cap can stop the walk. With the cap below the entry count, the
        // walk stops early and reports it honestly.
        let opts = SearchOptions {
            query: "no-such-name".to_string(),
            max_entries_examined: 10,
            ..Default::default()
        };
        let out = search(&dir, &opts);
        assert!(out.hits.is_empty());
        assert!(!out.truncated, "no result cap was hit");
        assert!(out.examined_capped, "the work cap stopped the walk");
    }

    #[test]
    fn the_total_content_budget_is_honoured_across_files() {
        let tmp = tempfile::tempdir().unwrap();
        // Several files each holding the needle; the total budget is smaller than
        // their combined size, so some are left unread and the flag is set.
        for i in 0..6 {
            fs::write(tmp.path().join(format!("doc{i}.txt")), b"needle inside").unwrap();
        }
        let dir = cap(tmp.path());
        let opts = SearchOptions {
            query: "needle".to_string(),
            match_names: false,
            match_content: true,
            max_total_content_bytes: 20,
            ..Default::default()
        };
        let out = search(&dir, &opts);
        assert!(
            out.content_budget_exhausted,
            "the total content budget engaged across files",
        );
    }

    #[test]
    fn a_tree_deeper_than_max_depth_stops_descending() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("a/b/c")).unwrap();
        fs::write(tmp.path().join("a/b/c/buried.txt"), b"x").unwrap();
        fs::write(tmp.path().join("a/shallow.txt"), b"x").unwrap();
        let dir = cap(tmp.path());
        // max_depth 1 descends the root (depth 0) and `a` (depth 1) but not
        // `a/b`, so the depth-2 file is never reached.
        let opts = SearchOptions {
            query: ".txt".to_string(),
            max_depth: 1,
            ..Default::default()
        };
        let out = search(&dir, &opts);
        assert_eq!(hit_paths(&out), vec!["a/shallow.txt".to_string()]);
    }

    #[test]
    fn max_depth_zero_lists_only_the_root() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("sub")).unwrap();
        fs::write(tmp.path().join("sub/deep.txt"), b"x").unwrap();
        fs::write(tmp.path().join("top.txt"), b"x").unwrap();

        let dir = cap(tmp.path());
        let opts = SearchOptions {
            query: ".txt".to_string(),
            max_depth: 0,
            ..Default::default()
        };
        let out = search(&dir, &opts);
        // Only the root-level file matched; the subdir was not descended.
        assert_eq!(hit_paths(&out), vec!["top.txt".to_string()]);
    }

    #[test]
    fn max_results_bounds_the_walk_and_sets_truncated() {
        let tmp = tempfile::tempdir().unwrap();
        for i in 0..10 {
            fs::write(tmp.path().join(format!("file{i}.txt")), b"x").unwrap();
        }
        let dir = cap(tmp.path());
        let opts = SearchOptions {
            query: "file".to_string(),
            max_results: 3,
            ..Default::default()
        };
        let out = search(&dir, &opts);
        assert_eq!(out.hits.len(), 3);
        assert!(out.truncated, "hitting the result cap sets truncated");
    }

    #[test]
    fn a_huge_file_is_name_matched_but_its_content_is_not_read() {
        let tmp = tempfile::tempdir().unwrap();
        // A file larger than the per-file cap, whose only copy of the needle is
        // interior, so a content scan would have to read past the cap to find it.
        let cap_bytes: usize = 4096;
        let mut body = vec![b'.'; cap_bytes * 2];
        body.extend_from_slice(b"interiorneedle");
        fs::write(tmp.path().join("big.log"), &body).unwrap();

        let dir = cap(tmp.path());
        let opts = SearchOptions {
            query: "interiorneedle".to_string(),
            match_names: false,
            match_content: true,
            max_file_bytes: cap_bytes as u64,
            ..Default::default()
        };
        let out = search(&dir, &opts);
        // The interior needle is NOT found (the file exceeded the per-file cap),
        // and the partial-content flag is set.
        assert!(out.hits.is_empty(), "oversized content is not read");
        assert!(out.content_budget_exhausted);
    }

    #[test]
    fn content_match_straddling_a_chunk_boundary_is_found() {
        let tmp = tempfile::tempdir().unwrap();
        // Place the needle across the 64 KiB chunk boundary to exercise the
        // sliding-window overlap.
        let mut body = vec![b'a'; CONTENT_CHUNK_BYTES - 3];
        body.extend_from_slice(b"BOUNDARY");
        body.extend_from_slice(&[b'b'; 16]);
        fs::write(tmp.path().join("split.bin"), &body).unwrap();

        let dir = cap(tmp.path());
        let opts = SearchOptions {
            query: "boundary".to_string(),
            match_names: false,
            match_content: true,
            ..Default::default()
        };
        let out = search(&dir, &opts);
        assert_eq!(hit_paths(&out), vec!["split.bin".to_string()]);
    }

    #[test]
    fn cannot_escape_the_capability_and_an_escaping_symlink_is_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("inside.txt"), b"x").unwrap();
        // A symlink to an absolute outside path is a leaf, never descended, so
        // the walk still completes over the in-root entries.
        std::os::unix::fs::symlink("/etc", tmp.path().join("escape")).unwrap();

        let dir = cap(tmp.path());
        let out = search(&dir, &name_search("inside"));
        assert_eq!(hit_paths(&out), vec!["inside.txt".to_string()]);
    }

    #[test]
    fn skip_hidden_excludes_dot_entries_and_their_subtrees() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join(".git")).unwrap();
        fs::write(tmp.path().join(".git/config.txt"), b"x").unwrap();
        fs::write(tmp.path().join("visible.txt"), b"x").unwrap();

        let dir = cap(tmp.path());
        let opts = SearchOptions {
            query: ".txt".to_string(),
            skip_hidden: true,
            ..Default::default()
        };
        let out = search(&dir, &opts);
        // The dot-directory was neither matched nor descended.
        assert_eq!(hit_paths(&out), vec!["visible.txt".to_string()]);
    }

    #[test]
    fn an_empty_query_yields_no_hits() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("a.txt"), b"content").unwrap();
        let dir = cap(tmp.path());
        let opts = SearchOptions {
            query: String::new(),
            match_names: true,
            match_content: true,
            ..Default::default()
        };
        let out = search(&dir, &opts);
        assert!(out.hits.is_empty());
    }
}

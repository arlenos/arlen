//! The file-manager filesystem-operations core (FM-R2): the synchronous,
//! UI-agnostic primitives the Tauri backend drives to create, rename, copy,
//! move and trash entries (`file-manager-plan.md` §135-141, §154).
//!
//! Every mutation goes through a cap-std [`Dir`] capability with a path
//! *relative* to it, exactly like [`crate::list_dir`]: cap-std resolves each
//! component against the granted root and refuses to escape it at the syscall,
//! so a malicious `..`/absolute path is rejected by the kernel (surfacing as
//! [`OpError::Io`]), not by a string check here. The open capability is the
//! authority. The only absolute path this module handles is the original
//! location written into a `.trashinfo` file, which is *recorded text*, never a
//! path handed to a capability.
//!
//! Symlinks are never followed for classification, copy, move, trash or delete:
//! a copy recreates the link, a delete removes the link, never the target. The
//! recursive directory copy is hand-walked through the capabilities because
//! cap-std has no recursive copy and `Dir::copy` follows links (it is
//! `std::fs::copy`, which copies a file's *contents*).
//!
//! Progress reporting, cancellation, the FM op-undo log and drag-drop are the
//! host's layer built on these primitives (out of scope here, but the
//! per-entry signatures let the host drive a multi-selection entry by entry and
//! apply a conflict policy). The file manager never writes the Knowledge Graph
//! (plan §118): these are plain filesystem operations.
//!
//! Placement: these ops live in the file-browser core (the plan's backend lane,
//! "FS access + the ops + Trash") so the FM app drives them directly. Only
//! [`trash_entry`] pulls the heavier `time` + `percent-encoding` (the freedesktop
//! `.trashinfo` timestamp + path encoding); the rest need only cap-std (+ `libc`
//! for the cross-device `EXDEV` check). If a future confined xdg picker hosts
//! this crate and wants it leaner, gate the trash op + those two deps behind an
//! off-by-default `trash` feature so the picker (which never trashes) drops them;
//! the browse + new-folder + rename it needs stay dependency-light.
//!
//! Known edge (low): under [`ConflictPolicy::Rename`] a target name with invalid
//! UTF-8 bytes is matched against its lossily-decoded form when computing the
//! `(copy N)` suffix, so a pathological non-UTF-8 name could get a renamed target
//! that differs from the intended bytes. The copy itself is byte-faithful and
//! never panics or escapes; acceptable for an interactive file manager.

use std::ffi::OsString;
use std::io;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

use cap_std::fs::{Dir, OpenOptions};
use percent_encoding::{percent_decode, percent_encode, AsciiSet, NON_ALPHANUMERIC};
use std::io::Read as _;
use std::io::Write as _;
use time::OffsetDateTime;

use crate::{kind_of, EntryKind};

/// Why a filesystem operation failed. Every variant is a refusal: the op made
/// no change, EXCEPT a recursive copy/move that fails midway, which reports
/// [`OpError::Partial`] with the partial output it left behind (see the op
/// docs).
#[derive(Debug, thiserror::Error)]
pub enum OpError {
    /// The destination name already exists and the policy did not resolve it
    /// (returned for [`ConflictPolicy::Fail`], and for the create-exactly-this
    /// ops [`new_folder`]/[`rename`] which never auto-resolve a collision).
    #[error("destination already exists: {name}")]
    AlreadyExists {
        /// The colliding name.
        name: String,
    },

    /// A user-typed name was empty, a path separator, or `.`/`..`, which would
    /// not name a single child. Refused before any syscall so a bad name can
    /// neither traverse nor land two levels down.
    #[error("invalid name: {name:?}")]
    InvalidName {
        /// The rejected name.
        name: String,
    },

    /// A recursive copy or move failed partway. `written` is the relative path,
    /// under the destination capability, of the partial tree left behind (the
    /// caller decides whether to roll it back). The source is untouched: a copy
    /// never mutates the source, and a move never reaches its delete step.
    #[error("operation incomplete, partial output at {written:?}: {source}")]
    Partial {
        /// The destination-relative root of the partial tree left behind.
        written: PathBuf,
        /// The underlying error that interrupted the operation.
        #[source]
        source: io::Error,
    },

    /// Any other underlying filesystem error (permission, ENOENT, ENOSPC, a
    /// cap-std escape refusal, a type clash, etc.). The op made no change.
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// The result of a filesystem operation in this module.
pub type OpResult<T> = Result<T, OpError>;

/// What to do when a copy/move target name already exists in the destination.
/// The host picks this per op (or per entry for a batch, applying its own
/// "apply to all"); this module just executes the chosen policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictPolicy {
    /// Leave the existing target untouched and skip this entry. The op reports
    /// [`OpOutcome::Skipped`].
    Skip,
    /// Overwrite the existing target. A file replaces a file; a directory
    /// merges into an existing directory (per child, recursively, each child
    /// re-evaluated under this same policy). A file-vs-directory type clash is
    /// refused (`OpError::Io`, the kernel's EISDIR/ENOTDIR) rather than blindly
    /// removing the other kind.
    Replace,
    /// Keep the existing target and write this one under a freed-up name
    /// (`foo.txt` -> `foo (copy).txt` -> `foo (copy 2).txt` ...). The op reports
    /// the final renamed path.
    Rename,
    /// Make the conflict a hard error ([`OpError::AlreadyExists`]). The default
    /// a host uses when it has not yet asked the user.
    Fail,
}

/// The outcome of one conflict check at a single target path.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ConflictResolution {
    /// No conflict (or [`ConflictPolicy::Replace`]): write to this exact path.
    Proceed,
    /// [`ConflictPolicy::Skip`]: do nothing, report skipped.
    Skip,
    /// [`ConflictPolicy::Rename`]: write to this freed-up relative path instead.
    UseName(PathBuf),
}

/// What an operation did, so the host can update its model and op-undo log
/// without re-statting. `target` is always the FINAL relative path under the
/// destination capability (it differs from the requested name when the policy
/// renamed it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpOutcome {
    /// A new entry was created at `target`.
    Created {
        /// The destination-relative path of the new entry.
        target: PathBuf,
    },
    /// The entry was renamed/moved to `target` in place (rename op, or a move).
    Renamed {
        /// The destination-relative path of the renamed/moved entry.
        target: PathBuf,
    },
    /// The conflict policy skipped this entry; nothing changed.
    Skipped,
}

/// The result of trashing an entry: the final name used inside the trash
/// (after any collision suffix), enough for the host to drive restore/undo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrashedEntry {
    /// The basename under `<Trash>/files/<name>` and
    /// `<Trash>/info/<name>.trashinfo` (the two always share this name, per the
    /// trash-spec pairing invariant).
    pub trashed_name: String,
}

/// The parsed contents of one `<Trash>/info/<name>.trashinfo`: the original
/// location the entry was deleted from (the recorded `Path=`, percent-DECODED)
/// and the recorded deletion time.
///
/// `original_path` is RECORDED DATA, not a capability target: it is an absolute
/// path written by whoever trashed the entry (untrusted - a crafted info file
/// could record `/etc/passwd`). The host uses it to DISPLAY the prior location
/// and to drive its OWN trust-checked resolution into a [`restore_entry`]
/// destination capability + relative path; this module never feeds it to a
/// cap-std method. See [`restore_entry`] for the division.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrashInfo {
    /// The percent-decoded original absolute path from `Path=`. A `PathBuf`
    /// (not a `String`) because a trashed name may carry non-UTF-8 bytes that
    /// percent-decode to a non-UTF-8 path; `OsString`/`PathBuf` is lossless,
    /// a `String` would not be.
    pub original_path: PathBuf,
    /// The recorded `DeletionDate=` verbatim (the spec `YYYY-MM-DDThh:mm:ss`
    /// local-time string), kept as-is for display/sorting. Not parsed to a
    /// timestamp here: the Trash UI displays it; turning it into a typed time
    /// is the host's job if it wants to sort chronologically.
    pub deletion_date: String,
}

/// One restorable entry in the trash: the paired `files/<trashed_name>` +
/// `info/<trashed_name>.trashinfo`, with the recorded original location and
/// deletion time, for the Trash place/UI ([`list_trash`]).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TrashedItem {
    /// The shared basename: `<Trash>/files/<trashed_name>` and
    /// `<Trash>/info/<trashed_name>.trashinfo`. The token the host passes back
    /// to [`restore_entry`] / a future `delete_from_trash`.
    pub trashed_name: String,
    /// The percent-decoded original absolute path (recorded DATA, see
    /// [`TrashInfo`]).
    pub original_path: PathBuf,
    /// The recorded deletion date string (verbatim, see [`TrashInfo`]).
    pub deletion_date: String,
}

/// The hard bound on the rename-suffix walk, guarding a pathological directory
/// where thousands of `(copy N)` names are all taken.
const MAX_UNIQUE_ATTEMPTS: u32 = 10_000;

/// The hard bound on a `.trashinfo` read ([`read_trashinfo`]). A well-formed
/// file is three short lines; this generous cap refuses a hostile or corrupt
/// multi-megabyte `info/*.trashinfo` (oversized fails closed rather than being
/// truncated and parsed, since a truncated `Path=` is a wrong restore target).
const MAX_TRASHINFO_BYTES: usize = 64 * 1024;

/// The set of bytes percent-escaped in a `.trashinfo` `Path=` value: everything
/// non-alphanumeric, then the RFC 3986 unreserved punctuation (`-_.~`) and the
/// path separator `/` removed so they pass through literally. Keeping `/`
/// literal makes the recorded path readable and restorable, per the trash-spec
/// intent and every implementation.
const TRASH_PATH_SET: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'/')
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~');

/// Reject an empty/`.`/`..`/separator-bearing user-typed name before any
/// syscall: such a name does not denote a single child. This is a clarity guard
/// on user input, not the security boundary (cap-std refuses traversal at the
/// syscall regardless).
fn validate_name(name: &str) -> OpResult<()> {
    let bad = name.is_empty()
        || name == "."
        || name == ".."
        || name.contains('/')
        || name.contains('\\');
    if bad {
        Err(OpError::InvalidName {
            name: name.to_string(),
        })
    } else {
        Ok(())
    }
}

/// The last component of `p` as an owned string, or [`OpError::InvalidName`] if
/// `p` has no file name (e.g. is empty or ends in `..`).
fn basename_of(p: &Path) -> OpResult<String> {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or_else(|| OpError::InvalidName {
            name: p.to_string_lossy().into_owned(),
        })
}

/// Split a name into `(stem, ext)` on the LAST `.`, so a suffix can be inserted
/// before the extension. A name with no `.`, or a leading-dot dotfile (whose
/// only `.` is at index 0, e.g. `.bashrc`), is treated as all-stem with an
/// empty extension. The extension, when present, includes its leading `.`.
fn split_name(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    }
}

/// Whether `target_rel` exists under `dir`, testing the entry ITSELF without
/// following a final symlink (a dangling link still counts as present, so we
/// never write through it). Uses `symlink_metadata`, since `Dir::exists`
/// follows the final link.
fn exists_no_follow(dir: &Dir, target_rel: &Path) -> bool {
    dir.symlink_metadata(target_rel).is_ok()
}

/// Find the first free name of the form `name`, then `{stem} (copy){ext}`, then
/// `{stem} (copy {n}){ext}` for `n = 2, 3, ...`, in the directory `parent_rel`
/// under `dir`, returning the full destination-relative path. Freeness is
/// tested without following a final symlink.
///
/// TOCTOU note: the existence check then create is racy under a concurrent
/// writer; this is acceptable for an interactive single-user file manager (the
/// race the whole desktop already has), and the actual create uses
/// `create_dir`/`create_new`/`symlink` which fail-closed on a lost race rather
/// than clobber.
fn unique_name(dir: &Dir, parent_rel: &Path, name: &str) -> OpResult<PathBuf> {
    let first = parent_rel.join(name);
    if !exists_no_follow(dir, &first) {
        return Ok(first);
    }
    let (stem, ext) = split_name(name);
    for n in 1..=MAX_UNIQUE_ATTEMPTS {
        let candidate = if n == 1 {
            format!("{stem} (copy){ext}")
        } else {
            format!("{stem} (copy {n}){ext}")
        };
        let path = parent_rel.join(&candidate);
        if !exists_no_follow(dir, &path) {
            return Ok(path);
        }
    }
    Err(OpError::Io(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "no free name found for the destination",
    )))
}

/// Decide what to do at `target_rel` under `dst` given a collision policy. The
/// existence test does not follow a final symlink. [`ConflictPolicy::Fail`]
/// errors on a collision; the others resolve it.
fn resolve_conflict(
    dst: &Dir,
    target_rel: &Path,
    policy: ConflictPolicy,
) -> OpResult<ConflictResolution> {
    if !exists_no_follow(dst, target_rel) {
        return Ok(ConflictResolution::Proceed);
    }
    match policy {
        ConflictPolicy::Skip => Ok(ConflictResolution::Skip),
        ConflictPolicy::Replace => Ok(ConflictResolution::Proceed),
        ConflictPolicy::Rename => {
            let parent = target_rel.parent().unwrap_or_else(|| Path::new(""));
            let name = basename_of(target_rel)?;
            Ok(ConflictResolution::UseName(unique_name(dst, parent, &name)?))
        }
        ConflictPolicy::Fail => Err(OpError::AlreadyExists {
            name: basename_of(target_rel)?,
        }),
    }
}

/// Whether an I/O error is the kernel's "cross-device link" (EXDEV), meaning a
/// same-filesystem rename is impossible and a copy+delete fallback is needed.
fn is_cross_device(e: &io::Error) -> bool {
    e.raw_os_error() == Some(libc::EXDEV)
}

/// Whether an I/O error is "directory not empty" / a non-empty-dir collision,
/// which a `rename` raises when asked to replace a non-empty directory target.
/// A directory `Replace` must route to the recursive copy path so it MERGES
/// instead of erroring.
fn is_dir_not_empty(e: &io::Error) -> bool {
    matches!(e.raw_os_error(), Some(c) if c == libc::ENOTEMPTY || c == libc::EEXIST)
}

/// Create a new directory named `name` inside `parent` (relative to `dir`).
/// Refuses to overwrite an existing entry of any kind: this is "make a new
/// folder here", not a merge. An empty/`.`/`..`/separator-bearing `name` is
/// refused as [`OpError::InvalidName`] before any syscall.
///
/// The refusal of an existing name is the kernel's (mkdir's EEXIST), atomic,
/// not a racy pre-check.
pub fn new_folder(dir: &Dir, parent: impl AsRef<Path>, name: &str) -> OpResult<OpOutcome> {
    validate_name(name)?;
    let target = parent.as_ref().join(name);
    match dir.create_dir(&target) {
        Ok(()) => Ok(OpOutcome::Created { target }),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Err(OpError::AlreadyExists {
            name: name.to_string(),
        }),
        Err(e) => Err(e.into()),
    }
}

/// Create a symbolic link `name` in `parent` (relative to `dir`) pointing at
/// `link_target`. `link_target` is the link's contents; it may be relative or
/// dangling, but cap-std refuses a target that escapes the capability (an
/// absolute path, or one that climbs out via `..`) at creation — the confined
/// FM cannot mint a symlink pointing outside its root. Refuses an invalid
/// `name` or an existing entry (no silent replace), mirroring [`new_folder`].
pub fn create_symlink(
    dir: &Dir,
    parent: impl AsRef<Path>,
    name: &str,
    link_target: &str,
) -> OpResult<OpOutcome> {
    validate_name(name)?;
    let link = parent.as_ref().join(name);
    match dir.symlink(link_target, &link) {
        Ok(()) => Ok(OpOutcome::Created { target: link }),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Err(OpError::AlreadyExists {
            name: name.to_string(),
        }),
        Err(e) => Err(e.into()),
    }
}

/// Rename the entry `from_name` to `to_name` within the same directory `parent`
/// (relative to `dir`). The conventional inline-rename / F2. Refuses if
/// `to_name` already exists (no silent replace) and refuses an invalid
/// `to_name`.
///
/// `rename` would otherwise silently REPLACE an existing file target, a
/// data-loss footgun for an interactive rename, so the collision is pre-checked
/// without following a final symlink. TOCTOU note: a file appearing in the gap
/// is then replaced by the rename, acceptable and matching every file manager
/// (the alternative, `RENAME_NOREPLACE`, is not in cap-std's portable surface).
pub fn rename(
    dir: &Dir,
    parent: impl AsRef<Path>,
    from_name: &str,
    to_name: &str,
) -> OpResult<OpOutcome> {
    validate_name(to_name)?;
    let parent = parent.as_ref();
    let target = parent.join(to_name);
    if exists_no_follow(dir, &target) {
        return Err(OpError::AlreadyExists {
            name: to_name.to_string(),
        });
    }
    dir.rename(parent.join(from_name), dir, &target)?;
    Ok(OpOutcome::Renamed { target })
}

/// Set the Unix permission bits of the entry at `path` (relative to the
/// capability `dir`) to `mode` - the conventional `chmod`, the editable half of
/// the info panel's metadata. Only the low 12 bits (rwx + setuid/setgid/sticky)
/// are honoured; higher bits (the file-type bits a caller might pass straight
/// from a `st_mode`) are masked off so they cannot be mistaken for a type
/// change. The entry must exist; a missing path surfaces the underlying
/// `ENOENT` rather than creating anything. `dir` is a cap-std capability, so the
/// relative `path` cannot escape it.
pub fn set_permissions(dir: &Dir, path: impl AsRef<Path>, mode: u32) -> OpResult<()> {
    use std::os::unix::fs::PermissionsExt;
    let perms = cap_std::fs::Permissions::from_std(std::fs::Permissions::from_mode(mode & 0o7777));
    dir.set_permissions(path.as_ref(), perms)?;
    Ok(())
}

/// The largest file [`safe_rewrite`] loads into memory to transform. Metadata
/// edits target media + document files, not arbitrary huge blobs; a larger file
/// is refused rather than buffered whole.
pub const MAX_REWRITE_BYTES: u64 = 256 * 1024 * 1024;

/// The hidden temp-sibling path [`safe_rewrite`] writes through, next to the
/// target so the final rename stays within one directory (and thus one
/// filesystem, so it is atomic). The pid keeps two processes from colliding; a
/// stale leftover from a crashed process has a dead pid, so it never collides
/// with a live edit, and `create_new` fail-closes on any collision regardless.
fn rewrite_temp_name(path: &Path) -> OpResult<PathBuf> {
    let name = path.file_name().ok_or_else(|| {
        OpError::Io(io::Error::new(
            io::ErrorKind::InvalidInput,
            "path has no file name to rewrite",
        ))
    })?;
    let mut tmp = OsString::from(".");
    tmp.push(name);
    tmp.push(format!(".arlen-meta-tmp.{}", std::process::id()));
    Ok(match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.join(tmp),
        _ => PathBuf::from(tmp),
    })
}

/// Atomically rewrite the file at `path` (under the capability `dir`) by handing
/// its bytes to `transform` and replacing it with the result, fail-safe so a
/// metadata edit can never corrupt the file. The new bytes are written to a
/// hidden temp sibling, fsync'd, READ BACK and verified byte-for-byte (so a
/// short or failed write never lands), then atomically renamed over the
/// original. Any failure before the rename leaves the original byte-identical
/// and removes the temp; a `transform` that returns an error aborts with nothing
/// written.
///
/// This is the corruption-safe substrate for the metadata-edit write-back (the
/// writable EXIF / filename / permission half of the info panel): never corrupt
/// the file, always verify the readback. The capability `dir` confines every
/// path, so `path` cannot escape it. A file larger than [`MAX_REWRITE_BYTES`] is
/// refused rather than buffered whole.
pub fn safe_rewrite<F>(dir: &Dir, path: impl AsRef<Path>, transform: F) -> OpResult<()>
where
    F: FnOnce(&[u8]) -> OpResult<Vec<u8>>,
{
    let path = path.as_ref();
    let len = dir.metadata(path)?.len();
    if len > MAX_REWRITE_BYTES {
        return Err(OpError::Io(io::Error::new(
            io::ErrorKind::InvalidInput,
            "file is too large to rewrite metadata in memory",
        )));
    }
    let original = dir.read(path)?;
    let new_bytes = transform(&original)?;

    let temp = rewrite_temp_name(path)?;
    {
        let mut file = dir.open_with(
            &temp,
            OpenOptions::new().write(true).create_new(true),
        )?;
        if let Err(e) = file.write_all(&new_bytes).and_then(|()| file.sync_all()) {
            drop(file);
            let _ = dir.remove_file(&temp);
            return Err(OpError::Io(e));
        }
    }

    // Verify readback: the temp must re-read byte-for-byte as written, or a
    // partial / failed write would otherwise land on the rename.
    match dir.read(&temp) {
        Ok(readback) if readback == new_bytes => {}
        Ok(_) => {
            let _ = dir.remove_file(&temp);
            return Err(OpError::Io(io::Error::new(
                io::ErrorKind::InvalidData,
                "metadata rewrite failed readback verification",
            )));
        }
        Err(e) => {
            let _ = dir.remove_file(&temp);
            return Err(OpError::Io(e));
        }
    }

    if let Err(e) = dir.rename(&temp, dir, path) {
        let _ = dir.remove_file(&temp);
        return Err(OpError::Io(e));
    }
    Ok(())
}

/// Copy the entry at `src` (relative to `src_dir`) into the destination
/// directory `dst` (relative to `dst_dir`), under the entry's own basename,
/// applying `policy` to a name collision. Handles a regular file, a directory
/// (copied RECURSIVELY by hand through the capabilities), and a symlink (the
/// LINK is copied, never its target).
///
/// `src`/`dst` may live under the same or different capabilities (intra- or
/// inter-root copy); cap-std confines each side to its own capability.
///
/// Atomicity: a single file or symlink copy is atomic (one syscall) and leaves
/// nothing behind on failure. A directory copy is NOT atomic; if it fails
/// midway it leaves a partial tree at the (possibly suffixed) target and
/// reports [`OpError::Partial`], with the source untouched. The caller decides
/// whether to roll the partial back.
pub fn copy_entry(
    src_dir: &Dir,
    src: impl AsRef<Path>,
    dst_dir: &Dir,
    dst: impl AsRef<Path>,
    policy: ConflictPolicy,
) -> OpResult<OpOutcome> {
    let src = src.as_ref();
    let name = basename_of(src)?;
    let mut target_rel = dst.as_ref().join(&name);
    match resolve_conflict(dst_dir, &target_rel, policy)? {
        ConflictResolution::Skip => return Ok(OpOutcome::Skipped),
        ConflictResolution::UseName(p) => target_rel = p,
        ConflictResolution::Proceed => {}
    }
    copy_into_target(src_dir, src, dst_dir, &target_rel, policy)?;
    Ok(OpOutcome::Created { target: target_rel })
}

/// Copy the source entry to an ALREADY-RESOLVED `target` (no further conflict
/// resolution at this level), classifying without following symlinks and
/// dispatching per kind. Shared by [`copy_entry`] and the cross-filesystem
/// [`move_entry`] fallback.
fn copy_into_target(
    src_dir: &Dir,
    src: &Path,
    dst_dir: &Dir,
    target: &Path,
    policy: ConflictPolicy,
) -> OpResult<()> {
    let st = src_dir.symlink_metadata(src)?;
    match kind_of(&st.file_type()) {
        EntryKind::Symlink => copy_symlink(src_dir, src, dst_dir, target, policy),
        EntryKind::Directory => copy_recursive_into(src_dir, src, dst_dir, target, policy),
        EntryKind::File | EntryKind::Other => {
            copy_file_replacing(src_dir, src, dst_dir, target, policy)?;
            Ok(())
        }
    }
}

/// Recreate the symlink at `src` (under `src_dir`) at `target` (under
/// `dst_dir`), copying the LINK (its raw target string) verbatim, never its
/// target's contents. Under `Replace` an existing non-directory target is
/// removed first so the link can be created; a directory target is a type clash
/// surfaced as the kernel's error.
///
/// A relative link may resolve differently once recreated in another directory;
/// that is correct freedesktop behaviour (the link text is preserved), not a
/// bug to fix. A link with an ABSOLUTE target is refused by cap-std at the
/// create (`Dir::symlink` rejects an absolute `original` as an escape attempt,
/// a deliberate hardening so a confined writer cannot plant an absolute link
/// for an unconfined reader to follow out), surfacing here as an
/// [`OpError::Io`]: copying such a link through the capability is the conserva-
/// tively refused, not silently mangled. The capability is the authority.
fn copy_symlink(
    src_dir: &Dir,
    src: &Path,
    dst_dir: &Dir,
    target: &Path,
    policy: ConflictPolicy,
) -> OpResult<()> {
    let link_target = src_dir.read_link(src)?;
    if policy == ConflictPolicy::Replace && exists_no_follow(dst_dir, target) {
        let existing = dst_dir.symlink_metadata(target)?;
        if existing.file_type().is_dir() {
            // A link cannot replace a directory blindly; surface the clash.
            return Err(OpError::Io(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "cannot replace a directory with a symlink",
            )));
        }
        dst_dir.remove_file(target)?;
    }
    dst_dir.symlink(&link_target, target)?;
    Ok(())
}

/// Copy a file (or other non-dir, non-symlink) entry, removing an existing
/// NON-directory target FIRST under `Replace`.
///
/// `Dir::copy` opens the destination following a final symlink, so without this
/// a `Replace` over an existing symlink would write THROUGH the link and clobber
/// the link's target (a within-root file the user never named) instead of
/// replacing the named entry. Removing a non-directory target first (mirroring
/// [`copy_symlink`]) makes `Replace` replace the entry; a directory target is
/// left for `Dir::copy` to clash on (the kernel's `EISDIR`), the documented
/// refusal rather than a blind removal.
fn copy_file_replacing(
    src_dir: &Dir,
    src: &Path,
    dst_dir: &Dir,
    target: &Path,
    policy: ConflictPolicy,
) -> io::Result<()> {
    if policy == ConflictPolicy::Replace && exists_no_follow(dst_dir, target) {
        let existing = dst_dir.symlink_metadata(target)?;
        if !existing.file_type().is_dir() {
            dst_dir.remove_file(target)?;
        }
    }
    src_dir.copy(src, dst_dir, target).map(|_| ())
}

/// Recursively copy the directory at `src` (under `src_dir`) into a new or
/// existing directory at `target` (under `dst_dir`), applying `policy` per
/// child. The destination directory is created if absent; if it exists and the
/// policy is `Replace`, children MERGE into it; `Skip`/`Rename` apply per child.
///
/// cap-std confines every read and write to its capability, so the recursion
/// cannot escape either root. On any mid-walk failure returns
/// [`OpError::Partial`] with `written = target` (the partial tree's root), the
/// source untouched, for the caller to roll back or resume. The copy is NOT
/// atomic.
fn copy_recursive_into(
    src_dir: &Dir,
    src: &Path,
    dst_dir: &Dir,
    target: &Path,
    policy: ConflictPolicy,
) -> OpResult<()> {
    // Create the destination directory, or accept an existing one only if it is
    // a directory (a merge); a non-directory at `target` is a type clash.
    match dst_dir.create_dir(target) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
            let ft = dst_dir
                .symlink_metadata(target)
                .map_err(|e| partial(target, e))?
                .file_type();
            if !ft.is_dir() {
                return Err(partial(
                    target,
                    io::Error::new(
                        io::ErrorKind::AlreadyExists,
                        "cannot merge a directory into a non-directory",
                    ),
                ));
            }
        }
        Err(e) => return Err(partial(target, e)),
    }
    // Best-effort: carry the source directory's mode onto a fresh target. A
    // failure here never fails the copy.
    if let Ok(meta) = src_dir.symlink_metadata(src) {
        let _ = dst_dir.set_permissions(target, meta.permissions());
    }
    let read_dir = src_dir.read_dir(src).map_err(|e| partial(target, e))?;
    for entry in read_dir {
        let entry = entry.map_err(|e| partial(target, e))?;
        let child_name = entry.file_name();
        let child_src = src.join(&child_name);
        let mut child_target = target.join(&child_name);
        match resolve_conflict(dst_dir, &child_target, policy).map_err(|e| flatten(target, e))? {
            ConflictResolution::Skip => continue,
            ConflictResolution::UseName(p) => child_target = p,
            ConflictResolution::Proceed => {}
        }
        let child_type = entry.file_type().map_err(|e| partial(target, e))?;
        match kind_of(&child_type) {
            EntryKind::Directory => {
                // The recursion reports its own `Partial` rooted at the
                // outermost `target`, so re-root any nested partial here.
                copy_recursive_into(src_dir, &child_src, dst_dir, &child_target, policy)
                    .map_err(|e| flatten(target, e))?;
            }
            EntryKind::Symlink => {
                copy_symlink(src_dir, &child_src, dst_dir, &child_target, policy)
                    .map_err(|e| flatten(target, e))?;
            }
            EntryKind::File | EntryKind::Other => {
                copy_file_replacing(src_dir, &child_src, dst_dir, &child_target, policy)
                    .map_err(|e| partial(target, e))?;
            }
        }
    }
    Ok(())
}

/// Wrap an `io::Error` as an [`OpError::Partial`] rooted at `written`.
fn partial(written: &Path, source: io::Error) -> OpError {
    OpError::Partial {
        written: written.to_path_buf(),
        source,
    }
}

/// Re-root any error from a nested step under the outermost partial `written`,
/// preserving the originating `io::Error` so the caller sees the real cause.
fn flatten(written: &Path, e: OpError) -> OpError {
    match e {
        OpError::Partial { source, .. } => partial(written, source),
        OpError::Io(source) => partial(written, source),
        // A name/exists error during the walk still leaves a partial tree.
        OpError::AlreadyExists { name } => partial(
            written,
            io::Error::new(io::ErrorKind::AlreadyExists, name),
        ),
        OpError::InvalidName { name } => partial(
            written,
            io::Error::new(io::ErrorKind::InvalidInput, name),
        ),
    }
}

/// Move the entry at `src` (relative to `src_dir`) into the destination
/// directory `dst` (relative to `dst_dir`), under its own basename, applying
/// `policy` to a collision. Tries an atomic rename first (the same-filesystem
/// fast path, O(1) even for a whole directory tree); if the kernel refuses
/// because the two capabilities are on different filesystems (EXDEV), or
/// because the target is a non-empty directory to merge, falls back to
/// copy-then-delete. Symlinks move as links; directories move whole (rename) or
/// are recursively copied then removed (cross-fs / merge).
///
/// Atomicity: a same-filesystem move is atomic (renameat) - the entry is at the
/// new location or the old, never both, never partial. A cross-filesystem or
/// merging move is NOT atomic: the copy phase can leave a partial tree
/// ([`OpError::Partial`]), in which case the source is NOT deleted (it is the
/// only intact copy); a copy that succeeds but whose source-delete then fails
/// leaves the entry in both places (a duplicate, not data loss, the safe
/// failure direction).
pub fn move_entry(
    src_dir: &Dir,
    src: impl AsRef<Path>,
    dst_dir: &Dir,
    dst: impl AsRef<Path>,
    policy: ConflictPolicy,
) -> OpResult<OpOutcome> {
    let src = src.as_ref();
    let name = basename_of(src)?;
    let mut target_rel = dst.as_ref().join(&name);
    match resolve_conflict(dst_dir, &target_rel, policy)? {
        ConflictResolution::Skip => return Ok(OpOutcome::Skipped),
        ConflictResolution::UseName(p) => target_rel = p,
        ConflictResolution::Proceed => {}
    }
    move_resolved(src_dir, src, dst_dir, &target_rel, policy)?;
    Ok(OpOutcome::Renamed { target: target_rel })
}

/// Move `src` (under `src_dir`) to an ALREADY-RESOLVED `target` (under
/// `dst_dir`): the rename-then-cross-fs-copy+delete core shared by
/// [`move_entry`] (after it derives the target from the source basename) and
/// [`restore_entry`] (after it resolves the host-chosen exact destination). The
/// caller has already run [`resolve_conflict`], so `target` is the final path
/// and no second conflict resolution happens here.
///
/// Tries an atomic rename across the two capabilities first (the same-filesystem
/// O(1) fast path, symlink-correct). If the kernel refuses because the
/// capabilities are on different filesystems (EXDEV), or because the target is a
/// non-empty directory to merge (ENOTEMPTY/EEXIST), falls back to copy-into-the-
/// target then delete-the-source. On a partial copy the source is left intact;
/// a copy that succeeds but whose source delete fails leaves a duplicate (the
/// safe failure direction, see [`move_entry`]).
fn move_resolved(
    src_dir: &Dir,
    src: &Path,
    dst_dir: &Dir,
    target: &Path,
    policy: ConflictPolicy,
) -> OpResult<()> {
    // Fast path: an atomic rename across the two capabilities. The collision was
    // already cleared, so a no-conflict move or a `Replace` over a file target
    // (which rename overwrites cleanly). A `Replace` over a NON-EMPTY directory
    // raises ENOTEMPTY/EEXIST; route that, like a cross-device move, to the
    // merging copy+delete fallback.
    match src_dir.rename(src, dst_dir, target) {
        Ok(()) => return Ok(()),
        Err(e) if is_cross_device(&e) || is_dir_not_empty(&e) => {
            // fall through to the copy+delete fallback
        }
        Err(e) => return Err(e.into()),
    }
    // Fallback: copy fully into the resolved target, then delete the source.
    copy_into_target(src_dir, src, dst_dir, target, policy)?;
    remove_source(src_dir, src)?;
    Ok(())
}

/// Remove a moved source entry after a successful cross-fs copy: a file/symlink/
/// special node via `remove_file` (the link itself for a symlink, never its
/// target), a directory via `remove_dir_all`. Classified without following
/// symlinks.
fn remove_source(src_dir: &Dir, src: &Path) -> OpResult<()> {
    let st = src_dir.symlink_metadata(src)?;
    if st.file_type().is_dir() {
        src_dir.remove_dir_all(src)?;
    } else {
        src_dir.remove_file(src)?;
    }
    Ok(())
}

/// Permanently delete the entry at `src` (relative to `dir`) - the
/// `Shift+Delete` path (plan §138), no trash. A directory is removed
/// recursively; a file/symlink/special node is removed directly.
///
/// Classified without following symlinks, so a symlink-to-a-directory is
/// removed as the LINK (`remove_file`), never recursed into (`remove_dir_all`
/// would delete the target's contents).
pub fn delete_permanent(dir: &Dir, src: impl AsRef<Path>) -> OpResult<()> {
    let src = src.as_ref();
    let st = dir.symlink_metadata(src)?;
    if st.file_type().is_dir() {
        dir.remove_dir_all(src)?;
    } else {
        dir.remove_file(src)?;
    }
    Ok(())
}

/// Send the entry at `src` (relative to `src_dir`) to the freedesktop
/// home-trash (trash-spec 1.0): move it into `<Trash>/files/<name>` and write a
/// matching `<Trash>/info/<name>.trashinfo` recording its ORIGINAL absolute
/// path and the deletion time, so the Trash UI can restore it. A name already
/// used in the trash is disambiguated with a `(copy N)` suffix, and the `files`
/// entry and its `.trashinfo` always share that final name (the pairing
/// invariant).
///
/// The trash lives OUTSIDE the browsed root's capability, so the caller passes a
/// SEPARATE `trash_dir` capability the host opened on `~/.local/share/Trash`
/// (already containing `files/` and `info/`), plus `original_abs` - the entry's
/// real absolute path as the user sees it - which is RECORDED in the
/// `.trashinfo` (it is data, never handed to any capability).
///
/// Scope: this implements the HOME-trash only. When the entry is on a different
/// filesystem than the home-trash (EXDEV on the move), it falls back to
/// copy-into-the-home-trash + delete, so trashing still works. The per-volume
/// topdir-trash (`$topdir/.Trash-$uid`, recording a topdir-relative path) is a
/// documented follow-up. A symlink trashes as the link; the deletion is not
/// followed into its target.
pub fn trash_entry(
    src_dir: &Dir,
    src: impl AsRef<Path>,
    trash_dir: &Dir,
    original_abs: &Path,
) -> OpResult<TrashedEntry> {
    let now = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());
    trash_entry_at(src_dir, src, trash_dir, original_abs, now)
}

/// [`trash_entry`] with an injected deletion time, so the `.trashinfo`
/// `DeletionDate` is deterministic in tests.
pub fn trash_entry_at(
    src_dir: &Dir,
    src: impl AsRef<Path>,
    trash_dir: &Dir,
    original_abs: &Path,
    now: OffsetDateTime,
) -> OpResult<TrashedEntry> {
    let src = src.as_ref();
    let name = basename_of(src)?;
    let info_bytes = trashinfo_bytes(original_abs, now);

    // Reserve a collision-free name by creating the `.trashinfo` EXCLUSIVELY
    // (O_EXCL) first: it is the atomic lock that prevents two concurrent
    // trashes from grabbing the same name, and it pins the pairing. Walk the
    // `(copy N)` suffixes until both the info and files sides are free.
    let (final_name, info_rel) = reserve_trash_name(trash_dir, &name, &info_bytes)?;

    // Move the entry into `files/<final_name>`. The same-fs home-trash rename is
    // atomic and moves a symlink as the link. On EXDEV fall back to the
    // symlink-correct copy + delete. On ANY failure, roll back the reserved
    // info file so no orphan info points at an entry still in place.
    let files_rel = Path::new("files").join(&final_name);
    let moved = match src_dir.rename(src, trash_dir, &files_rel) {
        Ok(()) => Ok(()),
        Err(e) if is_cross_device(&e) => copy_into_target(src_dir, src, trash_dir, &files_rel, ConflictPolicy::Fail)
            .and_then(|()| remove_source(src_dir, src)),
        Err(e) => Err(e.into()),
    };
    if let Err(e) = moved {
        let _ = trash_dir.remove_file(&info_rel);
        // The cross-device fallback may have copied the entry into `files/`
        // before a later step failed (a partial copy, or the source delete), so
        // also remove that orphan: a `files/` entry with no `.trashinfo` would
        // break the pairing invariant. `remove_source` classifies no-follow and
        // is best-effort here (a same-fs rename failure left nothing to remove,
        // and the original source is intact either way - the safe direction).
        let _ = remove_source(trash_dir, &files_rel);
        return Err(e);
    }
    Ok(TrashedEntry {
        trashed_name: final_name,
    })
}

/// Walk `(copy N)` suffixes until a name is free on BOTH the `info/` and
/// `files/` sides, then claim it by exclusively creating `info/<name>.trashinfo`
/// with the given bytes. Returns the claimed name and the info-file relative
/// path. A lost O_EXCL race bumps the suffix and retries.
fn reserve_trash_name(
    trash_dir: &Dir,
    name: &str,
    info_bytes: &[u8],
) -> OpResult<(String, PathBuf)> {
    let (stem, ext) = split_name(name);
    for n in 0..=MAX_UNIQUE_ATTEMPTS {
        let candidate = if n == 0 {
            name.to_string()
        } else if n == 1 {
            format!("{stem} (copy){ext}")
        } else {
            format!("{stem} (copy {n}){ext}")
        };
        let files_rel = Path::new("files").join(&candidate);
        let info_rel = Path::new("info").join(format!("{candidate}.trashinfo"));
        // The pairing must be free as a unit; skip a name taken on either side.
        if exists_no_follow(trash_dir, &files_rel) || exists_no_follow(trash_dir, &info_rel) {
            continue;
        }
        match trash_dir.open_with(
            &info_rel,
            OpenOptions::new().write(true).create_new(true),
        ) {
            Ok(mut f) => {
                f.write_all(info_bytes)?;
                return Ok((candidate, info_rel));
            }
            // Lost the O_EXCL race for this name; try the next suffix.
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.into()),
        }
    }
    Err(OpError::Io(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "no free name found in the trash",
    )))
}

/// The exact bytes of a `.trashinfo` file for `original_abs` deleted at `now`:
/// the `[Trash Info]` header, the percent-encoded `Path=` (separators kept
/// literal), and the `DeletionDate=` in `YYYY-MM-DDThh:mm:ss` local time with no
/// offset suffix, with a trailing newline.
fn trashinfo_bytes(original_abs: &Path, now: OffsetDateTime) -> Vec<u8> {
    let encoded_path = percent_encode(original_abs.as_os_str().as_bytes(), TRASH_PATH_SET);
    let date = format_deletion_date(now);
    format!("[Trash Info]\nPath={encoded_path}\nDeletionDate={date}\n").into_bytes()
}

/// Format a deletion time as the trash-spec `YYYY-MM-DDThh:mm:ss` (no timezone
/// suffix). Built by hand from the date/time parts so it never depends on a
/// formatting description string.
fn format_deletion_date(now: OffsetDateTime) -> String {
    let d = now.date();
    let t = now.time();
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
        d.year(),
        u8::from(d.month()),
        d.day(),
        t.hour(),
        t.minute(),
        t.second(),
    )
}

/// Read and parse `<Trash>/info/<trashed_name>.trashinfo` (relative to the
/// `trash_dir` capability) into a [`TrashInfo`]. The `Path=` value is
/// percent-DECODED into [`TrashInfo::original_path`]; `DeletionDate=` is kept
/// verbatim. The exact inverse of [`trash_entry`]'s recorded bytes, so a
/// trash-then-read round-trips the original path.
///
/// The recorded `Path=` is RECORDED DATA the host displays/resolves, NEVER a
/// path this module feeds to a capability (see [`restore_entry`]).
///
/// Parsing is defensive and fail-closed: the read is size-bounded
/// ([`MAX_TRASHINFO_BYTES`]), the `[Trash Info]` group header must be present,
/// the `Path=` key is required, and a malformed/oversized/duplicate-key/
/// missing-`Path`/missing-header file is refused as [`OpError::Io`]
/// (`InvalidData`) rather than guessed at - a half-parsed restore target is
/// worse than no restore. A missing `DeletionDate=` is tolerated (it only
/// degrades display) and yields an empty string. A malformed percent escape in
/// the value decodes leniently (the bytes are data the host re-vets, never a
/// capability target), so a cosmetically broken escape does not block an
/// otherwise-restorable entry.
pub fn read_trashinfo(trash_dir: &Dir, trashed_name: &str) -> OpResult<TrashInfo> {
    validate_name(trashed_name)?;
    let info_rel = Path::new("info").join(format!("{trashed_name}.trashinfo"));
    let file = trash_dir.open(&info_rel)?;
    // Bound the read: take one byte past the cap so an oversized file is
    // detected (and refused) instead of truncated and parsed.
    let mut buf = Vec::new();
    file.take(MAX_TRASHINFO_BYTES as u64 + 1)
        .read_to_end(&mut buf)?;
    if buf.len() > MAX_TRASHINFO_BYTES {
        return Err(OpError::Io(io::Error::new(
            io::ErrorKind::InvalidData,
            "trashinfo too large",
        )));
    }
    parse_trashinfo(&buf)
}

/// Parse the bytes of a `.trashinfo` key-file (freedesktop subset) into a
/// [`TrashInfo`]. Pure (no filesystem), so the byte-shape cases are unit-tested
/// directly. Parses line-oriented over the raw bytes (the keys/header are
/// ASCII, the `Path=` value is percent-encoded ASCII or `%XX`), so a stray
/// non-UTF-8 byte in the value fails the value decode, not a blanket UTF-8
/// check that would reject before the structure is even seen.
fn parse_trashinfo(bytes: &[u8]) -> OpResult<TrashInfo> {
    let invalid = |msg: &'static str| OpError::Io(io::Error::new(io::ErrorKind::InvalidData, msg));

    let mut header_seen = false;
    let mut path_value: Option<Vec<u8>> = None;
    let mut date_value: Option<String> = None;

    for raw in bytes.split(|&b| b == b'\n') {
        // Trim a trailing `\r` and surrounding ASCII whitespace.
        let line = trim_ascii(raw);
        if line.is_empty() || line.first() == Some(&b'#') {
            continue;
        }
        if line == b"[Trash Info]" {
            header_seen = true;
            continue;
        }
        // A `key=value` line: split on the first `=`. The key is ASCII.
        let Some(eq) = line.iter().position(|&b| b == b'=') else {
            continue;
        };
        let key = &line[..eq];
        let value = &line[eq + 1..];
        if key == b"Path" {
            // A duplicate `Path=` is malformed/adversarial (a crafted file could
            // hide a second value behind the first); fail closed rather than pick.
            if path_value.is_some() {
                return Err(invalid("trashinfo has duplicate Path"));
            }
            path_value = Some(value.to_vec());
        } else if key == b"DeletionDate" {
            if date_value.is_some() {
                return Err(invalid("trashinfo has duplicate DeletionDate"));
            }
            date_value = Some(String::from_utf8_lossy(value).into_owned());
        }
    }

    if !header_seen {
        return Err(invalid("trashinfo missing [Trash Info] header"));
    }
    let path_value = path_value.ok_or_else(|| invalid("trashinfo missing Path"))?;

    // Percent-decode the value into a lossless PathBuf. A value with no `%`
    // round-trips unchanged; a malformed `%` escape is passed through literally
    // (lenient decode), acceptable because the result is data the host re-vets.
    let decoded: Vec<u8> = percent_decode(&path_value).collect();
    let original_path = PathBuf::from(OsString::from_vec(decoded));

    Ok(TrashInfo {
        original_path,
        // A missing DeletionDate degrades display only, so tolerate it (Path is
        // the load-bearing field); empty string when absent.
        deletion_date: date_value.unwrap_or_default(),
    })
}

/// Trim leading and trailing ASCII whitespace (including a trailing `\r`) from a
/// byte line, without allocating.
fn trim_ascii(mut b: &[u8]) -> &[u8] {
    while let [first, rest @ ..] = b {
        if first.is_ascii_whitespace() {
            b = rest;
        } else {
            break;
        }
    }
    while let [rest @ .., last] = b {
        if last.is_ascii_whitespace() {
            b = rest;
        } else {
            break;
        }
    }
    b
}

/// Enumerate the home-trash: every `<Trash>/info/*.trashinfo` paired with its
/// `<Trash>/files/<name>` entry, as [`TrashedItem`]s for the Trash place/UI.
///
/// An info file with no matching `files/` entry (a half-state orphan), a
/// `files/` entry with no info (untracked), a non-`.trashinfo` name, a
/// non-UTF-8 name, or a malformed/unreadable info file is SKIPPED, not reported
/// and not an error: `list_trash` returns only well-paired, parseable items, so
/// the UI never shows a half-entry it cannot restore. The order is unspecified
/// (the host sorts, e.g. by deletion date). A future "clean trash" maintenance
/// op could surface the skipped half-states; out of scope here.
///
/// The only error returned is [`OpError::Io`] when `info/` itself cannot be
/// opened (e.g. an uninitialised trash); per-entry problems are skips.
pub fn list_trash(trash_dir: &Dir) -> OpResult<Vec<TrashedItem>> {
    let mut items = Vec::new();
    for entry in trash_dir.read_dir("info")? {
        let Ok(entry) = entry else { continue };
        // The name must be valid UTF-8 (it becomes the `String` restore token)
        // and end in `.trashinfo`; anything else is skipped.
        let Ok(file_name) = entry.file_name().into_string() else {
            continue;
        };
        let Some(trashed_name) = file_name.strip_suffix(".trashinfo") else {
            continue;
        };
        if trashed_name.is_empty() {
            continue;
        }
        // The matching `files/<trashed_name>` must exist (a dangling-symlink
        // entry still counts, so it is listed and restorable as a link). An
        // orphan info (no files entry) is skipped.
        let files_rel = Path::new("files").join(trashed_name);
        if !exists_no_follow(trash_dir, &files_rel) {
            continue;
        }
        // A malformed/unreadable info file is skipped, not an error.
        let Ok(info) = read_trashinfo(trash_dir, trashed_name) else {
            continue;
        };
        items.push(TrashedItem {
            trashed_name: trashed_name.to_string(),
            original_path: info.original_path,
            deletion_date: info.deletion_date,
        });
    }
    Ok(items)
}

/// Empty the trash: for every `info/<name>.trashinfo`, remove the paired
/// `files/<name>` payload (a file or symlink is unlinked without following it,
/// a directory is removed recursively) and then the `.trashinfo`. Returns the
/// number of entries cleared.
///
/// Enumeration is `info/`-driven (like [`list_trash`]); an orphan info with no
/// payload is still cleared (it would otherwise linger as a ghost entry). A
/// per-entry removal error is skipped, so one stuck entry never strands the
/// rest; the count reflects entries actually cleared.
pub fn empty_trash(trash_dir: &Dir) -> OpResult<usize> {
    let mut cleared = 0usize;
    for entry in trash_dir.read_dir("info")? {
        let Ok(entry) = entry else { continue };
        let Ok(file_name) = entry.file_name().into_string() else {
            continue;
        };
        let Some(trashed_name) = file_name.strip_suffix(".trashinfo") else {
            continue;
        };
        if trashed_name.is_empty() {
            continue;
        }
        let files_rel = Path::new("files").join(trashed_name);
        let info_rel = Path::new("info").join(&file_name);
        // Remove the payload by its real kind (never follow a symlink: a
        // dir-symlink is unlinked, not recursed into).
        let payload_cleared = match trash_dir.symlink_metadata(&files_rel) {
            Ok(md) if md.file_type().is_dir() => trash_dir.remove_dir_all(&files_rel).is_ok(),
            Ok(_) => trash_dir.remove_file(&files_rel).is_ok(),
            Err(_) => true, // already absent (an orphan info): still clear it
        };
        if payload_cleared {
            let _ = trash_dir.remove_file(&info_rel);
            cleared += 1;
        }
    }
    Ok(cleared)
}

/// Restore a trashed entry: move `<Trash>/files/<trashed_name>` to
/// `dest_dir`/`dest_rel` (the host-resolved destination), then remove BOTH the
/// `files/` entry and the matching `info/<trashed_name>.trashinfo` on success.
/// The inverse of [`trash_entry`].
///
/// PATH-SAFETY (the central rule): the destination is decided by the HOST and
/// passed as a `dest_dir` capability + a `dest_rel` path RELATIVE to it. This
/// function NEVER reads the `.trashinfo`'s recorded original path to drive the
/// write - that path is untrusted recorded data (a crafted info file could
/// record `/etc/passwd`, and `info/` is writable by anything with the trash
/// capability). The host calls [`read_trashinfo`] to learn the original
/// location, applies its own trust policy to choose a legitimate restore root,
/// and hands this function the capability + relative path; cap-std then refuses
/// any `..`/absolute/escaping `dest_rel` at the syscall ([`OpError::Io`]). The
/// capability is the authority. This is where the module-header invariant ("the
/// only absolute path this module handles is recorded text, never handed to a
/// capability") pays off: the recorded path leaves via [`read_trashinfo`] as
/// data, full stop.
///
/// Conflict handling reuses the move path: if `dest_rel` already exists,
/// `policy` decides it (the Skip/Replace/Rename/Fail cases), via the same
/// [`resolve_conflict`] and move-resolved logic as every other op; the returned
/// [`OpOutcome::Renamed`] carries the FINAL destination-relative path (which
/// differs from `dest_rel` under `Rename`). A [`ConflictPolicy::Skip`] returns
/// [`OpOutcome::Skipped`] and leaves the trash pair INTACT (only a successful
/// move removes the pair). Restore never returns [`OpOutcome::Created`]: it is a
/// move.
///
/// Ordering / crash-safety: the `files/` entry is MOVED OUT first (the
/// committing step); only then is the `.trashinfo` removed (cleanup that may
/// safely lag). A crash between the two leaves the entry restored and an ORPHAN
/// `.trashinfo` (which [`list_trash`] skips), never a removed info with the
/// entry still trapped in the trash. The `.trashinfo` removal is best-effort
/// with respect to the OUTCOME: the entry is already restored, so a failure to
/// delete the info does not fail the restore (the leftover is a harmless orphan
/// a maintenance pass sweeps). This mirrors [`trash_entry`]'s inverse ordering
/// (reserve info first, move second, roll back info on move failure).
///
/// Cross-filesystem: the move reuses the same-fs-rename / cross-fs-copy+delete
/// fallback as [`move_entry`], so a restore from a trash on one filesystem to a
/// destination on another works and is symlink-correct (a trashed link restores
/// as the link).
pub fn restore_entry(
    trash_dir: &Dir,
    trashed_name: &str,
    dest_dir: &Dir,
    dest_rel: &Path,
    policy: ConflictPolicy,
) -> OpResult<OpOutcome> {
    validate_name(trashed_name)?;
    let files_rel = Path::new("files").join(trashed_name);
    let info_rel = Path::new("info").join(format!("{trashed_name}.trashinfo"));

    // Resolve the conflict against the HOST-CHOSEN exact destination (not the
    // trashed basename): restore must honour `dest_rel` so a `doc (copy).txt`
    // trapped in the trash can land back as its original `doc.txt`.
    let target = match resolve_conflict(dest_dir, dest_rel, policy)? {
        ConflictResolution::Skip => return Ok(OpOutcome::Skipped),
        ConflictResolution::UseName(p) => p,
        ConflictResolution::Proceed => dest_rel.to_path_buf(),
    };

    // Move the files entry out FIRST (the committing step). cap-std confines the
    // write to `dest_dir`, so an escaping `dest_rel`/`target` is refused here,
    // before any info removal, leaving the trash pair intact.
    move_resolved(trash_dir, &files_rel, dest_dir, &target, policy)?;

    // The entry is restored; remove the now-orphan `.trashinfo`. Best-effort:
    // a failure here must NOT fail a completed restore (a leftover info is the
    // orphan `list_trash` skips, swept by a maintenance pass).
    let _ = trash_dir.remove_file(&info_rel);

    Ok(OpOutcome::Renamed { target })
}

#[cfg(test)]
mod tests {
    use super::*;
    use cap_std::ambient_authority;
    use std::fs;
    use time::macros::datetime;

    /// Open `path` as a cap-std capability rooted there.
    fn cap(path: &Path) -> Dir {
        Dir::open_ambient_dir(path, ambient_authority()).unwrap()
    }

    /// Read a file's bytes through the OS (the test inspects the real fs).
    fn read(path: &Path) -> Vec<u8> {
        fs::read(path).unwrap()
    }

    /// A fixed deletion time for deterministic `.trashinfo` date assertions.
    fn fixed_now() -> OffsetDateTime {
        datetime!(2026 - 06 - 11 14:03:09 UTC)
    }

    // ---- new_folder -------------------------------------------------------

    #[test]
    fn new_folder_creates_a_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = cap(tmp.path());
        let out = new_folder(&dir, ".", "fresh").unwrap();
        assert_eq!(out, OpOutcome::Created { target: PathBuf::from("./fresh") });
        assert!(tmp.path().join("fresh").is_dir());
    }

    #[test]
    fn new_folder_refuses_an_existing_name() {
        let tmp = tempfile::tempdir().unwrap();
        // Collide with an existing file...
        fs::write(tmp.path().join("taken"), b"x").unwrap();
        let dir = cap(tmp.path());
        assert!(matches!(
            new_folder(&dir, ".", "taken"),
            Err(OpError::AlreadyExists { .. })
        ));
        assert_eq!(read(&tmp.path().join("taken")), b"x", "the file is untouched");
        // ...and with an existing directory.
        fs::create_dir(tmp.path().join("dir")).unwrap();
        assert!(matches!(
            new_folder(&dir, ".", "dir"),
            Err(OpError::AlreadyExists { .. })
        ));
    }

    #[test]
    fn new_folder_refuses_an_invalid_name() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = cap(tmp.path());
        for bad in ["", ".", "..", "a/b"] {
            assert!(
                matches!(new_folder(&dir, ".", bad), Err(OpError::InvalidName { .. })),
                "name {bad:?} must be rejected"
            );
        }
        // Nothing was created for any of them.
        assert_eq!(fs::read_dir(tmp.path()).unwrap().count(), 0);
    }

    #[test]
    fn new_folder_cannot_escape_the_capability() {
        let outer = tempfile::tempdir().unwrap();
        fs::create_dir(outer.path().join("root")).unwrap();
        let before: Vec<_> = fs::read_dir(outer.path()).unwrap().collect();
        let dir = cap(&outer.path().join("root"));
        // A parent-traversal parent is refused by cap-std (an io error), and an
        // absolute parent likewise. `new_folder` validates the name, so the
        // escape vector is the `parent` path.
        assert!(matches!(new_folder(&dir, "..", "x"), Err(OpError::Io(_))));
        assert!(matches!(new_folder(&dir, "/tmp", "x"), Err(OpError::Io(_))));
        // Nothing appeared outside the granted root.
        assert!(!outer.path().join("x").exists());
        assert_eq!(
            fs::read_dir(outer.path()).unwrap().count(),
            before.len(),
            "the parent directory is unchanged"
        );
    }

    // ---- rename -----------------------------------------------------------

    #[test]
    fn rename_moves_a_name_within_a_dir() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("a.txt"), b"data").unwrap();
        let dir = cap(tmp.path());
        let out = rename(&dir, ".", "a.txt", "b.txt").unwrap();
        assert_eq!(out, OpOutcome::Renamed { target: PathBuf::from("./b.txt") });
        assert!(!tmp.path().join("a.txt").exists());
        assert_eq!(read(&tmp.path().join("b.txt")), b"data");
    }

    #[test]
    fn set_permissions_changes_the_mode() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("a.txt");
        fs::write(&f, b"data").unwrap();
        fs::set_permissions(&f, fs::Permissions::from_mode(0o644)).unwrap();
        let dir = cap(tmp.path());
        set_permissions(&dir, "a.txt", 0o600).unwrap();
        let mode = fs::metadata(&f).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn set_permissions_masks_the_type_bits() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("a.txt");
        fs::write(&f, b"data").unwrap();
        let dir = cap(tmp.path());
        // A full `st_mode` (regular-file type bits 0o100000 + perms 0o640) must
        // not leak past the permission bits.
        set_permissions(&dir, "a.txt", 0o100_640).unwrap();
        let mode = fs::metadata(&f).unwrap().permissions().mode() & 0o7777;
        assert_eq!(mode, 0o640);
    }

    #[test]
    fn safe_rewrite_replaces_content_and_leaves_no_temp() {
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("doc.bin");
        fs::write(&f, b"hello world").unwrap();
        let dir = cap(tmp.path());
        safe_rewrite(&dir, "doc.bin", |bytes| {
            Ok(bytes.to_ascii_uppercase())
        })
        .unwrap();
        assert_eq!(read(&f), b"HELLO WORLD");
        // No temp sibling left behind.
        let leftover: Vec<_> = fs::read_dir(tmp.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .filter(|n| n.to_string_lossy().contains("arlen-meta-tmp"))
            .collect();
        assert!(leftover.is_empty(), "no temp file remains: {leftover:?}");
    }

    #[test]
    fn safe_rewrite_leaves_the_original_untouched_on_transform_error() {
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("doc.bin");
        fs::write(&f, b"original bytes").unwrap();
        let dir = cap(tmp.path());
        let err = safe_rewrite(&dir, "doc.bin", |_bytes| {
            Err(OpError::Io(io::Error::other("no")))
        })
        .unwrap_err();
        assert!(matches!(err, OpError::Io(_)));
        // The file is byte-identical and no temp remains.
        assert_eq!(read(&f), b"original bytes");
        let leftover = fs::read_dir(tmp.path())
            .unwrap()
            .any(|e| e.unwrap().file_name().to_string_lossy().contains("arlen-meta-tmp"));
        assert!(!leftover, "a failed transform leaves no temp");
    }

    #[test]
    fn safe_rewrite_round_trips_identity() {
        // An identity transform leaves the bytes intact (the verify-readback path
        // accepts a faithful write).
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("img.bin");
        let content: Vec<u8> = (0u8..=255).cycle().take(4096).collect();
        fs::write(&f, &content).unwrap();
        let dir = cap(tmp.path());
        safe_rewrite(&dir, "img.bin", |bytes| Ok(bytes.to_vec())).unwrap();
        assert_eq!(read(&f), content);
    }

    #[test]
    fn rename_refuses_an_existing_target() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("a.txt"), b"AAA").unwrap();
        fs::write(tmp.path().join("b.txt"), b"BBB").unwrap();
        let dir = cap(tmp.path());
        assert!(matches!(
            rename(&dir, ".", "a.txt", "b.txt"),
            Err(OpError::AlreadyExists { .. })
        ));
        // No silent replace: both files are untouched.
        assert_eq!(read(&tmp.path().join("a.txt")), b"AAA");
        assert_eq!(read(&tmp.path().join("b.txt")), b"BBB");
    }

    #[test]
    fn rename_refuses_invalid_target() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("a.txt"), b"x").unwrap();
        let dir = cap(tmp.path());
        for bad in ["a/b", "..", ""] {
            assert!(matches!(
                rename(&dir, ".", "a.txt", bad),
                Err(OpError::InvalidName { .. })
            ));
        }
        assert!(tmp.path().join("a.txt").exists(), "source untouched");
    }

    #[test]
    fn rename_cannot_escape() {
        let outer = tempfile::tempdir().unwrap();
        fs::create_dir(outer.path().join("root")).unwrap();
        fs::write(outer.path().join("secret"), b"s").unwrap();
        let dir = cap(&outer.path().join("root"));
        // A `from_name` that traverses out is refused by cap-std.
        assert!(matches!(
            rename(&dir, ".", "../secret", "stolen.txt"),
            Err(OpError::Io(_))
        ));
        assert!(!outer.path().join("root/stolen.txt").exists());
        assert_eq!(read(&outer.path().join("secret")), b"s");
    }

    // ---- copy_entry: files ------------------------------------------------

    #[test]
    fn copy_file_into_dir() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("f.txt"), b"hello").unwrap();
        fs::create_dir(tmp.path().join("sub")).unwrap();
        let dir = cap(tmp.path());
        let out = copy_entry(&dir, "f.txt", &dir, "sub", ConflictPolicy::Fail).unwrap();
        assert_eq!(out, OpOutcome::Created { target: PathBuf::from("sub/f.txt") });
        assert_eq!(read(&tmp.path().join("sub/f.txt")), b"hello");
        assert_eq!(read(&tmp.path().join("f.txt")), b"hello", "source intact");
    }

    #[test]
    fn copy_file_across_capabilities() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        fs::write(a.path().join("f.txt"), b"xfer").unwrap();
        let src = cap(a.path());
        let dst = cap(b.path());
        copy_entry(&src, "f.txt", &dst, ".", ConflictPolicy::Fail).unwrap();
        assert_eq!(read(&b.path().join("f.txt")), b"xfer");
        assert!(a.path().join("f.txt").exists(), "source root A intact");
    }

    #[test]
    fn copy_does_not_follow_symlink() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("target.txt"), b"TARGET").unwrap();
        std::os::unix::fs::symlink("target.txt", tmp.path().join("link")).unwrap();
        fs::create_dir(tmp.path().join("dst")).unwrap();
        let dir = cap(tmp.path());
        copy_entry(&dir, "link", &dir, "dst", ConflictPolicy::Fail).unwrap();
        // The copy is a symlink with the SAME raw target, not a regular file
        // carrying target.txt's contents.
        let md = fs::symlink_metadata(tmp.path().join("dst/link")).unwrap();
        assert!(md.file_type().is_symlink(), "the copied entry is a symlink");
        assert_eq!(
            fs::read_link(tmp.path().join("dst/link")).unwrap(),
            PathBuf::from("target.txt")
        );

        // A relative DANGLING link (target does not exist) still copies as a
        // dangling link, with no out-of-root read.
        std::os::unix::fs::symlink("does-not-exist.txt", tmp.path().join("dangle")).unwrap();
        copy_entry(&dir, "dangle", &dir, "dst", ConflictPolicy::Fail).unwrap();
        let dm = fs::symlink_metadata(tmp.path().join("dst/dangle")).unwrap();
        assert!(dm.file_type().is_symlink());
        assert_eq!(
            fs::read_link(tmp.path().join("dst/dangle")).unwrap(),
            PathBuf::from("does-not-exist.txt")
        );

        // A link with an ABSOLUTE target is refused by cap-std at the create
        // (escape-attempt hardening): copying it through the capability fails
        // closed rather than planting an absolute escaping link.
        std::os::unix::fs::symlink("/nonexistent/outside", tmp.path().join("abslink")).unwrap();
        assert!(
            matches!(
                copy_entry(&dir, "abslink", &dir, "dst", ConflictPolicy::Fail),
                Err(OpError::Io(_))
            ),
            "an absolute-target link is refused by the capability"
        );
        assert!(!tmp.path().join("dst/abslink").exists(), "nothing planted");
    }

    // ---- copy_entry: directories (recursive) ------------------------------

    #[test]
    fn copy_dir_recursive() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir(tmp.path().join("d")).unwrap();
        fs::write(tmp.path().join("d/a.txt"), b"A").unwrap();
        fs::create_dir(tmp.path().join("d/sub")).unwrap();
        fs::write(tmp.path().join("d/sub/b.txt"), b"B").unwrap();
        fs::create_dir(tmp.path().join("d/empty")).unwrap();
        fs::create_dir(tmp.path().join("into")).unwrap();
        let dir = cap(tmp.path());
        let out = copy_entry(&dir, "d", &dir, "into", ConflictPolicy::Fail).unwrap();
        assert_eq!(out, OpOutcome::Created { target: PathBuf::from("into/d") });
        assert_eq!(read(&tmp.path().join("into/d/a.txt")), b"A");
        assert_eq!(read(&tmp.path().join("into/d/sub/b.txt")), b"B");
        assert!(tmp.path().join("into/d/empty").is_dir());
        assert!(tmp.path().join("d/a.txt").exists(), "source tree intact");
    }

    #[test]
    fn copy_dir_recursive_preserves_a_nested_symlink_as_link() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir(tmp.path().join("d")).unwrap();
        fs::write(tmp.path().join("d/real.txt"), b"R").unwrap();
        std::os::unix::fs::symlink("real.txt", tmp.path().join("d/lnk")).unwrap();
        fs::create_dir(tmp.path().join("into")).unwrap();
        let dir = cap(tmp.path());
        copy_entry(&dir, "d", &dir, "into", ConflictPolicy::Fail).unwrap();
        let md = fs::symlink_metadata(tmp.path().join("into/d/lnk")).unwrap();
        assert!(md.file_type().is_symlink(), "nested symlink stays a link");
        assert_eq!(
            fs::read_link(tmp.path().join("into/d/lnk")).unwrap(),
            PathBuf::from("real.txt")
        );
    }

    // ---- conflict policy (driven via copy_entry) --------------------------

    #[test]
    fn conflict_skip_leaves_target_and_skips() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("f.txt"), b"Y").unwrap();
        fs::create_dir(tmp.path().join("dst")).unwrap();
        fs::write(tmp.path().join("dst/f.txt"), b"X").unwrap();
        let dir = cap(tmp.path());
        let out = copy_entry(&dir, "f.txt", &dir, "dst", ConflictPolicy::Skip).unwrap();
        assert_eq!(out, OpOutcome::Skipped);
        assert_eq!(read(&tmp.path().join("dst/f.txt")), b"X", "target untouched");
    }

    #[test]
    fn conflict_replace_overwrites_file() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("f.txt"), b"YY").unwrap();
        fs::create_dir(tmp.path().join("dst")).unwrap();
        fs::write(tmp.path().join("dst/f.txt"), b"XXXX").unwrap();
        let dir = cap(tmp.path());
        let out = copy_entry(&dir, "f.txt", &dir, "dst", ConflictPolicy::Replace).unwrap();
        assert_eq!(out, OpOutcome::Created { target: PathBuf::from("dst/f.txt") });
        assert_eq!(read(&tmp.path().join("dst/f.txt")), b"YY");
    }

    #[test]
    fn conflict_replace_over_a_symlink_replaces_the_entry_not_its_target() {
        // Regression for the MEDIUM finding: a Replace whose target is a symlink
        // must replace the LINK (the named entry), never write THROUGH it onto the
        // link's within-root target (which would clobber a file the user never
        // named). `Dir::copy` follows a final symlink, so copy_file_replacing
        // removes the non-dir target first.
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("s.txt"), b"SRC").unwrap();
        fs::create_dir(tmp.path().join("into")).unwrap();
        fs::write(tmp.path().join("into/data"), b"INNOCENT").unwrap();
        std::os::unix::fs::symlink("data", tmp.path().join("into/s.txt")).unwrap();
        let dir = cap(tmp.path());
        copy_entry(&dir, "s.txt", &dir, "into", ConflictPolicy::Replace).unwrap();
        // The named entry is now a regular file with the source bytes...
        let meta = std::fs::symlink_metadata(tmp.path().join("into/s.txt")).unwrap();
        assert!(
            meta.file_type().is_file(),
            "the symlink was replaced by a file, not written through"
        );
        assert_eq!(read(&tmp.path().join("into/s.txt")), b"SRC");
        // ...and the link's former target is UNTOUCHED.
        assert_eq!(read(&tmp.path().join("into/data")), b"INNOCENT");
    }

    #[test]
    fn conflict_replace_merges_directory() {
        let tmp = tempfile::tempdir().unwrap();
        // Source tree d/ with new.txt and a colliding keep.txt.
        fs::create_dir(tmp.path().join("d")).unwrap();
        fs::write(tmp.path().join("d/new.txt"), b"NEW").unwrap();
        fs::write(tmp.path().join("d/keep.txt"), b"FRESH").unwrap();
        // Destination already has d/ with keep.txt + an unrelated other.txt.
        fs::create_dir(tmp.path().join("into")).unwrap();
        fs::create_dir(tmp.path().join("into/d")).unwrap();
        fs::write(tmp.path().join("into/d/keep.txt"), b"OLD").unwrap();
        fs::write(tmp.path().join("into/d/other.txt"), b"SIBLING").unwrap();
        let dir = cap(tmp.path());
        copy_entry(&dir, "d", &dir, "into", ConflictPolicy::Replace).unwrap();
        // Merge, not delete-recreate: the sibling survives, keep.txt is overwritten,
        // new.txt is added.
        assert_eq!(read(&tmp.path().join("into/d/keep.txt")), b"FRESH");
        assert_eq!(read(&tmp.path().join("into/d/new.txt")), b"NEW");
        assert_eq!(
            read(&tmp.path().join("into/d/other.txt")),
            b"SIBLING",
            "merge kept the sibling the source did not carry"
        );
    }

    #[test]
    fn conflict_rename_suffixes() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("f.txt"), b"SRC").unwrap();
        fs::create_dir(tmp.path().join("dst")).unwrap();
        fs::write(tmp.path().join("dst/f.txt"), b"ORIG").unwrap();
        let dir = cap(tmp.path());
        let out = copy_entry(&dir, "f.txt", &dir, "dst", ConflictPolicy::Rename).unwrap();
        assert_eq!(
            out,
            OpOutcome::Created { target: PathBuf::from("dst/f (copy).txt") }
        );
        assert_eq!(read(&tmp.path().join("dst/f.txt")), b"ORIG", "original kept");
        assert_eq!(read(&tmp.path().join("dst/f (copy).txt")), b"SRC");
        // A second Rename copy bumps to (copy 2).
        let out2 = copy_entry(&dir, "f.txt", &dir, "dst", ConflictPolicy::Rename).unwrap();
        assert_eq!(
            out2,
            OpOutcome::Created { target: PathBuf::from("dst/f (copy 2).txt") }
        );
    }

    #[test]
    fn conflict_rename_extension_and_dotfile_splitting() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir(tmp.path().join("dst")).unwrap();
        // Multi-dot extension: suffix lands before the LAST dot.
        fs::write(tmp.path().join("a.tar.gz"), b"S").unwrap();
        fs::write(tmp.path().join("dst/a.tar.gz"), b"O").unwrap();
        // Leading-dot dotfile: whole name is the stem, suffix appended.
        fs::write(tmp.path().join(".bashrc"), b"S").unwrap();
        fs::write(tmp.path().join("dst/.bashrc"), b"O").unwrap();
        let dir = cap(tmp.path());
        let out = copy_entry(&dir, "a.tar.gz", &dir, "dst", ConflictPolicy::Rename).unwrap();
        assert_eq!(
            out,
            OpOutcome::Created { target: PathBuf::from("dst/a.tar (copy).gz") }
        );
        let out2 = copy_entry(&dir, ".bashrc", &dir, "dst", ConflictPolicy::Rename).unwrap();
        assert_eq!(
            out2,
            OpOutcome::Created { target: PathBuf::from("dst/.bashrc (copy)") }
        );
    }

    #[test]
    fn conflict_fail_errors() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("f.txt"), b"S").unwrap();
        fs::create_dir(tmp.path().join("dst")).unwrap();
        fs::write(tmp.path().join("dst/f.txt"), b"O").unwrap();
        let dir = cap(tmp.path());
        assert!(matches!(
            copy_entry(&dir, "f.txt", &dir, "dst", ConflictPolicy::Fail),
            Err(OpError::AlreadyExists { .. })
        ));
        assert_eq!(read(&tmp.path().join("dst/f.txt")), b"O", "nothing written");
    }

    #[test]
    fn conflict_replace_file_over_directory_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("x"), b"FILE").unwrap();
        fs::create_dir(tmp.path().join("dst")).unwrap();
        fs::create_dir(tmp.path().join("dst/x")).unwrap();
        let dir = cap(tmp.path());
        // A file source over an existing directory target clashes (the kernel's
        // EISDIR), not a blind removal of the directory.
        assert!(matches!(
            copy_entry(&dir, "x", &dir, "dst", ConflictPolicy::Replace),
            Err(OpError::Io(_))
        ));
        assert!(tmp.path().join("dst/x").is_dir(), "directory not removed");
    }

    // ---- move_entry -------------------------------------------------------

    #[test]
    fn move_same_fs_is_a_rename() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir(tmp.path().join("a")).unwrap();
        fs::create_dir(tmp.path().join("b")).unwrap();
        fs::write(tmp.path().join("a/f.txt"), b"M").unwrap();
        let dir = cap(tmp.path());
        let out = move_entry(&dir, "a/f.txt", &dir, "b", ConflictPolicy::Fail).unwrap();
        assert_eq!(out, OpOutcome::Renamed { target: PathBuf::from("b/f.txt") });
        assert_eq!(read(&tmp.path().join("b/f.txt")), b"M");
        assert!(!tmp.path().join("a/f.txt").exists(), "source gone (a real move)");
    }

    #[test]
    fn move_directory_same_fs() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir(tmp.path().join("tree")).unwrap();
        fs::write(tmp.path().join("tree/a.txt"), b"A").unwrap();
        fs::create_dir(tmp.path().join("tree/sub")).unwrap();
        fs::write(tmp.path().join("tree/sub/b.txt"), b"B").unwrap();
        fs::create_dir(tmp.path().join("dest")).unwrap();
        let dir = cap(tmp.path());
        move_entry(&dir, "tree", &dir, "dest", ConflictPolicy::Fail).unwrap();
        assert!(!tmp.path().join("tree").exists(), "whole tree moved");
        assert_eq!(read(&tmp.path().join("dest/tree/a.txt")), b"A");
        assert_eq!(read(&tmp.path().join("dest/tree/sub/b.txt")), b"B");
    }

    #[test]
    fn move_applies_conflict_policy() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("f.txt"), b"SRC").unwrap();
        fs::create_dir(tmp.path().join("dst")).unwrap();
        fs::write(tmp.path().join("dst/f.txt"), b"ORIG").unwrap();
        let dir = cap(tmp.path());
        let out = move_entry(&dir, "f.txt", &dir, "dst", ConflictPolicy::Rename).unwrap();
        assert_eq!(out, OpOutcome::Renamed { target: PathBuf::from("dst/f (copy).txt") });
        assert!(!tmp.path().join("f.txt").exists(), "source gone");
        assert_eq!(read(&tmp.path().join("dst/f.txt")), b"ORIG");
        assert_eq!(read(&tmp.path().join("dst/f (copy).txt")), b"SRC");
    }

    #[test]
    fn move_does_not_follow_symlink() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("target.txt"), b"T").unwrap();
        std::os::unix::fs::symlink("target.txt", tmp.path().join("link")).unwrap();
        fs::create_dir(tmp.path().join("dst")).unwrap();
        let dir = cap(tmp.path());
        move_entry(&dir, "link", &dir, "dst", ConflictPolicy::Fail).unwrap();
        let md = fs::symlink_metadata(tmp.path().join("dst/link")).unwrap();
        assert!(md.file_type().is_symlink(), "moved the link itself");
        assert_eq!(
            fs::read_link(tmp.path().join("dst/link")).unwrap(),
            PathBuf::from("target.txt")
        );
        assert!(!tmp.path().join("link").exists(), "source link gone");
        assert_eq!(read(&tmp.path().join("target.txt")), b"T", "target untouched");
    }

    #[test]
    fn move_cross_fs_fallback_via_copy_into_target_then_delete() {
        // A real EXDEV is not portably forced in CI, so exercise the fallback's
        // building blocks directly: copy into an exact resolved target, then
        // remove the source. This is what move_entry does on EXDEV.
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir(tmp.path().join("src")).unwrap();
        fs::write(tmp.path().join("src/f.txt"), b"X").unwrap();
        fs::create_dir(tmp.path().join("dst")).unwrap();
        let dir = cap(tmp.path());
        copy_into_target(
            &dir,
            Path::new("src/f.txt"),
            &dir,
            Path::new("dst/f.txt"),
            ConflictPolicy::Fail,
        )
        .unwrap();
        remove_source(&dir, Path::new("src/f.txt")).unwrap();
        assert_eq!(read(&tmp.path().join("dst/f.txt")), b"X");
        assert!(!tmp.path().join("src/f.txt").exists());
    }

    // ---- delete_permanent -------------------------------------------------

    #[test]
    fn delete_permanent_removes_file_dir_and_does_not_follow_symlink() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("f.txt"), b"x").unwrap();
        fs::create_dir(tmp.path().join("tree")).unwrap();
        fs::write(tmp.path().join("tree/a.txt"), b"a").unwrap();
        fs::create_dir(tmp.path().join("targetdir")).unwrap();
        fs::write(tmp.path().join("targetdir/keep.txt"), b"k").unwrap();
        std::os::unix::fs::symlink("targetdir", tmp.path().join("dirlink")).unwrap();
        let dir = cap(tmp.path());

        delete_permanent(&dir, "f.txt").unwrap();
        assert!(!tmp.path().join("f.txt").exists());

        delete_permanent(&dir, "tree").unwrap();
        assert!(!tmp.path().join("tree").exists(), "directory removed recursively");

        // A symlink to a directory is removed as the LINK; the target survives.
        delete_permanent(&dir, "dirlink").unwrap();
        assert!(!tmp.path().join("dirlink").exists(), "link removed");
        assert!(
            tmp.path().join("targetdir/keep.txt").exists(),
            "the symlink's target directory is untouched"
        );
    }

    // ---- trash_entry ------------------------------------------------------

    /// Build a trash capability with `files/` and `info/` pre-created (the host's
    /// job in production).
    fn make_trash(tmp: &Path) -> Dir {
        let trash = tmp.join("Trash");
        fs::create_dir(&trash).unwrap();
        fs::create_dir(trash.join("files")).unwrap();
        fs::create_dir(trash.join("info")).unwrap();
        cap(&trash)
    }

    #[test]
    fn trash_moves_file_and_writes_trashinfo() {
        let tmp = tempfile::tempdir().unwrap();
        let src_root = tmp.path().join("home");
        fs::create_dir(&src_root).unwrap();
        fs::write(src_root.join("doc.txt"), b"CONTENT").unwrap();
        let src_dir = cap(&src_root);
        let trash_dir = make_trash(tmp.path());

        let out = trash_entry_at(
            &src_dir,
            "doc.txt",
            &trash_dir,
            Path::new("/home/u/my docs/doc.txt"),
            fixed_now(),
        )
        .unwrap();
        assert_eq!(out.trashed_name, "doc.txt");
        // The content moved into files/, the source is gone.
        assert_eq!(read(&tmp.path().join("Trash/files/doc.txt")), b"CONTENT");
        assert!(!src_root.join("doc.txt").exists());
        // The .trashinfo records the exact spec'd bytes (space -> %20, / literal).
        let info = String::from_utf8(read(&tmp.path().join("Trash/info/doc.txt.trashinfo"))).unwrap();
        assert_eq!(
            info,
            "[Trash Info]\nPath=/home/u/my%20docs/doc.txt\nDeletionDate=2026-06-11T14:03:09\n"
        );
    }

    #[test]
    fn trash_name_collision_suffixes_and_pairs() {
        let tmp = tempfile::tempdir().unwrap();
        let src_root = tmp.path().join("home");
        fs::create_dir(&src_root).unwrap();
        fs::write(src_root.join("doc.txt"), b"SECOND").unwrap();
        let src_dir = cap(&src_root);
        let trash_dir = make_trash(tmp.path());
        // Pre-occupy the pair `doc.txt` in the trash.
        fs::write(tmp.path().join("Trash/files/doc.txt"), b"FIRST").unwrap();
        fs::write(tmp.path().join("Trash/info/doc.txt.trashinfo"), b"[Trash Info]\n").unwrap();

        let out = trash_entry_at(
            &src_dir,
            "doc.txt",
            &trash_dir,
            Path::new("/home/u/doc.txt"),
            fixed_now(),
        )
        .unwrap();
        assert_eq!(out.trashed_name, "doc (copy).txt");
        // The new pair shares the suffixed name; the pre-existing pair is untouched.
        assert_eq!(read(&tmp.path().join("Trash/files/doc (copy).txt")), b"SECOND");
        assert!(tmp.path().join("Trash/info/doc (copy).txt.trashinfo").exists());
        assert_eq!(read(&tmp.path().join("Trash/files/doc.txt")), b"FIRST", "first kept");
    }

    #[test]
    fn trash_info_reservation_rolls_back_on_move_failure() {
        let tmp = tempfile::tempdir().unwrap();
        let src_root = tmp.path().join("home");
        fs::create_dir(&src_root).unwrap();
        // No `missing.txt` exists, so the rename into the trash fails (ENOENT).
        let src_dir = cap(&src_root);
        let trash_dir = make_trash(tmp.path());

        let res = trash_entry_at(
            &src_dir,
            "missing.txt",
            &trash_dir,
            Path::new("/home/u/missing.txt"),
            fixed_now(),
        );
        assert!(res.is_err(), "trashing a non-existent entry fails");
        // The reserved .trashinfo was rolled back: no orphan in info/.
        assert_eq!(
            fs::read_dir(tmp.path().join("Trash/info")).unwrap().count(),
            0,
            "no orphan .trashinfo left behind"
        );
        assert_eq!(
            fs::read_dir(tmp.path().join("Trash/files")).unwrap().count(),
            0
        );
    }

    #[test]
    fn trash_does_not_follow_symlink() {
        let tmp = tempfile::tempdir().unwrap();
        let src_root = tmp.path().join("home");
        fs::create_dir(&src_root).unwrap();
        fs::write(src_root.join("target.txt"), b"KEEP").unwrap();
        std::os::unix::fs::symlink("target.txt", src_root.join("link")).unwrap();
        let src_dir = cap(&src_root);
        let trash_dir = make_trash(tmp.path());

        let out = trash_entry_at(
            &src_dir,
            "link",
            &trash_dir,
            Path::new("/home/u/link"),
            fixed_now(),
        )
        .unwrap();
        assert_eq!(out.trashed_name, "link");
        // The trashed entry is the link itself; the target stays in place.
        let md = fs::symlink_metadata(tmp.path().join("Trash/files/link")).unwrap();
        assert!(md.file_type().is_symlink(), "trashed the link, not the target");
        assert_eq!(read(&src_root.join("target.txt")), b"KEEP", "target untouched");
    }

    #[test]
    fn trash_percent_encoding_of_original_path() {
        // A path with a space, a literal %, a non-ASCII byte and separators.
        let now = fixed_now();
        let bytes = trashinfo_bytes(Path::new("/home/u/a b/100%/café.txt"), now);
        let s = String::from_utf8(bytes).unwrap();
        assert!(s.contains("Path=/home/u/a%20b/100%25/caf%C3%A9.txt"), "got: {s}");
        // Separators stayed literal, the date is the spec format.
        assert!(s.contains("DeletionDate=2026-06-11T14:03:09\n"));
        assert!(s.starts_with("[Trash Info]\n"));
    }

    // ---- cap-std escape floor (one per mutating op) -----------------------

    #[test]
    fn copy_entry_refuses_dotdot_and_absolute() {
        let outer = tempfile::tempdir().unwrap();
        fs::create_dir(outer.path().join("root")).unwrap();
        fs::write(outer.path().join("secret"), b"s").unwrap();
        fs::create_dir(outer.path().join("root/dst")).unwrap();
        let dir = cap(&outer.path().join("root"));
        // A traversing source and an absolute source are both refused by cap-std.
        assert!(copy_entry(&dir, "../secret", &dir, "dst", ConflictPolicy::Fail).is_err());
        assert!(copy_entry(&dir, "/etc/hostname", &dir, "dst", ConflictPolicy::Fail).is_err());
        assert!(!outer.path().join("root/dst/secret").exists());
        assert!(!outer.path().join("root/dst/hostname").exists());
        assert_eq!(read(&outer.path().join("secret")), b"s", "outside file untouched");
    }

    #[test]
    fn move_entry_refuses_dotdot_and_absolute() {
        let outer = tempfile::tempdir().unwrap();
        fs::create_dir(outer.path().join("root")).unwrap();
        fs::write(outer.path().join("secret"), b"s").unwrap();
        fs::create_dir(outer.path().join("root/dst")).unwrap();
        let dir = cap(&outer.path().join("root"));
        assert!(move_entry(&dir, "../secret", &dir, "dst", ConflictPolicy::Fail).is_err());
        assert!(move_entry(&dir, "/etc/hostname", &dir, "dst", ConflictPolicy::Fail).is_err());
        assert!(!outer.path().join("root/dst/secret").exists());
        assert_eq!(read(&outer.path().join("secret")), b"s", "outside file not moved");
    }

    #[test]
    fn delete_permanent_refuses_dotdot_and_absolute() {
        let outer = tempfile::tempdir().unwrap();
        fs::create_dir(outer.path().join("root")).unwrap();
        fs::write(outer.path().join("secret"), b"s").unwrap();
        let dir = cap(&outer.path().join("root"));
        assert!(delete_permanent(&dir, "../secret").is_err());
        assert!(delete_permanent(&dir, "/etc/hostname").is_err());
        assert_eq!(read(&outer.path().join("secret")), b"s", "outside file not deleted");
    }

    #[test]
    fn trash_entry_refuses_dotdot_and_absolute_source() {
        let outer = tempfile::tempdir().unwrap();
        fs::create_dir(outer.path().join("root")).unwrap();
        fs::write(outer.path().join("secret"), b"s").unwrap();
        let src_dir = cap(&outer.path().join("root"));
        let trash_dir = make_trash(outer.path());
        // A traversing or absolute source is refused; the reserved info is rolled back.
        assert!(trash_entry_at(
            &src_dir,
            "../secret",
            &trash_dir,
            Path::new("/x"),
            fixed_now()
        )
        .is_err());
        assert!(trash_entry_at(
            &src_dir,
            "/etc/hostname",
            &trash_dir,
            Path::new("/x"),
            fixed_now()
        )
        .is_err());
        assert_eq!(read(&outer.path().join("secret")), b"s", "outside file not trashed");
        assert_eq!(
            fs::read_dir(outer.path().join("Trash/info")).unwrap().count(),
            0,
            "no orphan info from the refused trashes"
        );
    }

    // ---- read_trashinfo / list_trash / restore_entry ----------------------

    #[test]
    fn read_trashinfo_round_trips_trash_entry() {
        let tmp = tempfile::tempdir().unwrap();
        let src_root = tmp.path().join("home");
        fs::create_dir(&src_root).unwrap();
        fs::write(src_root.join("doc.txt"), b"C").unwrap();
        let src_dir = cap(&src_root);
        let trash_dir = make_trash(tmp.path());
        trash_entry_at(
            &src_dir,
            "doc.txt",
            &trash_dir,
            Path::new("/home/u/my docs/doc.txt"),
            fixed_now(),
        )
        .unwrap();

        let info = read_trashinfo(&trash_dir, "doc.txt").unwrap();
        // The %20 was decoded back to a space; the date is verbatim.
        assert_eq!(info.original_path, PathBuf::from("/home/u/my docs/doc.txt"));
        assert_eq!(info.deletion_date, "2026-06-11T14:03:09");
    }

    #[test]
    fn read_trashinfo_decodes_nonascii_and_percent() {
        let tmp = tempfile::tempdir().unwrap();
        let src_root = tmp.path().join("home");
        fs::create_dir(&src_root).unwrap();
        fs::write(src_root.join("café.txt"), b"C").unwrap();
        let src_dir = cap(&src_root);
        let trash_dir = make_trash(tmp.path());
        let original = Path::new("/home/u/100%/café.txt");
        let out = trash_entry_at(&src_dir, "café.txt", &trash_dir, original, fixed_now()).unwrap();

        let info = read_trashinfo(&trash_dir, &out.trashed_name).unwrap();
        // The exact inverse of the percent-encode: %25 -> %, %C3%A9 -> é bytes.
        assert_eq!(info.original_path, original.to_path_buf());
    }

    #[test]
    fn parse_trashinfo_tolerates_key_ordering() {
        let bytes = b"# a leading comment\n\n[Trash Info]\nDeletionDate=2026-01-02T03:04:05\nPath=/home/u/x.txt\n\n";
        let info = parse_trashinfo(bytes).unwrap();
        assert_eq!(info.original_path, PathBuf::from("/home/u/x.txt"));
        assert_eq!(info.deletion_date, "2026-01-02T03:04:05");
    }

    #[test]
    fn parse_trashinfo_path_with_no_encoding() {
        let info = parse_trashinfo(b"[Trash Info]\nPath=/home/u/plain.txt\n").unwrap();
        assert_eq!(info.original_path, PathBuf::from("/home/u/plain.txt"));
    }

    #[test]
    fn parse_trashinfo_missing_path_fails_closed() {
        let err = parse_trashinfo(b"[Trash Info]\nDeletionDate=2026-01-02T03:04:05\n");
        assert!(matches!(err, Err(OpError::Io(_))), "missing Path is refused");
    }

    #[test]
    fn parse_trashinfo_missing_header_fails_closed() {
        let err = parse_trashinfo(b"Path=/x\nDeletionDate=2026-01-02T03:04:05\n");
        assert!(matches!(err, Err(OpError::Io(_))), "missing header is refused");
    }

    #[test]
    fn parse_trashinfo_duplicate_path_fails_closed() {
        let bytes = b"[Trash Info]\nPath=/home/u/benign.txt\nPath=/etc/passwd\n";
        let err = parse_trashinfo(bytes);
        assert!(
            matches!(err, Err(OpError::Io(_))),
            "a duplicate Path is refused, not last-wins-spoofed"
        );
    }

    #[test]
    fn parse_trashinfo_missing_date_is_tolerated() {
        let info = parse_trashinfo(b"[Trash Info]\nPath=/home/u/x.txt\n").unwrap();
        assert_eq!(info.original_path, PathBuf::from("/home/u/x.txt"));
        assert_eq!(info.deletion_date, "", "missing DeletionDate -> empty string");
    }

    #[test]
    fn read_trashinfo_oversized_fails_closed() {
        let tmp = tempfile::tempdir().unwrap();
        let trash_dir = make_trash(tmp.path());
        // Write an info file larger than the bound.
        let mut bytes = b"[Trash Info]\nPath=/home/u/x.txt\nDeletionDate=2026-01-02T03:04:05\n".to_vec();
        bytes.extend(std::iter::repeat_n(b'#', MAX_TRASHINFO_BYTES + 1024));
        fs::write(tmp.path().join("Trash/info/big.trashinfo"), &bytes).unwrap();
        let err = read_trashinfo(&trash_dir, "big");
        assert!(
            matches!(err, Err(OpError::Io(_))),
            "an oversized trashinfo is refused, not truncated and parsed"
        );
    }

    #[test]
    fn parse_trashinfo_non_utf8_value_decodes_to_pathbuf() {
        // %FF decodes to the raw byte 0xFF, which is not valid UTF-8: the
        // PathBuf must carry it losslessly (proving OsString::from_vec, not a
        // lossy String).
        let info = parse_trashinfo(b"[Trash Info]\nPath=/x/%FF\n").unwrap();
        let expected = PathBuf::from(OsString::from_vec(vec![b'/', b'x', b'/', 0xFF]));
        assert_eq!(info.original_path, expected);
        // The raw byte survived (the path is not representable as UTF-8).
        assert!(info.original_path.to_str().is_none(), "the path is non-UTF-8");
    }

    /// Trash a real file from `src_root` and return its trashed name.
    fn trash_one(tmp: &Path, src_root: &Path, trash_dir: &Dir, name: &str, original: &str) -> String {
        fs::write(src_root.join(name), b"C").unwrap();
        let _ = tmp; // the trash lives under tmp via trash_dir
        let src_dir = cap(src_root);
        trash_entry_at(&src_dir, name, trash_dir, Path::new(original), fixed_now())
            .unwrap()
            .trashed_name
    }

    #[test]
    fn list_trash_pairs_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let src_root = tmp.path().join("home");
        fs::create_dir(&src_root).unwrap();
        let trash_dir = make_trash(tmp.path());
        trash_one(tmp.path(), &src_root, &trash_dir, "a.txt", "/home/u/a.txt");
        trash_one(tmp.path(), &src_root, &trash_dir, "b.txt", "/home/u/b.txt");

        let mut items = list_trash(&trash_dir).unwrap();
        items.sort_by(|x, y| x.trashed_name.cmp(&y.trashed_name));
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].trashed_name, "a.txt");
        assert_eq!(items[0].original_path, PathBuf::from("/home/u/a.txt"));
        assert_eq!(items[0].deletion_date, "2026-06-11T14:03:09");
        assert_eq!(items[1].trashed_name, "b.txt");
        assert_eq!(items[1].original_path, PathBuf::from("/home/u/b.txt"));
    }

    #[test]
    fn list_trash_skips_orphan_info() {
        let tmp = tempfile::tempdir().unwrap();
        let src_root = tmp.path().join("home");
        fs::create_dir(&src_root).unwrap();
        let trash_dir = make_trash(tmp.path());
        // A real, well-paired entry...
        trash_one(tmp.path(), &src_root, &trash_dir, "real.txt", "/home/u/real.txt");
        // ...and an orphan info with NO matching files/ entry.
        fs::write(
            tmp.path().join("Trash/info/ghost.txt.trashinfo"),
            b"[Trash Info]\nPath=/home/u/ghost.txt\nDeletionDate=2026-01-02T03:04:05\n",
        )
        .unwrap();

        let items = list_trash(&trash_dir).unwrap();
        assert_eq!(items.len(), 1, "only the well-paired entry is listed");
        assert_eq!(items[0].trashed_name, "real.txt");
    }

    #[test]
    fn list_trash_skips_orphan_files_entry() {
        let tmp = tempfile::tempdir().unwrap();
        let trash_dir = make_trash(tmp.path());
        // A files/ entry with no info/ pairing: enumeration is info-driven, so it
        // is never listed.
        fs::write(tmp.path().join("Trash/files/x"), b"X").unwrap();
        let items = list_trash(&trash_dir).unwrap();
        assert!(items.is_empty(), "an untracked files/ entry is not listed");
    }

    #[test]
    fn list_trash_skips_malformed_info() {
        let tmp = tempfile::tempdir().unwrap();
        let src_root = tmp.path().join("home");
        fs::create_dir(&src_root).unwrap();
        let trash_dir = make_trash(tmp.path());
        // A valid, well-paired entry survives the listing...
        trash_one(tmp.path(), &src_root, &trash_dir, "ok.txt", "/home/u/ok.txt");
        // ...a malformed info (missing Path) paired with a files/ entry is skipped.
        fs::write(tmp.path().join("Trash/files/m"), b"M").unwrap();
        fs::write(
            tmp.path().join("Trash/info/m.trashinfo"),
            b"[Trash Info]\nDeletionDate=2026-01-02T03:04:05\n",
        )
        .unwrap();

        let items = list_trash(&trash_dir).unwrap();
        assert_eq!(items.len(), 1, "the malformed entry is skipped");
        assert_eq!(items[0].trashed_name, "ok.txt");
    }

    #[test]
    fn list_trash_empty_is_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let trash_dir = make_trash(tmp.path());
        assert_eq!(list_trash(&trash_dir).unwrap(), vec![]);
    }

    #[test]
    fn empty_trash_clears_paired_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let src_root = tmp.path().join("home");
        fs::create_dir(&src_root).unwrap();
        let trash_dir = make_trash(tmp.path());
        trash_one(tmp.path(), &src_root, &trash_dir, "a.txt", "/home/u/a.txt");
        trash_one(tmp.path(), &src_root, &trash_dir, "b.txt", "/home/u/b.txt");

        assert_eq!(empty_trash(&trash_dir).unwrap(), 2);
        assert!(list_trash(&trash_dir).unwrap().is_empty());
        // Both sides are gone, not just the listing.
        assert!(fs::read_dir(tmp.path().join("Trash/files")).unwrap().next().is_none());
        assert!(fs::read_dir(tmp.path().join("Trash/info")).unwrap().next().is_none());
    }

    #[test]
    fn empty_trash_clears_an_orphan_info() {
        let tmp = tempfile::tempdir().unwrap();
        let trash_dir = make_trash(tmp.path());
        fs::write(
            tmp.path().join("Trash/info/ghost.txt.trashinfo"),
            b"[Trash Info]\nPath=/home/u/ghost.txt\nDeletionDate=2026-01-02T03:04:05\n",
        )
        .unwrap();
        assert_eq!(empty_trash(&trash_dir).unwrap(), 1);
        assert!(fs::read_dir(tmp.path().join("Trash/info")).unwrap().next().is_none());
    }

    #[test]
    fn empty_trash_on_empty_is_zero() {
        let tmp = tempfile::tempdir().unwrap();
        let trash_dir = make_trash(tmp.path());
        assert_eq!(empty_trash(&trash_dir).unwrap(), 0);
    }

    #[test]
    fn create_symlink_makes_a_link_and_refuses_collision() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = cap(tmp.path());
        // A relative, in-capability target: cap-std can read it back verbatim.
        let out = create_symlink(&dir, ".", "link", "target.txt").unwrap();
        assert_eq!(out, OpOutcome::Created { target: PathBuf::from("./link") });
        let md = dir.symlink_metadata("link").unwrap();
        assert!(md.file_type().is_symlink());
        assert_eq!(dir.read_link("link").unwrap(), PathBuf::from("target.txt"));
        // A second create at the same name is refused (no silent replace).
        assert!(matches!(
            create_symlink(&dir, ".", "link", "other"),
            Err(OpError::AlreadyExists { .. })
        ));
        // An absolute / escaping target is refused at creation: the confined
        // FM cannot mint a symlink pointing outside its capability root.
        assert!(create_symlink(&dir, ".", "abs", "/etc/hostname").is_err());
    }

    #[test]
    fn restore_round_trips_a_file_back() {
        let tmp = tempfile::tempdir().unwrap();
        let src_root = tmp.path().join("home");
        fs::create_dir(&src_root).unwrap();
        fs::write(src_root.join("doc.txt"), b"CONTENT").unwrap();
        let src_dir = cap(&src_root);
        let trash_dir = make_trash(tmp.path());
        trash_entry_at(&src_dir, "doc.txt", &trash_dir, Path::new("/home/u/doc.txt"), fixed_now())
            .unwrap();

        // The host chose `dest_dir` = the home root and `dest_rel` = doc.txt.
        let dest_dir = cap(&src_root);
        let out = restore_entry(
            &trash_dir,
            "doc.txt",
            &dest_dir,
            Path::new("doc.txt"),
            ConflictPolicy::Fail,
        )
        .unwrap();
        assert_eq!(out, OpOutcome::Renamed { target: PathBuf::from("doc.txt") });
        assert_eq!(read(&src_root.join("doc.txt")), b"CONTENT", "restored with its bytes");
        // BOTH trash sides are gone.
        assert!(!tmp.path().join("Trash/files/doc.txt").exists(), "files entry removed");
        assert!(
            !tmp.path().join("Trash/info/doc.txt.trashinfo").exists(),
            "trashinfo removed"
        );
    }

    #[test]
    fn restore_to_a_different_name_than_the_trashed_name() {
        let tmp = tempfile::tempdir().unwrap();
        let src_root = tmp.path().join("home");
        fs::create_dir(&src_root).unwrap();
        fs::write(src_root.join("doc.txt"), b"SECOND").unwrap();
        let src_dir = cap(&src_root);
        let trash_dir = make_trash(tmp.path());
        // Pre-occupy `doc.txt` in the trash so the new one is suffixed.
        fs::write(tmp.path().join("Trash/files/doc.txt"), b"FIRST").unwrap();
        fs::write(tmp.path().join("Trash/info/doc.txt.trashinfo"), b"[Trash Info]\n").unwrap();
        let out = trash_entry_at(
            &src_dir,
            "doc.txt",
            &trash_dir,
            Path::new("/home/u/doc.txt"),
            fixed_now(),
        )
        .unwrap();
        assert_eq!(out.trashed_name, "doc (copy).txt");

        // Restore the suffixed entry back to its original name `doc.txt`.
        fs::create_dir(tmp.path().join("restored")).unwrap();
        let dest_dir = cap(&tmp.path().join("restored"));
        let result = restore_entry(
            &trash_dir,
            "doc (copy).txt",
            &dest_dir,
            Path::new("doc.txt"),
            ConflictPolicy::Fail,
        )
        .unwrap();
        assert_eq!(result, OpOutcome::Renamed { target: PathBuf::from("doc.txt") });
        // It landed under the host-chosen name, not the trashed `(copy)` name.
        assert_eq!(read(&tmp.path().join("restored/doc.txt")), b"SECOND");
        assert!(!tmp.path().join("Trash/files/doc (copy).txt").exists());
        assert!(!tmp.path().join("Trash/info/doc (copy).txt.trashinfo").exists());
    }

    #[test]
    fn restore_is_conflict_aware_rename() {
        let tmp = tempfile::tempdir().unwrap();
        let src_root = tmp.path().join("home");
        fs::create_dir(&src_root).unwrap();
        fs::write(src_root.join("doc.txt"), b"TRASHED").unwrap();
        let src_dir = cap(&src_root);
        let trash_dir = make_trash(tmp.path());
        trash_entry_at(&src_dir, "doc.txt", &trash_dir, Path::new("/home/u/doc.txt"), fixed_now())
            .unwrap();
        // The destination already has a doc.txt.
        let dest_dir = cap(&src_root);
        fs::write(src_root.join("doc.txt"), b"EXISTING").unwrap();

        let out = restore_entry(
            &trash_dir,
            "doc.txt",
            &dest_dir,
            Path::new("doc.txt"),
            ConflictPolicy::Rename,
        )
        .unwrap();
        assert_eq!(out, OpOutcome::Renamed { target: PathBuf::from("doc (copy).txt") });
        assert_eq!(read(&src_root.join("doc.txt")), b"EXISTING", "the existing file is kept");
        assert_eq!(read(&src_root.join("doc (copy).txt")), b"TRASHED");
        // The trash pair was removed (a successful move).
        assert!(!tmp.path().join("Trash/files/doc.txt").exists());
        assert!(!tmp.path().join("Trash/info/doc.txt.trashinfo").exists());
    }

    #[test]
    fn restore_is_conflict_aware_skip() {
        let tmp = tempfile::tempdir().unwrap();
        let src_root = tmp.path().join("home");
        fs::create_dir(&src_root).unwrap();
        fs::write(src_root.join("doc.txt"), b"TRASHED").unwrap();
        let src_dir = cap(&src_root);
        let trash_dir = make_trash(tmp.path());
        trash_entry_at(&src_dir, "doc.txt", &trash_dir, Path::new("/home/u/doc.txt"), fixed_now())
            .unwrap();
        let dest_dir = cap(&src_root);
        fs::write(src_root.join("doc.txt"), b"EXISTING").unwrap();

        let out = restore_entry(
            &trash_dir,
            "doc.txt",
            &dest_dir,
            Path::new("doc.txt"),
            ConflictPolicy::Skip,
        )
        .unwrap();
        assert_eq!(out, OpOutcome::Skipped);
        assert_eq!(read(&src_root.join("doc.txt")), b"EXISTING", "destination untouched");
        // A skipped restore did NOT move the entry, so the trash pair stays INTACT.
        assert_eq!(read(&tmp.path().join("Trash/files/doc.txt")), b"TRASHED");
        assert!(tmp.path().join("Trash/info/doc.txt.trashinfo").exists());
    }

    #[test]
    fn restore_is_conflict_aware_fail() {
        let tmp = tempfile::tempdir().unwrap();
        let src_root = tmp.path().join("home");
        fs::create_dir(&src_root).unwrap();
        fs::write(src_root.join("doc.txt"), b"TRASHED").unwrap();
        let src_dir = cap(&src_root);
        let trash_dir = make_trash(tmp.path());
        trash_entry_at(&src_dir, "doc.txt", &trash_dir, Path::new("/home/u/doc.txt"), fixed_now())
            .unwrap();
        let dest_dir = cap(&src_root);
        fs::write(src_root.join("doc.txt"), b"EXISTING").unwrap();

        let err = restore_entry(
            &trash_dir,
            "doc.txt",
            &dest_dir,
            Path::new("doc.txt"),
            ConflictPolicy::Fail,
        );
        assert!(matches!(err, Err(OpError::AlreadyExists { .. })));
        assert_eq!(read(&src_root.join("doc.txt")), b"EXISTING", "destination untouched");
        // The trash pair is intact (the move never happened).
        assert_eq!(read(&tmp.path().join("Trash/files/doc.txt")), b"TRASHED");
        assert!(tmp.path().join("Trash/info/doc.txt.trashinfo").exists());
    }

    #[test]
    fn restore_refuses_a_dest_rel_escape() {
        // The cap-std floor: the untrusted recorded path can NEVER become an
        // arbitrary write, because restore only ever moves into the host's
        // capability + relative path. A `..` or absolute `dest_rel` is refused
        // by cap-std, before the info removal, leaving the trash pair intact.
        let outer = tempfile::tempdir().unwrap();
        fs::create_dir(outer.path().join("home")).unwrap();
        fs::write(outer.path().join("home/doc.txt"), b"TRASHED").unwrap();
        fs::write(outer.path().join("secret"), b"s").unwrap();
        let src_dir = cap(&outer.path().join("home"));
        let trash_dir = make_trash(outer.path());
        trash_entry_at(&src_dir, "doc.txt", &trash_dir, Path::new("/home/u/doc.txt"), fixed_now())
            .unwrap();
        // The host capability is the home root; an escaping dest_rel is refused.
        let dest_dir = cap(&outer.path().join("home"));

        assert!(matches!(
            restore_entry(
                &trash_dir,
                "doc.txt",
                &dest_dir,
                Path::new("../escape.txt"),
                ConflictPolicy::Fail
            ),
            Err(OpError::Io(_))
        ));
        assert!(matches!(
            restore_entry(
                &trash_dir,
                "doc.txt",
                &dest_dir,
                Path::new("/etc/evil"),
                ConflictPolicy::Fail
            ),
            Err(OpError::Io(_))
        ));
        // Nothing written outside the dest root...
        assert!(!outer.path().join("escape.txt").exists());
        assert!(!outer.path().join("home/../escape.txt").exists());
        assert_eq!(read(&outer.path().join("secret")), b"s", "outside file untouched");
        // ...and the trash pair is left intact (the move was refused before the
        // info removal).
        assert_eq!(read(&outer.path().join("Trash/files/doc.txt")), b"TRASHED");
        assert!(outer.path().join("Trash/info/doc.txt.trashinfo").exists());
    }

    #[test]
    fn restore_does_not_follow_symlink() {
        let tmp = tempfile::tempdir().unwrap();
        let src_root = tmp.path().join("home");
        fs::create_dir(&src_root).unwrap();
        fs::write(src_root.join("target.txt"), b"KEEP").unwrap();
        std::os::unix::fs::symlink("target.txt", src_root.join("link")).unwrap();
        let src_dir = cap(&src_root);
        let trash_dir = make_trash(tmp.path());
        // The link is trashed as a link (per trash_does_not_follow_symlink).
        trash_entry_at(&src_dir, "link", &trash_dir, Path::new("/home/u/link"), fixed_now())
            .unwrap();

        fs::create_dir(tmp.path().join("restored")).unwrap();
        let dest_dir = cap(&tmp.path().join("restored"));
        restore_entry(
            &trash_dir,
            "link",
            &dest_dir,
            Path::new("link"),
            ConflictPolicy::Fail,
        )
        .unwrap();
        // The restored entry is a symlink with the same raw target.
        let md = fs::symlink_metadata(tmp.path().join("restored/link")).unwrap();
        assert!(md.file_type().is_symlink(), "restored as the link, not the target");
        assert_eq!(
            fs::read_link(tmp.path().join("restored/link")).unwrap(),
            PathBuf::from("target.txt")
        );
        // The link's target file is untouched.
        assert_eq!(read(&src_root.join("target.txt")), b"KEEP");
    }

    #[test]
    fn restore_cross_fs_via_copy_into_target_then_delete() {
        // A real EXDEV is not portably forced in CI, so exercise the building
        // blocks the restore path uses on the cross-fs/merge fallback: copy from
        // files/ into the exact dest, remove the files/ source, remove the info.
        let tmp = tempfile::tempdir().unwrap();
        let src_root = tmp.path().join("home");
        fs::create_dir(&src_root).unwrap();
        fs::write(src_root.join("doc.txt"), b"X").unwrap();
        let src_dir = cap(&src_root);
        let trash_dir = make_trash(tmp.path());
        trash_entry_at(&src_dir, "doc.txt", &trash_dir, Path::new("/home/u/doc.txt"), fixed_now())
            .unwrap();

        fs::create_dir(tmp.path().join("restored")).unwrap();
        let dest_dir = cap(&tmp.path().join("restored"));
        copy_into_target(
            &trash_dir,
            Path::new("files/doc.txt"),
            &dest_dir,
            Path::new("doc.txt"),
            ConflictPolicy::Fail,
        )
        .unwrap();
        remove_source(&trash_dir, Path::new("files/doc.txt")).unwrap();
        trash_dir.remove_file("info/doc.txt.trashinfo").unwrap();
        assert_eq!(read(&tmp.path().join("restored/doc.txt")), b"X");
        assert!(!tmp.path().join("Trash/files/doc.txt").exists());
        assert!(!tmp.path().join("Trash/info/doc.txt.trashinfo").exists());
    }

    #[test]
    fn restore_of_a_nonexistent_trashed_name_fails_and_leaves_trash_unchanged() {
        let tmp = tempfile::tempdir().unwrap();
        let src_root = tmp.path().join("home");
        fs::create_dir(&src_root).unwrap();
        fs::write(src_root.join("real.txt"), b"R").unwrap();
        let src_dir = cap(&src_root);
        let trash_dir = make_trash(tmp.path());
        // An unrelated entry sits in the trash and must be untouched.
        trash_entry_at(&src_dir, "real.txt", &trash_dir, Path::new("/home/u/real.txt"), fixed_now())
            .unwrap();

        fs::create_dir(tmp.path().join("restored")).unwrap();
        let dest_dir = cap(&tmp.path().join("restored"));
        let err = restore_entry(
            &trash_dir,
            "nope.txt",
            &dest_dir,
            Path::new("nope.txt"),
            ConflictPolicy::Fail,
        );
        assert!(matches!(err, Err(OpError::Io(_))), "the files/ move ENOENTs");
        // The info removal never ran (the move failed); the unrelated entry stays.
        assert_eq!(read(&tmp.path().join("Trash/files/real.txt")), b"R");
        assert!(tmp.path().join("Trash/info/real.txt.trashinfo").exists());
        assert!(!tmp.path().join("restored/nope.txt").exists());
    }
}

//! The freedesktop home-trash primitive, shared by every Arlen component that
//! reversibly deletes (the AI executor's `fs.trash`, the trash-first `rm`, the file
//! manager). One implementation of the security-sensitive move so the no-clobber
//! TOCTOU close and the `.trashinfo` sidecar are written identically everywhere.
//!
//! [`trash_into`] reserves a unique slot under `Trash/files/`, writes the
//! `Trash/info/<name>.trashinfo` sidecar first (freedesktop info-first), and moves
//! the entity with [`rename_noreplace`] (`RENAME_NOREPLACE`, so a racing same-uid
//! process cannot make the move clobber a file the reversible tier promised to
//! restore). A failed move drops the orphaned sidecar, so a failure leaves no
//! partial state. Every candidate's canonical paths are validated before its move,
//! so a returned [`TrashSlot`] always yields a constructible inverse.
//!
//! HOME trash only, and that is a real limit rather than a naming choice.
//! The move is a rename, so an entity on another filesystem - a USB stick, a
//! second disk, anything under /tmp - cannot go into `$HOME`'s trash at all;
//! the kernel answers EXDEV. The spec's answer is a `.Trash-$uid` at the top of
//! THAT filesystem, with its own sticky-bit rules, and that is not implemented
//! here. Until it is, [`TrashError::CrossDevice`] names the case, so that every
//! caller (the viewer, the file manager, the executor's `fs.trash`, the
//! trash-first `rm`) can say which thing went wrong instead of showing a kernel
//! string.

use arlen_ai_undo_core::effect_model::CanonicalPath;
use std::ffi::CString;
use std::path::{Path, PathBuf};

/// Why an atomic no-clobber rename could not complete.
#[derive(Debug)]
pub enum RenameError {
    /// `to` already exists; the kernel refused to clobber it (`EEXIST`).
    DestinationExists,
    /// The kernel or filesystem does not support `RENAME_NOREPLACE`. Refuse the
    /// move rather than fall back to a clobbering rename.
    Unsupported,
    /// The source is on a different filesystem from the home trash (`EXDEV`), so
    /// a rename cannot move it there at all.
    CrossDevice,
    /// Any other rename failure (permissions, a NUL in the path, ...).
    Other(String),
}

/// Rename `from` to `to`, refusing to overwrite an existing `to`
/// (`RENAME_NOREPLACE`). The kernel creates `to` only if it did not already
/// exist, so this closes the check-then-rename TOCTOU: a racing same-uid process
/// cannot make the move clobber (and thus irreversibly destroy) a file the
/// reversible tier promised to be able to restore. Both paths must be canonical-
/// absolute, so `AT_FDCWD` is a placeholder the kernel ignores.
pub fn rename_noreplace(from: &str, to: &str) -> Result<(), RenameError> {
    let nul = |_| RenameError::Other("path contains an interior NUL byte".to_string());
    let cfrom = CString::new(from).map_err(nul)?;
    let cto = CString::new(to).map_err(nul)?;
    // SAFETY: both pointers are valid NUL-terminated C strings that outlive the
    // call; `renameat2` with `AT_FDCWD` and absolute paths ignores the dir fds.
    let rc = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            cfrom.as_ptr(),
            libc::AT_FDCWD,
            cto.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if rc == 0 {
        return Ok(());
    }
    let err = std::io::Error::last_os_error();
    match err.raw_os_error() {
        Some(libc::EEXIST) => Err(RenameError::DestinationExists),
        // The flag or the syscall is unavailable (old kernel / exotic fs).
        Some(libc::EINVAL | libc::ENOSYS | libc::EOPNOTSUPP) => Err(RenameError::Unsupported),
        // Named rather than folded into `Other`, because it is the one failure
        // here that is about WHERE the file is rather than about a fault: a
        // picture on a USB stick or under /tmp is on another filesystem, and a
        // rename into the home trash cannot cross it. The caller can say that in
        // words; it could not say anything useful about `Io("Invalid
        // cross-device link")`, which is what a person deleting a photo used to
        // be shown.
        Some(libc::EXDEV) => Err(RenameError::CrossDevice),
        _ => Err(RenameError::Other(err.to_string())),
    }
}

/// The canonical trashed + sidecar paths of a reserved trash slot, for the inverse.
#[derive(Debug)]
pub struct TrashSlot {
    /// The entity's new location under `Trash/files/`.
    trashed: CanonicalPath,
    /// The companion `Trash/info/<name>.trashinfo` sidecar.
    trash_info: CanonicalPath,
}

impl TrashSlot {
    /// The entity's new canonical location under `Trash/files/`.
    pub fn trashed(&self) -> &CanonicalPath {
        &self.trashed
    }

    /// The companion `.trashinfo` sidecar's canonical path.
    pub fn trash_info(&self) -> &CanonicalPath {
        &self.trash_info
    }

    /// Consume the slot into its `(trashed, trash_info)` canonical paths (for a
    /// caller that moves both into a restore receipt).
    pub fn into_parts(self) -> (CanonicalPath, CanonicalPath) {
        (self.trashed, self.trash_info)
    }
}

/// Why a trash operation could not complete.
#[derive(Debug)]
pub enum TrashError {
    /// The source path does not exist.
    NotFound,
    /// The filesystem does not support an atomic no-clobber move.
    Unsupported,
    /// No free trash name was found within the dedup bound.
    NoSlot,
    /// A resolved trash path was not canonical-absolute (fail-closed; the inverse
    /// relies on canonical paths).
    NonCanonical,
    /// The entity is not on the same filesystem as the home trash, so a home
    /// trash cannot take it. The caller's answer is [`ensure_top_trash`] on the
    /// entity's own volume; this remains for a caller that has only a home trash.
    CrossDevice,
    /// The volume cannot host a trash at all: it is mounted read-only, or the
    /// top directory refuses the write.
    ///
    /// A REFUSAL, and never a fall-back to unlink. A permanent delete dressed as
    /// a trash is the one outcome this whole primitive exists to prevent, and it
    /// is the one that cannot be undone; being told "not deleted, and here is
    /// why" is the annoying answer and the correct one.
    NoTrashHere(String),
    /// Any other IO failure.
    Io(String),
}

/// The most trash names to try before giving up (a name collides only with an
/// existing trash entry of the same base name).
const MAX_TRASH_DEDUP: u32 = 10_000;

/// The user's home trash directory (`$XDG_DATA_HOME/Trash`, else
/// `$HOME/.local/share/Trash`). `None` if neither yields an absolute base, so a
/// trash never lands at a relative path.
pub fn home_trash_dir() -> Option<PathBuf> {
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
                .map(|h| h.join(".local/share"))
        })?;
    Some(data_home.join("Trash"))
}

/// The top directory of the filesystem `path` lives on: the highest ancestor
/// still on the same device, which is the mount point.
///
/// Found by walking up while `st_dev` is unchanged, because that is the only
/// definition that does not need `/proc/mounts` parsed and kept in step with it.
/// A path that cannot be stat'd has no answer.
pub fn top_directory_of(path: &Path) -> Option<PathBuf> {
    let dev_of = |p: &Path| -> Option<u64> {
        use std::os::unix::fs::MetadataExt;
        std::fs::metadata(p).ok().map(|m| m.dev())
    };
    let start = if path.is_dir() { path.to_path_buf() } else { path.parent()?.to_path_buf() };
    let dev = dev_of(&start)?;
    let mut top = start.clone();
    let mut cursor = start;
    while let Some(parent) = cursor.parent().map(Path::to_path_buf) {
        if parent == cursor {
            break;
        }
        match dev_of(&parent) {
            Some(d) if d == dev => {
                top = parent.clone();
                cursor = parent;
            }
            // A different device, or an ancestor we cannot stat: the last one that
            // matched is the mount point.
            _ => break,
        }
    }
    Some(top)
}

/// Which trash directory serves entities on `topdir`, per the spec's two forms.
///
/// `$topdir/.Trash/$uid` FIRST, and only when `.Trash` is a directory, is not a
/// symlink, and has the sticky bit: that one is administrator-provided and shared,
/// so all three conditions are what stop a hostile `.Trash` on a removable volume
/// from being a place this writes into. Without the sticky bit any user could
/// replace another's subdirectory, which is the attack the spec's rule exists for.
///
/// Otherwise `$topdir/.Trash-$uid`, created at 0700 - the per-user form, which
/// needs no cooperation from whoever formatted the volume.
///
/// NOT created here: this decides, `ensure_top_trash` creates. A read-only mount
/// answers the question fine and simply cannot be written to, and keeping those
/// apart means the refusal can say which one happened.
pub fn top_trash_dir(topdir: &Path, uid: u32) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let admin = topdir.join(".Trash");
    let usable = std::fs::symlink_metadata(&admin)
        .ok()
        .filter(|m| m.is_dir() && !m.file_type().is_symlink())
        .is_some_and(|m| m.permissions().mode() & 0o1000 != 0);
    if usable {
        admin.join(uid.to_string())
    } else {
        topdir.join(format!(".Trash-{uid}"))
    }
}

/// Create the trash `top_trash_dir` chose, and return its `files/` and `info/`.
///
/// 0700 on everything this makes: a trash holds what a person deleted, which is
/// frequently the most private thing on the volume, and on a shared disk the
/// per-user form is the only thing separating one user's deletions from another's
/// eyes. The administrator form's own `.Trash` is NOT created or re-permissioned
/// here - it belongs to whoever set the volume up, and this only ever makes the
/// `$uid` subdirectory inside it.
///
/// A read-only mount or a refused write comes back as [`TrashError::NoTrashHere`]
/// carrying the reason, so the caller can say which happened. It never falls back
/// to anything.
pub fn ensure_top_trash(topdir: &Path, uid: u32) -> Result<(PathBuf, PathBuf), TrashError> {
    use std::os::unix::fs::DirBuilderExt;
    let base = top_trash_dir(topdir, uid);
    let make = |p: &Path| -> Result<(), TrashError> {
        if p.is_dir() {
            return Ok(());
        }
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(p).map_err(|e| {
            TrashError::NoTrashHere(format!(
                "{}: {e}",
                p.display()
            ))
        })
    };
    make(&base)?;
    let files = base.join("files");
    let info = base.join("info");
    make(&files)?;
    make(&info)?;
    Ok((files, info))
}

/// The `Path` field for an entity trashed into a TOP-DIRECTORY trash: relative to
/// the top directory, per the spec.
///
/// This is what lets an entry survive the volume being mounted somewhere else.
/// An absolute `/run/media/tim/stick/notes.md` is a claim about where the volume
/// was that day; `notes.md` is a claim about the volume, and the volume carries
/// the trash with it. Returns `None` when the source is not under the top
/// directory, which is a caller error rather than something to paper over.
pub fn relative_to_top(topdir: &Path, source: &Path) -> Option<PathBuf> {
    source.strip_prefix(topdir).ok().map(Path::to_path_buf)
}

/// Reserve a unique trash slot, write its `.trashinfo` sidecar, and move `source`
/// into `files/<name>` atomically (no-clobber). The sidecar is created first
/// (freedesktop info-first) and removed on a move failure, so a failed trash leaves
/// no partial state. Each candidate's canonical paths are validated BEFORE its move,
/// so a returned slot always yields a constructible inverse.
pub fn trash_into(
    files_dir: &Path,
    info_dir: &Path,
    base_name: &str,
    source: &str,
) -> Result<TrashSlot, TrashError> {
    // The absolute source is what a home trash records, per the spec.
    trash_into_recording(files_dir, info_dir, base_name, source, source)
}

/// Trash `source` into whichever trash serves the volume it lives on.
///
/// THE ENTRY POINT A CALLER SHOULD USE. Trash follows the file, not the home
/// directory: the home trash is one case of the spec rather than the whole of it,
/// and a rename cannot cross a device - so a home-only implementation fails on a
/// USB stick, a second disk, and anything under `/tmp`, for every consumer of
/// this crate at once.
///
/// The home trash is preferred when the entity is already on its filesystem,
/// because that is where a person expects to find it and it is the one a desktop
/// shell empties. Otherwise the volume's own trash, with the recorded `Path`
/// relative to the top directory.
///
/// A volume that cannot host a trash comes back as
/// [`TrashError::NoTrashHere`] and nothing is deleted. That refusal is the whole
/// point: a permanent delete wearing the name of a reversible one is the outcome
/// this crate exists to prevent, and it is the one nobody can undo.
pub fn trash(source: &str, uid: u32) -> Result<TrashSlot, TrashError> {
    use std::os::unix::fs::MetadataExt;
    let src = Path::new(source);
    let base_name = src.file_name().and_then(|n| n.to_str()).ok_or(TrashError::NonCanonical)?;
    let dev_of = |p: &Path| std::fs::metadata(p).ok().map(|m| m.dev());
    let src_dev = dev_of(src).ok_or(TrashError::NotFound)?;

    // The home trash, when the entity is on its filesystem. `home_trash_dir` only
    // builds the path, so the device question is asked of the nearest ancestor
    // that exists - the trash itself may not have been created yet.
    if let Some(home) = home_trash_dir() {
        let probe = if home.exists() { Some(home.clone()) } else { home.parent().map(Path::to_path_buf) };
        let same = probe.as_deref().and_then(dev_of).is_some_and(|d| d == src_dev);
        if same {
            let files = home.join("files");
            let info = home.join("info");
            for d in [&files, &info] {
                std::fs::create_dir_all(d)
                    .map_err(|e| TrashError::NoTrashHere(format!("{}: {e}", d.display())))?;
            }
            return trash_into(&files, &info, base_name, source);
        }
    }

    let top = top_directory_of(src).ok_or(TrashError::NotFound)?;
    let (files, info) = ensure_top_trash(&top, uid)?;
    let recorded = relative_to_top(&top, src)
        .and_then(|r| r.to_str().map(str::to_string))
        .ok_or(TrashError::NonCanonical)?;
    trash_into_recording(&files, &info, base_name, source, &recorded)
}

/// [`trash_into`], with the `Path` field written into the sidecar given
/// separately from the entity being moved.
///
/// They are the same string for a home trash and they are NOT for a volume's own
/// trash, where the spec asks for a path relative to the top directory - which is
/// what lets an entry still resolve after the volume is mounted somewhere else.
/// One parameter rather than a flag, because the caller already knows which trash
/// it opened and the rule is about the recorded string, not about a mode.
pub fn trash_into_recording(
    files_dir: &Path,
    info_dir: &Path,
    base_name: &str,
    source: &str,
    recorded_path: &str,
) -> Result<TrashSlot, TrashError> {
    use std::io::Write;
    for n in 0..MAX_TRASH_DEDUP {
        let candidate = dedup_name(base_name, n);
        let trashed_path = files_dir.join(&candidate);
        let info_path = info_dir.join(format!("{candidate}.trashinfo"));
        // Canonicity check BEFORE any side effect for this candidate.
        let (Some(trashed_canon), Some(info_canon)) = (
            trashed_path.to_str().and_then(CanonicalPath::new),
            info_path.to_str().and_then(CanonicalPath::new),
        ) else {
            return Err(TrashError::NonCanonical);
        };
        // Atomically reserve the info slot (create-new); a taken name bumps n.
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&info_path) {
            Ok(mut f) => {
                if let Err(e) = f
                    .write_all(trashinfo_bytes(recorded_path).as_bytes())
                    .and_then(|()| f.sync_all())
                {
                    let _ = std::fs::remove_file(&info_path);
                    return Err(TrashError::Io(e.to_string()));
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(TrashError::Io(e.to_string())),
        }
        // Move the entity into files/<candidate> atomically, no-clobber.
        match rename_noreplace(source, trashed_canon.as_str()) {
            Ok(()) => return Ok(TrashSlot { trashed: trashed_canon, trash_info: info_canon }),
            Err(RenameError::DestinationExists) => {
                // An orphan file already occupies files/<candidate>; drop our sidecar
                // and try the next name.
                let _ = std::fs::remove_file(&info_path);
                continue;
            }
            Err(RenameError::Unsupported) => {
                let _ = std::fs::remove_file(&info_path);
                return Err(TrashError::Unsupported);
            }
            Err(RenameError::CrossDevice) => {
                let _ = std::fs::remove_file(&info_path);
                return Err(TrashError::CrossDevice);
            }
            Err(RenameError::Other(m)) => {
                let _ = std::fs::remove_file(&info_path);
                // A missing source gets a clearer error than a raw ENOENT.
                if !Path::new(source).exists() {
                    return Err(TrashError::NotFound);
                }
                return Err(TrashError::Io(m));
            }
        }
    }
    Err(TrashError::NoSlot)
}

/// The nth candidate trash name: the base for `n == 0`, else `<stem>.<n>.<ext>`
/// (or `<base>.<n>` without an extension), so a collision picks a fresh but still
/// recognizable name. A leading-dot file (`.bashrc`) is treated as extension-less.
fn dedup_name(base: &str, n: u32) -> String {
    if n == 0 {
        return base.to_string();
    }
    match base.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => format!("{stem}.{n}.{ext}"),
        _ => format!("{base}.{n}"),
    }
}

/// The freedesktop `.trashinfo` body for a file trashed from `original_path`.
fn trashinfo_bytes(original_path: &str) -> String {
    format!(
        "[Trash Info]\nPath={}\nDeletionDate={}\n",
        percent_encode_path(original_path),
        utc_iso8601_now(),
    )
}

/// Percent-encode a path for the `.trashinfo` `Path` field: unreserved bytes
/// (`A-Za-z0-9-._~`) and `/` pass through, every other byte becomes `%XX`.
fn percent_encode_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for &b in path.as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~' | b'/') {
            out.push(b as char);
        } else {
            out.push('%');
            out.push_str(&format!("{b:02X}"));
        }
    }
    out
}

/// The current UTC time as `YYYY-MM-DDThh:mm:ss` (the `.trashinfo` DeletionDate
/// shape). Freedesktop specifies local time; UTC without a zone suffix parses as a
/// naive datetime that trash viewers tolerate, keeping this dependency-free.
fn utc_iso8601_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (h, mi, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let (y, mo, d) = civil_from_days(days);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}")
}

/// Convert days since the Unix epoch to a `(year, month, day)` civil date (Howard
/// Hinnant's algorithm, pure integer arithmetic).
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod follows_the_file_tests {
    use super::*;

    /// A serial lock: these point `HOME`/`XDG_DATA_HOME` at their own directories
    /// and cargo runs them on threads of one process, so without it one test sets
    /// the variable underneath another. The same trap the searches store hit.
    static ENV: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// `/tmp` is its own mount, and `$HOME` is not - so a file there is exactly
    /// the case a home-only trash cannot take. It must land in the volume's own
    /// trash and come back out of it.
    #[test]
    fn a_file_on_another_volume_trashes_and_restores() {
        let _serial = ENV.lock().unwrap_or_else(|e| e.into_inner());
        // The data home has to be on a DIFFERENT device from the source, which is
        // the whole case - so it goes under the real `$HOME` while the source goes
        // under `/tmp`. Putting both in `temp_dir()` made this test pass through
        // the home-trash branch and assert nothing, which is how it was written
        // the first time.
        let Some(home_base) = std::env::var_os("HOME").map(std::path::PathBuf::from) else {
            eprintln!("no HOME: the cross-device case cannot be arranged, not asserting");
            return;
        };
        let home = home_base.join(format!(".cache/arlen-fh-home-{}", std::process::id()));
        std::fs::create_dir_all(&home).unwrap();
        // SAFETY: the test owns these temp dirs; the lock above serialises access.
        unsafe { std::env::set_var("XDG_DATA_HOME", &home) };

        // The source lives on /tmp, which is a different device from $HOME on any
        // systemd host - and identical to $HOME here only if /tmp is not a mount,
        // in which case this test is vacuous and says so rather than passing.
        let work = std::env::temp_dir().join(format!("arlen-fh-work-{}", std::process::id()));
        std::fs::create_dir_all(&work).unwrap();
        let doomed = work.join("notes.md");
        std::fs::write(&doomed, b"keep me").unwrap();

        // Say so rather than pass vacuously: on a host where /tmp is not its own
        // mount there is no cross-device case to test here.
        {
            use std::os::unix::fs::MetadataExt;
            let dev = |p: &Path| std::fs::metadata(p).unwrap().dev();
            if dev(&work) == dev(&home) {
                eprintln!("/tmp and $HOME are one filesystem here; nothing cross-device to assert");
                std::fs::remove_dir_all(&work).ok();
                std::fs::remove_dir_all(&home).ok();
                return;
            }
        }

        let slot = trash(doomed.to_str().unwrap(), 1000).expect("the volume takes it");
        assert!(!doomed.exists(), "the file left its place");
        assert!(
            slot.trashed().as_str().contains(".Trash-1000/files"),
            "into the volume's own trash: {}",
            slot.trashed().as_str()
        );

        // The recorded Path is relative to the top directory, so the entry still
        // resolves when the volume is mounted somewhere else next time.
        let info = std::fs::read_to_string(slot.trash_info().as_str()).unwrap();
        let path_line = info.lines().find(|l| l.starts_with("Path=")).unwrap();
        assert!(!path_line.contains("Path=/"), "relative, not absolute: {path_line}");
        assert!(path_line.contains("notes.md"), "{path_line}");

        // Restore: the inverse the slot yields puts it back with its contents.
        rename_noreplace(slot.trashed().as_str(), doomed.to_str().unwrap()).unwrap();
        assert_eq!(std::fs::read_to_string(&doomed).unwrap(), "keep me");

        std::fs::remove_dir_all(&work).ok();
        std::fs::remove_dir_all(&home).ok();
    }

    /// The reason the Path is relative: the same entry, read after the volume has
    /// been mounted somewhere else, still names the file.
    #[test]
    fn an_entry_survives_the_volume_moving() {
        let _serial = ENV.lock().unwrap_or_else(|e| e.into_inner());
        let top = std::env::temp_dir().join(format!("arlen-fh-vol-{}", std::process::id()));
        std::fs::create_dir_all(top.join("sub")).unwrap();
        let (files, info) = ensure_top_trash(&top, 1000).unwrap();
        let src = top.join("sub/a.md");
        std::fs::write(&src, b"x").unwrap();
        let recorded = relative_to_top(&top, &src).unwrap();
        let slot = trash_into_recording(
            &files,
            &info,
            "a.md",
            src.to_str().unwrap(),
            recorded.to_str().unwrap(),
        )
        .unwrap();

        // "Mounted somewhere else": move the whole volume directory, then read the
        // entry and resolve it against the new location.
        let moved = std::env::temp_dir().join(format!("arlen-fh-vol2-{}", std::process::id()));
        std::fs::remove_dir_all(&moved).ok();
        std::fs::rename(&top, &moved).unwrap();
        let entry = std::fs::read_to_string(
            slot.trash_info().as_str().replace(
                top.to_str().unwrap(),
                moved.to_str().unwrap(),
            ),
        )
        .unwrap();
        let rel = entry
            .lines()
            .find_map(|l| l.strip_prefix("Path="))
            .expect("the entry has a Path");
        assert_eq!(moved.join(rel), moved.join("sub/a.md"), "resolves against the new mount");
        std::fs::remove_dir_all(&moved).ok();
    }

    /// A trash that cannot complete leaves the file where it was.
    ///
    /// The refusal this asserts is the reachable one. A genuinely read-only MOUNT
    /// needs privileges to arrange, so `ensure_top_trash`'s own test covers that
    /// half by making the top directory unwritable and reading the
    /// `NoTrashHere` back. Here the source sits in a directory that refuses the
    /// move - and the first version of this test asserted `NoTrashHere` from
    /// `trash()`, which was wrong: `top_directory_of` walks up to `/tmp`, the
    /// volume hosts a trash perfectly well, and what fails is the rename. The
    /// property that matters survives either way and is what is checked: the
    /// file is still there afterwards.
    #[test]
    fn a_trash_that_cannot_complete_deletes_nothing() {
        use std::os::unix::fs::PermissionsExt;
        let _serial = ENV.lock().unwrap_or_else(|e| e.into_inner());
        let Some(home_base) = std::env::var_os("HOME").map(std::path::PathBuf::from) else {
            eprintln!("no HOME: not asserting");
            return;
        };
        let home = home_base.join(format!(".cache/arlen-fh-h2-{}", std::process::id()));
        std::fs::create_dir_all(&home).unwrap();
        // SAFETY: the test owns this temp dir; the lock above serialises access.
        unsafe { std::env::set_var("XDG_DATA_HOME", &home) };

        let holder = std::env::temp_dir().join(format!("arlen-fh-ro-{}", std::process::id()));
        std::fs::create_dir_all(&holder).unwrap();
        let src = holder.join("a.md");
        std::fs::write(&src, b"still here").unwrap();
        std::fs::set_permissions(&holder, std::fs::Permissions::from_mode(0o500)).unwrap();

        let outcome = trash(src.to_str().unwrap(), 1000);
        assert!(outcome.is_err(), "a move it cannot make must not report success");
        std::fs::set_permissions(&holder, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(
            std::fs::read_to_string(&src).unwrap(),
            "still here",
            "nothing was deleted, which is the one thing that must hold"
        );
        std::fs::remove_dir_all(&holder).ok();
        std::fs::remove_dir_all(&home).ok();
    }
}

#[cfg(test)]
mod top_dir_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// `/tmp` is a mount of its own on this machine and on any systemd host, and
    /// `$HOME` is not - which is the whole reason a home-only trash fails there.
    #[test]
    fn a_path_resolves_to_the_mount_it_lives_on() {
        let tmp = top_directory_of(Path::new("/tmp")).expect("tmp is stat-able");
        assert_eq!(tmp, Path::new("/tmp"), "a mount point is its own top directory");
        let root = top_directory_of(Path::new("/usr/bin")).expect("/usr/bin is stat-able");
        assert!(
            root == Path::new("/") || root == Path::new("/usr"),
            "walks up to the mount, got {}",
            root.display()
        );
    }

    #[test]
    fn a_file_answers_for_the_directory_holding_it() {
        let f = std::env::temp_dir().join(format!("arlen-top-{}", std::process::id()));
        std::fs::write(&f, b"x").unwrap();
        assert_eq!(top_directory_of(&f).unwrap(), top_directory_of(Path::new("/tmp")).unwrap());
        std::fs::remove_file(&f).ok();
    }

    #[test]
    fn without_a_sticky_admin_trash_it_is_the_per_user_one() {
        let dir = std::env::temp_dir().join(format!("arlen-tt-a-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(top_trash_dir(&dir, 1000), dir.join(".Trash-1000"));

        // Present but NOT sticky: still refused, because without the sticky bit
        // one user can replace another's subdirectory inside it.
        let admin = dir.join(".Trash");
        std::fs::create_dir_all(&admin).unwrap();
        std::fs::set_permissions(&admin, std::fs::Permissions::from_mode(0o777)).unwrap();
        assert_eq!(top_trash_dir(&dir, 1000), dir.join(".Trash-1000"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_sticky_admin_trash_is_preferred_and_per_uid() {
        let dir = std::env::temp_dir().join(format!("arlen-tt-b-{}", std::process::id()));
        let admin = dir.join(".Trash");
        std::fs::create_dir_all(&admin).unwrap();
        std::fs::set_permissions(&admin, std::fs::Permissions::from_mode(0o1777)).unwrap();
        assert_eq!(top_trash_dir(&dir, 1000), admin.join("1000"));
        assert_eq!(top_trash_dir(&dir, 42), admin.join("42"), "per uid, not shared");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A symlink named `.Trash` is the case the spec's rule is written against:
    /// it can point anywhere, including somewhere the attacker can read.
    #[test]
    fn a_symlinked_admin_trash_is_refused() {
        let dir = std::env::temp_dir().join(format!("arlen-tt-c-{}", std::process::id()));
        let elsewhere = dir.join("elsewhere");
        std::fs::create_dir_all(&elsewhere).unwrap();
        std::fs::set_permissions(&elsewhere, std::fs::Permissions::from_mode(0o1777)).unwrap();
        std::os::unix::fs::symlink(&elsewhere, dir.join(".Trash")).unwrap();
        assert_eq!(top_trash_dir(&dir, 1000), dir.join(".Trash-1000"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_per_user_trash_is_created_private_with_both_halves() {
        let dir = std::env::temp_dir().join(format!("arlen-tt-d-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (files, info) = ensure_top_trash(&dir, 1000).expect("a writable volume hosts one");
        assert_eq!(files, dir.join(".Trash-1000/files"));
        assert_eq!(info, dir.join(".Trash-1000/info"));
        for p in [dir.join(".Trash-1000"), files, info] {
            let mode = std::fs::metadata(&p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o700, "{} is owner-only, got {mode:o}", p.display());
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn creating_it_twice_is_the_same_answer() {
        let dir = std::env::temp_dir().join(format!("arlen-tt-e-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let first = ensure_top_trash(&dir, 1000).unwrap();
        let again = ensure_top_trash(&dir, 1000).unwrap();
        assert_eq!(first, again);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The refusal the decision insists on. A volume that cannot host a trash
    /// must say so and stop - the alternative is a permanent delete wearing the
    /// name of a reversible one.
    #[test]
    fn a_volume_that_cannot_host_one_refuses_and_says_why() {
        let dir = std::env::temp_dir().join(format!("arlen-tt-f-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // Read-only stands in for a read-only mount: the write fails the same way
        // and needs no privileges to arrange.
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o500)).unwrap();
        match ensure_top_trash(&dir, 1000) {
            Err(TrashError::NoTrashHere(why)) => {
                assert!(why.contains(".Trash-1000"), "names the path it could not make: {why}");
            }
            other => panic!("expected a refusal naming the volume, got {other:?}"),
        }
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The administrator form: this makes the `$uid` subdirectory and leaves the
    /// shared `.Trash` exactly as whoever set the volume up left it.
    #[test]
    fn the_admin_trash_keeps_its_own_permissions() {
        let dir = std::env::temp_dir().join(format!("arlen-tt-g-{}", std::process::id()));
        let admin = dir.join(".Trash");
        std::fs::create_dir_all(&admin).unwrap();
        std::fs::set_permissions(&admin, std::fs::Permissions::from_mode(0o1777)).unwrap();
        let (files, _) = ensure_top_trash(&dir, 1000).unwrap();
        assert_eq!(files, admin.join("1000/files"));
        let mode = std::fs::metadata(&admin).unwrap().permissions().mode() & 0o7777;
        assert_eq!(mode, 0o1777, "the shared directory is not re-permissioned");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_recorded_path_is_relative_to_the_volume() {
        let top = Path::new("/run/media/tim/stick");
        assert_eq!(
            relative_to_top(top, Path::new("/run/media/tim/stick/notes/a.md")).unwrap(),
            Path::new("notes/a.md"),
            "so the entry still resolves when the volume mounts elsewhere"
        );
        assert!(relative_to_top(top, Path::new("/home/tim/a.md")).is_none());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// A fresh canonical-absolute temp dir (canonicalized so paths are the
    /// canonical-absolute form `trash_into` requires).
    fn tmp() -> std::path::PathBuf {
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let base = std::env::temp_dir().join(format!("fdt-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        base.canonicalize().unwrap()
    }

    #[test]
    fn dedup_name_bumps_before_the_extension() {
        assert_eq!(dedup_name("doc.txt", 0), "doc.txt");
        assert_eq!(dedup_name("doc.txt", 2), "doc.2.txt");
        assert_eq!(dedup_name("README", 3), "README.3");
        // A leading-dot file has no stem, so the counter appends.
        assert_eq!(dedup_name(".bashrc", 1), ".bashrc.1");
    }

    #[test]
    fn percent_encodes_only_reserved_bytes() {
        assert_eq!(percent_encode_path("/home/tim/a b.txt"), "/home/tim/a%20b.txt");
        assert_eq!(percent_encode_path("/x/y-_.~z"), "/x/y-_.~z");
    }

    #[test]
    fn trashinfo_body_has_the_freedesktop_shape() {
        let body = trashinfo_bytes("/home/tim/notes.txt");
        assert!(body.starts_with("[Trash Info]\nPath=/home/tim/notes.txt\nDeletionDate="));
        assert!(body.ends_with('\n'));
    }

    #[test]
    fn trash_into_moves_the_file_and_writes_a_sidecar() {
        let root = tmp();
        let files = root.join("files");
        let info = root.join("info");
        std::fs::create_dir_all(&files).unwrap();
        std::fs::create_dir_all(&info).unwrap();
        let src = root.join("doc.txt");
        std::fs::write(&src, b"hello").unwrap();

        let slot = trash_into(&files, &info, "doc.txt", src.to_str().unwrap()).unwrap();
        assert!(!src.exists(), "the source moved out");
        assert!(files.join("doc.txt").exists(), "landed under files/");
        assert!(info.join("doc.txt.trashinfo").exists(), "sidecar written");
        assert!(slot.trashed().as_str().ends_with("/files/doc.txt"));
        let (trashed, sidecar) = slot.into_parts();
        assert!(trashed.as_str().ends_with("/files/doc.txt"));
        assert!(sidecar.as_str().ends_with("/info/doc.txt.trashinfo"));
    }

    #[test]
    fn trash_into_dedups_on_a_name_collision() {
        let root = tmp();
        let files = root.join("files");
        let info = root.join("info");
        std::fs::create_dir_all(&files).unwrap();
        std::fs::create_dir_all(&info).unwrap();
        // An existing trash entry of the same base name occupies the first slot.
        std::fs::write(files.join("doc.txt"), b"old").unwrap();
        std::fs::write(info.join("doc.txt.trashinfo"), b"[Trash Info]\n").unwrap();

        let src = root.join("doc.txt");
        std::fs::write(&src, b"new").unwrap();
        let slot = trash_into(&files, &info, "doc.txt", src.to_str().unwrap()).unwrap();
        assert!(slot.trashed().as_str().ends_with("/files/doc.1.txt"), "picked a fresh name");
        assert_eq!(std::fs::read(files.join("doc.txt")).unwrap(), b"old", "old entry untouched");
    }

    #[test]
    fn rename_noreplace_refuses_to_clobber() {
        let root = tmp();
        let from = root.join("a");
        let to = root.join("b");
        std::fs::write(&from, b"src").unwrap();
        std::fs::write(&to, b"dst").unwrap();
        match rename_noreplace(from.to_str().unwrap(), to.to_str().unwrap()) {
            Err(RenameError::DestinationExists) => {}
            other => panic!("expected DestinationExists, got {other:?}"),
        }
        assert_eq!(std::fs::read(&to).unwrap(), b"dst", "target not clobbered");
        assert!(from.exists(), "source left in place");
    }

    #[test]
    fn trash_into_reports_a_missing_source() {
        let root = tmp();
        let files = root.join("files");
        let info = root.join("info");
        std::fs::create_dir_all(&files).unwrap();
        std::fs::create_dir_all(&info).unwrap();
        let gone = root.join("gone.txt");
        match trash_into(&files, &info, "gone.txt", gone.to_str().unwrap()) {
            Err(TrashError::NotFound) => {}
            other => panic!("expected NotFound, got {other:?}"),
        }
        // The orphaned sidecar was cleaned up on the failed move.
        assert!(!info.join("gone.txt.trashinfo").exists());
    }
}

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
    /// The entity is not on the same filesystem as the home trash, so this
    /// primitive cannot take it. Implementing the spec's per-device
    /// `.Trash-$uid` is the fix and is not in this crate's scope today - see the
    /// module doc - so the caller is told which case it is instead of being
    /// handed a kernel string.
    CrossDevice,
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
                    .write_all(trashinfo_bytes(source).as_bytes())
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

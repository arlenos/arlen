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
    /// Any other rename failure (`EXDEV`, permissions, a NUL in the path, ...).
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

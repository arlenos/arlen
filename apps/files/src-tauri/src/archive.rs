//! Archive extraction for the file manager (file-manager-plan.md bucket B,
//! "extract archives").
//!
//! Safety rests on the capability, not on path string-munging: entries are
//! written through a cap-std [`Dir`] opened at the DESTINATION directory, so an
//! absolute, `..`-traversing or symlink-escaping entry path is refused by the
//! capability itself (the same containment the FM core uses). On top of that we
//! reject anything but regular files and directories (a tar can carry symlinks,
//! hardlinks and device nodes - none are extracted), screen each entry path
//! before touching the filesystem, and bound the total extracted size and entry
//! count so a decompression bomb cannot fill the disk. Reading the archive runs
//! off the listing path (it can be slow); a failure leaves a partial extraction
//! the user can delete, the conventional archive-tool behaviour.

use std::io::{self, Read};
use std::path::{Component, Path};

use cap_std::fs::Dir;

/// Total extracted bytes a single archive may produce. A generous sanity bound
/// (real archives are far smaller); it exists only to stop a tiny bomb expanding
/// without limit, not to cap legitimate use.
const MAX_TOTAL_BYTES: u64 = 50 * 1024 * 1024 * 1024;

/// Maximum number of entries in one archive (another bomb bound).
const MAX_ENTRIES: u64 = 500_000;

/// Whether `path` is a safe relative path to write under the destination: only
/// normal components and `.` are allowed, never a root, prefix or `..`. cap-std
/// would refuse an escaping write anyway; this rejects it before any syscall and
/// gives a clear error.
fn is_safe_relative(path: &Path) -> bool {
    path.components().all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
        && path.components().next().is_some()
}

/// Extract a tar stream into `dest` (a cap-std directory at the extraction
/// target). Regular files and directories only; symlinks, hardlinks and special
/// files are skipped. Bounded by [`MAX_TOTAL_BYTES`] and [`MAX_ENTRIES`].
pub fn extract_tar<R: Read>(reader: R, dest: &Dir) -> Result<(), String> {
    let mut archive = tar::Archive::new(reader);
    let entries = archive.entries().map_err(|e| format!("read archive: {e}"))?;
    let mut total: u64 = 0;
    let mut count: u64 = 0;

    for entry in entries {
        let mut entry = entry.map_err(|e| format!("read entry: {e}"))?;
        count += 1;
        if count > MAX_ENTRIES {
            return Err("archive has too many entries".to_string());
        }
        let path = entry
            .path()
            .map_err(|e| format!("entry path: {e}"))?
            .into_owned();
        if !is_safe_relative(&path) {
            return Err(format!("unsafe archive entry path: {}", path.display()));
        }

        match entry.header().entry_type() {
            tar::EntryType::Directory => {
                dest.create_dir_all(&path)
                    .map_err(|e| format!("create dir {}: {e}", path.display()))?;
            }
            tar::EntryType::Regular | tar::EntryType::GNUSparse => {
                if let Some(parent) = path.parent() {
                    if parent.components().next().is_some() {
                        dest.create_dir_all(parent)
                            .map_err(|e| format!("create parent {}: {e}", parent.display()))?;
                    }
                }
                let size = entry.header().size().unwrap_or(0);
                total = total.saturating_add(size);
                if total > MAX_TOTAL_BYTES {
                    return Err("archive exceeds the extraction size limit".to_string());
                }
                let mut out = dest
                    .create(&path)
                    .map_err(|e| format!("create file {}: {e}", path.display()))?;
                io::copy(&mut entry, &mut out)
                    .map_err(|e| format!("write {}: {e}", path.display()))?;
            }
            // Symlinks, hardlinks, char/block devices, fifos: never extracted.
            _ => continue,
        }
    }
    Ok(())
}

/// Whether a filename looks like a gzip-compressed tar (`.tar.gz` / `.tgz`),
/// vs a plain `.tar`. Used to pick the decompression wrapper.
pub fn is_gzip_tar(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".tar.gz") || lower.ends_with(".tgz")
}

/// Whether a filename is a tar-family archive this module can extract.
pub fn is_extractable_tar(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".tar") || is_gzip_tar(name)
}

/// Extract a tar-family archive read from `reader`, choosing the decompressor
/// from `name`'s extension.
pub fn extract_named<R: Read>(name: &str, reader: R, dest: &Dir) -> Result<(), String> {
    if is_gzip_tar(name) {
        extract_tar(flate2::read::GzDecoder::new(reader), dest)
    } else if is_extractable_tar(name) {
        extract_tar(reader, dest)
    } else {
        Err(format!("unsupported archive format: {name}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cap_std::ambient_authority;

    /// Build an in-memory tar from (path, contents) entries.
    fn tar_of(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut b = tar::Builder::new(Vec::new());
        for (path, data) in files {
            let mut h = tar::Header::new_gnu();
            h.set_size(data.len() as u64);
            h.set_mode(0o644);
            h.set_cksum();
            b.append_data(&mut h, path, *data).unwrap();
        }
        b.into_inner().unwrap()
    }

    fn dest() -> (tempfile::TempDir, Dir) {
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = Dir::open_ambient_dir(tmp.path(), ambient_authority()).unwrap();
        (tmp, dir)
    }

    #[test]
    fn extracts_regular_files_into_the_destination() {
        let tar = tar_of(&[("a.txt", b"alpha"), ("sub/b.txt", b"bravo")]);
        let (tmp, dir) = dest();
        extract_tar(tar.as_slice(), &dir).unwrap();
        assert_eq!(std::fs::read(tmp.path().join("a.txt")).unwrap(), b"alpha");
        assert_eq!(std::fs::read(tmp.path().join("sub/b.txt")).unwrap(), b"bravo");
    }

    /// A raw single-entry tar with an arbitrary (unvalidated) name. The safe
    /// `tar::Builder` refuses to write a `..` path, but a tar from another tool
    /// can carry one, so the header is constructed by hand to exercise the
    /// extract-time guard against a genuinely malicious archive.
    fn raw_tar(name: &str) -> Vec<u8> {
        let mut h = [0u8; 512];
        let nb = name.as_bytes();
        h[..nb.len()].copy_from_slice(nb); // name
        h[100..108].copy_from_slice(b"0000644\0"); // mode
        h[108..116].copy_from_slice(b"0000000\0"); // uid
        h[116..124].copy_from_slice(b"0000000\0"); // gid
        h[124..136].copy_from_slice(b"00000000000\0"); // size 0
        h[136..148].copy_from_slice(b"00000000000\0"); // mtime
        h[156] = b'0'; // typeflag: regular file
        // checksum over the header with the chksum field counted as 8 spaces.
        for b in h.iter_mut().take(156).skip(148) {
            *b = b' ';
        }
        let sum: u32 = h.iter().map(|&b| b as u32).sum();
        let cks = format!("{sum:06o}\0 ");
        h[148..156].copy_from_slice(cks.as_bytes());
        // header + two zero blocks (tar EOF marker).
        let mut out = h.to_vec();
        out.extend_from_slice(&[0u8; 1024]);
        out
    }

    #[test]
    fn a_traversing_entry_is_rejected() {
        let tar = raw_tar("../escape.txt");
        let (tmp, dir) = dest();
        let err = extract_tar(tar.as_slice(), &dir).unwrap_err();
        assert!(err.contains("unsafe"), "got {err}");
        // Nothing escaped the destination's parent.
        assert!(!tmp.path().parent().unwrap().join("escape.txt").exists());
    }

    #[test]
    fn is_safe_relative_screens_paths() {
        assert!(is_safe_relative(Path::new("a/b.txt")));
        assert!(is_safe_relative(Path::new("./a")));
        assert!(!is_safe_relative(Path::new("../escape")));
        assert!(!is_safe_relative(Path::new("/etc/passwd")));
        assert!(!is_safe_relative(Path::new("a/../../escape")));
        assert!(!is_safe_relative(Path::new("")));
    }

    #[test]
    fn a_symlink_entry_is_skipped() {
        let mut b = tar::Builder::new(Vec::new());
        let mut h = tar::Header::new_gnu();
        h.set_entry_type(tar::EntryType::Symlink);
        h.set_size(0);
        h.set_mode(0o777);
        b.append_link(&mut h, "link", "/etc/passwd").unwrap();
        let tar = b.into_inner().unwrap();
        let (tmp, dir) = dest();
        extract_tar(tar.as_slice(), &dir).unwrap();
        assert!(!tmp.path().join("link").exists(), "the symlink was not extracted");
    }

    #[test]
    fn format_detection_by_extension() {
        assert!(is_gzip_tar("x.tar.gz"));
        assert!(is_gzip_tar("X.TGZ"));
        assert!(!is_gzip_tar("x.tar"));
        assert!(is_extractable_tar("x.tar"));
        assert!(is_extractable_tar("x.tgz"));
        assert!(!is_extractable_tar("x.zip"));
    }
}

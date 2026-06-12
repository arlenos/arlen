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

use std::io::{self, Read, Seek, Write};
use std::path::{Component, Path};

use cap_std::fs::Dir;
use serde::Serialize;

/// The unix mode bit marking a symlink (`S_IFLNK`), used to skip symlink entries
/// in a zip for parity with the tar path.
const S_IFLNK: u32 = 0o120000;

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

/// Append one source (a file, or a directory subtree, read through `root`) to
/// the tar `builder` under `archive_path`. Symlinks and special files are
/// skipped, for parity with [`extract_tar`].
fn append_entry<W: Write>(
    builder: &mut tar::Builder<W>,
    root: &Dir,
    src: &Path,
    archive_path: &Path,
) -> Result<(), String> {
    let meta = root
        .symlink_metadata(src)
        .map_err(|e| format!("stat {}: {e}", src.display()))?;
    let ft = meta.file_type();
    if ft.is_dir() {
        let mut h = tar::Header::new_gnu();
        h.set_entry_type(tar::EntryType::Directory);
        h.set_mode(0o755);
        h.set_size(0);
        builder
            .append_data(&mut h, archive_path, io::empty())
            .map_err(|e| format!("append dir {}: {e}", archive_path.display()))?;
        let read_dir = root
            .read_dir(src)
            .map_err(|e| format!("read dir {}: {e}", src.display()))?;
        for entry in read_dir {
            let entry = entry.map_err(|e| format!("read entry: {e}"))?;
            let name = entry.file_name();
            append_entry(builder, root, &src.join(&name), &archive_path.join(&name))?;
        }
    } else if ft.is_file() {
        let mut f = root
            .open(src)
            .map_err(|e| format!("open {}: {e}", src.display()))?;
        let mut h = tar::Header::new_gnu();
        h.set_size(meta.len());
        h.set_mode(0o644);
        builder
            .append_data(&mut h, archive_path, &mut f)
            .map_err(|e| format!("append {}: {e}", archive_path.display()))?;
    }
    // Symlinks and special files are skipped.
    Ok(())
}

/// Compress `sources` (root-relative paths) into a tar - gzip when `gzip` - written
/// to `writer`. Each source is stored under its basename, so extraction restores
/// the selected items without their absolute prefix. Read through the `root`
/// capability; symlinks and special files are skipped.
pub fn compress<W: Write>(
    root: &Dir,
    sources: &[String],
    writer: W,
    gzip: bool,
) -> Result<(), String> {
    fn add_all<W: Write>(b: &mut tar::Builder<W>, root: &Dir, sources: &[String]) -> Result<(), String> {
        for s in sources {
            let src = Path::new(s);
            let base = src
                .file_name()
                .ok_or_else(|| format!("source has no name: {s}"))?;
            append_entry(b, root, src, Path::new(base))?;
        }
        Ok(())
    }
    if gzip {
        let enc = flate2::write::GzEncoder::new(writer, flate2::Compression::default());
        let mut b = tar::Builder::new(enc);
        add_all(&mut b, root, sources)?;
        let enc = b.into_inner().map_err(|e| format!("finish tar: {e}"))?;
        enc.finish().map_err(|e| format!("finish gzip: {e}"))?;
    } else {
        let mut b = tar::Builder::new(writer);
        add_all(&mut b, root, sources)?;
        b.finish().map_err(|e| format!("finish tar: {e}"))?;
    }
    Ok(())
}

/// One entry in an archive listing (FM-R12 browse-into-archive). Read-only
/// metadata: nothing is written to disk, so listing carries no extraction risk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ArchiveEntry {
    /// The entry path inside the archive.
    pub path: String,
    /// The uncompressed size in bytes (0 for directories).
    pub size: u64,
    /// Whether the entry is a directory.
    pub is_dir: bool,
}

/// List the entries of a tar stream without extracting anything. Bounded by
/// [`MAX_ENTRIES`] so a malicious archive cannot make the listing unbounded.
pub fn list_tar<R: Read>(reader: R) -> Result<Vec<ArchiveEntry>, String> {
    let mut archive = tar::Archive::new(reader);
    let entries = archive.entries().map_err(|e| format!("read archive: {e}"))?;
    let mut out = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| format!("read entry: {e}"))?;
        if out.len() as u64 >= MAX_ENTRIES {
            return Err("archive has too many entries".to_string());
        }
        let path = entry
            .path()
            .map_err(|e| format!("entry path: {e}"))?
            .to_string_lossy()
            .into_owned();
        let is_dir = entry.header().entry_type().is_dir();
        let size = entry.header().size().unwrap_or(0);
        out.push(ArchiveEntry { path, size, is_dir });
    }
    Ok(out)
}

/// List a tar-family archive read from `reader`, choosing the decompressor from
/// `name`'s extension.
pub fn list_named<R: Read>(name: &str, reader: R) -> Result<Vec<ArchiveEntry>, String> {
    if is_gzip_tar(name) {
        list_tar(flate2::read::GzDecoder::new(reader))
    } else if is_extractable_tar(name) {
        list_tar(reader)
    } else {
        Err(format!("unsupported archive format: {name}"))
    }
}

/// Whether a filename is a `.zip` archive.
pub fn is_zip(name: &str) -> bool {
    name.to_ascii_lowercase().ends_with(".zip")
}

/// Whether a filename is any archive this module can extract or list (tar-family
/// or zip).
pub fn is_extractable(name: &str) -> bool {
    is_extractable_tar(name) || is_zip(name)
}

/// Extract a zip into `dest`. Like the tar path: entries are written through the
/// destination capability (traversal refused), the zip crate's own
/// `enclosed_name` guard rejects an escaping name before that, only regular files
/// and directories are written (symlinks are skipped via the unix mode), and the
/// total size and entry count are bounded.
pub fn zip_extract<R: Read + Seek>(reader: R, dest: &Dir) -> Result<(), String> {
    let mut archive = zip::ZipArchive::new(reader).map_err(|e| format!("open zip: {e}"))?;
    if archive.len() as u64 > MAX_ENTRIES {
        return Err("archive has too many entries".to_string());
    }
    let mut total: u64 = 0;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| format!("read entry: {e}"))?;
        // The zip crate's traversal guard: `None` for a name that would escape.
        let path = match entry.enclosed_name() {
            Some(p) => p,
            None => return Err(format!("unsafe zip entry path: {}", entry.name())),
        };
        if !is_safe_relative(&path) {
            return Err(format!("unsafe zip entry path: {}", path.display()));
        }
        if entry.is_dir() {
            dest.create_dir_all(&path)
                .map_err(|e| format!("create dir {}: {e}", path.display()))?;
            continue;
        }
        // Skip symlinks (a zip stores them as files with the target as content).
        if entry.unix_mode().is_some_and(|m| m & S_IFLNK == S_IFLNK) {
            continue;
        }
        if let Some(parent) = path.parent() {
            if parent.components().next().is_some() {
                dest.create_dir_all(parent)
                    .map_err(|e| format!("create parent {}: {e}", parent.display()))?;
            }
        }
        total = total.saturating_add(entry.size());
        if total > MAX_TOTAL_BYTES {
            return Err("archive exceeds the extraction size limit".to_string());
        }
        let mut out = dest
            .create(&path)
            .map_err(|e| format!("create file {}: {e}", path.display()))?;
        io::copy(&mut entry, &mut out).map_err(|e| format!("write {}: {e}", path.display()))?;
    }
    Ok(())
}

/// Append one source (a file, or a directory subtree, read through `root`) to a
/// zip writer under `archive_path`. Symlinks and special files are skipped.
fn zip_add_entry<W: Write + Seek>(
    zw: &mut zip::ZipWriter<W>,
    root: &Dir,
    src: &Path,
    archive_path: &Path,
    opts: zip::write::SimpleFileOptions,
) -> Result<(), String> {
    let meta = root
        .symlink_metadata(src)
        .map_err(|e| format!("stat {}: {e}", src.display()))?;
    let ft = meta.file_type();
    let name = archive_path.to_string_lossy().into_owned();
    if ft.is_dir() {
        zw.add_directory(name, opts)
            .map_err(|e| format!("add dir {}: {e}", archive_path.display()))?;
        let read_dir = root
            .read_dir(src)
            .map_err(|e| format!("read dir {}: {e}", src.display()))?;
        for entry in read_dir {
            let entry = entry.map_err(|e| format!("read entry: {e}"))?;
            let child = entry.file_name();
            zip_add_entry(zw, root, &src.join(&child), &archive_path.join(&child), opts)?;
        }
    } else if ft.is_file() {
        zw.start_file(name, opts)
            .map_err(|e| format!("add file {}: {e}", archive_path.display()))?;
        let mut f = root
            .open(src)
            .map_err(|e| format!("open {}: {e}", src.display()))?;
        io::copy(&mut f, zw).map_err(|e| format!("write {}: {e}", archive_path.display()))?;
    }
    // Symlinks and special files are skipped.
    Ok(())
}

/// Compress `sources` (root-relative paths) into a zip written to `writer`. Each
/// source is stored under its basename. Read through the `root` capability;
/// symlinks and special files are skipped. The writer needs `Seek` (a zip's
/// central directory is written at the end).
pub fn zip_compress<W: Write + Seek>(
    root: &Dir,
    sources: &[String],
    writer: W,
) -> Result<(), String> {
    let mut zw = zip::ZipWriter::new(writer);
    let opts = zip::write::SimpleFileOptions::default();
    for s in sources {
        let src = Path::new(s);
        let base = src
            .file_name()
            .ok_or_else(|| format!("source has no name: {s}"))?;
        zip_add_entry(&mut zw, root, src, Path::new(base), opts)?;
    }
    zw.finish().map_err(|e| format!("finish zip: {e}"))?;
    Ok(())
}

/// List a zip's entries without extracting. Bounded by [`MAX_ENTRIES`].
pub fn zip_list<R: Read + Seek>(reader: R) -> Result<Vec<ArchiveEntry>, String> {
    let mut archive = zip::ZipArchive::new(reader).map_err(|e| format!("open zip: {e}"))?;
    if archive.len() as u64 > MAX_ENTRIES {
        return Err("archive has too many entries".to_string());
    }
    let mut out = Vec::with_capacity(archive.len());
    for i in 0..archive.len() {
        let entry = archive.by_index(i).map_err(|e| format!("read entry: {e}"))?;
        out.push(ArchiveEntry {
            path: entry.name().to_string(),
            size: entry.size(),
            is_dir: entry.is_dir(),
        });
    }
    Ok(out)
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

    /// A source tree with a top-level file and a subdir-with-file.
    fn source_tree() -> (tempfile::TempDir, Dir) {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::write(tmp.path().join("top.txt"), b"top").unwrap();
        std::fs::create_dir(tmp.path().join("proj")).unwrap();
        std::fs::write(tmp.path().join("proj/inner.txt"), b"inner").unwrap();
        let dir = Dir::open_ambient_dir(tmp.path(), ambient_authority()).unwrap();
        (tmp, dir)
    }

    #[test]
    fn compress_then_extract_round_trips_a_file_and_a_dir() {
        let (_src_tmp, src) = source_tree();
        let mut buf = Vec::new();
        compress(
            &src,
            &["top.txt".to_string(), "proj".to_string()],
            &mut buf,
            false,
        )
        .unwrap();

        let (out_tmp, out) = dest();
        extract_tar(buf.as_slice(), &out).unwrap();
        assert_eq!(std::fs::read(out_tmp.path().join("top.txt")).unwrap(), b"top");
        assert_eq!(std::fs::read(out_tmp.path().join("proj/inner.txt")).unwrap(), b"inner");
    }

    #[test]
    fn gzip_compress_then_extract_round_trips() {
        let (_src_tmp, src) = source_tree();
        let mut buf = Vec::new();
        compress(&src, &["top.txt".to_string()], &mut buf, true).unwrap();

        let (out_tmp, out) = dest();
        extract_tar(flate2::read::GzDecoder::new(buf.as_slice()), &out).unwrap();
        assert_eq!(std::fs::read(out_tmp.path().join("top.txt")).unwrap(), b"top");
    }

    #[test]
    fn list_reports_entries_without_extracting() {
        let tar = tar_of(&[("readme.txt", b"hello"), ("docs/guide.md", b"# guide")]);
        let entries = list_tar(tar.as_slice()).unwrap();
        let names: Vec<&str> = entries.iter().map(|e| e.path.as_str()).collect();
        assert!(names.contains(&"readme.txt"));
        assert!(names.contains(&"docs/guide.md"));
        let readme = entries.iter().find(|e| e.path == "readme.txt").unwrap();
        assert_eq!(readme.size, 5);
        assert!(!readme.is_dir);
    }

    /// Build an in-memory zip from (name, contents) entries.
    fn zip_of(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(io::Cursor::new(Vec::new()));
        let opts = zip::write::SimpleFileOptions::default();
        for (name, data) in files {
            w.start_file(*name, opts).unwrap();
            w.write_all(data).unwrap();
        }
        w.finish().unwrap().into_inner()
    }

    #[test]
    fn zip_extracts_and_lists() {
        let bytes = zip_of(&[("readme.txt", b"hello"), ("docs/guide.md", b"# guide")]);
        // List.
        let entries = zip_list(io::Cursor::new(bytes.clone())).unwrap();
        assert!(entries.iter().any(|e| e.path == "readme.txt" && e.size == 5));
        // Extract.
        let (tmp, dir) = dest();
        zip_extract(io::Cursor::new(bytes), &dir).unwrap();
        assert_eq!(std::fs::read(tmp.path().join("readme.txt")).unwrap(), b"hello");
        assert_eq!(std::fs::read(tmp.path().join("docs/guide.md")).unwrap(), b"# guide");
    }

    #[test]
    fn zip_compress_then_extract_round_trips_a_file_and_a_dir() {
        let (_src_tmp, src) = source_tree();
        let mut buf = io::Cursor::new(Vec::new());
        zip_compress(&src, &["top.txt".to_string(), "proj".to_string()], &mut buf).unwrap();

        let (out_tmp, out) = dest();
        zip_extract(io::Cursor::new(buf.into_inner()), &out).unwrap();
        assert_eq!(std::fs::read(out_tmp.path().join("top.txt")).unwrap(), b"top");
        assert_eq!(std::fs::read(out_tmp.path().join("proj/inner.txt")).unwrap(), b"inner");
    }

    #[test]
    fn a_traversing_zip_entry_is_rejected() {
        let bytes = zip_of(&[("../escape.txt", b"x")]);
        let (tmp, dir) = dest();
        let err = zip_extract(io::Cursor::new(bytes), &dir).unwrap_err();
        assert!(err.contains("unsafe"), "got {err}");
        assert!(!tmp.path().parent().unwrap().join("escape.txt").exists());
    }

    #[test]
    fn zip_format_detection() {
        assert!(is_zip("x.zip"));
        assert!(is_zip("X.ZIP"));
        assert!(!is_zip("x.tar"));
        assert!(is_extractable("x.zip"));
        assert!(is_extractable("x.tar.gz"));
        assert!(!is_extractable("x.rar"));
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

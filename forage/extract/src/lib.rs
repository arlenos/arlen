//! Safe extraction of a fetched source archive into a build directory.
//!
//! The fetch phase stores a source as a tar archive (a `git archive`, or an
//! upstream `tar`/`tar.gz`/`tar.zst`). Before the build, that archive is
//! unpacked into a working directory. The archive is hash-pinned but still
//! attacker-shaped, so extraction is defended (forage-recipes.md section 17a):
//!
//! - **Path traversal**: an entry whose path is absolute or contains `..` (or a
//!   root/prefix component) is rejected; only relative, contained paths extract.
//! - **Symlinks and special files**: only regular files and directories are
//!   written. Symlinks, hardlinks, devices and fifos are rejected, so nothing
//!   can later be followed out of the build tree or create a device node.
//! - **Zip/decompression bomb**: extraction aborts once the entry count or the
//!   total written bytes exceed the configured caps.
//!
//! Compression is detected from magic bytes (gzip, zstd, else plain tar).

use std::io::Read;
use std::path::{Component, Path};

use thiserror::Error;

/// Caps that bound extraction against decompression bombs.
#[derive(Debug, Clone)]
pub struct ExtractLimits {
    /// Maximum number of archive entries.
    pub max_entries: usize,
    /// Maximum total bytes written across all files.
    pub max_total_bytes: u64,
}

impl Default for ExtractLimits {
    fn default() -> Self {
        // Generous for real source trees; bounds a bomb. A recipe-specific
        // override can come later.
        ExtractLimits {
            max_entries: 200_000,
            max_total_bytes: 4 * 1024 * 1024 * 1024,
        }
    }
}

/// A failure extracting an archive.
#[derive(Debug, Error)]
pub enum ExtractError {
    /// A filesystem or read error.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    /// Decompressing the archive failed.
    #[error("decompress: {0}")]
    Decompress(String),
    /// An entry path is absolute or escapes the destination.
    #[error("unsafe entry path: {0}")]
    UnsafePath(String),
    /// An entry is a symlink, hardlink, or special file (not allowed).
    #[error("unsupported entry type for {0}")]
    UnsupportedEntry(String),
    /// The archive has more entries than the cap allows.
    #[error("archive exceeds {0} entries")]
    TooManyEntries(usize),
    /// Extraction exceeded the total-bytes cap.
    #[error("extracted content exceeds {0} bytes")]
    TooLarge(u64),
}

/// What an extraction produced.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ExtractReport {
    /// Number of regular files written.
    pub files: usize,
    /// Total bytes written.
    pub total_bytes: u64,
}

/// Extract a source archive (`bytes`) into `dest`, enforcing `limits`.
///
/// Only regular files and directories are written, under relative,
/// `..`-free paths. `dest` is created if absent.
pub fn extract_tar(
    bytes: &[u8],
    dest: &Path,
    limits: &ExtractLimits,
) -> Result<ExtractReport, ExtractError> {
    std::fs::create_dir_all(dest)?;
    let reader = decompressor(bytes)?;
    let mut archive = tar::Archive::new(reader);
    // We write contents ourselves rather than letting tar unpack, so its own
    // path/permission handling never applies; nothing is preserved implicitly.
    let mut report = ExtractReport::default();
    let mut entry_count = 0usize;

    for entry in archive.entries()? {
        let mut entry = entry?;
        entry_count += 1;
        if entry_count > limits.max_entries {
            return Err(ExtractError::TooManyEntries(limits.max_entries));
        }

        let path = entry.path()?.into_owned();
        let path_str = path.display().to_string();
        if !is_safe_relative(&path) {
            return Err(ExtractError::UnsafePath(path_str));
        }
        let out = dest.join(&path);

        use tar::EntryType;
        match entry.header().entry_type() {
            EntryType::Directory => {
                std::fs::create_dir_all(&out)?;
            }
            EntryType::Regular | EntryType::GNUSparse => {
                if let Some(parent) = out.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                let remaining = limits.max_total_bytes - report.total_bytes;
                let written = copy_capped(&mut entry, &out, remaining, limits.max_total_bytes)?;
                report.total_bytes += written;
                report.files += 1;
            }
            // Symlinks, hardlinks, char/block devices, fifos: refused, so a
            // later step cannot follow a link out of the tree or hit a device.
            _ => return Err(ExtractError::UnsupportedEntry(path_str)),
        }
    }

    Ok(report)
}

/// Copy a reader into `out`, failing with [`ExtractError::TooLarge`] if it would
/// write more than `remaining` bytes (the budget left under the total cap).
fn copy_capped<R: Read>(
    src: &mut R,
    out: &Path,
    remaining: u64,
    cap: u64,
) -> Result<u64, ExtractError> {
    use std::io::Write;
    let mut file = std::fs::File::create(out)?;
    let mut written: u64 = 0;
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = src.read(&mut buf)?;
        if n == 0 {
            break;
        }
        written += n as u64;
        if written > remaining {
            return Err(ExtractError::TooLarge(cap));
        }
        file.write_all(&buf[..n])?;
    }
    Ok(written)
}

/// Choose a decompressing reader for `bytes` based on magic bytes.
fn decompressor(bytes: &[u8]) -> Result<Box<dyn Read + '_>, ExtractError> {
    if bytes.starts_with(&[0x1f, 0x8b]) {
        Ok(Box::new(flate2::read::GzDecoder::new(bytes)))
    } else if bytes.starts_with(&[0x28, 0xb5, 0x2f, 0xfd]) {
        let dec = zstd::Decoder::new(bytes).map_err(|e| ExtractError::Decompress(e.to_string()))?;
        Ok(Box::new(dec))
    } else {
        Ok(Box::new(bytes))
    }
}

/// Whether a path is relative and free of `..`/root/prefix components.
fn is_safe_relative(p: &Path) -> bool {
    if p.is_absolute() {
        return false;
    }
    p.components()
        .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Build a plain tar with the given (path, entry_type, data) entries.
    fn build_tar(entries: &[(&str, tar::EntryType, &[u8])]) -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        for (path, etype, data) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_entry_type(*etype);
            header.set_mode(0o644);
            if *etype == tar::EntryType::Symlink {
                header.set_link_name("/etc/passwd").unwrap();
            }
            builder.append_data(&mut header, path, &data[..]).unwrap();
        }
        builder.into_inner().unwrap()
    }

    /// Craft a raw single-entry tar with an arbitrary `name`, bypassing the
    /// `tar` crate's writer (which refuses `..`). This is how a non-Rust tool
    /// could ship a traversal entry, and what the reader-side guard must catch.
    fn raw_tar_entry(name: &str, data: &[u8]) -> Vec<u8> {
        let mut h = [0u8; 512];
        h[0..name.len()].copy_from_slice(name.as_bytes());
        h[100..108].copy_from_slice(b"0000644\0");
        h[108..116].copy_from_slice(b"0000000\0");
        h[116..124].copy_from_slice(b"0000000\0");
        h[124..136].copy_from_slice(format!("{:011o}\0", data.len()).as_bytes());
        h[136..148].copy_from_slice(b"00000000000\0");
        h[156] = b'0'; // regular file
        h[257..263].copy_from_slice(b"ustar\0");
        h[263..265].copy_from_slice(b"00");
        for b in &mut h[148..156] {
            *b = b' ';
        }
        let sum: u32 = h.iter().map(|&b| b as u32).sum();
        h[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
        let mut out = h.to_vec();
        out.extend_from_slice(data);
        let pad = (512 - data.len() % 512) % 512;
        out.resize(out.len() + pad + 1024, 0); // entry pad + EOF
        out
    }

    fn gzip(bytes: &[u8]) -> Vec<u8> {
        let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        e.write_all(bytes).unwrap();
        e.finish().unwrap()
    }

    #[test]
    fn extracts_plain_tar() {
        let tar = build_tar(&[
            ("src/main.rs", tar::EntryType::Regular, b"fn main(){}"),
            ("README", tar::EntryType::Regular, b"hi"),
        ]);
        let dest = tempfile::tempdir().unwrap();
        let r = extract_tar(&tar, dest.path(), &ExtractLimits::default()).unwrap();
        assert_eq!(r.files, 2);
        assert_eq!(
            std::fs::read(dest.path().join("src/main.rs")).unwrap(),
            b"fn main(){}"
        );
    }

    #[test]
    fn extracts_gzip_and_zstd() {
        let tar = build_tar(&[("f", tar::EntryType::Regular, b"data")]);
        for archive in [gzip(&tar), zstd::encode_all(&tar[..], 0).unwrap()] {
            let dest = tempfile::tempdir().unwrap();
            let r = extract_tar(&archive, dest.path(), &ExtractLimits::default()).unwrap();
            assert_eq!(r.files, 1);
            assert_eq!(std::fs::read(dest.path().join("f")).unwrap(), b"data");
        }
    }

    #[test]
    fn rejects_path_traversal() {
        // `tar` strips a leading `/`, so an absolute entry is normalised to a
        // contained relative path; the real traversal vector is `..`.
        for bad in ["../evil", "a/../../b", "../../etc/cron.d/x"] {
            let tar = raw_tar_entry(bad, b"x");
            let dest = tempfile::tempdir().unwrap();
            assert!(
                matches!(
                    extract_tar(&tar, dest.path(), &ExtractLimits::default()),
                    Err(ExtractError::UnsafePath(_))
                ),
                "`{bad}` must be rejected"
            );
        }
    }

    #[test]
    fn rejects_symlink_entries() {
        let tar = build_tar(&[("link", tar::EntryType::Symlink, b"")]);
        let dest = tempfile::tempdir().unwrap();
        assert!(matches!(
            extract_tar(&tar, dest.path(), &ExtractLimits::default()),
            Err(ExtractError::UnsupportedEntry(_))
        ));
        assert!(!dest.path().join("link").exists());
    }

    #[test]
    fn enforces_entry_count_cap() {
        let tar = build_tar(&[
            ("a", tar::EntryType::Regular, b"1"),
            ("b", tar::EntryType::Regular, b"2"),
            ("c", tar::EntryType::Regular, b"3"),
        ]);
        let dest = tempfile::tempdir().unwrap();
        let limits = ExtractLimits {
            max_entries: 2,
            max_total_bytes: 1 << 30,
        };
        assert!(matches!(
            extract_tar(&tar, dest.path(), &limits),
            Err(ExtractError::TooManyEntries(2))
        ));
    }

    #[test]
    fn enforces_total_size_cap() {
        let tar = build_tar(&[("big", tar::EntryType::Regular, &vec![7u8; 1000])]);
        let dest = tempfile::tempdir().unwrap();
        let limits = ExtractLimits {
            max_entries: 100,
            max_total_bytes: 100,
        };
        assert!(matches!(
            extract_tar(&tar, dest.path(), &limits),
            Err(ExtractError::TooLarge(100))
        ));
    }
}

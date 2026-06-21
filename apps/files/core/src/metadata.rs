//! The media-metadata-edit write-back (the editable EXIF half of the info
//! panel). Behind the off-by-default `metadata-edit` feature so the lean
//! embeddable browser (the picker) drops the `little_exif` dependency.
//!
//! Every write rides [`crate::ops::safe_rewrite`], so a metadata edit can never
//! corrupt the file: the new bytes are spliced in memory, written to a temp
//! sibling, verified on readback, then atomically swapped over the original; any
//! failure leaves the original byte-identical. JPEG only for now - the in-memory
//! EXIF splice `little_exif` exposes (`write_to_vec`) is JPEG-only; a non-JPEG
//! path is refused before any write rather than silently doing nothing.

use std::io;
use std::path::Path;

use cap_std::fs::Dir;
use little_exif::exif_tag::ExifTag;
use little_exif::filetype::FileExtension;
use little_exif::metadata::Metadata;

use crate::ops::{safe_rewrite, OpError, OpResult};

/// The editable string metadata fields the info-panel editor exposes. A `Some`
/// field overwrites that EXIF tag; a `None` leaves the existing value untouched.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ExifEdits {
    /// `ImageDescription` (0x010e): the free-text caption.
    pub description: Option<String>,
    /// `Artist` (0x013b): the author/creator.
    pub artist: Option<String>,
    /// `Copyright` (0x8298): the rights statement.
    pub copyright: Option<String>,
}

impl ExifEdits {
    /// Whether any field is set (an all-`None` edit is a no-op).
    pub fn is_empty(&self) -> bool {
        self.description.is_none() && self.artist.is_none() && self.copyright.is_none()
    }
}

/// Whether `path`'s extension names a JPEG (the only format the in-memory EXIF
/// splice supports today).
pub fn is_jpeg_path(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("jpg") || e.eq_ignore_ascii_case("jpeg"))
        .unwrap_or(false)
}

/// Write the requested editable EXIF tags into the JPEG at `path` (under the
/// capability `dir`), fail-safe via [`safe_rewrite`]. Existing tags are read,
/// the requested ones overwritten, and the result spliced + verified + atomically
/// swapped; a failed write never corrupts the file. A non-JPEG path or an empty
/// edit is refused before any write. `dir` is a cap-std capability, so `path`
/// cannot escape it.
pub fn write_exif_tags(dir: &Dir, path: impl AsRef<Path>, edits: &ExifEdits) -> OpResult<()> {
    let path = path.as_ref();
    if edits.is_empty() {
        return Err(OpError::Io(io::Error::new(
            io::ErrorKind::InvalidInput,
            "no metadata fields to write",
        )));
    }
    if !is_jpeg_path(path) {
        return Err(OpError::Io(io::Error::new(
            io::ErrorKind::InvalidInput,
            "EXIF metadata edit is supported only for JPEG files",
        )));
    }
    safe_rewrite(dir, path, |original| {
        let original_vec = original.to_vec();
        // A JPEG without existing EXIF parses to no metadata; start fresh in that
        // case rather than failing the edit.
        let mut metadata =
            Metadata::new_from_vec(&original_vec, FileExtension::JPEG).unwrap_or_else(|_| Metadata::new());
        if let Some(description) = &edits.description {
            metadata.set_tag(ExifTag::ImageDescription(description.clone()));
        }
        if let Some(artist) = &edits.artist {
            metadata.set_tag(ExifTag::Artist(artist.clone()));
        }
        if let Some(copyright) = &edits.copyright {
            metadata.set_tag(ExifTag::Copyright(copyright.clone()));
        }
        let mut out = original_vec;
        metadata
            .write_to_vec(&mut out, FileExtension::JPEG)
            .map_err(OpError::Io)?;
        Ok(out)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use cap_std::ambient_authority;

    const SAMPLE_JPEG: &[u8] = include_bytes!("../test-fixtures/sample.jpg");

    fn cap(path: &Path) -> Dir {
        Dir::open_ambient_dir(path, ambient_authority()).unwrap()
    }

    #[test]
    fn write_exif_description_lands_and_keeps_a_valid_jpeg() {
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("photo.jpg");
        std::fs::write(&f, SAMPLE_JPEG).unwrap();
        let dir = cap(tmp.path());
        let edits = ExifEdits {
            description: Some("a red square".to_string()),
            ..Default::default()
        };
        write_exif_tags(&dir, "photo.jpg", &edits).unwrap();

        let bytes = std::fs::read(&f).unwrap();
        // Still a JPEG (SOI marker preserved by the splice).
        assert_eq!(&bytes[..2], &[0xFF, 0xD8], "the file stays a JPEG");
        // The description text is embedded in the EXIF segment.
        let needle = b"a red square";
        assert!(
            bytes.windows(needle.len()).any(|w| w == needle),
            "the written description appears in the file"
        );
        // And it re-parses as EXIF metadata (a faithful round trip).
        assert!(
            Metadata::new_from_vec(&bytes, FileExtension::JPEG).is_ok(),
            "the rewritten file re-parses as EXIF"
        );
    }

    #[test]
    fn write_exif_refuses_a_non_jpeg() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("note.png"), b"not really a png").unwrap();
        let dir = cap(tmp.path());
        let edits = ExifEdits {
            description: Some("x".to_string()),
            ..Default::default()
        };
        let err = write_exif_tags(&dir, "note.png", &edits).unwrap_err();
        assert!(matches!(err, OpError::Io(_)));
        // The file is untouched (the refusal is before any write).
        assert_eq!(std::fs::read(tmp.path().join("note.png")).unwrap(), b"not really a png");
    }

    #[test]
    fn an_empty_edit_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("photo.jpg"), SAMPLE_JPEG).unwrap();
        let dir = cap(tmp.path());
        let err = write_exif_tags(&dir, "photo.jpg", &ExifEdits::default()).unwrap_err();
        assert!(matches!(err, OpError::Io(_)), "an all-None edit writes nothing");
    }
}

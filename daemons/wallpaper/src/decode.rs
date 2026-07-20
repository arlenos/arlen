//! Decode a static wallpaper image asset to RGBA8 for the background-layer
//! renderer client.
//!
//! The renderer resolves the active [`crate::manifest::Source`] (via
//! [`crate::schedule`]) and, for an `Image` source, uploads the decoded pixels
//! into a `wl_shm` buffer on its `wlr-layer-shell` background surface. This module
//! is that decode step, kept render-independent so it is unit-tested without a
//! Wayland connection. Video and shader sources are the separate live-renderer
//! path and do not pass through here.
//!
//! Fail-safe by construction: a decode error, a zero dimension, or an image whose
//! pixel count exceeds the cap is an [`DecodeError`], never a panic or an
//! unbounded allocation - the renderer keeps the previous frame (or the flat
//! fallback) rather than crashing the desktop background on a bad asset.

use thiserror::Error;

/// The pixel-count ceiling (width * height). At 4 bytes/pixel this bounds the
/// decoded buffer to ~256 MiB, comfortably above any real display resolution
/// (an 8K panel is ~33 M pixels) while refusing a decompression-bomb asset.
pub const MAX_WALLPAPER_PIXELS: u64 = 64_000_000;

/// A decoded RGBA8 image: row-major, 4 bytes per pixel, `width * height * 4`
/// bytes, ready to copy into a `wl_shm` buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedImage {
    /// Image width in pixels.
    pub width: u32,
    /// Image height in pixels.
    pub height: u32,
    /// RGBA8 pixels, `width * height * 4` bytes.
    pub rgba: Vec<u8>,
}

/// Why a wallpaper image could not be turned into pixels.
#[derive(Debug, Error)]
pub enum DecodeError {
    /// The image crate could not open or decode the asset.
    #[error("could not decode wallpaper image: {0}")]
    Decode(String),
    /// The image is empty or larger than [`MAX_WALLPAPER_PIXELS`].
    #[error("wallpaper image dimensions {width}x{height} are empty or over the cap")]
    Size {
        /// Decoded width.
        width: u32,
        /// Decoded height.
        height: u32,
    },
}

/// Decode the image at `path` to RGBA8, refusing an empty or over-cap image. The
/// dimension check runs BEFORE `to_rgba8` materialises the buffer, so a
/// decompression bomb is rejected without allocating its expansion.
pub fn load_image_rgba(path: &str) -> Result<DecodedImage, DecodeError> {
    let img = image::open(path).map_err(|e| DecodeError::Decode(e.to_string()))?;
    let (width, height) = (img.width(), img.height());
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > MAX_WALLPAPER_PIXELS {
        return Err(DecodeError::Size { width, height });
    }
    Ok(DecodedImage { width, height, rgba: img.to_rgba8().into_raw() })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Write a small solid-colour PNG to a temp file and return its path.
    fn fixture_png(dir: &std::path::Path, w: u32, h: u32) -> String {
        let path = dir.join("wp.png");
        let buf = image::RgbaImage::from_pixel(w, h, image::Rgba([10, 20, 30, 255]));
        buf.save(&path).unwrap();
        path.to_string_lossy().into_owned()
    }

    #[test]
    fn decodes_a_png_to_the_right_rgba_buffer() {
        let dir = tempfile::tempdir().unwrap();
        let path = fixture_png(dir.path(), 4, 3);
        let out = load_image_rgba(&path).unwrap();
        assert_eq!((out.width, out.height), (4, 3));
        assert_eq!(out.rgba.len(), 4 * 3 * 4); // w*h*4
        assert_eq!(&out.rgba[0..4], &[10, 20, 30, 255]); // first pixel round-trips
    }

    #[test]
    fn a_missing_or_undecodable_asset_is_an_error_not_a_panic() {
        assert!(matches!(load_image_rgba("/nonexistent/wp.png"), Err(DecodeError::Decode(_))));
        let dir = tempfile::tempdir().unwrap();
        let junk = dir.path().join("junk.png");
        std::fs::write(&junk, b"not a png").unwrap();
        assert!(load_image_rgba(&junk.to_string_lossy()).is_err());
    }
}

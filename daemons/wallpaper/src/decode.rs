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

use crate::manifest::{Scale, WallpaperKind, WallpaperManifest};
use crate::schedule::{source_for_monitor, TimeContext};
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

/// Compose a decoded image into an `out_w * out_h` RGBA8 buffer (the shm buffer
/// size) per the [`Scale`] mode, aspect-preserving, nearest-neighbour sampled:
///   - [`Scale::Fill`] scales the image to COVER the output (largest scale) and
///     centre-crops the overflow, so the output is fully painted.
///   - [`Scale::Zoom`] scales the image to FIT inside the output (smallest scale),
///     centres it, and fills the letterbox margins with `letterbox`.
/// Pure (no I/O, no Wayland), so it is unit-tested; the renderer calls it once per
/// output/source change and copies the result into its `wl_shm` buffer. `out_w`/
/// `out_h` of 0 yield an empty buffer.
pub fn compose_to_output(
    img: &DecodedImage,
    out_w: u32,
    out_h: u32,
    scale: Scale,
    letterbox: [u8; 4],
) -> Vec<u8> {
    let (ow, oh) = (out_w as usize, out_h as usize);
    let mut out = Vec::with_capacity(ow * oh * 4);
    if ow == 0 || oh == 0 || img.width == 0 || img.height == 0 {
        return out;
    }
    let (iw, ih) = (img.width as f64, img.height as f64);
    let (fw, fh) = (out_w as f64, out_h as f64);
    // Fill covers (max scale); Zoom fits (min scale). Both preserve aspect.
    let s = match scale {
        Scale::Fill => (fw / iw).max(fh / ih),
        Scale::Zoom => (fw / iw).min(fh / ih),
    };
    let scaled_w = iw * s;
    let scaled_h = ih * s;
    // Top-left of the scaled image in output space (negative for Fill's crop,
    // positive for Zoom's letterbox).
    let off_x = (fw - scaled_w) / 2.0;
    let off_y = (fh - scaled_h) / 2.0;
    for oy in 0..oh {
        for ox in 0..ow {
            // Map the output pixel back to a source pixel.
            let sx = ((ox as f64 + 0.5 - off_x) / s).floor() as i64;
            let sy = ((oy as f64 + 0.5 - off_y) / s).floor() as i64;
            if sx >= 0 && sy >= 0 && sx < img.width as i64 && sy < img.height as i64 {
                let i = ((sy as usize * img.width as usize) + sx as usize) * 4;
                out.extend_from_slice(&img.rgba[i..i + 4]);
            } else {
                // Outside the source (Zoom's letterbox margin).
                out.extend_from_slice(&letterbox);
            }
        }
    }
    out
}

/// Produce the composed RGBA frame for one output, or `None` when this static-
/// image renderer should paint nothing: a `Video`/`Shader` wallpaper is the
/// sandboxed live-renderer's job, and a decode failure leaves the client on its
/// previous frame / the flat fallback rather than crashing the background. Ties
/// the manifest + [`crate::schedule`] source selection + [`load_image_rgba`] +
/// [`compose_to_output`] together; the Wayland client copies the returned buffer
/// into its `wl_shm` buffer. Pure, so the whole pipeline is tested without a
/// compositor.
pub fn frame_for_output(
    manifest: &WallpaperManifest,
    connector: &str,
    ctx: &TimeContext,
    out_w: u32,
    out_h: u32,
    letterbox: [u8; 4],
) -> Option<Vec<u8>> {
    if matches!(manifest.kind, WallpaperKind::Video | WallpaperKind::Shader) {
        return None;
    }
    let source = source_for_monitor(manifest, connector, ctx);
    let decoded = load_image_rgba(&source.asset).ok()?;
    Some(compose_to_output(&decoded, out_w, out_h, source.scale, letterbox))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::Source;

    fn image_manifest(kind: WallpaperKind, asset: &str) -> WallpaperManifest {
        WallpaperManifest {
            kind,
            default: Source { asset: asset.to_string(), scale: Scale::Fill, loop_playback: false },
            per_monitor: Default::default(),
            variants: Vec::new(),
            transition_ms: 0,
        }
    }

    fn solid(w: u32, h: u32, px: [u8; 4]) -> DecodedImage {
        DecodedImage { width: w, height: h, rgba: px.repeat((w * h) as usize) }
    }

    fn pixel(buf: &[u8], w: u32, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * w + x) * 4) as usize;
        [buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]
    }

    /// Write a small solid-colour PNG to a temp file and return its path.
    fn fixture_png(dir: &std::path::Path, w: u32, h: u32) -> String {
        let path = dir.join("wp.png");
        let buf = image::RgbaImage::from_pixel(w, h, image::Rgba([10, 20, 30, 255]));
        buf.save(&path).unwrap();
        path.to_string_lossy().into_owned()
    }

    #[test]
    fn fill_paints_the_whole_output_with_no_letterbox() {
        let red = [200, 0, 0, 255];
        let out = compose_to_output(&solid(1, 1, red), 4, 4, Scale::Fill, [0, 0, 255, 255]);
        assert_eq!(out.len(), 4 * 4 * 4);
        // Every pixel is the source colour: Fill covers, so no letterbox shows.
        assert!(out.chunks_exact(4).all(|p| p == red));
    }

    #[test]
    fn zoom_letterboxes_a_mismatched_aspect() {
        let red = [200, 0, 0, 255];
        let blue = [0, 0, 255, 255];
        // A 4x1 source into a 4x4 output fits to width (scale 1), centred vertically
        // -> only the middle row carries the image, the rest is letterbox.
        let out = compose_to_output(&solid(4, 1, red), 4, 4, Scale::Zoom, blue);
        assert_eq!(out.len(), 4 * 4 * 4);
        assert_eq!(pixel(&out, 4, 0, 0), blue); // top margin
        assert_eq!(pixel(&out, 4, 0, 3), blue); // bottom margin
        assert_eq!(pixel(&out, 4, 0, 1), red); // the image row
    }

    #[test]
    fn a_zero_output_is_an_empty_buffer() {
        assert!(compose_to_output(&solid(2, 2, [1, 2, 3, 4]), 0, 4, Scale::Fill, [0; 4]).is_empty());
    }

    #[test]
    fn frame_for_output_renders_an_image_manifest_and_skips_live_kinds() {
        let dir = tempfile::tempdir().unwrap();
        let path = fixture_png(dir.path(), 2, 2);
        let ctx = TimeContext::at_minute(600);

        // Image kind + a decodable asset -> a full output frame.
        let m = image_manifest(WallpaperKind::Image, &path);
        let frame = frame_for_output(&m, "DP-1", &ctx, 8, 4, [0; 4]).unwrap();
        assert_eq!(frame.len(), 8 * 4 * 4);

        // Video/Shader are the live-renderer's job: this static client paints
        // nothing (None), never a wrong frame.
        for live in [WallpaperKind::Video, WallpaperKind::Shader] {
            assert!(frame_for_output(&image_manifest(live, &path), "DP-1", &ctx, 8, 4, [0; 4]).is_none());
        }

        // A missing asset -> None (keep the previous frame / flat fallback).
        let bad = image_manifest(WallpaperKind::Image, "/nonexistent/wp.png");
        assert!(frame_for_output(&bad, "DP-1", &ctx, 8, 4, [0; 4]).is_none());
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

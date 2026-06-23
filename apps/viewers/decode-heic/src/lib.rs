//! The HEIC/AVIF decoder worker's decode logic (`quickview-plan.md`).
//!
//! HEIF bytes -> a validated [`DecodedImage`] (8-bit RGBA) via the system
//! `libheif` (HEIC through libde265, AVIF through dav1d). This is the one
//! C-linked decoder: per the one-sandboxed-process-per-decoder model its host
//! confinement gets the wider seccomp profile (the threaded C codecs need
//! `clone`), confined to this worker process only - the pure-Rust decoders keep
//! their tight allowlist. The decode is unit-tested against real `cjxl`-class
//! fixtures (a 4x4 AVIF and a 4x4 HEIC, both generated with `vips`).

use arlen_viewers_core::decode::{DecodedImage, MAX_PIXELS};
use libheif_rs::{ColorSpace, HeifContext, LibHeif, RgbChroma};

/// Decode HEIC/AVIF bytes to RGBA. Errors on a corrupt stream or over-large image.
pub fn decode_heic(bytes: &[u8]) -> Result<DecodedImage, String> {
    let lib = LibHeif::new();
    let ctx = HeifContext::read_from_bytes(bytes).map_err(|e| format!("read: {e}"))?;
    let handle = ctx
        .primary_image_handle()
        .map_err(|e| format!("primary handle: {e}"))?;

    let width = handle.width();
    let height = handle.height();
    if u64::from(width) * u64::from(height) > MAX_PIXELS {
        return Err(format!("image too large: {width}x{height}"));
    }

    let image = lib
        .decode(&handle, ColorSpace::Rgb(RgbChroma::Rgba), None)
        .map_err(|e| format!("decode: {e}"))?;
    let planes = image.planes();
    let plane = planes.interleaved.ok_or("no interleaved RGBA plane")?;

    let w = plane.width as usize;
    let h = plane.height as usize;
    let stride = plane.stride;
    let row_bytes = w * 4;
    if stride < row_bytes || plane.data.len() < stride * h {
        return Err("short interleaved plane".to_string());
    }

    // Copy row by row, dropping the per-row stride padding into a tight RGBA buffer.
    let mut rgba = Vec::with_capacity(row_bytes * h);
    for row in 0..h {
        let start = row * stride;
        rgba.extend_from_slice(&plane.data[start..start + row_bytes]);
    }

    DecodedImage::new(w as u32, h as u32, rgba).map_err(|e| format!("raster: {e:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 4x4 fixtures generated with `vips copy tiny.png tiny.{avif,heic}`;
    /// libheif decode-only, so the inputs cannot be produced in-test.
    const TINY_AVIF: &[u8] = include_bytes!("../tests/fixtures/tiny.avif");
    const TINY_HEIC: &[u8] = include_bytes!("../tests/fixtures/tiny.heic");

    #[test]
    fn decodes_the_real_avif_fixture_to_rgba() {
        let decoded = decode_heic(TINY_AVIF).expect("decode the avif fixture");
        assert_eq!((decoded.width, decoded.height), (4, 4));
        assert_eq!(decoded.rgba.len(), 4 * 4 * 4);
    }

    #[test]
    fn decodes_the_real_heic_fixture_to_rgba() {
        let decoded = decode_heic(TINY_HEIC).expect("decode the heic fixture");
        assert_eq!((decoded.width, decoded.height), (4, 4));
        assert_eq!(decoded.rgba.len(), 4 * 4 * 4);
    }

    #[test]
    fn rejects_garbage() {
        assert!(decode_heic(b"not a heif stream at all").is_err());
    }
}

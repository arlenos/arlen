//! The JPEG XL decoder worker's decode logic (`quickview-plan.md`).
//!
//! Pure decode: JXL bytes -> a validated [`DecodedImage`] (8-bit RGBA) via the
//! pure-Rust `jxl-oxide` (image-rs covers neither JXL nor HEIC; this is the JXL
//! half of that gap). The binary is the thin stdin/stdout shell run inside the
//! bwrap sandbox; this function does the decode, so it is unit-tested against a
//! real committed `.jxl` fixture (generated with `cjxl`) without spawning.

use arlen_viewers_core::decode::{DecodedImage, MAX_PIXELS};
use jxl_oxide::JxlImage;

/// Decode JXL bytes to RGBA. Errors on a corrupt stream or over-large dimensions.
pub fn decode_jxl(bytes: &[u8]) -> Result<DecodedImage, String> {
    let image = JxlImage::builder()
        .read(std::io::Cursor::new(bytes))
        .map_err(|e| format!("read: {e}"))?;
    let width = image.width();
    let height = image.height();
    if u64::from(width) * u64::from(height) > MAX_PIXELS {
        return Err(format!("image too large: {width}x{height}"));
    }
    let render = image.render_frame(0).map_err(|e| format!("render: {e}"))?;
    let fb = render.image_all_channels();
    let channels = fb.channels();
    let buf = fb.buf(); // interleaved f32 samples in [0, 1]

    let mut rgba = Vec::with_capacity((width as usize) * (height as usize) * 4);
    let to_u8 = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    for px in buf.chunks(channels.max(1)) {
        let (r, g, b, a) = match channels {
            1 => (px[0], px[0], px[0], 1.0),
            2 => (px[0], px[0], px[0], px[1]),
            3 => (px[0], px[1], px[2], 1.0),
            _ => (px[0], px[1], px[2], px[3]),
        };
        rgba.push(to_u8(r));
        rgba.push(to_u8(g));
        rgba.push(to_u8(b));
        rgba.push(to_u8(a));
    }
    DecodedImage::new(width, height, rgba).map_err(|e| format!("raster: {e:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 4x4 JXL generated with `cjxl` (committed fixture); jxl-oxide is
    /// decode-only so the input cannot be produced in-test.
    const TINY_JXL: &[u8] = include_bytes!("../tests/fixtures/tiny.jxl");

    #[test]
    fn decodes_the_real_jxl_fixture_to_rgba() {
        let decoded = decode_jxl(TINY_JXL).expect("decode the jxl fixture");
        assert_eq!((decoded.width, decoded.height), (4, 4));
        assert_eq!(decoded.rgba.len(), 4 * 4 * 4);
        // Every pixel is opaque (alpha 255) - the source had no alpha.
        assert!(decoded.rgba.chunks(4).all(|p| p[3] == 255));
    }

    #[test]
    fn rejects_garbage() {
        assert!(decode_jxl(b"not a jxl stream").is_err());
    }
}

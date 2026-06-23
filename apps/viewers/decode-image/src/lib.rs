//! The image decoder worker's decode logic (`quickview-plan.md`).
//!
//! Pure decode: encoded image bytes -> a validated [`DecodedImage`] (8-bit
//! RGBA). The binary ([`main`](../main.rs)) is the thin stdin/stdout shell run
//! inside the bwrap sandbox; this function is where the actual `image-rs` decode
//! happens, so it is unit-tested by round-tripping a real encoded image without
//! spawning a process. Only the pure-Rust codecs are enabled (png/jpeg/gif/webp/
//! bmp/tiff); AVIF + the long tail are later slices (AVIF needs the wider
//! seccomp profile for its C-linked dav1d).

use arlen_viewers_core::decode::{DecodedImage, MAX_PIXELS};

/// Decode encoded image bytes to RGBA. Returns a human-readable error on an
/// unsupported/corrupt image or one whose dimensions exceed [`MAX_PIXELS`].
pub fn decode_image(bytes: &[u8]) -> Result<DecodedImage, String> {
    // Read the dimensions first and reject an over-large image before decoding
    // the full raster (a decompression-bomb guard, in addition to the frame
    // bound the viewer re-checks).
    let reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| format!("read: {e}"))?;
    if let Ok((w, h)) = reader.into_dimensions() {
        if u64::from(w) * u64::from(h) > MAX_PIXELS {
            return Err(format!("image too large: {w}x{h}"));
        }
    }
    let img = image::load_from_memory(bytes).map_err(|e| format!("decode: {e}"))?;
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    DecodedImage::new(w, h, rgba.into_raw()).map_err(|e| format!("raster: {e:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageFormat, RgbaImage};
    use std::io::Cursor;

    /// Encode a small image to PNG bytes (the worker's input shape).
    fn png_bytes(w: u32, h: u32) -> Vec<u8> {
        let mut img = RgbaImage::new(w, h);
        for (i, px) in img.pixels_mut().enumerate() {
            *px = image::Rgba([i as u8, 0, 0, 255]);
        }
        let mut out = Vec::new();
        img.write_to(&mut Cursor::new(&mut out), ImageFormat::Png).unwrap();
        out
    }

    #[test]
    fn decodes_a_real_png_to_rgba() {
        let decoded = decode_image(&png_bytes(3, 2)).unwrap();
        assert_eq!((decoded.width, decoded.height), (3, 2));
        assert_eq!(decoded.rgba.len(), 3 * 2 * 4);
        // First pixel was (0,0,0,255); alpha is opaque.
        assert_eq!(decoded.rgba[3], 255);
    }

    #[test]
    fn round_trips_jpeg() {
        // JPEG has no alpha channel, so encode RGB; decode still yields RGBA.
        let mut out = Vec::new();
        image::RgbImage::from_pixel(4, 4, image::Rgb([10, 20, 30]))
            .write_to(&mut Cursor::new(&mut out), ImageFormat::Jpeg)
            .unwrap();
        let decoded = decode_image(&out).unwrap();
        assert_eq!((decoded.width, decoded.height), (4, 4));
        assert_eq!(decoded.rgba.len(), 4 * 4 * 4, "decoded to RGBA regardless of source");
    }

    #[test]
    fn rejects_garbage() {
        assert!(decode_image(b"not an image at all").is_err());
    }
}

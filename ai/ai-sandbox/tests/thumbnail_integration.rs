//! Integration test for the sandboxed thumbnailer: the worker decodes an
//! untrusted image and produces a downscaled PNG end-to-end, through the real
//! Landlock + seccomp subprocess lockdown. This is the check that image decode
//! actually works under the sandbox (the seccomp allowlist permits what the
//! decoder needs), and that a non-image input fails closed.

#![cfg(all(target_os = "linux", feature = "thumbnail"))]

use std::path::Path;

const BIN: &str = env!("CARGO_BIN_EXE_arlen-thumbnail-sandbox");

/// A synthetic PNG of `w`x`h` to feed the worker.
fn png_bytes(w: u32, h: u32) -> Vec<u8> {
    let img = image::RgbImage::from_fn(w, h, |x, _| image::Rgb([(x % 256) as u8, 90, 160]));
    let mut out = Vec::new();
    image::DynamicImage::ImageRgb8(img)
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .unwrap();
    out
}

#[test]
fn thumbnail_decodes_an_untrusted_image_inside_the_sandbox() {
    use image::GenericImageView;
    // 1024x512 -> fits within the 256 box at 2:1 aspect -> 256x128.
    let src = png_bytes(1024, 512);
    let thumb = arlen_ai_sandbox::thumbnail(Path::new(BIN), &src)
        .expect("the sandboxed worker should decode and downscale the image");
    let decoded = image::load_from_memory(&thumb).expect("the worker output is a valid PNG");
    assert_eq!(decoded.dimensions(), (256, 128));
}

#[test]
fn thumbnail_worker_fails_closed_on_non_image_input() {
    let err = arlen_ai_sandbox::thumbnail(Path::new(BIN), b"this is plainly not an image")
        .expect_err("a non-image input must not yield a thumbnail");
    // The worker exits non-zero on the decode failure, surfaced as WorkerFailed.
    assert!(matches!(err, arlen_ai_sandbox::SandboxError::WorkerFailed(_)));
}

/// Encode the synthetic image to `format`'s bytes.
fn encode(w: u32, h: u32, format: image::ImageFormat) -> Vec<u8> {
    let img = image::RgbImage::from_fn(w, h, |x, y| {
        image::Rgb([(x % 256) as u8, (y % 256) as u8, 160])
    });
    let mut out = Vec::new();
    image::DynamicImage::ImageRgb8(img)
        .write_to(&mut std::io::Cursor::new(&mut out), format)
        .unwrap();
    out
}

#[test]
fn every_enabled_decoder_works_under_the_sandbox() {
    // Only PNG was exercised under the live lockdown; verify each encodable
    // decoder (JPEG / GIF / BMP) also decodes under the seccomp allowlist, so a
    // codec needing a syscall the filter blocks is caught here rather than in
    // production. (WebP encoding is not available in pure-Rust `image`, so its
    // decode path is covered separately if a fixture is added.)
    for format in [
        image::ImageFormat::Jpeg,
        image::ImageFormat::Gif,
        image::ImageFormat::Bmp,
    ] {
        let src = encode(320, 240, format);
        let thumb = arlen_ai_sandbox::thumbnail(Path::new(BIN), &src)
            .unwrap_or_else(|e| panic!("{format:?} must decode under the sandbox: {e}"));
        assert!(
            !thumb.is_empty(),
            "{format:?} produced an empty thumbnail"
        );
        image::load_from_memory(&thumb)
            .unwrap_or_else(|e| panic!("{format:?} thumbnail is not a valid PNG: {e}"));
    }
}

#[test]
fn the_thumbnail_does_not_carry_source_trailing_bytes() {
    // The worker re-encodes the decoded pixels, so nothing of the source file
    // (metadata, trailing payloads) survives. Append a marker after a valid PNG
    // (decoders ignore trailing bytes) and assert it never appears in the output.
    const MARKER: &[u8] = b"SECRET_SOURCE_MARKER_DO_NOT_LEAK";
    let mut src = png_bytes(128, 96);
    src.extend_from_slice(MARKER);
    let thumb = arlen_ai_sandbox::thumbnail(Path::new(BIN), &src).expect("decodes");
    assert!(
        !thumb.windows(MARKER.len()).any(|w| w == MARKER),
        "the source trailing marker must not survive the re-encode"
    );
}

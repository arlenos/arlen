//! Integration test for the sandboxed image-view worker (`apps/viewers`): the
//! worker decodes an untrusted image and produces a full-resolution PNG
//! end-to-end, through the real Landlock + seccomp subprocess lockdown. This is
//! the check that the viewer's image decode actually works under the sandbox
//! (the seccomp allowlist permits what the decoder needs), keeps a sub-cap image
//! at native resolution, caps an oversize one, and fails closed on non-image
//! input.

#![cfg(all(target_os = "linux", feature = "thumbnail"))]

use std::path::Path;

const BIN: &str = env!("CARGO_BIN_EXE_arlen-image-view-sandbox");

/// A synthetic PNG of `w`x`h` (a smooth gradient, so it compresses small enough
/// to stay well under the worker's output cap even at viewer resolution).
fn png_bytes(w: u32, h: u32) -> Vec<u8> {
    let img = image::RgbImage::from_fn(w, h, |x, y| {
        image::Rgb([(x % 256) as u8, (y % 256) as u8, 160])
    });
    let mut out = Vec::new();
    image::DynamicImage::ImageRgb8(img)
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .unwrap();
    out
}

#[test]
fn view_decodes_an_untrusted_image_at_full_resolution_inside_the_sandbox() {
    use image::GenericImageView;
    // 1600x900 is under VIEWER_MAX_DIM on both sides, so it must come back at
    // its native resolution (the viewer shows the picture, not a thumbnail).
    let src = png_bytes(1600, 900);
    let png = arlen_ai_sandbox::view_image(Path::new(BIN), &src)
        .expect("the sandboxed worker should decode the image");
    let decoded = image::load_from_memory(&png).expect("the worker output is a valid PNG");
    assert_eq!(decoded.dimensions(), (1600, 900));
}

#[test]
fn view_caps_an_oversize_image_under_the_sandbox() {
    use image::GenericImageView;
    // 6000x3000 exceeds VIEWER_MAX_DIM on the long side: downscaled to fit,
    // aspect preserved.
    let src = png_bytes(6000, 3000);
    let png = arlen_ai_sandbox::view_image(Path::new(BIN), &src)
        .expect("the sandboxed worker should decode and cap the image");
    let (w, h) = image::load_from_memory(&png).unwrap().dimensions();
    assert_eq!((w, h), (arlen_ai_sandbox::VIEWER_MAX_DIM, arlen_ai_sandbox::VIEWER_MAX_DIM / 2));
}

/// A 16x8 JPEG XL, generated once with `cjxl` (image-rs cannot encode JXL).
const SAMPLE_JXL: &[u8] = include_bytes!("../test-fixtures/sample.jxl");

#[test]
fn view_decodes_a_jxl_image_under_the_sandbox() {
    use image::GenericImageView;
    // jxl-oxide is pure-Rust but may want syscalls (threads, mmap) the seccomp
    // allowlist must permit; this is the check that JXL decode works under the
    // real lockdown, not just in-process.
    let png = arlen_ai_sandbox::view_image(Path::new(BIN), SAMPLE_JXL)
        .expect("the sandboxed worker should decode the JXL");
    let (w, h) = image::load_from_memory(&png).unwrap().dimensions();
    assert_eq!((w, h), (16, 8));
}

#[test]
fn view_worker_fails_closed_on_non_image_input() {
    let err = arlen_ai_sandbox::view_image(Path::new(BIN), b"this is plainly not an image")
        .expect_err("a non-image input must not yield a decoded image");
    // The worker exits non-zero on the decode failure, surfaced as WorkerFailed.
    assert!(matches!(err, arlen_ai_sandbox::SandboxError::WorkerFailed(_)));
}

#[test]
fn the_decoded_image_does_not_carry_source_trailing_bytes() {
    // The worker re-encodes the decoded pixels, so nothing of the source file
    // (metadata, trailing payloads) survives. Append a marker after a valid PNG
    // (decoders ignore trailing bytes) and assert it never appears in the output.
    const MARKER: &[u8] = b"SECRET_SOURCE_MARKER_DO_NOT_LEAK";
    let mut src = png_bytes(256, 192);
    src.extend_from_slice(MARKER);
    let png = arlen_ai_sandbox::view_image(Path::new(BIN), &src).expect("decodes");
    assert!(
        !png.windows(MARKER.len()).any(|w| w == MARKER),
        "the source trailing marker must not survive the re-encode"
    );
}

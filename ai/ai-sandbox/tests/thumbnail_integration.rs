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

//! Integration test for the AVIF/HEIC codec worker (`apps/viewers`): a real
//! AVIF decodes end-to-end through the dav1d C decoder running under the
//! THREADED seccomp profile (the per-decoder isolation Tim approved). This is
//! the check that AVIF decode actually works under the wider-but-confined
//! sandbox - dav1d's thread pool needs `clone`, which only this worker permits.

#![cfg(all(target_os = "linux", feature = "codec"))]

use std::path::Path;

const BIN: &str = env!("CARGO_BIN_EXE_arlen-image-codec-sandbox");

/// A 16x8 AVIF, generated once with `avifenc` (image-rs cannot encode AVIF).
const SAMPLE_AVIF: &[u8] = include_bytes!("../test-fixtures/sample.avif");

#[test]
fn avif_is_detected_as_a_codec_format() {
    assert!(arlen_ai_sandbox::is_codec_format(SAMPLE_AVIF), "ftyp 'avif' brand");
    // A PNG is not a codec format (it goes to the tight worker).
    assert!(!arlen_ai_sandbox::is_codec_format(b"\x89PNG\r\n\x1a\n and more"));
    assert!(!arlen_ai_sandbox::is_codec_format(b"short"));
}

#[test]
fn avif_decodes_under_the_threaded_codec_worker() {
    use image::GenericImageView;
    let png = arlen_ai_sandbox::view_image_codec(Path::new(BIN), SAMPLE_AVIF)
        .expect("the codec worker should decode the AVIF via dav1d under the threaded sandbox");
    let (w, h) = image::load_from_memory(&png).unwrap().dimensions();
    assert_eq!((w, h), (16, 8));
}

/// A 16x8 HEIC, generated once with `heif-enc` (image-rs cannot decode HEIC).
const SAMPLE_HEIC: &[u8] = include_bytes!("../test-fixtures/sample.heic");

#[test]
fn heic_is_detected_as_a_codec_format() {
    assert!(arlen_ai_sandbox::is_codec_format(SAMPLE_HEIC), "ftyp 'heic' brand");
}

#[test]
fn heic_decodes_under_the_threaded_codec_worker() {
    use image::GenericImageView;
    // libheif + libde265 spawn decode threads; only the threaded worker permits
    // the clone, so this proves the per-decoder isolation covers HEIC too.
    let png = arlen_ai_sandbox::view_image_codec(Path::new(BIN), SAMPLE_HEIC)
        .expect("the codec worker should decode the HEIC via libheif under the threaded sandbox");
    let (w, h) = image::load_from_memory(&png).unwrap().dimensions();
    assert_eq!((w, h), (16, 8));
}

#[test]
fn the_codec_worker_fails_closed_on_non_image_input() {
    let err = arlen_ai_sandbox::view_image_codec(Path::new(BIN), b"this is plainly not an image")
        .expect_err("a non-image input must not yield a decoded image");
    assert!(matches!(err, arlen_ai_sandbox::SandboxError::WorkerFailed(_)));
}

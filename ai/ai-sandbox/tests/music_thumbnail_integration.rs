//! Music cover-art thumbnailing exercised through the REAL sandbox worker.
//!
//! The parent feeds a hand-built FLAC (with and without embedded art) to the
//! locked-down `arlen-music-thumbnail-sandbox` worker and checks the result. It
//! proves the lofty tag parse AND the image decode both run under Landlock +
//! seccomp (a syscall the lockdown forbids would kill the worker, so a returned
//! thumbnail is also evidence the sandbox permits the parse+decode path).
//!
//! Marked `#[ignore]` like the image integration test: it needs a Landlock-capable
//! kernel and the built worker binary. Run with `--features music -- --ignored`.

#![cfg(all(target_os = "linux", feature = "music"))]

use std::path::Path;

/// The built worker binary, provided by cargo to the integration test.
fn worker_bin() -> &'static str {
    env!("CARGO_BIN_EXE_arlen-music-thumbnail-sandbox")
}

/// A small PNG to embed as cover art.
fn tiny_png() -> Vec<u8> {
    let img = image::RgbImage::from_pixel(48, 48, image::Rgb([10, 120, 200]));
    let mut buf = Vec::new();
    image::DynamicImage::ImageRgb8(img)
        .write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)
        .unwrap();
    buf
}

fn be24(v: u32) -> [u8; 3] {
    [(v >> 16) as u8, (v >> 8) as u8, v as u8]
}

/// A minimal FLAC (magic + STREAMINFO + optional PICTURE block). No audio frames;
/// the worker reads with properties off, so the metadata alone parses.
fn minimal_flac(png: Option<&[u8]>) -> Vec<u8> {
    let mut f = Vec::new();
    f.extend_from_slice(b"fLaC");
    f.push(if png.is_some() { 0x00 } else { 0x80 });
    f.extend_from_slice(&be24(34));
    let mut si = [0u8; 34];
    si[0..2].copy_from_slice(&4096u16.to_be_bytes());
    si[2..4].copy_from_slice(&4096u16.to_be_bytes());
    let packed: u64 = (44_100u64 << 44) | (1u64 << 41) | (15u64 << 36);
    si[10..18].copy_from_slice(&packed.to_be_bytes());
    f.extend_from_slice(&si);
    if let Some(png) = png {
        let mut pic = Vec::new();
        pic.extend_from_slice(&3u32.to_be_bytes());
        let mime = b"image/png";
        pic.extend_from_slice(&(mime.len() as u32).to_be_bytes());
        pic.extend_from_slice(mime);
        for _ in 0..5 {
            pic.extend_from_slice(&0u32.to_be_bytes()); // description, w, h, depth, colours
        }
        pic.extend_from_slice(&(png.len() as u32).to_be_bytes());
        pic.extend_from_slice(png);
        f.push(0x86);
        f.extend_from_slice(&be24(pic.len() as u32));
        f.extend_from_slice(&pic);
    }
    f
}

#[test]
#[ignore]
fn embedded_cover_art_is_extracted_and_thumbnailed_under_lockdown() {
    let file = minimal_flac(Some(&tiny_png()));
    let thumb = arlen_ai_sandbox::album_art_thumbnail(Path::new(worker_bin()), &file)
        .expect("the worker ran under the sandbox")
        .expect("embedded art produces a thumbnail");
    // The output re-decodes as an image: the worker really extracted + downscaled.
    let img = image::load_from_memory(&thumb).expect("worker output is a valid image");
    assert!(img.width() <= 256 && img.height() <= 256, "downscaled to the thumbnail bound");
}

#[test]
#[ignore]
fn a_file_without_art_yields_no_thumbnail_under_lockdown() {
    let file = minimal_flac(None);
    let out = arlen_ai_sandbox::album_art_thumbnail(Path::new(worker_bin()), &file)
        .expect("the worker ran under the sandbox");
    assert!(out.is_none(), "no embedded art means no thumbnail (fall back to the icon)");
}

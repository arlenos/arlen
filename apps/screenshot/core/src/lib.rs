//! Command logic for the screenshot app's Tauri backend (screenshot-capture-plan.md).
//!
//! The app has no `src-tauri` yet, so its built annotate frontend cannot go live; this
//! crate holds the app-specific command LOGIC the thin `#[command]` wrappers will call,
//! kept separate so it is unit-tested without the webkit/Wayland runtime. The capture
//! and clipboard SIDE EFFECTS stay in `src-tauri` (they need the live compositor); this
//! crate owns the pure transforms - encoding a captured PNG for the webview, saving the
//! annotated result the webview hands back, and the serializable source-picker DTOs over
//! `sdk/screen-capture`.

use std::io;
use std::path::{Path, PathBuf};

use base64::Engine;
use serde::{Deserialize, Serialize};

use arlen_screen_capture::{default_filename, screenshots_dir, CapturedImage, OutputInfo, WindowInfo};

/// Encode a captured image's RGBA pixels to in-memory PNG bytes. `sdk::write_png` only
/// writes to a file, but the `capture_*` commands need the bytes in memory to return a
/// data URL to the webview; this mirrors its encoder settings (RGBA8) over a `Vec`.
pub fn to_png_bytes(image: &CapturedImage) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, image.width, image.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        // The buffer is exactly `width*height*4` RGBA bytes (the capture invariant), so
        // the header/pixel writes cannot fail on a well-formed image; a malformed one is
        // a capture-side bug, surfaced loudly rather than returning a corrupt PNG.
        let mut writer = encoder.write_header().expect("png header for a captured image");
        writer.write_image_data(&image.rgba).expect("png pixels for a captured image");
    }
    out
}

/// Why a PNG handed back for the clipboard could not be decoded to RGBA.
#[derive(Debug)]
pub struct PngDecodeError(pub String);

impl std::fmt::Display for PngDecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "png decode: {}", self.0)
    }
}

impl std::error::Error for PngDecodeError {}

/// Decode PNG `bytes` back to `(rgba, width, height)` for a clipboard image offer
/// (`arboard` wants raw RGBA, not a PNG). The inverse of [`to_png_bytes`].
///
/// A browser canvas encoding an OPAQUE capture to PNG may drop the alpha channel and
/// emit an RGB (color type 2) image, so an 8-bit RGB PNG is accepted and expanded to
/// RGBA with a full-opaque alpha rather than rejected; an 8-bit RGBA PNG is taken as
/// is. Other forms (non-8-bit, grayscale, palette) a capture/annotate canvas does not
/// produce, so they fail closed rather than guess a channel layout.
pub fn decode_png_rgba(bytes: &[u8]) -> Result<(Vec<u8>, u32, u32), PngDecodeError> {
    let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    let mut reader = decoder.read_info().map_err(|e| PngDecodeError(e.to_string()))?;
    let size = reader
        .output_buffer_size()
        .ok_or_else(|| PngDecodeError("image too large to buffer".into()))?;
    let mut buf = vec![0u8; size];
    let info = reader.next_frame(&mut buf).map_err(|e| PngDecodeError(e.to_string()))?;
    if info.bit_depth != png::BitDepth::Eight {
        return Err(PngDecodeError(format!("expected 8-bit, got {:?}", info.bit_depth)));
    }
    let (w, h) = (info.width, info.height);
    let px = (w as usize) * (h as usize);
    let rgba = match info.color_type {
        png::ColorType::Rgba => {
            buf.truncate(px * 4);
            buf
        }
        png::ColorType::Rgb => {
            let mut out = Vec::with_capacity(px * 4);
            for chunk in buf[..px * 3].chunks_exact(3) {
                out.extend_from_slice(chunk);
                out.push(255);
            }
            out
        }
        other => return Err(PngDecodeError(format!("unsupported color type {other:?}"))),
    };
    Ok((rgba, w, h))
}

/// Encode PNG `bytes` as a `data:image/png;base64,...` URL the webview draws onto its
/// capture canvas. The capture side (`sdk/screen-capture`) produces the PNG; this is the
/// wire form the `capture_*` commands return to the frontend.
pub fn png_data_url(bytes: &[u8]) -> String {
    let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
    format!("data:image/png;base64,{b64}")
}

/// Encode a captured image straight to the webview data URL (the `capture_*` command
/// path): `to_png_bytes` then `png_data_url`.
pub fn capture_to_data_url(image: &CapturedImage) -> String {
    png_data_url(&to_png_bytes(image))
}

/// Write the annotated capture's PNG `bytes` (the webview hands back the flattened
/// canvas) into `dir` as `filename`, creating `dir` if absent, and return the full path.
/// The bytes are already a complete PNG, so they are written verbatim - distinct from
/// `sdk::write_png`, which encodes a raw `CapturedImage`.
pub fn save_png_bytes(bytes: &[u8], dir: &Path, filename: &str) -> io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(filename);
    std::fs::write(&path, bytes)?;
    Ok(path)
}

/// Save an annotated capture to the freedesktop screenshots directory under the default
/// `Screenshot-<timestamp>.png` name. `timestamp` is formatted by the caller
/// (`%Y%m%d-%H%M%S`), matching `sdk::default_filename`.
pub fn save_capture(bytes: &[u8], timestamp: &str) -> io::Result<PathBuf> {
    save_png_bytes(bytes, &screenshots_dir(), &default_filename(timestamp))
}

/// A display output offered in the capture-source picker: a stable IPC shape over
/// `sdk::OutputInfo`, so the frontend does not couple to the capture crate's internals.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputDto {
    /// The output's capture index (passed back to a `capture_output` command).
    pub index: usize,
    /// The connector name, e.g. `DP-1`, when the compositor advertised one.
    pub name: Option<String>,
    /// The output's pixel width.
    pub width: i32,
    /// The output's pixel height.
    pub height: i32,
}

impl From<&OutputInfo> for OutputDto {
    fn from(o: &OutputInfo) -> Self {
        Self {
            index: o.index,
            name: o.name.clone(),
            width: o.width,
            height: o.height,
        }
    }
}

/// A toplevel window offered in the capture-source picker: a stable shape over
/// `sdk::WindowInfo`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowDto {
    /// The window's capture index (passed back to a `capture_window` command).
    pub index: usize,
    /// The window title, when the compositor advertised one.
    pub title: Option<String>,
    /// The window's app id, when advertised.
    pub app_id: Option<String>,
}

impl From<&WindowInfo> for WindowDto {
    fn from(w: &WindowInfo) -> Self {
        Self {
            index: w.index,
            title: w.title.clone(),
            app_id: w.app_id.clone(),
        }
    }
}

/// Map the capture crate's output list to the picker DTOs, preserving order.
pub fn output_dtos(outputs: &[OutputInfo]) -> Vec<OutputDto> {
    outputs.iter().map(OutputDto::from).collect()
}

/// Map the capture crate's window list to the picker DTOs, preserving order.
pub fn window_dtos(windows: &[WindowInfo]) -> Vec<WindowDto> {
    windows.iter().map(WindowDto::from).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // A minimal 1x1 PNG (the exact bytes are irrelevant to these transforms; only that
    // they are carried verbatim and base64-round-trip).
    const PNG: &[u8] = &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 1, 2, 3, 4];

    #[test]
    fn to_png_bytes_encodes_a_decodable_rgba_image() {
        // A 2x1 RGBA image: one red pixel, one green.
        let img = CapturedImage {
            width: 2,
            height: 1,
            rgba: vec![255, 0, 0, 255, 0, 255, 0, 255],
        };
        let png = to_png_bytes(&img);
        assert_eq!(
            &png[..8],
            &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a],
            "PNG magic bytes"
        );
        // Decode it back: the dimensions and pixels must survive the encode.
        let decoder = png::Decoder::new(std::io::Cursor::new(&png));
        let mut reader = decoder.read_info().unwrap();
        let mut buf = vec![0u8; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut buf).unwrap();
        assert_eq!((info.width, info.height), (2, 1));
        assert_eq!(&buf[..8], &img.rgba[..], "RGBA pixels round-trip");
        // The data-url path wraps exactly this PNG.
        let url = capture_to_data_url(&img);
        let b64 = url.strip_prefix("data:image/png;base64,").unwrap();
        assert_eq!(
            base64::engine::general_purpose::STANDARD.decode(b64).unwrap(),
            png,
            "capture_to_data_url carries the encoded PNG"
        );
    }

    #[test]
    fn decode_png_rgba_round_trips_an_rgba_image() {
        let img = CapturedImage {
            width: 2,
            height: 1,
            rgba: vec![255, 0, 0, 255, 0, 255, 0, 128],
        };
        let png = to_png_bytes(&img);
        let (rgba, w, h) = decode_png_rgba(&png).unwrap();
        assert_eq!((w, h), (2, 1));
        assert_eq!(rgba, img.rgba, "the alpha channel survives the round trip");
    }

    #[test]
    fn decode_png_rgba_expands_an_opaque_rgb_png_to_rgba() {
        // A browser encoding an opaque capture may emit an RGB PNG (no alpha); build
        // one directly and confirm decode expands it to full-opaque RGBA.
        let mut rgb_png = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut rgb_png, 2, 1);
            enc.set_color(png::ColorType::Rgb);
            enc.set_depth(png::BitDepth::Eight);
            let mut w = enc.write_header().unwrap();
            w.write_image_data(&[10, 20, 30, 40, 50, 60]).unwrap();
        }
        let (rgba, w, h) = decode_png_rgba(&rgb_png).unwrap();
        assert_eq!((w, h), (2, 1));
        assert_eq!(rgba, vec![10, 20, 30, 255, 40, 50, 60, 255], "rgb pixels gain opaque alpha");
    }

    #[test]
    fn decode_png_rgba_rejects_non_png_bytes() {
        assert!(decode_png_rgba(b"not a png at all").is_err());
    }

    #[test]
    fn png_data_url_has_the_png_mime_and_round_trips_the_bytes() {
        let url = png_data_url(PNG);
        let b64 = url.strip_prefix("data:image/png;base64,").expect("png data-url prefix");
        let decoded = base64::engine::general_purpose::STANDARD.decode(b64).unwrap();
        assert_eq!(decoded, PNG, "the exact capture bytes survive the encode");
    }

    #[test]
    fn save_png_bytes_creates_the_dir_and_writes_verbatim() {
        let tmp = tempfile::tempdir().unwrap();
        // A not-yet-existing nested dir must be created.
        let dir = tmp.path().join("shots");
        let path = save_png_bytes(PNG, &dir, "Screenshot-x.png").unwrap();
        assert_eq!(path, dir.join("Screenshot-x.png"));
        assert_eq!(std::fs::read(&path).unwrap(), PNG, "bytes written unmodified");
    }

    #[test]
    fn save_capture_uses_the_default_name_under_the_screenshots_dir() {
        // Pin the screenshots dir to a temp location so the test never writes to the
        // real Pictures dir; save_capture composes screenshots_dir() + default_filename().
        let tmp = tempfile::tempdir().unwrap();
        // SAFETY: single-threaded test; the env is restored by tempdir drop irrelevance.
        std::env::set_var("XDG_SCREENSHOTS_DIR", tmp.path());
        let path = save_capture(PNG, "20260725-064500").unwrap();
        std::env::remove_var("XDG_SCREENSHOTS_DIR");
        assert_eq!(path.file_name().unwrap(), "Screenshot-20260725-064500.png");
        assert_eq!(path.parent().unwrap(), tmp.path());
        assert_eq!(std::fs::read(&path).unwrap(), PNG);
    }

    #[test]
    fn output_and_window_dtos_preserve_fields_and_order() {
        let outputs = vec![
            OutputInfo {
                index: 0,
                name: Some("DP-1".into()),
                width: 2560,
                height: 1440,
                logical_x: 0,
                logical_y: 0,
                logical_width: 2560,
                logical_height: 1440,
            },
            OutputInfo {
                index: 1,
                name: None,
                width: 1920,
                height: 1080,
                logical_x: 2560,
                logical_y: 0,
                logical_width: 1920,
                logical_height: 1080,
            },
        ];
        let dtos = output_dtos(&outputs);
        assert_eq!(dtos.len(), 2);
        assert_eq!(dtos[0], OutputDto { index: 0, name: Some("DP-1".into()), width: 2560, height: 1440 });
        assert_eq!(dtos[1].index, 1);
        assert!(dtos[1].name.is_none());

        let windows = vec![WindowInfo {
            index: 3,
            title: Some("Editor".into()),
            app_id: Some("org.arlen.Editor".into()),
        }];
        let wd = window_dtos(&windows);
        assert_eq!(wd, vec![WindowDto { index: 3, title: Some("Editor".into()), app_id: Some("org.arlen.Editor".into()) }]);
    }
}

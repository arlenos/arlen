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

use arlen_screen_capture::{default_filename, screenshots_dir, OutputInfo, WindowInfo};

/// Encode PNG `bytes` as a `data:image/png;base64,...` URL the webview draws onto its
/// capture canvas. The capture side (`sdk/screen-capture`) produces the PNG; this is the
/// wire form the `capture_*` commands return to the frontend.
pub fn png_data_url(bytes: &[u8]) -> String {
    let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
    format!("data:image/png;base64,{b64}")
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

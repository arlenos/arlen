//! Arlen screenshot app backend host.
//!
//! Thin Tauri shell around `sdk/screen-capture` (the Wayland capture) and
//! `arlen-screenshot-core` (the tested encode/save/DTO logic): the `capture_*`
//! commands hand the webview a PNG data URL to draw and annotate; `save_screenshot`
//! writes the flattened annotated PNG the webview sends back to the freedesktop
//! screenshots directory. The capture + filesystem side effects live here because
//! they need the live compositor; the transforms they call are unit-tested in the
//! core. The clipboard-copy command is a follow-up (it needs a PNG->RGBA decode +
//! the clipboard offer).

use arlen_screen_capture as capture;
use arlen_screenshot_core::{
    capture_to_data_url, output_dtos, save_capture, window_dtos, OutputDto, WindowDto,
};
use base64::Engine;

/// Whether the compositor advertises the screencopy interface the capture path needs,
/// so the frontend can show a clear "capture unavailable here" state rather than fail
/// on the first attempt.
#[tauri::command]
fn capture_available() -> bool {
    capture::capture_support()
        .map(|s| s.has_copy_manager())
        .unwrap_or(false)
}

/// The display outputs offered in the capture-source picker.
#[tauri::command]
fn list_outputs() -> Result<Vec<OutputDto>, String> {
    capture::list_outputs()
        .map(|o| output_dtos(&o))
        .map_err(|e| e.to_string())
}

/// The toplevel windows offered in the capture-source picker.
#[tauri::command]
fn list_windows() -> Result<Vec<WindowDto>, String> {
    capture::list_windows()
        .map(|w| window_dtos(&w))
        .map_err(|e| e.to_string())
}

/// Capture a whole output and return it as a PNG data URL for the annotate canvas.
#[tauri::command]
fn capture_output(index: usize, include_cursor: bool) -> Result<String, String> {
    let img = capture::capture_output(index, include_cursor).map_err(|e| e.to_string())?;
    Ok(capture_to_data_url(&img))
}

/// Capture a single window and return it as a PNG data URL.
#[tauri::command]
fn capture_window(index: usize, include_cursor: bool) -> Result<String, String> {
    let img = capture::capture_window(index, include_cursor).map_err(|e| e.to_string())?;
    Ok(capture_to_data_url(&img))
}

/// Capture a rectangular region of an output and return it as a PNG data URL.
#[tauri::command]
fn capture_region(
    output_index: usize,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    include_cursor: bool,
) -> Result<String, String> {
    let img = capture::capture_region(output_index, x, y, w, h, include_cursor)
        .map_err(|e| e.to_string())?;
    Ok(capture_to_data_url(&img))
}

/// Save the flattened annotated capture (the webview hands back base64 PNG bytes) to
/// the freedesktop screenshots directory under `Screenshot-<timestamp>.png`, and
/// return the saved path for the frontend to surface.
#[tauri::command]
fn save_screenshot(png_base64: String) -> Result<String, String> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(png_base64.as_bytes())
        .map_err(|e| format!("bad PNG payload: {e}"))?;
    let path = save_capture(&bytes, &now_timestamp()).map_err(|e| e.to_string())?;
    Ok(path.to_string_lossy().into_owned())
}

/// Copy the annotated capture (base64 PNG from the webview) to the system clipboard as
/// an image. `arboard` wants raw RGBA, so the core decodes the PNG (expanding an opaque
/// RGB encoding to RGBA) before the offer.
#[tauri::command]
fn copy_png(png_base64: String) -> Result<(), String> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(png_base64.as_bytes())
        .map_err(|e| format!("bad PNG payload: {e}"))?;
    let (rgba, w, h) = arlen_screenshot_core::decode_png_rgba(&bytes).map_err(|e| e.to_string())?;
    let mut clipboard = arboard::Clipboard::new().map_err(|e| e.to_string())?;
    clipboard
        .set_image(arboard::ImageData {
            width: w as usize,
            height: h as usize,
            bytes: std::borrow::Cow::Owned(rgba),
        })
        .map_err(|e| e.to_string())
}

/// A frontend log line surfaced on the app's stdout (the webview has no DevTools in
/// the Arlen shell, so this is how the UI reports diagnostics).
#[tauri::command]
fn frontend_log(message: String) {
    log::info!("[screenshot-frontend] {message}");
}

/// The current local time as `YYYYMMDD-HHMMSS`, the stamp `sdk::default_filename`
/// expects. Falls back to UTC when the local offset is unavailable (a sandbox may
/// hide the zone), and to a fixed token if formatting itself fails.
fn now_timestamp() -> String {
    use time::macros::format_description;
    use time::OffsetDateTime;
    let now = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());
    let fmt = format_description!("[year][month][day]-[hour][minute][second]");
    now.format(fmt).unwrap_or_else(|_| "capture".to_string())
}

/// Run the screenshot app.
pub fn run() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            capture_available,
            list_outputs,
            list_windows,
            capture_output,
            capture_window,
            capture_region,
            save_screenshot,
            copy_png,
            frontend_log
        ])
        .run(tauri::generate_context!())
        .expect("error while running arlen-screenshot");
}

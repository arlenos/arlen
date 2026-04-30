//! Tauri command handlers grouped by config file.

pub mod about;
pub mod accessibility;
pub mod brightness;
pub mod config;
pub mod displays;
pub mod input;
pub mod knowledge;
pub mod modules;
pub mod night_light;
pub mod notifications;
pub mod picker;
pub mod search;
pub mod theme;
pub mod url;
pub mod waypointer_plugins;

/// Route a log line from the frontend into the Rust logger so it
/// shows up in the same stdout stream as backend logs. Used by
/// debug instrumentation when WebView DevTools are not reachable.
#[tauri::command]
pub fn frontend_log(level: String, msg: String) {
    match level.as_str() {
        "warn" => log::warn!("[frontend] {msg}"),
        "error" => log::error!("[frontend] {msg}"),
        _ => log::info!("[frontend] {msg}"),
    }
}

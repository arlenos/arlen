//! Tauri command handlers grouped by config file.

pub mod about;
pub mod accessibility;
pub mod activity;
pub mod ai;
pub mod app_facts;
pub mod app_lifecycle;
pub mod app_settings;
pub mod brightness;
pub mod capsules;
pub mod config;
pub mod displays;
pub mod extensions;
pub mod input;
pub mod knowledge;
pub mod layouts;
pub mod mo;
pub mod modules;
pub mod night_light;
pub mod notifications;
pub mod picker;
pub mod printers;
pub mod privacy;
pub mod search;
pub mod sensing;
pub mod sound;
pub mod theme;
pub mod topbar;
pub mod url;
pub mod values;
pub mod wallpaper;
pub mod waypointer_plugins;
pub mod windows_apps;

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

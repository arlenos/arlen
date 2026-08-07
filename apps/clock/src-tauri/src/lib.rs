//! The Arlen clock's Tauri shell.
//!
//! **This app is a view and owns nothing** (`clock-app.md` §1). The alarms, the
//! timers, the focus session and the stopwatch live in the clock daemon; this
//! window renders them and may be closed at any time without changing anything.
//! An alarm that stops existing when a window closes is not an alarm, which is
//! why the state is not here.
//!
//! **The clock commands are deliberately not registered yet.** The daemon they
//! forward to does not exist, and a stub that answered them would be a window
//! that looks connected while nothing is keeping time - the worst of the three
//! possible states. Unregistered, the frontend's own catch path engages and the
//! app runs on its fixture with `clockMocked` set, which is the honest one. They
//! land in the same change as the daemon.

/// A structured log line from the frontend into the app's stdout (the shell has
/// no devtools console an operator can open).
#[tauri::command]
fn frontend_log(level: String, message: String) {
    match level.as_str() {
        "error" => log::error!("[frontend] {message}"),
        "warn" => log::warn!("[frontend] {message}"),
        _ => log::info!("[frontend] {message}"),
    }
}

/// Start the window.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    tauri::Builder::default()
        .plugin(tauri_plugin_arlen_shell::init())
        .invoke_handler(tauri::generate_handler![frontend_log])
        .run(tauri::generate_context!())
        .expect("error while running the clock");
}

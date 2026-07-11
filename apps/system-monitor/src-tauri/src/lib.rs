//! The Arlen system monitor Tauri shell (the task manager). The landing is the
//! live process list - what is running, the hog on top, real CPU/memory/disk per
//! row - replacing the frontend fixture with a real `/proc` feed.
//!
//! `list_processes` samples `/proc` and computes CPU% + disk-rate deltas against
//! the previous sample (held in the [`procmon::Monitor`] managed state), so the
//! rates settle after the first poll while memory + names are real immediately.
//! The Stop/Freeze/Limit actions are the next increment; per-process network needs
//! eBPF/cgroup attribution and is reported as 0 until then.

use tauri::Manager;

mod procmon;

use procmon::{Monitor, Process};

/// A structured log line from the frontend into the app's stdout (the shell has no
/// devtools console the operator can open).
#[tauri::command]
fn frontend_log(level: String, message: String) {
    match level.as_str() {
        "error" => log::error!("[frontend] {message}"),
        "warn" => log::warn!("[frontend] {message}"),
        _ => log::info!("[frontend] {message}"),
    }
}

/// The live process list: real `/proc` processes mapped to the frontend `Process`
/// shape, the hog on top. CPU% and disk KB/s are 0 on the first call (no previous
/// sample to delta against) and real from the second poll on.
#[tauri::command]
fn list_processes(monitor: tauri::State<'_, Monitor>) -> Vec<Process> {
    monitor.sample()
}

/// Build + run the app.
pub fn run() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    tauri::Builder::default()
        .plugin(tauri_plugin_arlen_shell::init())
        .setup(|app| {
            app.manage(Monitor::new());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![frontend_log, list_processes])
        .run(tauri::generate_context!())
        .expect("error while running arlen-system-monitor");
}

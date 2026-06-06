//! Lunaris AI harness app backend.
//!
//! Tauri entry point for the GUI door to the AI layer (conversation +
//! agent observability). A1 is the skeleton: a runnable window and the
//! frontend-log bridge. The daemon (query/chat), agent D-Bus, audit
//! read, and Event Bus wiring land in A2+ (see
//! `docs/architecture/ai-app.md` §7).

mod activity;
mod ai_client;
mod capability;

/// Route a log line from the frontend into the Rust logger so it shows
/// up in the same stdout stream as backend logs. Tauri WebView DevTools
/// are not always reachable, so frontend diagnostics go through here.
#[tauri::command]
fn frontend_log(level: String, msg: String) {
    match level.as_str() {
        "warn" => log::warn!("[frontend] {msg}"),
        "error" => log::error!("[frontend] {msg}"),
        _ => log::info!("[frontend] {msg}"),
    }
}

/// Tauri application entry point invoked from `main.rs`.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            frontend_log,
            ai_client::ai_query,
            activity::ai_activity_recent,
            capability::ai_capability
        ])
        .run(tauri::generate_context!())
        .expect("error while running lunaris-harness");
}

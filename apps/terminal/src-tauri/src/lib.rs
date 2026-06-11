//! Arlen terminal app backend host.
//!
//! Thin Tauri shell around the embeddable terminal component
//! (`terminal.md` §2.1b): every `terminal_*` command is a one-line
//! wrapper over `arlen_terminal_core::stub`, so the engine can replace
//! the stubs behind unchanged signatures when it lands (TM-R1). The
//! UI is built and verified against this exact surface.

mod capability;

use arlen_terminal_core::{stub, Block, HistoryFilters, Project, Session};

/// Route a log line from the frontend into the Rust logger so it shows
/// up in the same stdout stream as backend logs.
#[tauri::command]
fn frontend_log(level: String, msg: String) {
    match level.as_str() {
        "warn" => log::warn!("[frontend] {msg}"),
        "error" => log::error!("[frontend] {msg}"),
        _ => log::info!("[frontend] {msg}"),
    }
}

/// The open shells.
#[tauri::command]
fn terminal_sessions() -> Vec<Session> {
    stub::sessions()
}

/// A session's blocks.
#[tauri::command]
fn terminal_blocks(session_id: String) -> Vec<Block> {
    stub::blocks(&session_id)
}

/// Feed input to a session's shell.
#[tauri::command]
fn terminal_input(session_id: String, input: String) -> Result<(), String> {
    stub::input(&session_id, &input)
}

/// Open a new shell.
#[tauri::command]
fn terminal_new_session() -> Result<Session, String> {
    stub::new_session()
}

/// Search past blocks.
#[tauri::command]
fn terminal_history_search(query: String, filters: HistoryFilters) -> Vec<Block> {
    stub::history_search(&query, &filters)
}

/// The projects to scope to.
#[tauri::command]
fn terminal_projects() -> Vec<Project> {
    stub::projects()
}

/// Tauri application entry point invoked from `main.rs`.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            frontend_log,
            terminal_sessions,
            terminal_blocks,
            terminal_input,
            terminal_new_session,
            terminal_history_search,
            terminal_projects,
            capability::ai_capability
        ])
        .run(tauri::generate_context!())
        .expect("error while running arlen-terminal");
}

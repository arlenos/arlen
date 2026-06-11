//! Arlen terminal app backend host.
//!
//! Thin Tauri shell around the embeddable terminal component
//! (`terminal.md` §2.1b). The host owns the session registry — the
//! app process is the truth about which shells are open; the engine
//! (TM-R1) attaches a PTY per entry behind the same commands when it
//! wires in. Everything else is a one-line wrapper over
//! `arlen_terminal_core::stub` until then.

mod capability;

use std::sync::Mutex;

use arlen_terminal_core::{stub, Block, HistoryFilters, Project, Session};
use tauri::State;

/// The open sessions, host-owned. The id is assigned here; the engine
/// will key its PTYs by it.
struct SessionRegistry {
    sessions: Vec<Session>,
    next_id: u64,
}

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

/// The open shells, from the host registry.
#[tauri::command]
fn terminal_sessions(registry: State<Mutex<SessionRegistry>>) -> Vec<Session> {
    registry
        .lock()
        .map(|r| r.sessions.clone())
        .unwrap_or_default()
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

/// Open a new shell: the stub provides the shape and home cwd, the
/// registry assigns the id and remembers the session.
#[tauri::command]
fn terminal_new_session(registry: State<Mutex<SessionRegistry>>) -> Result<Session, String> {
    let mut reg = registry.lock().map_err(|e| e.to_string())?;
    let mut session = stub::new_session()?;
    reg.next_id += 1;
    session.id = format!("s{}", reg.next_id);
    reg.sessions.push(session.clone());
    Ok(session)
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
        .manage(Mutex::new(SessionRegistry {
            sessions: Vec::new(),
            next_id: 0,
        }))
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

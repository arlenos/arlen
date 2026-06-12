//! Arlen terminal app backend host.
//!
//! Thin Tauri shell around the embeddable terminal component
//! (`terminal.md` §2.1b). The host owns the session registry — the
//! app process is the truth about which shells are open; the engine
//! (TM-R1) attaches a PTY per entry behind the same commands when it
//! wires in. Everything else is a one-line wrapper over
//! `arlen_terminal_core::stub` until then.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

use arlen_terminal_core::blocks::BlockAssembler;
use arlen_terminal_core::vt::VtEngine;
use arlen_terminal_core::{stub, Block, HistoryFilters, Project, Session, SessionStatus};
use arlen_terminal_engine::PtyEngine;
use tauri::State;

/// A live shell: the contract [`Session`] the UI sees, the [`PtyEngine`] driving
/// its real PTY, and the [`BlockAssembler`] turning the engine's OSC-mark events
/// into command blocks.
struct LiveSession {
    session: Session,
    engine: PtyEngine,
    assembler: BlockAssembler,
}

/// The open shells, host-owned and keyed by id. `order` preserves creation order
/// for the sidebar; the id is assigned here and keys the engine's PTY.
struct SessionRegistry {
    sessions: HashMap<String, LiveSession>,
    order: Vec<String>,
    next_id: u64,
}

/// Whether the app runs under the Arlen shell (the event-bus socket
/// exists); the topbar quick actions only make sense there.
#[tauri::command]
fn shell_present() -> bool {
    std::path::Path::new("/run/arlen/event-bus-producer.sock").exists()
}

/// Export `ARLEN_TERM_ZDOTDIR` so a spawned shell sources the block-mark
/// integration via the curated zsh config (TM-R2/R3). Honors an explicit
/// override; otherwise prefers the installed location and falls back to the
/// in-repo dir for `cargo tauri dev`. Does nothing when none exists - the shell
/// then uses its normal startup, and in a production image the system zshrc
/// sources the integration directly. Called once at startup before any spawn.
fn ensure_curated_zdotdir() {
    if std::env::var_os(arlen_terminal_engine::ZDOTDIR_ENV).is_some() {
        return;
    }
    const CANDIDATES: &[&str] = &[
        "/usr/share/arlen/terminal/zdotdir",
        concat!(env!("CARGO_MANIFEST_DIR"), "/../integration/zdotdir"),
    ];
    for dir in CANDIDATES {
        if std::path::Path::new(dir).is_dir() {
            std::env::set_var(arlen_terminal_engine::ZDOTDIR_ENV, dir);
            return;
        }
    }
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

/// The open shells, from the host registry, in creation order.
#[tauri::command]
fn terminal_sessions(registry: State<Mutex<SessionRegistry>>) -> Vec<Session> {
    let Ok(reg) = registry.lock() else {
        return Vec::new();
    };
    reg.order
        .iter()
        .filter_map(|id| reg.sessions.get(id).map(|s| s.session.clone()))
        .collect()
}

/// A session's blocks: drain the engine's new OSC-mark events into the session's
/// assembler and return the assembled command blocks. Called off the listing path
/// by the UI polling; a missing session yields an empty list rather than an error.
#[tauri::command]
fn terminal_blocks(session_id: String, registry: State<Mutex<SessionRegistry>>) -> Vec<Block> {
    let Ok(mut reg) = registry.lock() else {
        return Vec::new();
    };
    let Some(live) = reg.sessions.get_mut(&session_id) else {
        return Vec::new();
    };
    let events = live.engine.drain_events();
    live.assembler.consume(&events, Instant::now());
    live.assembler.blocks()
}

/// Feed input (keystrokes) to a session's shell PTY.
#[tauri::command]
fn terminal_input(
    session_id: String,
    input: String,
    registry: State<Mutex<SessionRegistry>>,
) -> Result<(), String> {
    let mut reg = registry.lock().map_err(|e| e.to_string())?;
    let live = reg
        .sessions
        .get_mut(&session_id)
        .ok_or_else(|| format!("no such session: {session_id}"))?;
    live.engine.send_input(input.as_bytes()).map_err(|e| e.to_string())
}

/// The shells to try, in preference order, when opening a session. zsh is first
/// because the block-mark integration is zsh-only (the marks fire only there);
/// `$SHELL` and `/bin/sh` are fallbacks so the terminal still opens a working
/// shell on a machine without zsh (the command frames just will not assemble).
/// `$SHELL` is skipped when it is already zsh (the first entry covers it).
fn shell_candidates() -> Vec<String> {
    let mut out = vec!["zsh".to_string()];
    if let Ok(sh) = std::env::var("SHELL") {
        if !sh.is_empty() && !sh.ends_with("/zsh") && sh != "zsh" {
            out.push(sh);
        }
    }
    out.push("/bin/sh".to_string());
    out
}

/// Open a new shell: spawn a real shell on a PTY via the engine (preferring zsh,
/// which sources the curated integration when `ARLEN_TERM_ZDOTDIR` points at it,
/// so the block marks fire), assign the id, and remember the live session. Falls
/// back through [`shell_candidates`] so a machine without zsh still gets a shell.
#[tauri::command]
fn terminal_new_session(registry: State<Mutex<SessionRegistry>>) -> Result<Session, String> {
    let home = dirs::home_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| "/".to_string());
    let mut last_err = String::from("no shell candidates");
    let engine = shell_candidates()
        .iter()
        .find_map(|prog| match PtyEngine::spawn(prog, &[], Some(&home), 80, 24) {
            Ok(eng) => Some(eng),
            Err(e) => {
                last_err = format!("{prog}: {e}");
                None
            }
        })
        .ok_or_else(|| format!("could not start a shell ({last_err})"))?;
    let mut reg = registry.lock().map_err(|e| e.to_string())?;
    reg.next_id += 1;
    let id = format!("s{}", reg.next_id);
    let session = Session {
        id: id.clone(),
        cwd: home.clone(),
        status: SessionStatus::Running,
        last_exit: None,
    };
    reg.sessions.insert(
        id.clone(),
        LiveSession {
            session: session.clone(),
            engine,
            assembler: BlockAssembler::new(home),
        },
    );
    reg.order.push(id);
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
    ensure_curated_zdotdir();

    tauri::Builder::default()
        .plugin(tauri_plugin_arlen_shell::init())
        .manage(Mutex::new(SessionRegistry {
            sessions: HashMap::new(),
            order: Vec::new(),
            next_id: 0,
        }))
        .invoke_handler(tauri::generate_handler![
            shell_present,
            frontend_log,
            terminal_sessions,
            terminal_blocks,
            terminal_input,
            terminal_new_session,
            terminal_history_search,
            terminal_projects
        ])
        .run(tauri::generate_context!())
        .expect("error while running arlen-terminal");
}

#[cfg(test)]
mod tests {
    use super::shell_candidates;

    #[test]
    fn shell_candidates_prefer_zsh_and_end_at_bin_sh() {
        let c = shell_candidates();
        assert_eq!(c.first().map(String::as_str), Some("zsh"));
        assert_eq!(c.last().map(String::as_str), Some("/bin/sh"));
        // zsh is never listed twice even when $SHELL is a zsh path.
        assert_eq!(c.iter().filter(|s| s.ends_with("zsh")).count(), 1);
    }
}

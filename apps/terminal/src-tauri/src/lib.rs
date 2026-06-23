//! Arlen terminal app backend host.
//!
//! Thin Tauri shell around the embeddable terminal component
//! (`terminal.md` §2.1b). The host owns the session registry — the
//! app process is the truth about which shells are open; the engine
//! (TM-R1) attaches a PTY per entry behind the same commands when it
//! wires in. Everything else is a one-line wrapper over
//! `arlen_terminal_core::stub` until then.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;

use arlen_terminal_core::blocks::BlockAssembler;
use arlen_terminal_core::vt::VtEngine;
use arlen_terminal_core::{
    stub, Block, BlockBodyKind, GridSnapshot, HistoryFilters, Project, Session, SessionStatus,
};
use arlen_terminal_engine::PtyEngine;
use tauri::State;

/// A live shell: the contract [`Session`] the UI sees, the [`PtyEngine`] driving
/// its real PTY, and the [`BlockAssembler`] turning the engine's OSC-mark events
/// into command blocks.
struct LiveSession {
    session: Session,
    engine: PtyEngine,
    assembler: BlockAssembler,
    /// Each finished command's captured output grid, accumulated in finish order
    /// (the engine hands them out once, so they are kept here). Attached to the
    /// matching finished block so the block renders its own output. Index-aligned
    /// with the assembler's finished blocks: the curated integration emits a
    /// command line and an exec-start mark together, so every block has exactly
    /// one captured output.
    outputs: Vec<GridSnapshot>,
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
    // Accumulate the per-command output grids the engine captured (handed out
    // once), then attach each to its finished block so the block renders its own
    // output. Finished blocks and captured outputs are produced in the same order
    // (one per command), so they align by index; the trailing pending block (a
    // running command) has no captured output yet and is left as the live grid.
    live.outputs.extend(live.engine.take_finished_outputs());
    let mut blocks = live.assembler.blocks();
    for (i, output) in live.outputs.iter().enumerate() {
        let Some(block) = blocks.get_mut(i) else { break };
        if block.body_kind == BlockBodyKind::Grid {
            if let Ok(body) = serde_json::to_value(output) {
                block.body = body;
            }
        }
    }
    blocks
}

/// A session's visible screen as text (terminal.md Option B): the webview renders
/// this so command output appears without the compositor grid-subsurface. The UI
/// polls it alongside `terminal_blocks`; a missing session yields an empty grid.
#[tauri::command]
fn terminal_grid(session_id: String, registry: State<Mutex<SessionRegistry>>) -> GridSnapshot {
    let Ok(reg) = registry.lock() else {
        return GridSnapshot::default();
    };
    reg.sessions
        .get(&session_id)
        .map(|live| live.engine.screen_snapshot())
        .unwrap_or_default()
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

/// Resize a session's PTY (and its screen model) to `cols`x`rows`. The frontend
/// computes the grid size from the rendered cell metrics and calls this on a
/// window/pane resize; the engine resizes the master PTY (sending SIGWINCH so the
/// shell and any running TUI reflow) and the VT parser to match. A missing
/// session is an error, not a panic.
#[tauri::command]
fn terminal_resize(
    session_id: String,
    cols: u16,
    rows: u16,
    registry: State<Mutex<SessionRegistry>>,
) -> Result<(), String> {
    let mut reg = registry.lock().map_err(|e| e.to_string())?;
    let live = reg
        .sessions
        .get_mut(&session_id)
        .ok_or_else(|| format!("no such session: {session_id}"))?;
    live.engine.resize(cols, rows).map_err(|e| e.to_string())
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
            outputs: Vec::new(),
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

/// Map project rows (`{ id, name, path }`) into the contract [`Project`],
/// skipping any row missing the id or name. Pure, so the shaping is unit-tested
/// without a daemon. Mirrors the file manager's identical projects mapper.
fn projects_from_rows(rows: &[std::collections::HashMap<String, serde_json::Value>]) -> Vec<Project> {
    rows.iter()
        .filter_map(|r| {
            let id = r.get("id").and_then(|v| v.as_str())?;
            let name = r.get("name").and_then(|v| v.as_str())?;
            let path = r.get("path").and_then(|v| v.as_str()).unwrap_or("");
            Some(Project {
                id: id.to_string(),
                name: name.to_string(),
                path: path.to_string(),
            })
        })
        .collect()
}

/// The live KG projects to scope to (the same `FILE_PART_OF` projects the file
/// manager surfaces). Best-effort: an absent daemon or an out-of-scope read
/// yields no entries. Only live projects (`expired_at IS NULL`); the query is
/// static (no interpolation), so no escaping.
#[tauri::command]
async fn terminal_projects() -> Vec<Project> {
    let socket = os_sdk::runtime::socket_path("ARLEN_KNOWLEDGE_SOCKET", "knowledge.sock");
    let client = os_sdk::graph::UnixGraphClient::new(socket.to_string_lossy().into_owned());
    let cypher = "MATCH (p:Project) WHERE p.expired_at IS NULL \
                  RETURN p.id AS id, p.name AS name, p.root_path AS path LIMIT 64";
    match client.query_rows(cypher).await {
        Ok(rows) => projects_from_rows(&rows),
        Err(_) => Vec::new(),
    }
}

/// The terminal's persisted config (`~/.config/arlen/terminal.toml`). Today the
/// monospace font size - terminal-ui-plan.md §5b, the load-bearing readability
/// setting ("the text is too small"); the zoom shortcuts apply a transient delta
/// over this base, and the grid font size the cosmic-comp subsurface reads is
/// driven from it once that render lands.
#[derive(serde::Deserialize, serde::Serialize, Clone, Debug, PartialEq)]
struct TerminalConfig {
    /// Monospace font size in px for the terminal text.
    #[serde(default = "default_font_size")]
    font_size: f32,
}

/// A readable default, larger than a typical terminal's (the shipped default read
/// too small).
fn default_font_size() -> f32 {
    14.0
}

impl Default for TerminalConfig {
    fn default() -> Self {
        Self {
            font_size: default_font_size(),
        }
    }
}

/// Clamp a font size to a sane on-screen range, so a bad value can never make the
/// terminal unreadable or break layout; a non-finite value falls back to default.
fn clamp_font_size(px: f32) -> f32 {
    if px.is_finite() {
        px.clamp(6.0, 72.0)
    } else {
        default_font_size()
    }
}

/// `$XDG_CONFIG_HOME/arlen/terminal.toml` (else `~/.config/...`).
fn terminal_config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("arlen").join("terminal.toml"))
}

/// Load the config from `path`, or the default when absent / unreadable /
/// invalid - a broken config never breaks the terminal, it falls back to the
/// readable default. The stored size is clamped on read too.
fn load_config(path: &Path) -> TerminalConfig {
    match std::fs::read_to_string(path) {
        Ok(text) => {
            let mut cfg: TerminalConfig = toml::from_str(&text).unwrap_or_default();
            cfg.font_size = clamp_font_size(cfg.font_size);
            cfg
        }
        Err(_) => TerminalConfig::default(),
    }
}

/// Persist the config to `path`, creating the parent directory.
fn save_config(path: &Path, config: &TerminalConfig) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let text = toml::to_string(config).map_err(|e| e.to_string())?;
    std::fs::write(path, text).map_err(|e| e.to_string())
}

/// The terminal config the UI applies to its text (defaulting when unset).
#[tauri::command]
fn terminal_config_get() -> TerminalConfig {
    terminal_config_path()
        .map(|p| load_config(&p))
        .unwrap_or_default()
}

/// Persist a new font size (clamped to a sane range).
#[tauri::command]
fn terminal_config_set(font_size: f32) -> Result<(), String> {
    let path = terminal_config_path().ok_or("no config directory")?;
    let config = TerminalConfig {
        font_size: clamp_font_size(font_size),
    };
    save_config(&path, &config)
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
            terminal_grid,
            terminal_input,
            terminal_resize,
            terminal_new_session,
            terminal_history_search,
            terminal_projects,
            terminal_config_get,
            terminal_config_set
        ])
        .run(tauri::generate_context!())
        .expect("error while running arlen-terminal");
}

#[cfg(test)]
mod tests {
    use super::{
        clamp_font_size, default_font_size, load_config, projects_from_rows, save_config,
        shell_candidates, TerminalConfig,
    };
    use std::collections::HashMap;

    #[test]
    fn projects_map_rows_and_skip_incomplete_ones() {
        let mut full = HashMap::new();
        full.insert("id".to_string(), serde_json::json!("proj-1"));
        full.insert("name".to_string(), serde_json::json!("Arlen"));
        full.insert("path".to_string(), serde_json::json!("/home/tim/arlen"));
        // A row missing the name is skipped; a missing path defaults to empty.
        let mut no_name = HashMap::new();
        no_name.insert("id".to_string(), serde_json::json!("proj-2"));
        let mut no_path = HashMap::new();
        no_path.insert("id".to_string(), serde_json::json!("proj-3"));
        no_path.insert("name".to_string(), serde_json::json!("Loose"));

        let projects = projects_from_rows(&[full, no_name, no_path]);
        assert_eq!(projects.len(), 2);
        assert_eq!(projects[0].name, "Arlen");
        assert_eq!(projects[0].path, "/home/tim/arlen");
        assert_eq!(projects[1].name, "Loose");
        assert_eq!(projects[1].path, "");
    }

    #[test]
    fn shell_candidates_prefer_zsh_and_end_at_bin_sh() {
        let c = shell_candidates();
        assert_eq!(c.first().map(String::as_str), Some("zsh"));
        assert_eq!(c.last().map(String::as_str), Some("/bin/sh"));
        // zsh is never listed twice even when $SHELL is a zsh path.
        assert_eq!(c.iter().filter(|s| s.ends_with("zsh")).count(), 1);
    }

    #[test]
    fn clamp_keeps_sane_sizes_and_rejects_insane() {
        assert_eq!(clamp_font_size(14.0), 14.0);
        assert_eq!(clamp_font_size(1.0), 6.0);
        assert_eq!(clamp_font_size(1000.0), 72.0);
        assert_eq!(clamp_font_size(f32::NAN), default_font_size());
    }

    #[test]
    fn a_missing_config_is_the_default() {
        let cfg = load_config(std::path::Path::new("/no/such/terminal.toml"));
        assert_eq!(cfg, TerminalConfig::default());
        assert_eq!(cfg.font_size, default_font_size());
    }

    #[test]
    fn config_round_trips_and_clamps_on_read() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("terminal.toml");
        save_config(&path, &TerminalConfig { font_size: 18.0 }).unwrap();
        assert_eq!(load_config(&path).font_size, 18.0);
        // A hand-edited absurd value is clamped on read, never breaking the UI.
        std::fs::write(&path, "font_size = 9999.0").unwrap();
        assert_eq!(load_config(&path).font_size, 72.0);
    }
}

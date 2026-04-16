//! Settings search index export and CLI argument handling.

use std::path::PathBuf;
use std::sync::Mutex;

use serde::Serialize;

/// Parsed launch args. `take()` clears after first read so the
/// frontend only navigates once even if it calls the command again.
static LAUNCH_ARGS: Mutex<Option<LaunchTarget>> = Mutex::new(None);

#[derive(Debug, Clone, Serialize)]
pub struct LaunchTarget {
    pub panel: String,
    pub anchor: Option<String>,
}

/// Store the parsed CLI args so `get_launch_args` can return them.
/// Called once from `lib.rs` setup before the frontend mounts.
pub fn store_launch_args() {
    if let Some((panel, anchor)) = parse_cli_args() {
        *LAUNCH_ARGS.lock().unwrap() = Some(LaunchTarget { panel, anchor });
    }
}

/// Return the launch navigation target (if any) and clear it so
/// subsequent calls return `None`. The frontend calls this once in
/// `onMount` after all stores are initialised.
#[tauri::command]
pub fn get_launch_args() -> Option<LaunchTarget> {
    LAUNCH_ARGS.lock().unwrap().take()
}

/// Where the exported index lives so Waypointer can read it without
/// starting the Settings app.
fn index_path() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("lunaris")
        .join("settings-index.json")
}

/// Write the settings search index to disk. Called from the frontend
/// at startup with the pre-built JSON payload.
#[tauri::command]
pub fn export_settings_index(json: String) -> Result<(), String> {
    let path = index_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create dir: {e}"))?;
    }
    std::fs::write(&path, &json).map_err(|e| format!("write: {e}"))?;
    log::info!(
        "settings index exported ({} bytes) to {}",
        json.len(),
        path.display()
    );
    Ok(())
}

/// Parse CLI arguments into a navigation target. Returns `(panel, anchor)`
/// if the user passed `--panel <id>` and optionally `--section <name>`.
///
/// Called from setup() and the result is emitted as a Tauri event so
/// the frontend can navigate on mount.
pub fn parse_cli_args() -> Option<(String, Option<String>)> {
    let args: Vec<String> = std::env::args().collect();

    let mut panel: Option<String> = None;
    let mut section: Option<String> = None;

    let mut i = 1; // skip binary name
    while i < args.len() {
        match args[i].as_str() {
            "--panel" if i + 1 < args.len() => {
                panel = Some(args[i + 1].clone());
                i += 2;
            }
            "--section" | "--setting" if i + 1 < args.len() => {
                section = Some(args[i + 1].clone());
                i += 2;
            }
            _ => {
                i += 1;
            }
        }
    }

    panel.map(|p| (p, section))
}

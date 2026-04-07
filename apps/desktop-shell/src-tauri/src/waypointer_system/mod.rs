/// Waypointer plugin system (Phase 2: internal compiled plugins).
///
/// Defines the `WaypointerPlugin` trait and `PluginManager` that aggregates
/// results from all registered plugins, sorted by relevance and priority.
///
/// See `docs/architecture/waypointer-migration.md`.

mod plugin;
mod manager;
pub mod plugins;

pub use plugin::*;
pub use manager::*;

use std::sync::Mutex;

/// Tauri managed state for the plugin manager.
pub type PluginManagerState = Mutex<PluginManager>;

/// Search via the plugin manager (new Tauri command).
#[tauri::command]
pub fn waypointer_search(
    query: String,
    state: tauri::State<'_, PluginManagerState>,
) -> Vec<SearchResult> {
    let mgr = state.lock().unwrap();
    mgr.search(&query)
}

/// Execute a search result via the plugin manager.
#[tauri::command]
pub fn waypointer_execute(
    result: SearchResult,
    state: tauri::State<'_, PluginManagerState>,
) -> Result<(), String> {
    let mgr = state.lock().unwrap();
    mgr.execute(&result).map_err(|e| e.to_string())
}

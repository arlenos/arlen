/// Waypointer plugin system (Phase 2: internal compiled plugins).
///
/// Defines the `WaypointerPlugin` trait and `PluginManager` that aggregates
/// results from all registered plugins, sorted by relevance and priority.
///
/// See `docs/architecture/waypointer-migration.md`.

mod plugin;
mod manager;
pub mod plugins;
pub mod registry;

pub use plugin::*;
pub use manager::*;

use std::sync::RwLock;

/// Tauri managed state for the plugin manager.
///
/// `RwLock` rather than `Mutex`: registration happens once at startup
/// (brief `.write()`), while `search` / `execute` are called on every
/// Waypointer keystroke from multiple plugins in parallel. Previously
/// a `Mutex` serialised every lookup, so a slow plugin (e.g. Files'
/// graph round-trip) blocked all other plugins' searches. Since
/// `search_plugin` takes `&self` under the hood — `WaypointerPlugin`
/// methods are immutable — concurrent reads are always safe.
pub type PluginManagerState = RwLock<PluginManager>;

/// Search via the plugin manager (new Tauri command).
#[tauri::command]
pub fn waypointer_search(
    query: String,
    state: tauri::State<'_, PluginManagerState>,
) -> Vec<SearchResult> {
    let mgr = state.read().unwrap();
    mgr.search(&query)
}

/// Execute a search result via the plugin manager.
#[tauri::command]
pub fn waypointer_execute(
    result: SearchResult,
    state: tauri::State<'_, PluginManagerState>,
) -> Result<(), String> {
    let mgr = state.read().unwrap();
    mgr.execute(&result).map_err(|e| e.to_string())
}

/// List all currently-registered built-in plugins with their metadata.
/// The same data is written to the on-disk registry file at startup
/// (see `registry::write_registry`); this command is the in-process
/// equivalent used by the shell's own UI.
#[tauri::command]
pub fn waypointer_list_plugins(
    state: tauri::State<'_, PluginManagerState>,
) -> Vec<PluginDescriptor> {
    let mgr = state.read().unwrap();
    mgr.plugin_descriptors()
}

/// Query a single plugin by id. The Waypointer frontend uses this to
/// surface dedicated plugins (e.g. `core.power`) as their own
/// CommandGroup sections without routing through the generic search —
/// see `search_plugin` on `PluginManager` for why that matters.
#[tauri::command]
pub fn waypointer_search_plugin(
    plugin_id: String,
    query: String,
    state: tauri::State<'_, PluginManagerState>,
) -> Vec<SearchResult> {
    // DEBUG level: this fires per keystroke per registered plugin
    // group (~4 calls per key). Keeping it at info was ~80% of the
    // shell log noise during a short Waypointer session.
    log::debug!("waypointer_search_plugin: plugin_id='{plugin_id}' query='{query}'");
    let mgr = state.read().unwrap();
    let results = mgr.search_plugin(&plugin_id, &query);
    log::debug!(
        "waypointer_search_plugin: plugin_id='{plugin_id}' returned {} results",
        results.len()
    );
    results
}

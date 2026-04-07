/// App Search plugin: searches .desktop files by name/description.

use crate::waypointer_system::plugin::*;

pub struct AppSearchPlugin {
    index: Vec<AppEntry>,
}

/// Minimal app entry for search (mirrors app_index::AppEntry fields we need).
#[derive(Clone)]
struct AppEntry {
    name: String,
    desktop_entry: String,
    description: String,
    icon: Option<String>,
}

impl AppSearchPlugin {
    pub fn new() -> Self {
        Self { index: Vec::new() }
    }
}

impl WaypointerPlugin for AppSearchPlugin {
    fn id(&self) -> &str { "core.app-search" }
    fn name(&self) -> &str { "Applications" }
    fn priority(&self) -> u32 { 0 }
    fn max_results(&self) -> usize { 8 }

    fn init(&mut self) -> Result<(), PluginError> {
        // In full integration, this would load from the shared AppIndex.
        // For now, the plugin is a stub that delegates to the existing
        // app_index::search_apps Tauri command.
        Ok(())
    }

    fn search(&self, query: &str) -> Vec<SearchResult> {
        // Stub: actual search delegates to app_index::search_apps via Tauri state.
        // The PluginManager integration will pass the AppIndex state in Phase 3.
        let _ = query;
        Vec::new()
    }

    fn execute(&self, result: &SearchResult) -> Result<(), PluginError> {
        if let Action::Launch { ref desktop_entry } = result.action {
            // Delegates to app_index::launch_app.
            let _ = desktop_entry;
        }
        Ok(())
    }
}

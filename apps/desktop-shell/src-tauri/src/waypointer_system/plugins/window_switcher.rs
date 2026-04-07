/// Window switcher plugin: find and activate open windows.

use crate::waypointer_system::plugin::*;

pub struct WindowSwitcherPlugin;

impl WaypointerPlugin for WindowSwitcherPlugin {
    fn id(&self) -> &str { "core.window-switcher" }
    fn name(&self) -> &str { "Windows" }
    fn priority(&self) -> u32 { 10 }

    fn search(&self, query: &str) -> Vec<SearchResult> {
        // Stub: actual window list comes from wayland_client::WindowList
        // which is Tauri-managed state. The PluginManager integration
        // will inject the window list in Phase 3.
        let _ = query;
        Vec::new()
    }

    fn execute(&self, result: &SearchResult) -> Result<(), PluginError> {
        // Would call wayland_client::activate_window.
        let _ = result;
        Ok(())
    }
}

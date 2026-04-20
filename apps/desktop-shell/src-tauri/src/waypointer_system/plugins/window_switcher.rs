/// Window switcher plugin: find and activate open windows.

use crate::wayland_client;
use crate::waypointer_system::plugin::*;

/// Plugin that searches open windows by title/app_id.
pub struct WindowSwitcherPlugin {
    windows: wayland_client::WindowList,
}

impl WindowSwitcherPlugin {
    pub fn new(windows: wayland_client::WindowList) -> Self {
        Self { windows }
    }
}

impl WaypointerPlugin for WindowSwitcherPlugin {
    fn id(&self) -> &str { "core.window-switcher" }
    fn name(&self) -> &str { "Windows" }
    fn description(&self) -> &str { "Find and focus an open window by its title or application id." }
    fn priority(&self) -> u32 { 10 }
    fn max_results(&self) -> usize { 8 }

    fn search(&self, query: &str) -> Vec<SearchResult> {
        let query = query.trim().to_lowercase();
        if query.is_empty() {
            return Vec::new();
        }

        let windows = self.windows.lock().unwrap();
        let mut results = Vec::new();

        for win in windows.iter() {
            let title_lower = win.title.to_lowercase();
            let app_lower = win.app_id.to_lowercase();

            let relevance = if title_lower.contains(&query) {
                0.8
            } else if app_lower.contains(&query) {
                0.6
            } else {
                continue;
            };

            results.push(SearchResult {
                id: format!("win-{}", win.id),
                title: win.title.clone(),
                description: Some(win.app_id.clone()),
                icon: None,
                relevance,
                action: Action::Custom {
                    handler: "activate_window".into(),
                    data: serde_json::json!({ "id": win.id }),
                },
                plugin_id: String::new(),
            });

            if results.len() >= 8 {
                break;
            }
        }

        results
    }

    fn execute(&self, result: &SearchResult) -> Result<(), PluginError> {
        let _ = result;
        Ok(())
    }
}

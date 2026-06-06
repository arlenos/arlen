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

            // Layered relevance:
            //   1.0  title exactly equals query
            //   0.95 title starts with query
            //   0.85 title contains query
            //   0.7  app_id starts with query
            //   0.55 app_id contains query
            // Previously: binary 0.8 / 0.6, which made sort order
            // non-deterministic on ties.
            let relevance = if title_lower == query {
                1.0
            } else if title_lower.starts_with(&query) {
                0.95
            } else if title_lower.contains(&query) {
                0.85
            } else if app_lower.starts_with(&query) {
                0.7
            } else if app_lower.contains(&query) {
                0.55
            } else {
                continue;
            };

            // Use app_id as the visible title when the raw title is
            // empty. Chromium-based apps sometimes ship blank titles
            // right after window creation; without this fallback those
            // rows rendered as empty strings and were un-selectable.
            let display_title = if win.title.trim().is_empty() {
                win.app_id.clone()
            } else {
                win.title.clone()
            };

            results.push(SearchResult {
                id: format!("win-{}", win.id),
                title: display_title,
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

        // Stable secondary sort by title after relevance — the manager
        // sort is by relevance only and would otherwise leave equal-
        // relevance rows in filesystem-iteration order, which jitters
        // visibly across keystrokes.
        results.sort_by(|a, b| {
            b.relevance
                .partial_cmp(&a.relevance)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
        });

        results
    }

    fn execute(&self, result: &SearchResult) -> Result<(), PluginError> {
        let _ = result;
        Ok(())
    }
}

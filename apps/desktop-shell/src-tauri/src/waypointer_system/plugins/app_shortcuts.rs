//! `core.app-shortcuts` Waypointer plugin.
//!
//! Reads the focused app's registered shortcuts (via the
//! shared `ShortcutsState` populated by `event_bus.rs`) and
//! returns one `SearchResult` per matching shortcut.
//!
//! Per-app routing — the focused window's `app_id` selects
//! which shortcut list to render. Multi-window apps share one
//! list across all their windows (foundation §6.4 Listing 13;
//! shortcuts-api.md FA1).
//!
//! Dispatch goes through the existing
//! `dispatch_app_action` Tauri command (used by toolbar
//! clicks). The shortcut surface emits
//! `app.shortcut.action_invoked`; the source app's
//! tauri-plugin-shell consumer routes it back as a
//! `arlen://app-action` Tauri event.

use crate::app_state::ShortcutsState;
use crate::wayland_client;
use crate::waypointer_system::plugin::*;

pub struct AppShortcutsPlugin {
    windows: wayland_client::WindowList,
    shortcuts: ShortcutsState,
}

impl AppShortcutsPlugin {
    pub fn new(windows: wayland_client::WindowList, shortcuts: ShortcutsState) -> Self {
        Self { windows, shortcuts }
    }

    /// Look up the focused window's app_id, returning empty
    /// string when nothing is focused.
    fn focused_app_id(&self) -> String {
        let windows = self.windows.lock().unwrap();
        windows
            .iter()
            .find(|w| w.active)
            .map(|w| w.app_id.clone())
            .unwrap_or_default()
    }
}

impl WaypointerPlugin for AppShortcutsPlugin {
    fn id(&self) -> &str {
        "core.app-shortcuts"
    }
    fn name(&self) -> &str {
        "App Shortcuts"
    }
    fn description(&self) -> &str {
        "Quick actions registered by the focused application — \
         New Issue, Run Tests, Push, etc."
    }
    fn priority(&self) -> u32 {
        15
    }
    fn max_results(&self) -> usize {
        8
    }

    fn search(&self, query: &str) -> Vec<SearchResult> {
        let app_id = self.focused_app_id();
        if app_id.is_empty() {
            return Vec::new();
        }

        let shortcuts = self.shortcuts.lock().unwrap();
        let Some(list) = shortcuts.get(&app_id) else {
            return Vec::new();
        };

        let q = query.trim().to_lowercase();

        let mut results = Vec::new();
        for sc in list {
            if !sc.enabled {
                continue;
            }
            // Match label OR action against the query. Empty
            // query shows everything (typical "browse" mode).
            let relevance = if q.is_empty() {
                0.5
            } else {
                let label_lower = sc.label.to_lowercase();
                let action_lower = sc.action.to_lowercase();
                if label_lower == q {
                    1.0
                } else if label_lower.starts_with(&q) {
                    0.9
                } else if label_lower.contains(&q) {
                    0.8
                } else if action_lower.contains(&q) {
                    0.6
                } else {
                    continue;
                }
            };

            // Subtitle includes the optional badge for visibility,
            // and the app id for context (so multiple apps with
            // a "Save" shortcut are distinguishable).
            let subtitle = match &sc.badge {
                Some(b) => format!("{}  ·  {}  ·  [{}]", sc.action, b, app_id),
                None => format!("{}  ·  {}", sc.action, app_id),
            };

            results.push(SearchResult {
                id: format!("{}::{}", app_id, sc.action),
                title: sc.label.clone(),
                description: Some(subtitle),
                icon: Some(sc.icon.clone()),
                relevance,
                action: Action::Custom {
                    handler: "app_shortcut_invoke".into(),
                    data: serde_json::json!({
                        "appId": app_id,
                        "action": sc.action,
                        "confirm": sc.confirm,
                    }),
                },
                plugin_id: String::new(),
            });
        }

        results.sort_by(|a, b| {
            b.relevance
                .partial_cmp(&a.relevance)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        results.truncate(self.max_results());
        results
    }

    fn execute(&self, _result: &SearchResult) -> Result<(), PluginError> {
        // Dispatch happens through the dedicated Tauri command
        // `app_shortcut_invoke` (frontend invokes when the user
        // picks a result). This is a no-op success — the same
        // pattern as `quick_actions.rs::execute`.
        Ok(())
    }
}

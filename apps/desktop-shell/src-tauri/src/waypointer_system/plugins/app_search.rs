/// App Search plugin: searches .desktop files by name/description.

use std::sync::{Arc, Mutex};

use crate::app_index;
use crate::waypointer_system::plugin::*;

/// Plugin that searches the .desktop file index.
pub struct AppSearchPlugin {
    index: app_index::AppIndex,
}

impl AppSearchPlugin {
    pub fn new(index: app_index::AppIndex) -> Self {
        Self { index }
    }
}

impl WaypointerPlugin for AppSearchPlugin {
    fn id(&self) -> &str { "core.app-search" }
    fn name(&self) -> &str { "Applications" }
    fn description(&self) -> &str { "Search installed applications by name, description, or category." }
    fn priority(&self) -> u32 { 0 }
    fn max_results(&self) -> usize { 8 }

    fn search(&self, query: &str) -> Vec<SearchResult> {
        let query = query.trim().to_lowercase();
        if query.is_empty() {
            return Vec::new();
        }

        let index = self.index.lock().unwrap();
        let mut results = Vec::new();

        for entry in index.iter() {
            let name_lower = entry.name.to_lowercase();
            let desc_lower = entry.description.to_lowercase();

            let relevance = if name_lower == query {
                1.0
            } else if name_lower.starts_with(&query) {
                0.9
            } else if name_lower.contains(&query) {
                0.7
            } else if desc_lower.contains(&query) {
                0.4
            } else {
                continue;
            };

            results.push(SearchResult {
                id: format!("app-{}", entry.exec),
                title: entry.name.clone(),
                description: if entry.description.is_empty() {
                    None
                } else {
                    Some(entry.description.clone())
                },
                icon: entry.icon_data.clone().or_else(|| {
                    if entry.icon_name.is_empty() { None } else { Some(entry.icon_name.clone()) }
                }),
                relevance,
                action: Action::Launch {
                    desktop_entry: entry.exec.clone(),
                },
                plugin_id: String::new(),
            });

            if results.len() >= 8 {
                break;
            }
        }

        results.sort_by(|a, b| {
            b.relevance.partial_cmp(&a.relevance).unwrap_or(std::cmp::Ordering::Equal)
        });

        results
    }

    fn execute(&self, result: &SearchResult) -> Result<(), PluginError> {
        if let Action::Launch { ref desktop_entry } = result.action {
            std::process::Command::new("gtk-launch")
                .arg(desktop_entry)
                .env("DISPLAY", "")
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .map_err(|e| PluginError::ExecuteFailed(e.to_string()))?;
        }
        Ok(())
    }
}

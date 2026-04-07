/// Man page plugin: open man pages (prefix "#").

use crate::waypointer_system::plugin::*;

pub struct ManPlugin;

impl WaypointerPlugin for ManPlugin {
    fn id(&self) -> &str { "core.man" }
    fn name(&self) -> &str { "Man Pages" }
    fn prefix(&self) -> Option<&str> { Some("#") }
    fn priority(&self) -> u32 { 0 }

    fn search(&self, query: &str) -> Vec<SearchResult> {
        let page = query.trim();
        if page.is_empty() {
            return Vec::new();
        }

        vec![SearchResult {
            id: "man-open".into(),
            title: format!("man {page}"),
            description: Some("Open man page in terminal".into()),
            icon: Some("book-open".into()),
            relevance: 1.0,
            action: Action::Custom {
                handler: "man_page".into(),
                data: serde_json::json!({ "page": page }),
            },
            plugin_id: String::new(),
        }]
    }

    fn execute(&self, result: &SearchResult) -> Result<(), PluginError> {
        if let Action::Custom { ref data, .. } = result.action {
            if let Some(page) = data.get("page").and_then(|v| v.as_str()) {
                // Open man page in terminal. Terminal discovery delegated to
                // shell_runner in full integration.
                std::process::Command::new("sh")
                    .args(["-c", &format!("man {page}")])
                    .spawn()
                    .map_err(|e| PluginError::ExecuteFailed(e.to_string()))?;
            }
        }
        Ok(())
    }
}

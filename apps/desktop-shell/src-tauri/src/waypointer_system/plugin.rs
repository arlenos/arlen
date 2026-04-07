/// WaypointerPlugin trait and supporting types.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// A single search result from a plugin.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    /// Unique ID within this plugin's results.
    pub id: String,
    /// Primary display text.
    pub title: String,
    /// Optional secondary text.
    pub description: Option<String>,
    /// Lucide icon name or file path.
    pub icon: Option<String>,
    /// Relevance score (0.0 to 1.0). Higher = more relevant.
    pub relevance: f32,
    /// What to do when the user selects this result.
    pub action: Action,
    /// Which plugin produced this result (set by PluginManager).
    #[serde(default)]
    pub plugin_id: String,
}

/// Action to execute when a search result is selected.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Action {
    /// Launch a .desktop application.
    Launch { desktop_entry: String },
    /// Open a file or directory.
    Open { path: PathBuf },
    /// Open a URL in the default browser.
    OpenUrl { url: String },
    /// Execute a shell command.
    Execute { command: String },
    /// Copy text to clipboard.
    Copy { text: String },
    /// Plugin-defined custom action.
    Custom {
        handler: String,
        data: serde_json::Value,
    },
}

/// Plugin errors.
#[derive(Debug, Error)]
pub enum PluginError {
    #[error("init failed: {0}")]
    InitFailed(String),
    #[error("search failed: {0}")]
    SearchFailed(String),
    #[error("execute failed: {0}")]
    ExecuteFailed(String),
}

/// Trait for Waypointer search plugins.
///
/// Phase 2: Compiled into the shell binary. Phase 3 will extract these
/// into loadable modules.
pub trait WaypointerPlugin: Send + Sync {
    /// Unique plugin identifier (e.g. "core.calculator", "core.app-search").
    fn id(&self) -> &str;

    /// Human-readable name.
    fn name(&self) -> &str;

    /// Optional query prefix that activates this plugin exclusively.
    /// `None` means the plugin is always active (no prefix needed).
    fn prefix(&self) -> Option<&str> {
        None
    }

    /// Optional regex pattern that triggers this plugin.
    fn detect_pattern(&self) -> Option<&str> {
        None
    }

    /// Priority (lower = higher priority). Used to sort results from
    /// multiple plugins. System plugins use 0-9, first-party 10-99,
    /// third-party 100+.
    fn priority(&self) -> u32;

    /// Maximum number of results this plugin returns.
    fn max_results(&self) -> usize {
        8
    }

    /// Search for results matching the query.
    fn search(&self, query: &str) -> Vec<SearchResult>;

    /// Execute the action for a selected result.
    fn execute(&self, result: &SearchResult) -> Result<(), PluginError>;

    /// Called once when the plugin is registered.
    fn init(&mut self) -> Result<(), PluginError> {
        Ok(())
    }

    /// Called when the plugin is being unregistered.
    fn shutdown(&self) {}

    /// Called when a result is highlighted (for preview).
    fn on_selected(&self, _result: &SearchResult) {}
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal test plugin for verification.
    struct EchoPlugin;

    impl WaypointerPlugin for EchoPlugin {
        fn id(&self) -> &str { "test.echo" }
        fn name(&self) -> &str { "Echo" }
        fn priority(&self) -> u32 { 100 }

        fn search(&self, query: &str) -> Vec<SearchResult> {
            vec![SearchResult {
                id: "echo-1".into(),
                title: query.to_string(),
                description: Some("Echo result".into()),
                icon: None,
                relevance: 1.0,
                action: Action::Copy { text: query.into() },
                plugin_id: String::new(),
            }]
        }

        fn execute(&self, _result: &SearchResult) -> Result<(), PluginError> {
            Ok(())
        }
    }

    #[test]
    fn test_plugin_search() {
        let plugin = EchoPlugin;
        let results = plugin.search("hello");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "hello");
    }

    #[test]
    fn test_plugin_defaults() {
        let plugin = EchoPlugin;
        assert!(plugin.prefix().is_none());
        assert!(plugin.detect_pattern().is_none());
        assert_eq!(plugin.max_results(), 8);
    }

    #[test]
    fn test_action_variants() {
        let launch = Action::Launch { desktop_entry: "firefox.desktop".into() };
        let open = Action::Open { path: "/home/user/doc.pdf".into() };
        let url = Action::OpenUrl { url: "https://example.com".into() };
        let exec = Action::Execute { command: "ls -la".into() };
        let copy = Action::Copy { text: "hello".into() };
        let custom = Action::Custom {
            handler: "my_handler".into(),
            data: serde_json::json!({"key": "value"}),
        };
        // Verify serialization works for all variants.
        for action in [launch, open, url, exec, copy, custom] {
            let json = serde_json::to_string(&action).unwrap();
            assert!(!json.is_empty());
        }
    }
}

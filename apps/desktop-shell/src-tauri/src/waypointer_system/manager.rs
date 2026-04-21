/// Plugin manager: aggregates search results from all registered plugins.

use super::plugin::{PluginDescriptor, PluginError, SearchResult, WaypointerPlugin};

/// Manages registered Waypointer plugins and aggregates their results.
pub struct PluginManager {
    plugins: Vec<Box<dyn WaypointerPlugin>>,
}

impl PluginManager {
    pub fn new() -> Self {
        Self {
            plugins: Vec::new(),
        }
    }

    /// Register a plugin. Calls `init()` on it.
    pub fn register(&mut self, mut plugin: Box<dyn WaypointerPlugin>) -> Result<(), PluginError> {
        plugin.init()?;
        log::info!("waypointer: registered plugin '{}'", plugin.id());
        self.plugins.push(plugin);
        // Keep sorted by priority for deterministic ordering.
        self.plugins.sort_by_key(|p| p.priority());
        Ok(())
    }

    /// Unregister a plugin by ID. Calls `shutdown()` on it.
    pub fn unregister(&mut self, plugin_id: &str) {
        if let Some(idx) = self.plugins.iter().position(|p| p.id() == plugin_id) {
            self.plugins[idx].shutdown();
            self.plugins.remove(idx);
        }
    }

    /// Search across all plugins and return aggregated, sorted results.
    ///
    /// If the query starts with a plugin's prefix (e.g. "=" for calculator),
    /// only that plugin is queried. Otherwise all non-prefix plugins are queried.
    pub fn search(&self, query: &str) -> Vec<SearchResult> {
        let query = query.trim();
        if query.is_empty() {
            return Vec::new();
        }

        // Check if query matches a prefix-activated plugin.
        for plugin in &self.plugins {
            if let Some(prefix) = plugin.prefix() {
                if query.starts_with(prefix) {
                    let stripped = query[prefix.len()..].trim();
                    if stripped.is_empty() {
                        return Vec::new();
                    }
                    return stamp_results(plugin.as_ref(), plugin.search(stripped));
                }
            }
        }

        // No prefix match: query all non-prefix plugins.
        let mut all_results = Vec::new();
        for plugin in &self.plugins {
            if plugin.prefix().is_some() {
                continue; // Skip prefix-only plugins in general search.
            }
            let mut results = stamp_results(plugin.as_ref(), plugin.search(query));
            // Respect max_results per plugin.
            results.truncate(plugin.max_results());
            all_results.extend(results);
        }

        // Sort by: relevance (descending), then plugin priority (ascending).
        all_results.sort_by(|a, b| {
            b.relevance
                .partial_cmp(&a.relevance)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| {
                    let pa = self.plugin_priority(&a.plugin_id);
                    let pb = self.plugin_priority(&b.plugin_id);
                    pa.cmp(&pb)
                })
        });

        all_results
    }

    /// Query a **single** plugin by id, bypassing the prefix-routing
    /// logic used by `search`. Useful for UI layers that already know
    /// which plugin they want to surface as a dedicated section —
    /// e.g. the Waypointer's "Power Actions" group pulls from
    /// `core.power` on every keystroke regardless of prefix. Returns
    /// an empty vec if the plugin id isn't registered.
    ///
    /// Results are stamped with `plugin_id` and truncated to the
    /// plugin's `max_results()` cap, matching the guarantees of the
    /// generic `search` path.
    pub fn search_plugin(&self, plugin_id: &str, query: &str) -> Vec<SearchResult> {
        let query = query.trim();
        if query.is_empty() {
            return Vec::new();
        }
        let Some(plugin) = self.plugins.iter().find(|p| p.id() == plugin_id) else {
            return Vec::new();
        };
        let mut results = stamp_results(plugin.as_ref(), plugin.search(query));
        results.truncate(plugin.max_results());
        results
    }

    /// Execute the action for a result (dispatches to the owning plugin).
    pub fn execute(&self, result: &SearchResult) -> Result<(), PluginError> {
        let plugin = self
            .plugins
            .iter()
            .find(|p| p.id() == result.plugin_id)
            .ok_or_else(|| {
                PluginError::ExecuteFailed(format!("plugin '{}' not found", result.plugin_id))
            })?;
        plugin.execute(result)
    }

    /// Notify the owning plugin that a result was highlighted.
    pub fn on_selected(&self, result: &SearchResult) {
        if let Some(plugin) = self.plugins.iter().find(|p| p.id() == result.plugin_id) {
            plugin.on_selected(result);
        }
    }

    /// Number of registered plugins.
    pub fn len(&self) -> usize {
        self.plugins.len()
    }

    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }

    /// List registered plugin IDs.
    pub fn plugin_ids(&self) -> Vec<&str> {
        self.plugins.iter().map(|p| p.id()).collect()
    }

    /// Snapshot all registered plugins' metadata. Used by the registry
    /// writer and the `waypointer_list_plugins` Tauri command.
    pub fn plugin_descriptors(&self) -> Vec<PluginDescriptor> {
        self.plugins
            .iter()
            .map(|p| PluginDescriptor {
                id: p.id().to_string(),
                name: p.name().to_string(),
                description: p.description().to_string(),
                priority: p.priority(),
                prefix: p.prefix().map(String::from),
                pattern: p.detect_pattern().map(String::from),
            })
            .collect()
    }

    fn plugin_priority(&self, plugin_id: &str) -> u32 {
        self.plugins
            .iter()
            .find(|p| p.id() == plugin_id)
            .map(|p| p.priority())
            .unwrap_or(u32::MAX)
    }
}

/// Set the plugin_id on all results.
fn stamp_results(plugin: &dyn WaypointerPlugin, mut results: Vec<SearchResult>) -> Vec<SearchResult> {
    let id = plugin.id().to_string();
    for r in &mut results {
        r.plugin_id = id.clone();
    }
    results
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::waypointer_system::plugin::Action;

    /// Simple echo plugin: returns the query as a result.
    struct EchoPlugin { prio: u32 }
    impl WaypointerPlugin for EchoPlugin {
        fn id(&self) -> &str { "test.echo" }
        fn name(&self) -> &str { "Echo" }
        fn priority(&self) -> u32 { self.prio }
        fn search(&self, query: &str) -> Vec<SearchResult> {
            vec![SearchResult {
                id: "e1".into(), title: format!("Echo: {query}"),
                description: None, icon: None, relevance: 0.5,
                action: Action::Copy { text: query.into() },
                plugin_id: String::new(),
            }]
        }
        fn execute(&self, _: &SearchResult) -> Result<(), PluginError> { Ok(()) }
    }

    /// Prefix plugin: only activated with "=" prefix.
    struct CalcPlugin;
    impl WaypointerPlugin for CalcPlugin {
        fn id(&self) -> &str { "test.calc" }
        fn name(&self) -> &str { "Calc" }
        fn prefix(&self) -> Option<&str> { Some("=") }
        fn priority(&self) -> u32 { 0 }
        fn search(&self, query: &str) -> Vec<SearchResult> {
            vec![SearchResult {
                id: "c1".into(), title: format!("= {query}"),
                description: None, icon: None, relevance: 1.0,
                action: Action::Copy { text: query.into() },
                plugin_id: String::new(),
            }]
        }
        fn execute(&self, _: &SearchResult) -> Result<(), PluginError> { Ok(()) }
    }

    #[test]
    fn test_register_and_search() {
        let mut mgr = PluginManager::new();
        mgr.register(Box::new(EchoPlugin { prio: 50 })).unwrap();
        assert_eq!(mgr.len(), 1);

        let results = mgr.search("hello");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Echo: hello");
        assert_eq!(results[0].plugin_id, "test.echo");
    }

    #[test]
    fn test_prefix_plugin_exclusive() {
        let mut mgr = PluginManager::new();
        mgr.register(Box::new(EchoPlugin { prio: 50 })).unwrap();
        mgr.register(Box::new(CalcPlugin)).unwrap();

        // Query with "=" prefix: only CalcPlugin responds.
        let results = mgr.search("=2+2");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].plugin_id, "test.calc");
        assert_eq!(results[0].title, "= 2+2");
    }

    #[test]
    fn test_prefix_plugin_skipped_in_general() {
        let mut mgr = PluginManager::new();
        mgr.register(Box::new(EchoPlugin { prio: 50 })).unwrap();
        mgr.register(Box::new(CalcPlugin)).unwrap();

        // Query without prefix: CalcPlugin skipped.
        let results = mgr.search("hello");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].plugin_id, "test.echo");
    }

    #[test]
    fn test_priority_sorting() {
        let mut mgr = PluginManager::new();
        mgr.register(Box::new(EchoPlugin { prio: 100 })).unwrap();

        struct HighPrioPlugin;
        impl WaypointerPlugin for HighPrioPlugin {
            fn id(&self) -> &str { "test.high" }
            fn name(&self) -> &str { "High" }
            fn priority(&self) -> u32 { 10 }
            fn search(&self, _: &str) -> Vec<SearchResult> {
                vec![SearchResult {
                    id: "h1".into(), title: "High".into(),
                    description: None, icon: None, relevance: 0.5,
                    action: Action::Copy { text: "h".into() },
                    plugin_id: String::new(),
                }]
            }
            fn execute(&self, _: &SearchResult) -> Result<(), PluginError> { Ok(()) }
        }

        mgr.register(Box::new(HighPrioPlugin)).unwrap();

        let results = mgr.search("test");
        assert_eq!(results.len(), 2);
        // Same relevance -> sorted by priority (10 < 100).
        assert_eq!(results[0].plugin_id, "test.high");
        assert_eq!(results[1].plugin_id, "test.echo");
    }

    #[test]
    fn test_unregister() {
        let mut mgr = PluginManager::new();
        mgr.register(Box::new(EchoPlugin { prio: 50 })).unwrap();
        assert_eq!(mgr.len(), 1);

        mgr.unregister("test.echo");
        assert_eq!(mgr.len(), 0);
        assert!(mgr.search("hello").is_empty());
    }

    #[test]
    fn test_search_plugin_bypasses_prefix_routing() {
        // CalcPlugin has a "=" prefix — via `search("hello")` it's
        // skipped (prefix-only plugins aren't run in general queries).
        // Via `search_plugin("test.calc", "hello")` it MUST run anyway
        // because the caller is explicitly addressing it.
        let mut mgr = PluginManager::new();
        mgr.register(Box::new(EchoPlugin { prio: 50 })).unwrap();
        mgr.register(Box::new(CalcPlugin)).unwrap();

        let r = mgr.search_plugin("test.calc", "hello");
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].plugin_id, "test.calc");
    }

    #[test]
    fn test_search_plugin_unknown_id_empty() {
        let mgr = PluginManager::new();
        assert!(mgr.search_plugin("nope", "anything").is_empty());
    }

    #[test]
    fn test_search_plugin_empty_query_empty() {
        let mut mgr = PluginManager::new();
        mgr.register(Box::new(EchoPlugin { prio: 50 })).unwrap();
        assert!(mgr.search_plugin("test.echo", "").is_empty());
        assert!(mgr.search_plugin("test.echo", "   ").is_empty());
    }

    #[test]
    fn test_execute() {
        let mut mgr = PluginManager::new();
        mgr.register(Box::new(EchoPlugin { prio: 50 })).unwrap();

        let results = mgr.search("hello");
        assert!(mgr.execute(&results[0]).is_ok());
    }

    #[test]
    fn test_empty_query() {
        let mut mgr = PluginManager::new();
        mgr.register(Box::new(EchoPlugin { prio: 50 })).unwrap();
        assert!(mgr.search("").is_empty());
        assert!(mgr.search("   ").is_empty());
    }

    #[test]
    fn test_prefix_only_query() {
        let mut mgr = PluginManager::new();
        mgr.register(Box::new(CalcPlugin)).unwrap();
        // Just the prefix with no query: returns empty.
        assert!(mgr.search("=").is_empty());
        assert!(mgr.search("= ").is_empty());
    }

    #[test]
    fn test_plugin_ids() {
        let mut mgr = PluginManager::new();
        mgr.register(Box::new(EchoPlugin { prio: 50 })).unwrap();
        mgr.register(Box::new(CalcPlugin)).unwrap();

        let ids = mgr.plugin_ids();
        assert!(ids.contains(&"test.calc")); // priority 0
        assert!(ids.contains(&"test.echo")); // priority 50
    }
}

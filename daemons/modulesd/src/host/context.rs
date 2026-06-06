/// Per-module runtime context held by the host imports.
///
/// One `CapabilityContext` is created per loaded WASM module instance
/// and stored in its Wasmtime `Store`. Every host import consults it
/// before performing the underlying operation. Denials are returned as
/// typed errors over the WIT boundary; modules that need a capability
/// they did not declare always observe a structured failure rather
/// than a panic, so they can fall back gracefully.

use arlen_modules::ModuleCapabilities;

/// Snapshot of a module's declared capabilities, taken at load time.
/// Capabilities never expand at runtime (Foundation §625): an
/// undeclared capability cannot become available without a fresh
/// manifest review at install time.
#[derive(Debug, Clone, Default)]
pub struct CapabilityContext {
    pub module_id: String,
    pub capabilities: ModuleCapabilities,
}

impl CapabilityContext {
    pub fn new(module_id: impl Into<String>, capabilities: ModuleCapabilities) -> Self {
        Self {
            module_id: module_id.into(),
            capabilities,
        }
    }

    /// Empty context, used by tests that have no manifest.
    pub fn empty(module_id: impl Into<String>) -> Self {
        Self {
            module_id: module_id.into(),
            capabilities: ModuleCapabilities::default(),
        }
    }

    /// Is the host allowed to fetch from this URL? Decision is by host
    /// substring match (`api.exchangerate.host` matches an allowlist
    /// entry of `api.exchangerate.host`). Wildcards are not supported
    /// here: the manifest validator already rejects them.
    pub fn allow_network(&self, url: &str) -> bool {
        let Some(net) = &self.capabilities.network else {
            return false;
        };
        let Some(host) = host_from_url(url) else {
            return false;
        };
        net.allowed_domains.iter().any(|allowed| host == *allowed)
    }

    /// Is the module allowed to read graph entities of this namespace?
    /// Foundation §07: `graph.allow = ["read"]` declares broad read
    /// access; namespace-scoped entries declare narrower access. An
    /// empty `read` allowlist means no read access. The argument is the
    /// namespace prefix the call would touch (typically the entity
    /// type's namespace).
    pub fn allow_graph_read(&self, namespace: &str) -> bool {
        let Some(g) = &self.capabilities.graph else {
            return false;
        };
        prefix_match(&g.read, namespace)
    }

    pub fn allow_graph_write(&self, namespace: &str) -> bool {
        let Some(g) = &self.capabilities.graph else {
            return false;
        };
        prefix_match(&g.write, namespace)
    }

    /// Is the module allowed to publish events of this type? Match is
    /// by prefix: an allowlist entry of `focus.` allows `focus.changed`
    /// and `focus.activated` but not `window.focused`.
    pub fn allow_event_publish(&self, event_type: &str) -> bool {
        let Some(eb) = &self.capabilities.event_bus else {
            return false;
        };
        prefix_match(&eb.publish, event_type)
    }

    pub fn allow_event_subscribe(&self, event_type: &str) -> bool {
        let Some(eb) = &self.capabilities.event_bus else {
            return false;
        };
        prefix_match(&eb.subscribe, event_type)
    }

    pub fn allow_clipboard_read(&self) -> bool {
        self.capabilities
            .clipboard
            .as_ref()
            .is_some_and(|c| c.read)
    }

    pub fn allow_clipboard_write(&self) -> bool {
        self.capabilities
            .clipboard
            .as_ref()
            .is_some_and(|c| c.write)
    }

    pub fn allow_notifications(&self) -> bool {
        self.capabilities.notifications
    }
}

/// Returns true if any allowlist entry is `"*"` (wildcard, only valid
/// for first-party modules; manifest validator rejects it for
/// third-party) or is a prefix of `target`.
fn prefix_match(allowlist: &[String], target: &str) -> bool {
    allowlist
        .iter()
        .any(|entry| entry == "*" || target.starts_with(entry))
}

/// Extract the host portion of a URL without pulling in a full URL
/// parser. Sufficient because the manifest validator constrains domain
/// entries to bare hosts (no schemes, no paths).
fn host_from_url(url: &str) -> Option<&str> {
    let after_scheme = url.split_once("://")?.1;
    Some(after_scheme.split('/').next()?.split(':').next()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use arlen_modules::{
        EventBusCapability, GraphCapability, NetworkCapability,
    };

    fn ctx_with(caps: ModuleCapabilities) -> CapabilityContext {
        CapabilityContext::new("com.example.test", caps)
    }

    #[test]
    fn empty_caps_deny_everything() {
        let ctx = CapabilityContext::empty("x");
        assert!(!ctx.allow_network("https://example.com/foo"));
        assert!(!ctx.allow_graph_read("core.File"));
        assert!(!ctx.allow_graph_write("core.File"));
        assert!(!ctx.allow_event_publish("anything"));
        assert!(!ctx.allow_clipboard_read());
        assert!(!ctx.allow_notifications());
    }

    #[test]
    fn network_matches_exact_host() {
        let mut caps = ModuleCapabilities::default();
        caps.network = Some(NetworkCapability {
            allowed_domains: vec!["api.example.com".into()],
        });
        let ctx = ctx_with(caps);
        assert!(ctx.allow_network("https://api.example.com/v1/foo"));
        assert!(ctx.allow_network("http://api.example.com/"));
        assert!(!ctx.allow_network("https://api.evil.com/"));
        assert!(!ctx.allow_network("https://example.com/"));
    }

    #[test]
    fn network_rejects_port_mismatch_only_on_host_diff() {
        // Hosts with explicit ports still match because we strip ports
        // on the URL side. Manifest can't include ports anyway.
        let mut caps = ModuleCapabilities::default();
        caps.network = Some(NetworkCapability {
            allowed_domains: vec!["api.example.com".into()],
        });
        let ctx = ctx_with(caps);
        assert!(ctx.allow_network("https://api.example.com:8443/path"));
    }

    #[test]
    fn graph_prefix_match() {
        let mut caps = ModuleCapabilities::default();
        caps.graph = Some(GraphCapability {
            read: vec!["core.".into(), "shared.Person".into()],
            write: vec![],
        });
        let ctx = ctx_with(caps);
        assert!(ctx.allow_graph_read("core.File"));
        assert!(ctx.allow_graph_read("core.App"));
        assert!(ctx.allow_graph_read("shared.Person"));
        assert!(!ctx.allow_graph_read("shared.Organization"));
        assert!(!ctx.allow_graph_write("core.File"));
    }

    #[test]
    fn graph_wildcard_allows_all() {
        let mut caps = ModuleCapabilities::default();
        caps.graph = Some(GraphCapability {
            read: vec!["*".into()],
            write: vec![],
        });
        let ctx = ctx_with(caps);
        assert!(ctx.allow_graph_read("anything.at.all"));
    }

    #[test]
    fn events_prefix_publish() {
        let mut caps = ModuleCapabilities::default();
        caps.event_bus = Some(EventBusCapability {
            publish: vec!["module.example.".into()],
            subscribe: vec!["focus.".into()],
        });
        let ctx = ctx_with(caps);
        assert!(ctx.allow_event_publish("module.example.refreshed"));
        assert!(!ctx.allow_event_publish("module.other.x"));
        assert!(ctx.allow_event_subscribe("focus.activated"));
        assert!(!ctx.allow_event_subscribe("window.focused"));
    }

    #[test]
    fn host_extraction_handles_common_url_shapes() {
        assert_eq!(host_from_url("https://example.com/path"), Some("example.com"));
        assert_eq!(host_from_url("http://example.com:80"), Some("example.com"));
        assert_eq!(host_from_url("https://a.b.c.example.com/x?y=1"), Some("a.b.c.example.com"));
        assert_eq!(host_from_url("not-a-url"), None);
    }
}

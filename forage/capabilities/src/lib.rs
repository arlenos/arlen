//! Map a recipe's declared `[capabilities]` to an enforceable `PermissionProfile`.
//!
//! F-R2 first step (forage-recipes.md section 11a): a recipe declares
//! capabilities in reviewable, human terms; the runtime enforces a
//! `PermissionProfile`. This is the pure translation between the two, and it is
//! deliberately conservative. A known capability maps to the narrowest matching
//! grant; an unrecognised filesystem scope becomes a literal `custom` path
//! rather than widening to a standard directory; a graph scope without a
//! `read:`/`write:` prefix grants nothing. A capability with no profile
//! counterpart at all (`audio`, or any open `extra` category) is never silently
//! dropped: it is returned in [`CapabilityMapping::unmapped`] so the caller can
//! surface it. Nothing here widens access beyond what the recipe spelled out.
//!
//! This mapper is content-neutral: a filesystem scope of `/` or `/etc` is
//! faithfully passed through as a `custom` path, because that is what the recipe
//! literally declared. Treating such a declaration as suspect (a package asking
//! for `/`) is the job of the recipe-review and install-gate layer, not this
//! translation; the mapper must not invent reach, but it also must not hide what
//! the author asked for.

use std::path::PathBuf;

use arlen_forage_recipe::Capabilities;
use arlen_permissions::{
    AppTier, ClipboardPermissions, FilesystemPermissions, GraphPermissions, NetworkPermissions,
    NotificationPermissions, PermissionProfile, ProfileInfo,
};

/// The result of mapping recipe capabilities: the enforceable profile plus any
/// declared capability that has no profile counterpart (preserved, not dropped).
#[derive(Debug, Clone)]
pub struct CapabilityMapping {
    /// The enforceable permission profile.
    pub profile: PermissionProfile,
    /// Declared capabilities with no profile counterpart, e.g. `audio`, an
    /// unrecognised `extra` category, or a malformed graph scope. Surfaced so a
    /// declaration is never silently lost.
    pub unmapped: Vec<String>,
}

/// Map `caps` to a `PermissionProfile` for `app_id` at `tier`.
pub fn capabilities_to_profile(
    app_id: &str,
    tier: AppTier,
    caps: &Capabilities,
) -> CapabilityMapping {
    let mut unmapped = Vec::new();

    let filesystem = map_filesystem(&caps.filesystem);
    let network = map_network(&caps.network, &mut unmapped);
    let graph = map_graph(&caps.graph, &mut unmapped);

    let clipboard = if caps.clipboard {
        ClipboardPermissions {
            read: true,
            write: true,
            ..Default::default()
        }
    } else {
        ClipboardPermissions::default()
    };

    // Capabilities with no profile counterpart: preserve, do not drop.
    if caps.audio {
        unmapped.push("audio".to_string());
    }
    for key in caps.extra.keys() {
        unmapped.push(key.clone());
    }

    let profile = PermissionProfile {
        info: ProfileInfo {
            app_id: app_id.to_string(),
            tier,
        },
        graph,
        event_bus: Default::default(),
        filesystem,
        network,
        notifications: NotificationPermissions {
            enabled: caps.notifications,
        },
        clipboard,
        system: Default::default(),
        input: Default::default(),
        search: Default::default(),
        intents: Default::default(),
        mcp: Default::default(),
    };
    CapabilityMapping { profile, unmapped }
}

/// Map filesystem scope names: known XDG directory names set their flag; any
/// other string (a path, or an unrecognised name) becomes a `custom` entry, so
/// it grants only that literal path and never silently widens to a standard
/// directory.
fn map_filesystem(scopes: &[String]) -> FilesystemPermissions {
    let mut fs = FilesystemPermissions::default();
    for s in scopes {
        match s.to_ascii_lowercase().as_str() {
            "home" => fs.home = true,
            "documents" => fs.documents = true,
            "downloads" => fs.downloads = true,
            "pictures" => fs.pictures = true,
            "music" => fs.music = true,
            "videos" => fs.videos = true,
            _ => fs.custom.push(PathBuf::from(s)),
        }
    }
    fs
}

/// Map network `host:port` scopes to allowed domains. A bare `*` (or `*:*`)
/// grants all; otherwise the host (a trailing numeric `:port` stripped) is added
/// to the allow-list. A malformed entry adds a domain that simply matches
/// nothing, never `allow_all`.
fn map_network(scopes: &[String], unmapped: &mut Vec<String>) -> NetworkPermissions {
    let mut net = NetworkPermissions::default();
    for s in scopes {
        if s == "*" || s == "*:*" {
            net.allow_all = true;
            continue;
        }
        let host = match s.rsplit_once(':') {
            Some((h, port)) if !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()) => h,
            _ => s.as_str(),
        };
        if host.is_empty() {
            // A scope with no host (`:443`, ``) grants nothing; surface it rather
            // than drop it silently, matching the graph mapper's contract.
            unmapped.push(format!("network:{s}"));
        } else {
            net.allowed_domains.push(host.to_string());
        }
    }
    net
}

/// Map graph scopes `read:Type` / `write:Type` to read/write grants. A scope
/// without a recognised prefix, or with an empty type, grants nothing and is
/// recorded as unmapped rather than dropped.
fn map_graph(scopes: &[String], unmapped: &mut Vec<String>) -> GraphPermissions {
    let mut g = GraphPermissions::default();
    for s in scopes {
        if let Some(t) = s.strip_prefix("read:").filter(|t| !t.is_empty()) {
            g.read.push(t.to_string());
        } else if let Some(t) = s.strip_prefix("write:").filter(|t| !t.is_empty()) {
            g.write.push(t.to_string());
        } else {
            unmapped.push(format!("graph:{s}"));
        }
    }
    g
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caps() -> Capabilities {
        Capabilities::default()
    }

    fn map(c: &Capabilities) -> CapabilityMapping {
        capabilities_to_profile("org.example.app", AppTier::ThirdParty, c)
    }

    #[test]
    fn known_filesystem_names_set_flags_unknown_become_custom() {
        let mut c = caps();
        c.filesystem = vec![
            "home".into(),
            "Documents".into(),
            "/opt/data".into(),
            "weird".into(),
        ];
        let m = map(&c);
        assert!(m.profile.filesystem.home);
        assert!(m.profile.filesystem.documents);
        assert!(!m.profile.filesystem.downloads);
        // Path and unrecognised name are preserved as literal custom paths, not
        // widened to a standard directory.
        assert_eq!(
            m.profile.filesystem.custom,
            vec![PathBuf::from("/opt/data"), PathBuf::from("weird")]
        );
    }

    #[test]
    fn network_strips_port_and_star_means_all() {
        let mut c = caps();
        c.network = vec!["api.example.com:443".into(), "cdn.example.org".into()];
        let m = map(&c);
        assert!(!m.profile.network.allow_all);
        assert_eq!(
            m.profile.network.allowed_domains,
            vec!["api.example.com".to_string(), "cdn.example.org".to_string()]
        );

        let mut c2 = caps();
        c2.network = vec!["*".into()];
        assert!(map(&c2).profile.network.allow_all);

        // A scope with no host grants nothing and is surfaced, not dropped.
        let mut c3 = caps();
        c3.network = vec![":443".into()];
        let m3 = map(&c3);
        assert!(m3.profile.network.allowed_domains.is_empty());
        assert!(m3.unmapped.contains(&"network::443".to_string()));
    }

    #[test]
    fn graph_scopes_split_into_read_and_write() {
        let mut c = caps();
        c.graph = vec![
            "read:File".into(),
            "write:Tag".into(),
            "bogus".into(),
            "read:".into(),
        ];
        let m = map(&c);
        assert_eq!(m.profile.graph.read, vec!["File".to_string()]);
        assert_eq!(m.profile.graph.write, vec!["Tag".to_string()]);
        // A malformed scope and an empty type are surfaced, not granted.
        assert!(m.unmapped.contains(&"graph:bogus".to_string()));
        assert!(m.unmapped.contains(&"graph:read:".to_string()));
    }

    #[test]
    fn flag_capabilities_map_through() {
        let mut c = caps();
        c.notifications = true;
        c.clipboard = true;
        let m = map(&c);
        assert!(m.profile.notifications.enabled);
        assert!(m.profile.clipboard.read);
        assert!(m.profile.clipboard.write);
        // Conservative: clipboard does not imply sensitive or history access.
        assert!(!m.profile.clipboard.read_sensitive);
        assert!(!m.profile.clipboard.history);
    }

    #[test]
    fn audio_and_extra_are_preserved_as_unmapped() {
        let mut c = caps();
        c.audio = true;
        c.extra.insert("bluetooth".into(), toml::Value::Boolean(true));
        let m = map(&c);
        assert!(m.unmapped.contains(&"audio".to_string()));
        assert!(m.unmapped.contains(&"bluetooth".to_string()));
    }

    #[test]
    fn empty_capabilities_grant_nothing() {
        let m = map(&caps());
        assert!(m.unmapped.is_empty());
        assert!(!m.profile.network.allow_all);
        assert!(m.profile.network.allowed_domains.is_empty());
        assert!(m.profile.graph.read.is_empty());
        assert!(!m.profile.filesystem.home);
        assert!(!m.profile.notifications.enabled);
        assert!(!m.profile.clipboard.read);
        assert_eq!(m.profile.info.app_id, "org.example.app");
    }
}

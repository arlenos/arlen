/// Permission profile types for Lunaris OS.
///
/// Each app has a TOML profile at `/var/lib/lunaris/permissions/{uid}/{app_id}.toml`
/// defining what it can access: Knowledge Graph, Event Bus, filesystem, network, etc.
///
/// See `docs/architecture/permission-system.md`.

pub mod token;

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum PermissionError {
    #[error("profile not found for {app_id} (uid {uid})")]
    NotFound { uid: u32, app_id: String },
    #[error("IO: {0}")]
    Io(#[from] std::io::Error),
    #[error("parse: {0}")]
    Parse(String),
}

// ---------------------------------------------------------------------------
// App tier
// ---------------------------------------------------------------------------

/// Trust tier based on install location and signature.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AppTier {
    System,
    #[serde(alias = "first-party")]
    FirstParty,
    #[serde(alias = "third-party")]
    ThirdParty,
}

/// Detect tier from the executable path.
pub fn detect_tier(exe_path: &Path) -> AppTier {
    let s = exe_path.to_string_lossy();
    if s.starts_with("/usr/lib/lunaris/") || s.starts_with("/usr/bin/lunaris-") {
        AppTier::System
    } else if s.contains("/lunaris/first-party/") || s.starts_with("/usr/lib/lunaris-first-party/") {
        AppTier::FirstParty
    } else {
        AppTier::ThirdParty
    }
}

// ---------------------------------------------------------------------------
// Permission profile
// ---------------------------------------------------------------------------

/// Complete permission profile for one app.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionProfile {
    pub info: ProfileInfo,
    #[serde(default)]
    pub graph: GraphPermissions,
    #[serde(default)]
    pub event_bus: EventBusPermissions,
    #[serde(default)]
    pub filesystem: FilesystemPermissions,
    #[serde(default)]
    pub network: NetworkPermissions,
    #[serde(default)]
    pub notifications: NotificationPermissions,
    #[serde(default)]
    pub clipboard: ClipboardPermissions,
    #[serde(default)]
    pub system: SystemPermissions,
    #[serde(default)]
    pub input: InputPermissions,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileInfo {
    pub app_id: String,
    #[serde(default = "default_tier")]
    pub tier: AppTier,
}

fn default_tier() -> AppTier {
    AppTier::ThirdParty
}

// ── Graph ──

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GraphPermissions {
    #[serde(default)]
    pub read: Vec<String>,
    #[serde(default)]
    pub write: Vec<String>,
    #[serde(default)]
    pub app_isolated: bool,
}

impl GraphPermissions {
    /// Check if a pattern list matches an entity type.
    /// Patterns: `"com.app.Note"` (exact), `"com.app.*"` (namespace wildcard).
    pub fn can_read(&self, entity_type: &str) -> bool {
        pattern_matches(&self.read, entity_type)
    }

    pub fn can_write(&self, entity_type: &str) -> bool {
        pattern_matches(&self.write, entity_type)
    }
}

// ── Event Bus ──

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EventBusPermissions {
    #[serde(default)]
    pub publish: Vec<String>,
    #[serde(default)]
    pub subscribe: Vec<String>,
}

impl EventBusPermissions {
    /// Check if the app can publish to a given event type.
    pub fn can_publish(&self, event_type: &str) -> bool {
        pattern_matches(&self.publish, event_type)
    }

    /// Check if the app can subscribe to a given event type.
    pub fn can_subscribe(&self, event_type: &str) -> bool {
        pattern_matches(&self.subscribe, event_type)
    }
}

// ── Filesystem ──

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FilesystemPermissions {
    #[serde(default)]
    pub home: bool,
    #[serde(default)]
    pub documents: bool,
    #[serde(default)]
    pub downloads: bool,
    #[serde(default)]
    pub pictures: bool,
    #[serde(default)]
    pub music: bool,
    #[serde(default)]
    pub videos: bool,
    #[serde(default)]
    pub custom: Vec<PathBuf>,
}

// ── Network ──

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NetworkPermissions {
    #[serde(default)]
    pub allow_all: bool,
    #[serde(default)]
    pub allowed_domains: Vec<String>,
}

impl NetworkPermissions {
    /// Check if a domain is allowed.
    /// `api.example.com` matches `allowed_domains: ["example.com"]`.
    pub fn is_domain_allowed(&self, domain: &str) -> bool {
        if self.allow_all {
            return true;
        }
        let domain_lower = domain.to_lowercase();
        self.allowed_domains.iter().any(|allowed| {
            let allowed_lower = allowed.to_lowercase();
            domain_lower == allowed_lower
                || domain_lower.ends_with(&format!(".{allowed_lower}"))
        })
    }
}

// ── Notifications ──

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NotificationPermissions {
    #[serde(default)]
    pub enabled: bool,
}

// ── Clipboard ──

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ClipboardPermissions {
    #[serde(default)]
    pub read: bool,
    #[serde(default)]
    pub write: bool,
}

// ── System ──

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SystemPermissions {
    #[serde(default)]
    pub autostart: bool,
    #[serde(default)]
    pub background: bool,
}

// ── Input ──

/// Input subsystem permissions. Module manifests request these via
/// `[permissions].input = [...]`; the install daemon copies the
/// matching flags into the runtime profile stored under
/// `/var/lib/lunaris/permissions/`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct InputPermissions {
    /// Register keybindings that fire only while the module's own
    /// window has keyboard focus.
    #[serde(default)]
    pub register_focused_bindings: bool,
    /// Register keybindings that fire regardless of focus. Reserved
    /// for system and first-party modules; third-party modules must
    /// be granted this explicitly.
    #[serde(default)]
    pub register_global_bindings: bool,
}

impl InputPermissions {
    /// Default input permissions for a given trust tier. Third-party
    /// modules get only focused bindings; global bindings need an
    /// explicit grant.
    pub fn defaults_for_tier(tier: AppTier) -> Self {
        match tier {
            AppTier::System | AppTier::FirstParty => Self {
                register_focused_bindings: true,
                register_global_bindings: true,
            },
            AppTier::ThirdParty => Self {
                register_focused_bindings: true,
                register_global_bindings: false,
            },
        }
    }

    /// Apply a manifest-declared list of input permission strings on
    /// top of `self`. Unknown strings are ignored (forward-compat).
    pub fn apply_manifest_requests(&mut self, requests: &[String]) {
        for r in requests {
            match r.as_str() {
                "register_focused_bindings" => self.register_focused_bindings = true,
                "register_global_bindings" => self.register_global_bindings = true,
                _ => {}
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Loading
// ---------------------------------------------------------------------------

const PERMISSIONS_DIR: &str = "/var/lib/lunaris/permissions";

/// Get the profile file path for an app.
pub fn profile_path(uid: u32, app_id: &str) -> PathBuf {
    let base = std::env::var("LUNARIS_PERMISSIONS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(PERMISSIONS_DIR));
    base.join(uid.to_string()).join(format!("{app_id}.toml"))
}

/// Load a permission profile from disk.
pub fn load_profile(uid: u32, app_id: &str) -> Result<PermissionProfile, PermissionError> {
    let path = profile_path(uid, app_id);
    load_profile_from(&path, uid, app_id)
}

/// Load from an explicit path (for testing).
pub fn load_profile_from(
    path: &Path,
    uid: u32,
    app_id: &str,
) -> Result<PermissionProfile, PermissionError> {
    if !path.exists() {
        return Err(PermissionError::NotFound {
            uid,
            app_id: app_id.into(),
        });
    }
    let content = std::fs::read_to_string(path)?;
    toml::from_str(&content).map_err(|e| PermissionError::Parse(e.to_string()))
}

// ---------------------------------------------------------------------------
// Pattern matching
// ---------------------------------------------------------------------------

/// Check if any pattern in `patterns` matches `value`.
/// `"com.app.*"` matches `"com.app.Note"` and `"com.app.Deck"`.
/// `"com.app.Note"` matches only itself.
fn pattern_matches(patterns: &[String], value: &str) -> bool {
    patterns.iter().any(|p| {
        if let Some(prefix) = p.strip_suffix(".*") {
            value.starts_with(prefix) && value[prefix.len()..].starts_with('.')
        } else {
            p == value
        }
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const SAMPLE_PROFILE: &str = r#"
[info]
app_id = "com.example.notes"
tier = "third-party"

[graph]
read = ["com.example.notes.*", "shared.Person"]
write = ["com.example.notes.*"]
app_isolated = true

[event_bus]
publish = ["com.example.notes.*"]
subscribe = ["com.example.notes.*", "config.changed"]

[filesystem]
documents = true
downloads = true
custom = ["/tmp/notes"]

[network]
allowed_domains = ["api.example.com", "cdn.example.com"]

[notifications]
enabled = true

[clipboard]
read = true
write = true

[system]
autostart = false
background = true
"#;

    fn write_profile(dir: &Path, content: &str) -> PathBuf {
        let path = dir.join("com.example.notes.toml");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(content.as_bytes()).unwrap();
        path
    }

    // ── Round-trip ──

    #[test]
    fn test_roundtrip() {
        let profile: PermissionProfile = toml::from_str(SAMPLE_PROFILE).unwrap();
        assert_eq!(profile.info.app_id, "com.example.notes");
        assert_eq!(profile.info.tier, AppTier::ThirdParty);

        let serialized = toml::to_string_pretty(&profile).unwrap();
        let reparsed: PermissionProfile = toml::from_str(&serialized).unwrap();
        assert_eq!(reparsed.info.app_id, "com.example.notes");
        assert_eq!(reparsed.graph.read.len(), profile.graph.read.len());
    }

    // ── Loading ──

    #[test]
    fn test_load_from_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = write_profile(dir.path(), SAMPLE_PROFILE);
        let profile = load_profile_from(&path, 1000, "com.example.notes").unwrap();
        assert_eq!(profile.info.app_id, "com.example.notes");
        assert!(profile.graph.app_isolated);
        assert!(profile.filesystem.documents);
        assert!(!profile.filesystem.home);
    }

    #[test]
    fn test_load_not_found() {
        let result = load_profile_from(
            Path::new("/tmp/nonexistent-xyz.toml"),
            1000,
            "com.missing",
        );
        assert!(matches!(result, Err(PermissionError::NotFound { .. })));
    }

    // ── Tier detection ──

    #[test]
    fn test_detect_tier_system() {
        assert_eq!(
            detect_tier(Path::new("/usr/lib/lunaris/apps/system-monitor/bin/sm")),
            AppTier::System
        );
        assert_eq!(
            detect_tier(Path::new("/usr/bin/lunaris-graph-daemon")),
            AppTier::System
        );
    }

    #[test]
    fn test_detect_tier_third_party() {
        assert_eq!(
            detect_tier(Path::new("/home/user/.local/share/flatpak/app/com.app/bin/app")),
            AppTier::ThirdParty
        );
    }

    // ── Graph permissions ──

    #[test]
    fn test_graph_read_exact() {
        let g = GraphPermissions {
            read: vec!["shared.Person".into()],
            ..Default::default()
        };
        assert!(g.can_read("shared.Person"));
        assert!(!g.can_read("shared.Organization"));
    }

    #[test]
    fn test_graph_read_wildcard() {
        let g = GraphPermissions {
            read: vec!["com.app.*".into()],
            ..Default::default()
        };
        assert!(g.can_read("com.app.Note"));
        assert!(g.can_read("com.app.Deck"));
        assert!(!g.can_read("com.other.Note"));
    }

    #[test]
    fn test_graph_write() {
        let g = GraphPermissions {
            write: vec!["com.app.*".into()],
            ..Default::default()
        };
        assert!(g.can_write("com.app.Note"));
        assert!(!g.can_write("shared.Person"));
    }

    // ── Event Bus permissions ──

    #[test]
    fn test_event_bus_publish() {
        let e = EventBusPermissions {
            publish: vec!["com.app.*".into()],
            ..Default::default()
        };
        assert!(e.can_publish("com.app.note_created"));
        assert!(!e.can_publish("system.shutdown"));
    }

    #[test]
    fn test_event_bus_subscribe() {
        let e = EventBusPermissions {
            subscribe: vec!["com.app.*".into(), "config.changed".into()],
            ..Default::default()
        };
        assert!(e.can_subscribe("com.app.note_created"));
        assert!(e.can_subscribe("config.changed"));
        assert!(!e.can_subscribe("window.focused"));
    }

    // ── Network subdomain matching ──

    #[test]
    fn test_domain_exact() {
        let n = NetworkPermissions {
            allowed_domains: vec!["example.com".into()],
            ..Default::default()
        };
        assert!(n.is_domain_allowed("example.com"));
        assert!(!n.is_domain_allowed("other.com"));
    }

    #[test]
    fn test_domain_subdomain() {
        let n = NetworkPermissions {
            allowed_domains: vec!["example.com".into()],
            ..Default::default()
        };
        assert!(n.is_domain_allowed("api.example.com"));
        assert!(n.is_domain_allowed("cdn.api.example.com"));
        assert!(!n.is_domain_allowed("exampleX.com"));
        assert!(!n.is_domain_allowed("notexample.com"));
    }

    #[test]
    fn test_domain_case_insensitive() {
        let n = NetworkPermissions {
            allowed_domains: vec!["Example.COM".into()],
            ..Default::default()
        };
        assert!(n.is_domain_allowed("example.com"));
        assert!(n.is_domain_allowed("API.EXAMPLE.COM"));
    }

    #[test]
    fn test_domain_allow_all() {
        let n = NetworkPermissions {
            allow_all: true,
            ..Default::default()
        };
        assert!(n.is_domain_allowed("anything.com"));
    }

    // ── Defaults ──

    #[test]
    fn test_minimal_profile() {
        let minimal = r#"
[info]
app_id = "com.test"
"#;
        let profile: PermissionProfile = toml::from_str(minimal).unwrap();
        assert_eq!(profile.info.tier, AppTier::ThirdParty); // default
        assert!(!profile.graph.app_isolated);
        assert!(profile.graph.read.is_empty());
        assert!(!profile.network.allow_all);
        assert!(!profile.notifications.enabled);
    }

    // ── Pattern matching ──

    #[test]
    fn test_pattern_matches() {
        assert!(pattern_matches(&["com.app.*".into()], "com.app.Note"));
        assert!(pattern_matches(&["com.app.Note".into()], "com.app.Note"));
        assert!(!pattern_matches(&["com.app.*".into()], "com.app"));
        assert!(!pattern_matches(&["com.app.*".into()], "com.other.Note"));
        assert!(!pattern_matches(&[], "anything"));
    }

    // ── Input permissions ──

    #[test]
    fn input_permissions_parse() {
        let toml = r#"
[info]
app_id = "com.example"
[input]
register_global_bindings = true
register_focused_bindings = true
"#;
        let profile: PermissionProfile = toml::from_str(toml).unwrap();
        assert!(profile.input.register_global_bindings);
        assert!(profile.input.register_focused_bindings);
    }

    #[test]
    fn input_defaults_by_tier() {
        let third = InputPermissions::defaults_for_tier(AppTier::ThirdParty);
        assert!(third.register_focused_bindings);
        assert!(!third.register_global_bindings);

        let first = InputPermissions::defaults_for_tier(AppTier::FirstParty);
        assert!(first.register_focused_bindings);
        assert!(first.register_global_bindings);

        let system = InputPermissions::defaults_for_tier(AppTier::System);
        assert!(system.register_global_bindings);
    }

    #[test]
    fn input_apply_manifest_requests() {
        let mut p = InputPermissions::default();
        p.apply_manifest_requests(&[
            "register_focused_bindings".into(),
            "register_global_bindings".into(),
            "unknown_future_flag".into(),
        ]);
        assert!(p.register_focused_bindings);
        assert!(p.register_global_bindings);
    }

    #[test]
    fn input_section_optional() {
        // Profiles that predate the input section must still parse.
        let toml = r#"
[info]
app_id = "com.legacy"
"#;
        let profile: PermissionProfile = toml::from_str(toml).unwrap();
        assert!(!profile.input.register_focused_bindings);
        assert!(!profile.input.register_global_bindings);
    }
}

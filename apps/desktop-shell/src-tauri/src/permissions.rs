/// Permission profile reader for the Settings UI.
///
/// Reads profiles from `/var/lib/arlen/permissions/{uid}/` and exposes
/// them as Tauri commands for the frontend.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// Summary of an app's permissions for the UI list view.
#[derive(Debug, Clone, Serialize)]
pub struct AppPermissionSummary {
    pub app_id: String,
    pub tier: String,
    pub has_graph: bool,
    pub has_network: bool,
    pub has_filesystem: bool,
    pub has_notifications: bool,
    pub has_clipboard: bool,
    pub has_background: bool,
}

/// Full permission profile for the detail view.
#[derive(Debug, Clone, Serialize)]
pub struct AppPermissionDetail {
    pub app_id: String,
    pub tier: String,
    pub graph: GraphPermissions,
    pub event_bus: EventBusPermissions,
    pub filesystem: FilesystemPermissions,
    pub network: NetworkPermissions,
    pub notifications: bool,
    pub clipboard: ClipboardPermissions,
    pub system: SystemPermissions,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GraphPermissions {
    #[serde(default)]
    pub read: Vec<String>,
    #[serde(default)]
    pub write: Vec<String>,
    #[serde(default)]
    pub app_isolated: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EventBusPermissions {
    #[serde(default)]
    pub publish: Vec<String>,
    #[serde(default)]
    pub subscribe: Vec<String>,
}

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
    pub custom: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NetworkPermissions {
    #[serde(default)]
    pub allow_all: bool,
    #[serde(default)]
    pub allowed_domains: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ClipboardPermissions {
    #[serde(default)]
    pub read: bool,
    #[serde(default)]
    pub write: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SystemPermissions {
    #[serde(default)]
    pub autostart: bool,
    #[serde(default)]
    pub background: bool,
}

/// Internal profile structure matching the TOML format.
#[derive(Debug, Clone, Default, Deserialize)]
struct RawProfile {
    #[serde(default)]
    info: RawInfo,
    #[serde(default)]
    graph: Option<GraphPermissions>,
    #[serde(default)]
    event_bus: Option<EventBusPermissions>,
    #[serde(default)]
    filesystem: Option<FilesystemPermissions>,
    #[serde(default)]
    network: Option<NetworkPermissions>,
    #[serde(default)]
    notifications: Option<NotificationsSection>,
    #[serde(default)]
    clipboard: Option<ClipboardPermissions>,
    #[serde(default)]
    system: Option<SystemPermissions>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct RawInfo {
    #[serde(default)]
    app_id: String,
    #[serde(default)]
    tier: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct NotificationsSection {
    #[serde(default)]
    enabled: bool,
}


/// Load a raw profile from a TOML file.
fn load_raw_profile(path: &Path) -> Option<RawProfile> {
    let content = std::fs::read_to_string(path).ok()?;
    toml::from_str(&content).ok()
}

/// List all apps that have permission profiles for the current user, across BOTH
/// tiers.
///
/// This used to read the system tier alone, so every app installed the user-tier
/// way - which is where installd writes module profiles and where `profile_path`
/// resolves - was missing from a screen whose whole job is to say what is
/// installed and what it may do. An app you installed simply was not listed.
///
/// Where an app_id appears in both, the system-tier entry wins and the user one is
/// not shown, matching what `load_tiered` actually enforces: a root-owned profile
/// is authoritative and the `~/.config` overlay is ignored for that id. Listing
/// both would show a grant that is not the one in force.
#[tauri::command]
pub fn get_app_permissions() -> Result<Vec<AppPermissionSummary>, String> {
    let mut apps: Vec<AppPermissionSummary> = Vec::new();
    let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();

    // System first, so its entry claims the id and the user overlay is skipped.
    let dirs = [
        Some(arlen_permissions::system_permissions_dir()),
        arlen_permissions::permissions_dir(),
    ];
    for dir in dirs.into_iter().flatten() {
        if !dir.exists() {
            continue;
        }
        let entries = std::fs::read_dir(&dir).map_err(|e| e.to_string())?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map(|e| e == "toml").unwrap_or(false) {
                if let Some(profile) = load_raw_profile(&path) {
                    let app_id = if profile.info.app_id.is_empty() {
                        path.file_stem()
                            .map(|s| s.to_string_lossy().to_string())
                            .unwrap_or_default()
                    } else {
                        profile.info.app_id.clone()
                    };
                    if !seen.insert(app_id.clone()) {
                        continue; // The system tier already answered for this id.
                    }

                    apps.push(AppPermissionSummary {
                    app_id,
                    tier: if profile.info.tier.is_empty() {
                        "third-party".into()
                    } else {
                        profile.info.tier.clone()
                    },
                    has_graph: profile.graph.is_some(),
                    has_network: profile.network.as_ref().map(|n| n.allow_all || !n.allowed_domains.is_empty()).unwrap_or(false),
                    has_filesystem: profile.filesystem.as_ref().map(|f| f.home || f.documents || f.downloads || f.pictures || f.music || f.videos || !f.custom.is_empty()).unwrap_or(false),
                    has_notifications: profile.notifications.as_ref().map(|n| n.enabled).unwrap_or(false),
                    has_clipboard: profile.clipboard.as_ref().map(|c| c.read || c.write).unwrap_or(false),
                        has_background: profile.system.as_ref().map(|s| s.background).unwrap_or(false),
                    });
                }
            }
        }
    }

    apps.sort_by(|a, b| a.app_id.cmp(&b.app_id));
    Ok(apps)
}

/// Get full permission details for a specific app, from whichever tier is in
/// force.
///
/// System first and user only as a fallback, the same precedence the listing uses
/// and the same one `load_tiered` enforces. Reading the user file for an app that
/// has a system profile would show a grant the system tier overrides - a screen
/// stating permissions the app does not actually run under.
#[tauri::command]
pub fn get_app_permission_detail(app_id: String) -> Result<AppPermissionDetail, String> {
    let system = arlen_permissions::system_permissions_dir().join(format!("{app_id}.toml"));
    let profile = load_raw_profile(&system)
        .or_else(|| {
            let user = arlen_permissions::permissions_dir()?.join(format!("{app_id}.toml"));
            load_raw_profile(&user)
        })
        .ok_or_else(|| format!("no profile for {app_id}"))?;

    Ok(AppPermissionDetail {
        app_id: if profile.info.app_id.is_empty() {
            app_id
        } else {
            profile.info.app_id
        },
        tier: if profile.info.tier.is_empty() {
            "third-party".into()
        } else {
            profile.info.tier
        },
        graph: profile.graph.unwrap_or_default(),
        event_bus: profile.event_bus.unwrap_or_default(),
        filesystem: profile.filesystem.unwrap_or_default(),
        network: profile.network.unwrap_or_default(),
        notifications: profile
            .notifications
            .map(|n| n.enabled)
            .unwrap_or(false),
        clipboard: profile.clipboard.unwrap_or_default(),
        system: profile.system.unwrap_or_default(),
    })
}

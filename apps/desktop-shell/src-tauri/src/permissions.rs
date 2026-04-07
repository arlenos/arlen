/// Permission profile reader for the Settings UI.
///
/// Reads profiles from `/var/lib/lunaris/permissions/{uid}/` and exposes
/// them as Tauri commands for the frontend.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

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

/// Resolve the permissions directory.
fn permissions_dir() -> PathBuf {
    std::env::var("LUNARIS_PERMISSIONS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/var/lib/lunaris/permissions"))
}

/// Get the directory for the current user's profiles.
fn user_permissions_dir() -> PathBuf {
    let uid = unsafe { libc::getuid() };
    permissions_dir().join(uid.to_string())
}

/// Load a raw profile from a TOML file.
fn load_raw_profile(path: &Path) -> Option<RawProfile> {
    let content = std::fs::read_to_string(path).ok()?;
    toml::from_str(&content).ok()
}

/// List all apps that have permission profiles for the current user.
#[tauri::command]
pub fn get_app_permissions() -> Result<Vec<AppPermissionSummary>, String> {
    let dir = user_permissions_dir();
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut apps = Vec::new();
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

    apps.sort_by(|a, b| a.app_id.cmp(&b.app_id));
    Ok(apps)
}

/// Get full permission details for a specific app.
#[tauri::command]
pub fn get_app_permission_detail(app_id: String) -> Result<AppPermissionDetail, String> {
    let uid = unsafe { libc::getuid() };
    let path = permissions_dir()
        .join(uid.to_string())
        .join(format!("{app_id}.toml"));

    let profile =
        load_raw_profile(&path).ok_or_else(|| format!("no profile for {app_id}"))?;

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

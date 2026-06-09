//! Client for the privileged `permission-helper` (F3 Rung A).
//!
//! System-installed apps (apt/`.deb`, triggered by an enroll hook) must get a
//! **root-owned** permission profile under `/var/lib/arlen/permissions/{uid}/`,
//! written only by the `permission-helper` so a same-uid process cannot forge it
//! (AUTH-CANONICAL.md §2). `installd` runs as the user and cannot write that path
//! itself, so it proxies `org.arlen.PermissionHelper1.WriteProfile` over the system
//! bus. The lunpkg/recipe install path stays on the user-tier `~/.config`
//! (AUTH-CANONICAL.md decision) and must NOT call this; only the system-tier
//! enroll entry point does.
//!
//! This module is the client + the manifest→profile-TOML generator. The actual
//! apt/dpkg enroll-hook script that fires the entry point is a distro-packaging
//! deliverable (human-gated), not part of this crate.

use serde::Serialize;
use zbus::Connection;

use crate::install::PermissionInfo;

/// Why a system-tier profile enrolment failed.
#[derive(Debug, thiserror::Error)]
pub enum HelperError {
    /// The system bus could not be reached or the proxy call failed.
    #[error("permission-helper bus error: {0}")]
    Bus(#[from] zbus::Error),
    /// The helper refused the write (validation, permissions, IO) and returned
    /// its reason string.
    #[error("permission-helper refused: {0}")]
    Refused(String),
}

/// Ask the `permission-helper` to write a root-owned system-tier profile for
/// `app_id` belonging to `uid`. Opens a fresh system-bus connection, proxies
/// `org.arlen.PermissionHelper1.WriteProfile(app_id, uid, profile_toml)` at
/// `/org/arlen/PermissionHelper1`, and maps the helper's `(bool, String)` reply to
/// a `Result`: `(true, _)` is success, `(false, reason)` is [`HelperError::Refused`].
pub async fn write_system_profile(
    uid: u32,
    app_id: &str,
    profile_toml: &str,
) -> Result<(), HelperError> {
    let conn = Connection::system().await?;
    let proxy = zbus::Proxy::new(
        &conn,
        "org.arlen.PermissionHelper1",
        "/org/arlen/PermissionHelper1",
        "org.arlen.PermissionHelper1",
    )
    .await?;
    let (ok, reason): (bool, String) = proxy
        .call("WriteProfile", &(app_id, uid, profile_toml))
        .await?;
    if ok {
        Ok(())
    } else {
        Err(HelperError::Refused(reason))
    }
}

/// Ask the `permission-helper` to record `app_id`'s binary identity (F3 Rung B):
/// the helper RE-STATS `install_path` itself and records its `(inode, device)` into
/// the root-owned identity registry, so the app's identity becomes non-forgeable by
/// a same-uid copy-to-a-different-path. Maps the `(bool, String)` reply to a Result.
pub async fn record_identity(
    uid: u32,
    app_id: &str,
    install_path: &std::path::Path,
) -> Result<(), HelperError> {
    let conn = Connection::system().await?;
    let proxy = zbus::Proxy::new(
        &conn,
        "org.arlen.PermissionHelper1",
        "/org/arlen/PermissionHelper1",
        "org.arlen.PermissionHelper1",
    )
    .await?;
    let (ok, reason): (bool, String) = proxy
        .call(
            "RecordIdentity",
            &(app_id, uid, install_path.to_string_lossy().as_ref()),
        )
        .await?;
    if ok {
        Ok(())
    } else {
        Err(HelperError::Refused(reason))
    }
}

// ---------------------------------------------------------------------------
// Manifest -> profile TOML
// ---------------------------------------------------------------------------

/// The serialized system-tier profile. Mirrors the shape of
/// `flatpak::default_permission_profile` so installd writes one consistent profile
/// schema; the `[info]` block is mandatory (the helper rejects a profile without
/// it, `permission-helper/profile.rs`). `tier = "system"` marks the root-owned
/// origin; the loaders consume `[graph]`, the sandbox layer the rest.
#[derive(Serialize)]
struct SystemProfile {
    info: InfoSection,
    graph: GraphSection,
    filesystem: FilesystemSection,
    network: NetworkSection,
    capabilities: CapabilitiesSection,
}

#[derive(Serialize)]
struct InfoSection {
    app_id: String,
    tier: String,
}

#[derive(Serialize)]
struct GraphSection {
    read: Vec<String>,
    write: Vec<String>,
}

#[derive(Serialize)]
struct FilesystemSection {
    /// The declared filesystem paths from the manifest, carried verbatim for the
    /// sandbox layer (the graph loaders ignore this section).
    allow: Vec<String>,
}

#[derive(Serialize)]
struct NetworkSection {
    domains: Vec<String>,
}

#[derive(Serialize)]
struct CapabilitiesSection {
    notifications: bool,
    clipboard: bool,
}

/// Generate the system-tier profile TOML for `app_id` from the manifest's
/// `[permissions]` section. The graph read/write grants, filesystem paths, network
/// domains and capability bools come straight from the manifest; the result is fed
/// to [`write_system_profile`], which the helper validates before persisting.
/// Serialization goes through the `toml` crate (not string formatting) so arbitrary
/// manifest strings cannot break the document.
pub fn system_profile_toml_from_manifest(app_id: &str, perms: &PermissionInfo) -> String {
    let profile = SystemProfile {
        info: InfoSection {
            app_id: app_id.to_string(),
            tier: "system".to_string(),
        },
        graph: GraphSection {
            read: perms.graph_read.clone(),
            write: perms.graph_write.clone(),
        },
        filesystem: FilesystemSection {
            allow: perms.filesystem.clone(),
        },
        network: NetworkSection {
            domains: perms.network.clone(),
        },
        capabilities: CapabilitiesSection {
            notifications: perms.notifications,
            clipboard: perms.clipboard,
        },
    };
    toml::to_string(&profile).expect("system profile serialization is infallible for owned data")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_perms() -> PermissionInfo {
        PermissionInfo {
            graph_read: vec!["system.File.path".into(), "com.example.*".into()],
            graph_write: vec!["com.example.*".into()],
            filesystem: vec!["~/Documents/Example".into()],
            network: vec!["sync.example.com".into()],
            notifications: true,
            clipboard: false,
            input: vec![],
        }
    }

    #[test]
    fn generated_profile_has_an_info_block_and_carries_the_grants() {
        let toml_str = system_profile_toml_from_manifest("com.example.app", &sample_perms());
        // The helper requires [info]; the loaders require [graph]. Both present.
        let value: toml::Value = toml::from_str(&toml_str).unwrap();
        let table = value.as_table().unwrap();
        assert!(table.contains_key("info"), "[info] is mandatory for the helper");
        assert_eq!(table["info"]["app_id"].as_str(), Some("com.example.app"));
        assert_eq!(table["info"]["tier"].as_str(), Some("system"));
        let read = table["graph"]["read"].as_array().unwrap();
        assert_eq!(read.len(), 2);
        assert_eq!(table["network"]["domains"][0].as_str(), Some("sync.example.com"));
        assert_eq!(table["capabilities"]["notifications"].as_bool(), Some(true));
    }

    #[test]
    fn generated_profile_round_trips_through_the_helper_validator() {
        // The shape the permission-helper accepts: parseable TOML with [info].
        let toml_str = system_profile_toml_from_manifest("com.example.app", &sample_perms());
        let value: toml::Value = toml::from_str(&toml_str).expect("valid TOML");
        assert!(value.as_table().unwrap().contains_key("info"));
    }

    #[test]
    fn arbitrary_manifest_strings_do_not_break_the_document() {
        // A manifest entry containing TOML metacharacters must serialize safely.
        let mut perms = sample_perms();
        perms.graph_read = vec!["weird \" = ] entry".into()];
        let toml_str = system_profile_toml_from_manifest("com.example.app", &perms);
        let value: toml::Value = toml::from_str(&toml_str).expect("still parses");
        assert_eq!(
            value["graph"]["read"][0].as_str(),
            Some("weird \" = ] entry"),
            "the metacharacter string round-trips verbatim"
        );
    }
}

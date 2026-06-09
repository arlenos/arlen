/// D-Bus interface for the permission helper.
///
/// Interface: org.arlen.PermissionHelper1
/// Object path: /org/arlen/PermissionHelper1
///
/// Only authorized callers (installd, settings) may invoke methods.

use zbus::{interface, Connection};

use crate::profile;

/// Allowed caller binaries (resolved from /proc/{pid}/exe).
const ALLOWED_CALLERS: &[&str] = &[
    "arlen-installd",
    "arlen-settings",
    "arlen-permission-helper", // self-test
];

/// D-Bus interface implementation.
pub struct PermissionHelper;

#[interface(name = "org.arlen.PermissionHelper1")]
impl PermissionHelper {
    /// Write a permission profile for an app.
    async fn write_profile(
        &self,
        app_id: &str,
        uid: u32,
        profile_toml: &str,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] connection: &Connection,
    ) -> (bool, String) {
        // Validate caller.
        if let Err(e) = validate_caller(&header, connection, uid).await {
            return (false, e);
        }

        match profile::write_profile(uid, app_id, profile_toml) {
            Ok(path) => {
                tracing::info!("wrote profile for {app_id} (uid {uid}) at {}", path.display());
                (true, String::new())
            }
            Err(e) => {
                tracing::warn!("write_profile failed for {app_id}: {e}");
                (false, e.to_string())
            }
        }
    }

    /// Delete a permission profile for an app.
    async fn delete_profile(
        &self,
        app_id: &str,
        uid: u32,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] connection: &Connection,
    ) -> (bool, String) {
        if let Err(e) = validate_caller(&header, connection, uid).await {
            return (false, e);
        }

        match profile::delete_profile(uid, app_id) {
            Ok(()) => {
                tracing::info!("deleted profile for {app_id} (uid {uid})");
                (true, String::new())
            }
            Err(e) => {
                tracing::warn!("delete_profile failed for {app_id}: {e}");
                (false, e.to_string())
            }
        }
    }

    /// Check if a profile exists for an app.
    async fn profile_exists(&self, app_id: &str, uid: u32) -> bool {
        profile::profile_exists(uid, app_id)
    }

    /// Record an app's binary identity into the broker-owned registry (F3 Rung B).
    /// The helper RE-STATS `install_path` itself (never trusts a caller-supplied
    /// inode), so a compromised installd cannot record a lie; the registry is
    /// root-owned, so a same-uid process cannot rewrite the mapping. The caller may
    /// only record for its own uid (or, as root, any uid).
    async fn record_identity(
        &self,
        app_id: &str,
        uid: u32,
        install_path: &str,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] connection: &Connection,
    ) -> (bool, String) {
        if let Err(e) = validate_caller(&header, connection, uid).await {
            return (false, e);
        }
        match crate::identity::record_identity(uid, app_id, std::path::Path::new(install_path)) {
            Ok(path) => {
                tracing::info!("recorded identity for {app_id} (uid {uid}) at {}", path.display());
                (true, String::new())
            }
            Err(e) => {
                tracing::warn!("record_identity failed for {app_id}: {e}");
                (false, e.to_string())
            }
        }
    }
}

/// Validate that the D-Bus caller is an authorized process.
async fn validate_caller(
    header: &zbus::message::Header<'_>,
    connection: &Connection,
    target_uid: u32,
) -> Result<(), String> {
    let sender = header
        .sender()
        .ok_or_else(|| "no sender in message".to_string())?;

    // Get the caller's PID via D-Bus.
    let proxy = zbus::fdo::DBusProxy::new(connection)
        .await
        .map_err(|e| format!("DBusProxy: {e}"))?;

    let pid = proxy
        .get_connection_unix_process_id(sender.clone().into())
        .await
        .map_err(|e| format!("get PID: {e}"))?;

    // Read /proc/{pid}/exe to check the binary.
    let exe = std::fs::read_link(format!("/proc/{pid}/exe"))
        .map_err(|e| format!("read exe: {e}"))?;

    let exe_name = exe
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();

    if !ALLOWED_CALLERS.iter().any(|c| exe_name.contains(c)) {
        return Err(format!(
            "unauthorized caller: {exe_name} (pid {pid})"
        ));
    }

    // The caller may only act on its own uid's profile tree, unless it is root
    // (the apt enroll-hook). Without this an admitted caller could plant or delete
    // an authoritative profile in another user's tree on a multi-user host.
    let caller_uid = proxy
        .get_connection_unix_user(sender.clone().into())
        .await
        .map_err(|e| format!("get uid: {e}"))?;
    if caller_uid != 0 && caller_uid != target_uid {
        return Err(format!(
            "caller uid {caller_uid} may not write profiles for uid {target_uid}"
        ));
    }

    Ok(())
}

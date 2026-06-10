/// D-Bus interface for the permission helper.
///
/// Interface: org.arlen.PermissionHelper1
/// Object path: /org/arlen/PermissionHelper1
///
/// Only authorized callers (installd, settings) may invoke methods.

use arlen_permissions::identity::{app_id_from_pid, pid_start_time};
use zbus::{interface, Connection};

use crate::profile;

/// app_ids permitted to call the permission helper, resolved by the
/// anchored install-path resolver (`path_to_app_id`), never a basename
/// substring: a binary at an untrusted path resolves to a different
/// app_id (or no app_id) and is refused, so a same-uid process named
/// to contain an allowed token can no longer pass.
const ALLOWED_CALLER_APP_IDS: &[&str] = &["installd", "settings", "permission-helper"];

/// The cargo-run `dev.*` ids of the allowed callers, admitted only in
/// debug builds (exact match, not a broad `dev.` prefix). Each is
/// `dev.<bin-name>` for the corresponding allowed caller.
#[cfg(debug_assertions)]
const DEV_ALLOWED_CALLER_APP_IDS: &[&str] = &[
    "dev.arlen-installd",
    "dev.arlen-settings",
    "dev.arlen-permission-helper",
];

/// Whether a resolved caller `app_id` may invoke the helper.
fn caller_app_id_admitted(app_id: &str) -> bool {
    if ALLOWED_CALLER_APP_IDS.contains(&app_id) {
        return true;
    }
    #[cfg(debug_assertions)]
    {
        DEV_ALLOWED_CALLER_APP_IDS.contains(&app_id)
    }
    #[cfg(not(debug_assertions))]
    {
        false
    }
}

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

    // Anchored caller identity. `app_id_from_pid` reads `/proc/{pid}/exe`
    // with the openat/O_NOFOLLOW hardening (no symlink-swap TOCTOU) and
    // maps it to an app_id only through trusted install roots. A
    // start-time guard brackets the resolution so a process that exits
    // and has its pid reused mid-resolution is rejected (the kernel gives
    // the reused pid a different start tick).
    let start_before = pid_start_time(pid).map_err(|e| format!("pid start: {e}"))?;
    let app_id = app_id_from_pid(pid).map_err(|e| format!("resolve caller: {e}"))?;
    let start_after = pid_start_time(pid).map_err(|e| format!("pid start: {e}"))?;
    if start_before != start_after {
        return Err(format!("caller pid {pid} changed identity during validation"));
    }
    if !caller_app_id_admitted(&app_id) {
        return Err(format!("unauthorized caller app_id {app_id} (pid {pid})"));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admits_only_the_anchored_caller_app_ids() {
        for ok in ALLOWED_CALLER_APP_IDS {
            assert!(caller_app_id_admitted(ok), "{ok} should be admitted");
        }
        // A binary at an untrusted path resolving to a foreign or
        // unknown app_id, or a basename-spoof attempt, is refused.
        for bad in ["ai-agent", "knowledge", "com.attacker", "arlen-installd", ""] {
            assert!(!caller_app_id_admitted(bad), "{bad} must be refused");
        }
    }

    #[test]
    fn dev_ids_are_admitted_only_in_debug() {
        assert_eq!(
            caller_app_id_admitted("dev.arlen-installd"),
            cfg!(debug_assertions)
        );
        assert_eq!(
            caller_app_id_admitted("dev.arlen-settings"),
            cfg!(debug_assertions)
        );
        // An arbitrary dev crate is never admitted, even in debug.
        assert!(!caller_app_id_admitted("dev.evil"));
    }
}

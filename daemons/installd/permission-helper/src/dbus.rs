//! D-Bus interface for the permission helper.
//!
//! Interface: org.arlen.PermissionHelper1
//! Object path: /org/arlen/PermissionHelper1
//!
//! Only authorized callers (installd, settings) may invoke methods.

use zbus::{interface, Connection};

use crate::profile;

/// The callers this helper acts for, by attested app id: installd, Settings
/// and this helper's own self-test.
///
/// By id and not by file name. This list used to hold binary NAMES and the check
/// asked whether the caller's file name CONTAINED one of them, so any program a
/// user copied to `~/arlen-installd-x` was admitted by a helper that runs as
/// root. An id comes from `path_to_app_id`, which names a daemon only at its
/// root-owned install path.
const ALLOWED_CALLERS: &[&str] = &["installd", "settings", "dev.arlen.settings", "permission-helper"];

/// Whether an attested app id may call this helper.
fn is_allowed_caller(app_id: &str) -> bool {
    ALLOWED_CALLERS.contains(&app_id)
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
/// Authorise a caller, and leave a line when it is refused.
///
/// This helper runs as ROOT and writes authoritative permission profiles, so a
/// refused attempt on it is exactly what an operator reading the journal later
/// wants to find. Every rejection here returns `Err(reason)` to the caller and,
/// until 11 Aug, told nobody else - a peer probing for a way to plant a profile
/// left no line anywhere on the system.
async fn validate_caller(
    header: &zbus::message::Header<'_>,
    connection: &Connection,
    target_uid: u32,
) -> Result<(), String> {
    let r = validate_caller_inner(header, connection, target_uid).await;
    if let Err(reason) = &r {
        tracing::warn!("refused a caller: {reason}");
    }
    r
}

async fn validate_caller_inner(
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

    // Pinned first, so the pid the bus named cannot be recycled between that
    // answer and the read below. This helper is root, which is why the read
    // works here when a hardened per-user daemon's does not.
    let _pin = arlen_permissions::peer_pidfd::pidfd_open(pid)
        .ok_or_else(|| format!("caller pid {pid} is gone"))?;
    let app_id = arlen_permissions::identity::app_id_from_pid(pid)
        .map_err(|e| format!("caller pid {pid} has no attested identity: {e}"))?;

    if !is_allowed_caller(&app_id) {
        return Err(format!("unauthorized caller: {app_id} (pid {pid})"));
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
mod caller_tests {
    use super::is_allowed_caller;
    use arlen_permissions::identity::path_to_app_id;
    use std::path::Path;

    #[test]
    fn installd_at_its_install_path_is_admitted() {
        let id = path_to_app_id(Path::new("/usr/lib/arlen/libexec/arlen-installd")).unwrap();
        assert!(is_allowed_caller(&id), "{id}");
    }

    /// The hole this replaced: a file NAME that contains an allowed one.
    #[test]
    fn a_lookalike_name_in_a_home_directory_is_not() {
        for imposter in [
            "/home/u/arlen-installd",
            "/home/u/.local/bin/arlen-installd-x",
            "/tmp/my-arlen-settings",
        ] {
            let admitted = path_to_app_id(Path::new(imposter))
                .map(|id| is_allowed_caller(&id))
                .unwrap_or(false);
            assert!(!admitted, "{imposter} must not be admitted");
        }
    }
}

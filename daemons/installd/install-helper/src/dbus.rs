/// D-Bus interface for the install helper.
///
/// Interface: org.arlen.InstallHelper1
/// Object path: /org/arlen/InstallHelper1
///
/// Only arlen-installd may invoke methods. Caller identity is verified
/// via /proc/{pid}/exe.

use arlen_permissions::identity::{app_id_from_pid, pid_start_time};
use zbus::{interface, Connection};

use crate::install;

/// app_ids permitted to call the root install helper, resolved by the
/// anchored install-path resolver (`path_to_app_id`), never a basename
/// substring. The verbs here are root-privileged install/trash/signature,
/// so the spoofable basename check this replaces was an arbitrary-root-write
/// primitive for any same-uid binary named to contain an allowed token.
const ALLOWED_CALLER_APP_IDS: &[&str] = &["installd", "install-helper"];

/// The cargo-run `dev.*` ids of the allowed callers, admitted only in
/// debug builds (exact match, not a broad `dev.` prefix).
#[cfg(debug_assertions)]
const DEV_ALLOWED_CALLER_APP_IDS: &[&str] =
    &["dev.arlen-installd", "dev.arlen-install-helper"];

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
pub struct InstallHelper;

#[interface(name = "org.arlen.InstallHelper1")]
impl InstallHelper {
    /// Install an app to the system-wide location.
    ///
    /// Copies the prepared directory at `source_path` to
    /// `/usr/lib/arlen/apps/{app_id}/`. The source directory must
    /// contain the app structure (bin/, lib/, share/).
    ///
    /// Returns (success, error_message).
    async fn install_system(
        &self,
        app_id: &str,
        source_path: &str,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] connection: &Connection,
    ) -> (bool, String) {
        if let Err(e) = validate_caller(&header, connection).await {
            return (false, e);
        }

        match install::install_system(app_id, source_path) {
            Ok(path) => {
                tracing::info!("installed {app_id} at {}", path.display());
                (true, String::new())
            }
            Err(e) => {
                tracing::warn!("install_system failed for {app_id}: {e}");
                (false, e.to_string())
            }
        }
    }

    /// Uninstall a system-wide app.
    ///
    /// Removes `/usr/lib/arlen/apps/{app_id}/` and any system desktop
    /// entry for the app.
    ///
    /// Returns (success, error_message).
    async fn uninstall_system(
        &self,
        app_id: &str,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] connection: &Connection,
    ) -> (bool, String) {
        if let Err(e) = validate_caller(&header, connection).await {
            return (false, e);
        }

        match install::uninstall_system(app_id) {
            Ok(()) => {
                tracing::info!("uninstalled {app_id}");
                (true, String::new())
            }
            Err(e) => {
                tracing::warn!("uninstall_system failed for {app_id}: {e}");
                (false, e.to_string())
            }
        }
    }

    /// Write a desktop entry to /usr/share/applications/.
    ///
    /// `entry_content` must be valid desktop entry format. The file is
    /// named `{app_id}.desktop`.
    ///
    /// Returns (success, error_message).
    async fn create_desktop_entry(
        &self,
        app_id: &str,
        entry_content: &str,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] connection: &Connection,
    ) -> (bool, String) {
        if let Err(e) = validate_caller(&header, connection).await {
            return (false, e);
        }

        match install::create_desktop_entry(app_id, entry_content) {
            Ok(path) => {
                tracing::info!("desktop entry for {app_id} at {}", path.display());
                (true, String::new())
            }
            Err(e) => {
                tracing::warn!("create_desktop_entry failed for {app_id}: {e}");
                (false, e.to_string())
            }
        }
    }

    /// Check if a system-wide app is installed.
    async fn is_installed(&self, app_id: &str) -> bool {
        install::validate_app_id(app_id).is_ok() && {
            let base = std::env::var("ARLEN_SYSTEM_APPS_DIR")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|_| std::path::PathBuf::from("/usr/lib/arlen/apps"));
            base.join(app_id).exists()
        }
    }
}

/// Validate that the D-Bus caller is an authorized process.
async fn validate_caller(
    header: &zbus::message::Header<'_>,
    connection: &Connection,
) -> Result<(), String> {
    let sender = header
        .sender()
        .ok_or_else(|| "no sender in message".to_string())?;

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
    // start-time guard brackets the resolution so a pid reused
    // mid-resolution is rejected.
    let start_before = pid_start_time(pid).map_err(|e| format!("pid start: {e}"))?;
    let app_id = app_id_from_pid(pid).map_err(|e| format!("resolve caller: {e}"))?;
    let start_after = pid_start_time(pid).map_err(|e| format!("pid start: {e}"))?;
    if start_before != start_after {
        return Err(format!("caller pid {pid} changed identity during validation"));
    }
    if !caller_app_id_admitted(&app_id) {
        return Err(format!("unauthorized caller app_id {app_id} (pid {pid})"));
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
        for bad in ["settings", "ai-agent", "com.attacker", "arlen-installd", ""] {
            assert!(!caller_app_id_admitted(bad), "{bad} must be refused");
        }
    }

    #[test]
    fn dev_ids_are_admitted_only_in_debug() {
        assert_eq!(
            caller_app_id_admitted("dev.arlen-installd"),
            cfg!(debug_assertions)
        );
        assert!(!caller_app_id_admitted("dev.evil"));
    }
}

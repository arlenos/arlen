//! D-Bus interface for the install helper.
//!
//! Interface: org.arlen.InstallHelper1
//! Object path: /org/arlen/InstallHelper1
//!
//! Only arlen-installd may invoke methods. Caller identity is verified
//! by its attested app id.

use zbus::{interface, Connection};

use crate::install;

/// The callers this helper acts for, by attested app id: installd and this
/// helper's own self-test.
///
/// By id and not by file name. This list used to hold binary NAMES and the check
/// asked whether the caller's file name CONTAINED one of them, so any program a
/// user copied to `~/arlen-installd-x` was admitted by a helper that runs as
/// root. An id comes from `path_to_app_id`, which names a daemon only at its
/// root-owned install path.
const ALLOWED_CALLERS: &[&str] = &["installd", "install-helper"];

/// Whether an attested app id may call this helper.
fn is_allowed_caller(app_id: &str) -> bool {
    ALLOWED_CALLERS.contains(&app_id)
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
        // Through `install::apps_dir` rather than a second copy of it: this one
        // had the default path written out again as a literal, and now that the
        // override is debug-gated in one place a duplicate would answer a
        // different question in a release build than the install path does.
        install::validate_app_id(app_id).is_ok() && install::apps_dir().join(app_id).exists()
    }
}

/// Validate that the D-Bus caller is an authorized process.
/// Authorise a caller, and leave a line when it is refused.
///
/// This helper runs as ROOT and installs into system paths, so a refused attempt
/// on it belongs in the journal. Every rejection returns `Err(reason)` to the
/// caller and, until 11 Aug, was invisible to everyone else.
async fn validate_caller(
    header: &zbus::message::Header<'_>,
    connection: &Connection,
) -> Result<(), String> {
    let r = validate_caller_inner(header, connection).await;
    if let Err(reason) = &r {
        tracing::warn!("refused a caller: {reason}");
    }
    r
}

async fn validate_caller_inner(
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

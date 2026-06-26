//! CONN-R9 (§8.4): the per-app-grant-gated mount DECISION. Given a resolved caller
//! and the loaded accounts, decide whether the caller may mount an account's `Files`
//! drive and, if so, produce the rclone inline `fs` plus the mount point. This is the
//! pure decision the daemon's `Mount` method runs BEFORE spawning the confined
//! rclone; the spawn + `RcClient::mount` over the rc transport (arlen-run + Landlock
//! + cgroup) are the on-kernel layer on top.
//!
//! It composes the three connections cores: the [`AccessGate`] (the same capability
//! check `GetAccessToken` uses, here on the `Files` service), the account's
//! [`files_connection`](crate::config::AccountConfig::files_connection) descriptor,
//! and its [`to_connection_string`](crate::connection::SavedConnection::to_connection_string)
//! rendering with the broker secret injected only at this point (§8.1).

use std::path::{Path, PathBuf};

use crate::config::{AccountConfig, Service};
use crate::gate::{Access, AccessGate};

/// Why a mount was refused. Fail-closed: an ungranted caller, an unknown account, or
/// an account offering no `[files]` backend all yield an error, never a mount.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MountError {
    /// The caller holds no `Files` grant on this account (or the account does not
    /// exist / does not offer `Files`). The gate makes "unknown account" and "no
    /// grant" indistinguishable, so no caller can probe which accounts exist.
    Refused,
    /// The grant is held but the account declares no `[files]` mount backend, so
    /// there is nothing to mount.
    NoBackend,
}

/// A planned mount: the rclone inline `fs` (with the secret already injected) and the
/// mount point to attach it at. The daemon spawns the confined rclone and calls
/// `RcClient::mount(plan.fs, plan.mount_point)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MountPlan {
    /// The rclone inline connection string (`:backend,...:path`).
    pub fs: String,
    /// Where to attach the FUSE mount.
    pub mount_point: PathBuf,
}

/// The mount point for `id` under `runtime_dir`: `<runtime_dir>/arlen/mounts/<id>`.
/// Returns `None` when `id` is not a safe single path component (empty, `.`/`..`, or
/// containing a separator), so a malformed account id can never escape the mounts
/// directory. (The config loader already pins `id` to the file stem, so this is
/// defence in depth.)
pub fn mount_point_for(id: &str, runtime_dir: &Path) -> Option<PathBuf> {
    if id.is_empty() || id == "." || id == ".." || id.contains(['/', '\\']) {
        return None;
    }
    Some(runtime_dir.join("arlen").join("mounts").join(id))
}

/// Decide a mount for `caller_app_id` on `account_id`'s `Files` drive. Gate-checks
/// the `Files` capability (the same gate as the token handout), then renders the
/// account's `[files]` backend into the rclone `fs` with `secret` injected, and
/// resolves the mount point under `runtime_dir`. `secret` is `None` for key-file
/// auth. Fail-closed: any missing piece is a [`MountError`], never a partial plan.
pub fn plan_mount(
    accounts: &[AccountConfig],
    caller_app_id: &str,
    account_id: &str,
    runtime_dir: &Path,
    secret: Option<&str>,
) -> Result<MountPlan, MountError> {
    // The capability check first: an ungranted caller (or unknown account) is refused
    // before any descriptor is built, so a refusal leaks nothing about the account.
    match AccessGate::new(accounts).access(caller_app_id, account_id, Service::Files) {
        Access::Granted { .. } => {}
        Access::Refused => return Err(MountError::Refused),
    }
    // The gate confirmed the account exists and is granted; find it for its backend.
    let account = accounts
        .iter()
        .find(|a| a.id == account_id)
        .ok_or(MountError::Refused)?;
    let connection = account.files_connection().ok_or(MountError::NoBackend)?;
    let mount_point = mount_point_for(account_id, runtime_dir).ok_or(MountError::Refused)?;
    Ok(MountPlan {
        fs: connection.to_connection_string(secret),
        mount_point,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::parse_account;
    use std::path::Path as P;

    fn account(toml: &str, id: &str) -> AccountConfig {
        parse_account(P::new(&format!("/x/{id}.toml")), toml).unwrap()
    }

    const NAS: &str = r#"
        id = "nas"
        provider = "nextcloud"
        identity = "me@nas"
        services = ["files"]

        [[grant]]
        app_id = "org.arlen.files"
        services = ["files"]

        [files]
        backend = "sftp"
        host = "nas.local"
        user = "me"
        key_file = "/k"
    "#;

    #[test]
    fn a_granted_caller_gets_a_mount_plan() {
        let accounts = vec![account(NAS, "nas")];
        let rt = PathBuf::from("/run/user/1000");
        let plan = plan_mount(&accounts, "org.arlen.files", "nas", &rt, None).unwrap();
        assert_eq!(plan.fs, ":sftp,host=nas.local,user=me,key_file=/k:");
        assert_eq!(plan.mount_point, PathBuf::from("/run/user/1000/arlen/mounts/nas"));
    }

    #[test]
    fn an_ungranted_caller_is_refused() {
        let accounts = vec![account(NAS, "nas")];
        let rt = PathBuf::from("/run/user/1000");
        assert_eq!(
            plan_mount(&accounts, "other.app", "nas", &rt, None),
            Err(MountError::Refused)
        );
    }

    #[test]
    fn an_unknown_account_is_refused_indistinguishably() {
        let accounts = vec![account(NAS, "nas")];
        let rt = PathBuf::from("/run/user/1000");
        assert_eq!(
            plan_mount(&accounts, "org.arlen.files", "ghost", &rt, None),
            Err(MountError::Refused)
        );
    }

    #[test]
    fn a_granted_account_without_a_files_backend_has_nothing_to_mount() {
        let toml = r#"
            id = "cal"
            provider = "google"
            identity = "me@g"
            services = ["files"]

            [[grant]]
            app_id = "org.arlen.files"
            services = ["files"]
        "#;
        let accounts = vec![account(toml, "cal")];
        let rt = PathBuf::from("/run/user/1000");
        assert_eq!(
            plan_mount(&accounts, "org.arlen.files", "cal", &rt, None),
            Err(MountError::NoBackend)
        );
    }

    #[test]
    fn a_password_secret_is_injected_into_the_planned_fs() {
        let toml = r#"
            id = "dav"
            provider = "nextcloud"
            identity = "me@dav"
            services = ["files"]

            [[grant]]
            app_id = "org.arlen.files"
            services = ["files"]

            [files]
            backend = "webdav"
            url = "https://dav.example/d"
            user = "me"
        "#;
        let accounts = vec![account(toml, "dav")];
        let rt = PathBuf::from("/run/user/1000");
        let plan = plan_mount(&accounts, "org.arlen.files", "dav", &rt, Some("pw")).unwrap();
        assert!(plan.fs.contains("pass=pw"), "secret injected: {}", plan.fs);
    }

    #[test]
    fn an_unsafe_account_id_cannot_escape_the_mounts_dir() {
        let rt = PathBuf::from("/run/user/1000");
        assert!(mount_point_for("..", &rt).is_none());
        assert!(mount_point_for("a/b", &rt).is_none());
        assert!(mount_point_for("", &rt).is_none());
        assert_eq!(
            mount_point_for("ok", &rt),
            Some(PathBuf::from("/run/user/1000/arlen/mounts/ok"))
        );
    }
}

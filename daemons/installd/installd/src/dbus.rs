/// D-Bus interface for the install daemon.
///
/// Interface: org.arlen.InstallDaemon1
/// Object path: /org/arlen/InstallDaemon1
/// Bus: Session Bus

use std::sync::Arc;

use zbus::interface;

use crate::install;
use crate::jobs::{JobKind, JobQueue};

/// D-Bus interface implementation.
pub struct InstallDaemon {
    queue: Arc<JobQueue>,
}

impl InstallDaemon {
    /// Create a new daemon with the given job queue.
    pub fn new(queue: Arc<JobQueue>) -> Self {
        Self { queue }
    }
}

/// The principals allowed to change what is installed.
///
/// installd owns its name on the SESSION bus, so any same-uid process can reach
/// it, and it then presents the root `InstallHelper1` with an identity that
/// helper's allowlist admits. The helper is carefully guarded; installd was not
/// guarded at all, which made it a confused deputy on the one path that ends in
/// root install and delete.
///
/// A uid check does not close that: `enroll_system_app` can demand uid 0 because
/// root is a different principal, but every same-uid caller of the mutating
/// methods shares one uid, so uid tells them apart not at all. The question has
/// to be WHICH program is calling, which is what `app_id_from_pid` answers.
///
/// This is the interim, not the keystone. It stands at the call site the full
/// caller-identity work needs and uses the resolver that work will strengthen
/// (the inode registry today, AppArmor later), so it gets stronger where it
/// stands rather than being unpicked.
///
/// `store` is the only packaged caller. The other real client is the `forage`
/// CLI, which today resolves to `UnknownBinary` at every plausible path and is
/// not installed by any image build - so in a debug build it arrives as
/// `dev.forage` and is admitted by the dev rule below, and in a release build it
/// does not exist yet. **Packaging forage means adding its canonical path to
/// `path_to_app_id` and its id here in the same change**, or the CLI will lose
/// the ability to install.
const INSTALL_CALLERS: &[&str] = &["store"];

/// Whether a resolved caller may change what is installed.
///
/// A `dev.`-prefixed id is a cargo-run binary and is admitted in debug builds
/// only, the same allowance the audit daemon's ingest and the undo signer make so
/// a development session works without weakening the shipped gate.
fn caller_may_mutate(app_id: &str) -> bool {
    if INSTALL_CALLERS.contains(&app_id) {
        return true;
    }
    cfg!(debug_assertions) && app_id.starts_with("dev.")
}

/// Resolve the calling program's app id from its connection's pid.
///
/// Fails closed: no sender, an unreachable bus daemon, a dead pid or a binary the
/// resolver does not recognise all yield an error, and the caller refuses.
async fn caller_app_id(
    header: &zbus::message::Header<'_>,
    connection: &zbus::Connection,
) -> Result<String, String> {
    let sender = header
        .sender()
        .ok_or_else(|| "no sender in message".to_string())?;
    let proxy = zbus::fdo::DBusProxy::new(connection)
        .await
        .map_err(|e| format!("DBusProxy: {e}"))?;
    let pid = proxy
        .get_connection_unix_process_id(sender.clone().into())
        .await
        .map_err(|e| format!("get pid: {e}"))?;
    arlen_permissions::identity::app_id_from_pid(pid).map_err(|e| format!("resolve caller: {e}"))
}

/// Resolve and authorise the caller of a mutating method, logging a refusal.
///
/// Returns the app id on success so the caller can name it in its own log line.
async fn authorise_mutation(
    method: &str,
    header: &zbus::message::Header<'_>,
    connection: &zbus::Connection,
) -> Result<String, String> {
    match caller_app_id(header, connection).await {
        Ok(id) if caller_may_mutate(&id) => Ok(id),
        Ok(id) => {
            tracing::warn!("refused {method} from {id}: not an install caller");
            Err(format!("{id} may not change what is installed"))
        }
        Err(e) => {
            tracing::warn!("refused {method}: {e}");
            Err(e)
        }
    }
}

#[interface(name = "org.arlen.InstallDaemon1")]
impl InstallDaemon {
    /// Install a .lunpkg package from a local file path.
    ///
    /// Returns a job_id that can be used to track progress via signals.
    async fn install_package(
        &self,
        path: String,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> String {
        if authorise_mutation("InstallPackage", &header, connection).await.is_err() {
            // An empty job id is the refusal: there is no error channel on a
            // method whose result arrives later on a signal, and inventing one
            // would change the interface for every caller.
            return String::new();
        }
        let job_id = self.queue.enqueue(JobKind::InstallPackage { path });
        tracing::info!("enqueued install job {job_id}");
        job_id
    }

    /// Upgrade an already-installed app from a local .lunpkg.
    ///
    /// Separate from `InstallPackage`, which refuses an app that is already
    /// installed - and should, since replacing an installed app silently is how
    /// an install becomes an unreviewed update. This runs the capability gate
    /// against what the app was granted before: an update asking for nothing new
    /// applies, one that widens emits `ConsentRequired` and applies nothing.
    ///
    /// Returns a job_id; the outcome arrives on `JobCompleted`, and a widened
    /// update also emits `ConsentRequired` naming what it newly wants.
    async fn update(
        &self,
        path: String,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> String {
        if authorise_mutation("Update", &header, connection).await.is_err() {
            // An empty job id is the refusal: there is no error channel on a
            // method whose result arrives later on a signal, and inventing one
            // would change the interface for every caller.
            return String::new();
        }
        let job_id = self.queue.enqueue(JobKind::Upgrade { path });
        tracing::info!("enqueued upgrade job {job_id}");
        job_id
    }

    /// Install a Flatpak app.
    ///
    /// `remote` defaults to "flathub" if empty. Returns a job_id.
    async fn install_flatpak(
        &self,
        app_id: String,
        remote: String,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> String {
        if authorise_mutation("InstallFlatpak", &header, connection).await.is_err() {
            // An empty job id is the refusal: there is no error channel on a
            // method whose result arrives later on a signal, and inventing one
            // would change the interface for every caller.
            return String::new();
        }
        let job_id = self.queue.enqueue(JobKind::InstallFlatpak { app_id, remote });
        tracing::info!("enqueued flatpak install job {job_id}");
        job_id
    }

    /// Uninstall an app by app_id (.lunpkg source).
    ///
    /// Returns a job_id.
    async fn uninstall(
        &self,
        app_id: String,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> String {
        if authorise_mutation("Uninstall", &header, connection).await.is_err() {
            // An empty job id is the refusal: there is no error channel on a
            // method whose result arrives later on a signal, and inventing one
            // would change the interface for every caller.
            return String::new();
        }
        let job_id = self.queue.enqueue(JobKind::Uninstall { app_id });
        tracing::info!("enqueued uninstall job {job_id}");
        job_id
    }

    /// Uninstall a Flatpak app.
    ///
    /// Returns a job_id.
    async fn uninstall_flatpak(
        &self,
        app_id: String,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> String {
        if authorise_mutation("UninstallFlatpak", &header, connection).await.is_err() {
            // An empty job id is the refusal: there is no error channel on a
            // method whose result arrives later on a signal, and inventing one
            // would change the interface for every caller.
            return String::new();
        }
        let job_id = self.queue.enqueue(JobKind::UninstallFlatpak { app_id });
        tracing::info!("enqueued flatpak uninstall job {job_id}");
        job_id
    }

    /// List all installed apps (lunpkg + flatpak combined).
    ///
    /// Returns an array of (app_id, name, version, source).
    /// Source is "lunpkg", "flatpak", or "unknown".
    async fn list_installed(&self) -> Vec<(String, String, String, String)> {
        let mut apps = install::list_installed();
        apps.extend(crate::flatpak::list_installed_flatpaks());
        apps
    }

    /// What upgrading to a package would ask of the user, without installing it.
    ///
    /// Returns `(app_id, verdict, details)`. `verdict` is `silent` when the update
    /// asks for nothing new, `interruptive` when it wants something it did not
    /// have before, `unknown` when nothing was recorded for the app so no
    /// comparison is possible, and `error` when the package could not be read or
    /// verified. `details` carries the newly-requested capabilities in plain
    /// language for the first three, and the reason for the last.
    ///
    /// A read-only preview: it extracts and VERIFIES the package exactly as an
    /// install does, then throws the extraction away. Nothing is installed and no
    /// consent is recorded by asking.
    async fn preview_upgrade(&self, path: String) -> (String, String, Vec<String>) {
        match crate::consent::preview_upgrade(&path) {
            Ok(p) => match p.gate {
                crate::consent::UpgradeGate::Silent => (p.app_id, "silent".into(), Vec::new()),
                crate::consent::UpgradeGate::Interruptive { widened } => {
                    (p.app_id, "interruptive".into(), widened)
                }
                crate::consent::UpgradeGate::Unknown { requested } => {
                    (p.app_id, "unknown".into(), requested)
                }
            },
            Err(e) => (String::new(), "error".into(), vec![e.to_string()]),
        }
    }

    /// Restore a previously uninstalled app from the 30-day trash.
    ///
    /// Returns (success, error_message).
    async fn restore_app(
        &self,
        app_id: String,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> (bool, String) {
        if let Err(e) = authorise_mutation("RestoreApp", &header, connection).await {
            return (false, e);
        }
        match crate::trash::restore_app(&app_id) {
            Ok(()) => {
                tracing::info!("restored {app_id} from trash");
                (true, String::new())
            }
            Err(e) => {
                tracing::warn!("restore failed for {app_id}: {e}");
                (false, e.to_string())
            }
        }
    }

    /// List all apps in the 30-day trash.
    ///
    /// Returns an array of (app_id, app_name, app_version, deleted_at).
    async fn list_trashed(&self) -> Vec<(String, String, String, String)> {
        crate::trash::list_trashed()
            .into_iter()
            .map(|info| (info.app_id, info.app_name, info.app_version, info.deleted_at))
            .collect()
    }

    /// Run trash cleanup (remove entries older than 30 days).
    ///
    /// Called by the systemd timer and on daemon startup. Returns the
    /// number of entries permanently deleted.
    async fn cleanup_trash(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> u32 {
        // Zero deleted is the refusal. Emptying the trash is destructive and
        // permanent, so it is gated with the rest even though the timer that
        // normally drives it runs as this same user.
        if authorise_mutation("CleanupTrash", &header, connection).await.is_err() {
            return 0;
        }
        crate::trash::cleanup_trash() as u32
    }

    /// Enrol a system-installed (apt/`.deb`) app: generate its profile from the
    /// manifest at `manifest_path` and have the privileged `permission-helper` write
    /// it root-owned under `/var/lib/arlen/permissions/{uid}/` (F3 Rung A). The uid
    /// is this daemon's own (the user the app runs as). Returns (success, error).
    ///
    /// **Root-only.** The enrol entry point mints a root-owned, authoritative
    /// profile, so on the session bus uid alone cannot tell the legitimate (root)
    /// apt enroll-hook from a same-uid malware peer. The caller is therefore
    /// required to be root; a non-root peer is refused. The lunpkg path stays on the
    /// user-tier `~/.config` and does not call this. `app_id` must equal the
    /// manifest's declared id, so a caller cannot enrol a wide profile under another
    /// principal's name.
    async fn enroll_system_app(
        &self,
        app_id: String,
        manifest_path: String,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> (bool, String) {
        match caller_uid(&header, connection).await {
            Ok(0) => {}
            Ok(uid) => {
                return (false, format!("enrol requires root; caller uid {uid} refused"))
            }
            Err(e) => return (false, format!("resolve caller: {e}")),
        }
        let content = match std::fs::read_to_string(&manifest_path) {
            Ok(c) => c,
            Err(e) => return (false, format!("read manifest: {e}")),
        };
        let manifest: install::Manifest = match toml::from_str(&content) {
            Ok(m) => m,
            Err(e) => return (false, format!("parse manifest: {e}")),
        };
        if manifest.package.id != app_id {
            return (
                false,
                format!(
                    "app_id {app_id} does not match manifest id {}",
                    manifest.package.id
                ),
            );
        }
        let profile = crate::permission_helper::system_profile_toml_from_manifest(
            &app_id,
            &manifest.permissions,
        );
        // SAFETY: getuid never fails.
        let uid = unsafe { libc::getuid() };
        match crate::permission_helper::write_system_profile(uid, &app_id, &profile).await {
            Ok(()) => {
                tracing::info!("enrolled system-tier profile for {app_id} (uid {uid})");
                // The LCG projection is emitted by `write_system_profile` itself,
                // not here: /var/lib is outside the shell's profile watcher, so
                // that event is the only path into the graph and it must not be a
                // thing a caller remembers to do.
                (true, String::new())
            }
            Err(e) => {
                tracing::warn!("enroll_system_app failed for {app_id}: {e}");
                (false, e.to_string())
            }
        }
    }

    /// Get the current status of a job.
    ///
    /// Returns (progress: u8, state: String, status_message: String).
    /// State is one of: "pending", "running", "completed", "failed". ("cancelled"
    /// is defined in `JobState` but unreachable - there is no cancel method on
    /// this interface, so no job ever enters it.)
    /// Returns ("0", "unknown", "") if the job_id is not found.
    async fn get_job_status(&self, job_id: String) -> (u8, String, String) {
        self.queue
            .get_status(&job_id)
            .unwrap_or((0, "unknown".into(), String::new()))
    }

    // ── Signals ──────────────────────────────────────────────────────────

    /// Emitted when a job makes progress.
    #[zbus(signal)]
    pub async fn job_progress(
        signal_ctxt: &zbus::object_server::SignalEmitter<'_>,
        job_id: String,
        percent: u32,
        status: String,
    ) -> zbus::Result<()>;

    /// Emitted when a job completes (successfully or with error).
    #[zbus(signal)]
    pub async fn job_completed(
        signal_ctxt: &zbus::object_server::SignalEmitter<'_>,
        job_id: String,
        success: bool,
        error: String,
    ) -> zbus::Result<()>;

    /// Emitted when user consent is required for permissions.
    #[zbus(signal)]
    pub async fn consent_required(
        signal_ctxt: &zbus::object_server::SignalEmitter<'_>,
        job_id: String,
        app_id: String,
        app_name: String,
        permissions: Vec<String>,
    ) -> zbus::Result<()>;
}

/// Resolve the kernel-attested uid of a D-Bus caller from its message sender, via
/// the bus daemon's `GetConnectionUnixUser`. Used to gate the root-only enrol path.
async fn caller_uid(
    header: &zbus::message::Header<'_>,
    connection: &zbus::Connection,
) -> Result<u32, String> {
    let sender = header
        .sender()
        .ok_or_else(|| "no sender in message".to_string())?;
    let proxy = zbus::fdo::DBusProxy::new(connection)
        .await
        .map_err(|e| format!("DBusProxy: {e}"))?;
    proxy
        .get_connection_unix_user(sender.clone().into())
        .await
        .map_err(|e| format!("get uid: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_store_may_change_what_is_installed() {
        assert!(caller_may_mutate("store"));
        // The shell, the agent and any other same-uid peer are not install
        // callers, which is the whole point: uid does not tell them apart.
        assert!(!caller_may_mutate("desktop-shell"));
        assert!(!caller_may_mutate("ai-agent"));
        assert!(!caller_may_mutate("settings"));
        assert!(!caller_may_mutate(""));
    }

    #[test]
    fn a_cargo_run_binary_is_admitted_in_debug_only() {
        // A development session runs the store and forage from `target/debug`,
        // where the resolver reports `dev.<bin-name>`. The shipped gate must not
        // carry that allowance, so this asserts the build-dependent behaviour
        // rather than one arm of it.
        assert_eq!(caller_may_mutate("dev.forage"), cfg!(debug_assertions));
        assert_eq!(caller_may_mutate("dev.arlen-store"), cfg!(debug_assertions));
    }

    #[test]
    fn a_dev_prefix_is_not_a_wildcard_for_the_real_names() {
        // `dev.` is a prefix on the resolver's own dev ids, not a way to claim a
        // production principal: `developer` must not pass on a `dev` substring.
        assert!(!caller_may_mutate("developer"));
        assert!(!caller_may_mutate("notdev.store"));
    }
}

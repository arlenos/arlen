/// Job queue for serialized install/uninstall operations.
///
/// Each method on the D-Bus interface creates a Job and enqueues it.
/// A single worker task processes jobs sequentially to avoid
/// concurrent filesystem mutations.

use std::collections::HashMap;
use std::sync::Mutex;

use audit_proto::{AuditSink, LedgerAuditSink};
use serde::Serialize;
use tokio::sync::mpsc;
use uuid::Uuid;
use zbus::Connection;

use crate::audit::install_action_event;
use crate::install;

/// Job types supported by the daemon.
#[derive(Debug, Clone)]
pub enum JobKind {
    /// Install from a local .lunpkg file.
    InstallPackage { path: String },
    /// Install a Flatpak app.
    InstallFlatpak { app_id: String, remote: String },
    /// Uninstall an app by app_id (auto-detects source).
    Uninstall { app_id: String },
    /// Uninstall a Flatpak app.
    UninstallFlatpak { app_id: String },
    /// Upgrade an already-installed app from a local .lunpkg.
    ///
    /// Distinct from `InstallPackage` because install REFUSES an app that is
    /// already there (`AlreadyInstalled`) - correctly, since silently replacing
    /// an installed app is how an "install" becomes an unreviewed update. An
    /// upgrade is the reviewed form of that same write: it runs the capability
    /// gate against what the app was granted before, and only then replaces.
    Upgrade { path: String },
}

/// Current state of a job.
#[derive(Debug, Clone, Serialize)]
pub enum JobState {
    /// Queued, waiting for worker.
    Pending,
    /// Currently executing.
    Running,
    /// Finished successfully.
    Completed,
    /// Failed with error.
    Failed { error: String },
    /// Cancelled by user.
    Cancelled,
}

/// A tracked install/uninstall job.
#[derive(Debug, Clone)]
pub struct Job {
    pub id: String,
    pub kind: JobKind,
    pub state: JobState,
    pub progress: u8,
    pub status: String,
}

/// Serialized job queue.
///
/// Jobs are submitted via `enqueue()` and processed by `run_worker()`.
pub struct JobQueue {
    sender: mpsc::UnboundedSender<Job>,
    receiver: Mutex<Option<mpsc::UnboundedReceiver<Job>>>,
    /// Tracks job state for GetJobStatus.
    pub jobs: Mutex<HashMap<String, Job>>,
}

impl JobQueue {
    /// Create a new job queue.
    pub fn new() -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self {
            sender: tx,
            receiver: Mutex::new(Some(rx)),
            jobs: Mutex::new(HashMap::new()),
        }
    }

    /// Take the receiver (called once by the worker).
    pub fn take_receiver(&self) -> Option<mpsc::UnboundedReceiver<Job>> {
        self.receiver.lock().unwrap().take()
    }

    /// Enqueue a new job. Returns the job ID.
    pub fn enqueue(&self, kind: JobKind) -> String {
        let id = Uuid::new_v4().to_string();
        let job = Job {
            id: id.clone(),
            kind,
            state: JobState::Pending,
            progress: 0,
            status: "queued".into(),
        };
        self.jobs.lock().unwrap().insert(id.clone(), job.clone());
        let _ = self.sender.send(job);
        id
    }

    /// Update a job's progress and status.
    pub fn update_progress(&self, job_id: &str, progress: u8, status: &str) {
        if let Some(job) = self.jobs.lock().unwrap().get_mut(job_id) {
            job.progress = progress;
            job.status = status.into();
            job.state = JobState::Running;
        }
    }

    /// Mark a job as completed.
    pub fn mark_completed(&self, job_id: &str) {
        if let Some(job) = self.jobs.lock().unwrap().get_mut(job_id) {
            job.state = JobState::Completed;
            job.progress = 100;
            job.status = "complete".into();
        }
    }

    /// Mark a job as failed.
    pub fn mark_failed(&self, job_id: &str, error: &str) {
        if let Some(job) = self.jobs.lock().unwrap().get_mut(job_id) {
            job.state = JobState::Failed {
                error: error.into(),
            };
            job.status = "failed".into();
        }
    }

    /// Get the current state of a job.
    pub fn get_status(&self, job_id: &str) -> Option<(u8, String, String)> {
        let jobs = self.jobs.lock().unwrap();
        let job = jobs.get(job_id)?;
        let state_str = match &job.state {
            JobState::Pending => "pending",
            JobState::Running => "running",
            JobState::Completed => "completed",
            JobState::Failed { .. } => "failed",
            JobState::Cancelled => "cancelled",
        };
        Some((job.progress, state_str.to_string(), job.status.clone()))
    }
}

/// Emit the ConsentRequired signal: this update wants something the installed
/// version was not granted, so a surface has to ask before it can proceed.
async fn emit_consent_required(
    conn: &Connection,
    job_id: &str,
    app_id: &str,
    app_name: &str,
    permissions: Vec<String>,
) {
    let iface_ref = conn
        .object_server()
        .interface::<_, crate::dbus::InstallDaemon>("/org/arlen/InstallDaemon1")
        .await;
    if let Ok(iface) = iface_ref {
        let ctx = iface.signal_emitter();
        let _ = crate::dbus::InstallDaemon::consent_required(
            ctx,
            job_id.to_string(),
            app_id.to_string(),
            app_name.to_string(),
            permissions,
        )
        .await;
    }
}

/// Upgrade an installed app, gated on what it newly asks for.
///
/// The order is the point: verify, read the manifest, and run the capability
/// gate against the install lock BEFORE anything on disk is replaced. An update
/// that asks for nothing new applies silently (a prompt on every update is how
/// people learn to click through prompts); one that widens stops here, emits
/// `ConsentRequired`, and changes nothing.
///
/// Stopping is deliberate rather than a gap. There is no trusted channel yet for
/// a surface to answer that signal - the system consent dialog is a separate
/// strand - and the alternative, letting the caller assert its own consent, would
/// mean any caller could wave through exactly the widening the gate exists to
/// catch. Refusing until the answer path exists is the safe half, and
/// `forage update` already covers the interactive case on the terminal.
async fn run_upgrade(
    queue: &JobQueue,
    conn: &Connection,
    job_id: &str,
    path: &str,
) -> Result<(), install::InstallError> {
    queue.update_progress(job_id, 10, "checking the update");
    emit_progress(conn, job_id, 10, "checking the update").await;

    // Verifies structure + signature before reading any claim from the package.
    let (app_id, gate) = crate::consent::preview_upgrade(path)?;

    match gate {
        crate::consent::UpgradeGate::Silent => {}
        crate::consent::UpgradeGate::Interruptive { widened }
        | crate::consent::UpgradeGate::Unknown { requested: widened } => {
            queue.update_progress(job_id, 15, "waiting for your approval");
            emit_consent_required(conn, job_id, &app_id, &app_id, widened.clone()).await;
            return Err(install::InstallError::ConsentRequired(widened.join(", ")));
        }
    }

    // Nothing new is being asked for, so replace in place: stage the installed
    // version out of the way, then run the ordinary install path.
    queue.update_progress(job_id, 20, "replacing the installed version");
    emit_progress(conn, job_id, 20, "replacing the installed version").await;
    crate::trash::stage_for_deletion(&app_id)
        .map_err(|e| install::InstallError::TrashFailed(e.to_string()))?;

    run_install_package(queue, conn, job_id, path).await
}

/// Emit a JobProgress D-Bus signal.
async fn emit_progress(conn: &Connection, job_id: &str, percent: u8, status: &str) {
    let iface_ref = conn
        .object_server()
        .interface::<_, crate::dbus::InstallDaemon>("/org/arlen/InstallDaemon1")
        .await;
    if let Ok(iface) = iface_ref {
        let ctx = iface.signal_emitter();
        let _ = crate::dbus::InstallDaemon::job_progress(
            ctx,
            job_id.to_string(),
            percent as u32,
            status.to_string(),
        )
        .await;
    }
}

/// Emit a JobCompleted D-Bus signal.
async fn emit_completed(conn: &Connection, job_id: &str, success: bool, error: &str) {
    let iface_ref = conn
        .object_server()
        .interface::<_, crate::dbus::InstallDaemon>("/org/arlen/InstallDaemon1")
        .await;
    if let Ok(iface) = iface_ref {
        let ctx = iface.signal_emitter();
        let _ = crate::dbus::InstallDaemon::job_completed(
            ctx,
            job_id.to_string(),
            success,
            error.to_string(),
        )
        .await;
    }
}

/// Worker loop: processes jobs sequentially.
pub async fn run_worker(queue: std::sync::Arc<JobQueue>, conn: Connection) {
    let Some(mut rx) = queue.take_receiver() else {
        tracing::error!("job worker: receiver already taken");
        return;
    };

    tracing::info!("job worker started");

    // Content-free GAP-2 audit sink. Best-effort: building it is just storing
    // the ingest socket path, and each submit is off the critical path.
    let audit = LedgerAuditSink::at_default_socket();

    while let Some(job) = rx.recv().await {
        let job_id = job.id.clone();
        tracing::info!("job {job_id}: starting {:?}", job.kind);

        queue.update_progress(&job_id, 5, "starting");
        emit_progress(&conn, &job_id, 5, "starting").await;

        let result = match job.kind {
            JobKind::InstallPackage { ref path } => {
                run_install_package(&queue, &conn, &job_id, path).await
            }
            JobKind::InstallFlatpak {
                ref app_id,
                ref remote,
            } => run_install_flatpak(&queue, &conn, &job_id, app_id, remote).await,
            JobKind::Uninstall { ref app_id } => {
                run_uninstall(&queue, &conn, &job_id, app_id).await
            }
            JobKind::UninstallFlatpak { ref app_id } => {
                run_uninstall_flatpak(&queue, &conn, &job_id, app_id).await
            }
            JobKind::Upgrade { ref path } => run_upgrade(&queue, &conn, &job_id, path).await,
        };

        // Content-free GAP-2 audit of the completed action (action, source and
        // outcome — never a filesystem path or package contents). Captured here
        // before `result` is consumed by the match below; `job.kind` is matched
        // by reference so it stays available. Best-effort: the action has
        // already been applied, so a down ledger logs rather than failing it.
        let outcome = if result.is_ok() { "ok" } else { "failed" };
        let (action, source, subject_id) = match &job.kind {
            JobKind::InstallPackage { .. } => ("install", "lunpkg", None),
            JobKind::InstallFlatpak { app_id, .. } => ("install", "flatpak", Some(app_id.as_str())),
            JobKind::Uninstall { app_id } => ("uninstall", "lunpkg", Some(app_id.as_str())),
            JobKind::UninstallFlatpak { app_id } => {
                ("uninstall", "flatpak", Some(app_id.as_str()))
            }
            JobKind::Upgrade { .. } => ("upgrade", "lunpkg", None),
        };
        if let Err(e) = audit
            .submit(install_action_event(action, source, subject_id, outcome))
            .await
        {
            tracing::debug!("install audit submit failed: {e}");
        }

        match result {
            Ok(()) => {
                queue.mark_completed(&job_id);
                emit_completed(&conn, &job_id, true, "").await;
                tracing::info!("job {job_id}: completed");
            }
            Err(e) => {
                let msg = e.to_string();
                queue.mark_failed(&job_id, &msg);
                emit_completed(&conn, &job_id, false, &msg).await;
                tracing::warn!("job {job_id}: failed: {msg}");
            }
        }
    }
}

/// Execute a .lunpkg install job with transactional rollback.
///
/// If any step after extraction fails, the `InstallTransaction` Drop
/// impl rolls back all completed steps automatically.
async fn run_install_package(
    queue: &JobQueue,
    conn: &Connection,
    job_id: &str,
    path: &str,
) -> Result<(), install::InstallError> {
    use crate::transaction::InstallTransaction;

    // 1. Extract.
    queue.update_progress(job_id, 10, "extracting package");
    emit_progress(conn, job_id, 10, "extracting package").await;
    let temp_dir = install::extract_package(path)?;

    // 2. Validate package structure (signature.sig present).
    queue.update_progress(job_id, 15, "validating structure");
    emit_progress(conn, job_id, 15, "validating structure").await;
    install::validate_package_structure(&temp_dir)?;

    // 3. Verify Ed25519 signature.
    queue.update_progress(job_id, 18, "verifying signature");
    emit_progress(conn, job_id, 18, "verifying signature").await;
    crate::signature::verify_signature(&temp_dir).map_err(|e| {
        install::InstallError::SignatureVerificationFailed(e.to_string())
    })?;

    // 4. Load and validate manifest.
    queue.update_progress(job_id, 22, "reading manifest");
    emit_progress(conn, job_id, 22, "reading manifest").await;
    let manifest = install::load_manifest(&temp_dir)?;

    queue.update_progress(job_id, 25, "validating manifest");
    emit_progress(conn, job_id, 25, "validating manifest").await;
    install::validate_manifest(&manifest)?;

    // Capture the identity inputs before the manifest moves into the transaction.
    let app_id = manifest.package.id.clone();
    let binary_rel = manifest.binary.path.clone();
    // Capture the lock inputs here too: the manifest moves into the transaction
    // below, and after an upgrade the on-disk one is the NEW version, so this is
    // the last moment the old side of a future diff can be recorded.
    let lock_version = manifest.package.version.clone();
    let lock_granted = crate::consent::capabilities_from(&manifest.permissions);

    // 5. Begin transaction. From here, any error triggers auto-rollback.
    let mut txn = InstallTransaction::new(temp_dir, manifest);

    // 6. Check disk space (20% buffer).
    queue.update_progress(job_id, 30, "checking disk space");
    emit_progress(conn, job_id, 30, "checking disk space").await;
    txn.check_disk_space()?;

    // 7. Install app files (bin, lib, share).
    queue.update_progress(job_id, 40, "installing files");
    emit_progress(conn, job_id, 40, "installing files").await;
    txn.install_files()?;

    // 8. Install GSettings schemas.
    queue.update_progress(job_id, 55, "installing schemas");
    emit_progress(conn, job_id, 55, "installing schemas").await;
    txn.install_schemas()?;

    // 9. Install bundled modules.
    queue.update_progress(job_id, 65, "installing modules");
    emit_progress(conn, job_id, 65, "installing modules").await;
    txn.install_modules()?;

    // 10. Write compositor keybinding fragment, if the manifest ships
    //     any. Sits between modules and the desktop entry so the
    //     shortcut is live before the app becomes launchable.
    queue.update_progress(job_id, 72, "writing keybindings");
    emit_progress(conn, job_id, 72, "writing keybindings").await;
    txn.write_keybindings()?;

    // 11. Create desktop entry.
    queue.update_progress(job_id, 80, "creating desktop entry");
    emit_progress(conn, job_id, 80, "creating desktop entry").await;
    txn.create_desktop_entry()?;

    // 11. Commit -- marks transaction as successful, cleans up temp.
    queue.update_progress(job_id, 95, "committing");
    emit_progress(conn, job_id, 95, "committing").await;
    txn.commit();

    // 12. Record the app's binary identity into the broker-owned registry (F3
    //     Rung B), so a same-uid copy of the binary to a different path can no
    //     longer impersonate it. The helper re-stats the install path (it records
    //     the truth, not a value we pass). Best-effort: a record failure does NOT
    //     fail the install (the app is usable; its identity is the cooperative
    //     residual until recorded), and a missing helper (dev box) is non-fatal.
    // 13. Record what was installed, at what version, under which grants (U-1).
    //     This is the OLD side every later update diff reads: the manifest on
    //     disk is replaced by an upgrade, so nothing else remembers what the app
    //     was allowed to do before. Best-effort, like the identity record: a
    //     lock write failing must not undo a good install, but it is logged
    //     loudly because an update will then have nothing to compare against.
    record_in_lock(&app_id, &lock_version, lock_granted);

    let install_path = install::user_apps_dir_pub().join(&app_id).join(&binary_rel);
    // SAFETY: getuid never fails.
    let uid = unsafe { libc::getuid() };
    if let Err(e) = crate::permission_helper::record_identity(uid, &app_id, &install_path).await {
        tracing::warn!(
            error = %e,
            app_id = %app_id,
            "failed to record app identity; the app is installed but not yet inode-attested"
        );
    }

    Ok(())
}

/// Execute an uninstall job using staged deletion (30-day grace period).
async fn run_uninstall(
    queue: &JobQueue,
    conn: &Connection,
    job_id: &str,
    app_id: &str,
) -> Result<(), install::InstallError> {
    // 1. Stage for deletion (moves app to trash, removes schemas/modules).
    queue.update_progress(job_id, 20, "staging for deletion");
    emit_progress(conn, job_id, 20, "staging for deletion").await;
    crate::trash::stage_for_deletion(app_id).map_err(|e| {
        install::InstallError::TrashFailed(e.to_string())
    })?;

    // 2. Remove desktop entry (app no longer launchable).
    queue.update_progress(job_id, 60, "removing desktop entry");
    emit_progress(conn, job_id, 60, "removing desktop entry").await;
    install::remove_desktop_entry(app_id)?;

    // 3. Drop it from the lock: it is no longer installed, so it has no version
    //    or grants to compare a future install against. The app stays in the
    //    30-day trash, but a restore reinstalls and re-records.
    remove_from_lock(app_id);

    Ok(())
}

/// Record an install in the lock, logging rather than failing.
///
/// A lock write that fails leaves a working install whose next update has no old
/// side to diff against, so the capability gate would treat it as a first
/// install. Worth a loud log; not worth undoing an otherwise good install.
fn record_in_lock(app_id: &str, version: &str, granted: arlen_forage_recipe::Capabilities) {
    let path = crate::lock::lock_path();
    let mut lock = match crate::lock::Lock::load(&path) {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!(error = %e, "could not read the install lock; not recording");
            return;
        }
    };
    let installed_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    lock.record(crate::lock::LockEntry::new(
        app_id,
        "lunpkg",
        version,
        installed_at,
        granted,
    ));
    if let Err(e) = lock.save(&path) {
        tracing::warn!(
            error = %e,
            app_id = %app_id,
            "could not write the install lock; a later update will have nothing to diff against"
        );
    }
}

/// Drop an uninstalled app from the lock, logging rather than failing.
fn remove_from_lock(app_id: &str) {
    let path = crate::lock::lock_path();
    let Ok(mut lock) = crate::lock::Lock::load(&path) else {
        tracing::warn!("could not read the install lock; not clearing {app_id}");
        return;
    };
    if lock.remove(app_id) {
        if let Err(e) = lock.save(&path) {
            tracing::warn!(error = %e, "could not write the install lock after uninstall");
        }
    }
}

/// Execute a Flatpak install job.
async fn run_install_flatpak(
    queue: &JobQueue,
    conn: &Connection,
    job_id: &str,
    app_id: &str,
    remote: &str,
) -> Result<(), install::InstallError> {
    use crate::flatpak;

    // 0. Reject a malformed app_id before it reaches the flatpak CLI, the generated
    //    profile TOML (unescaped interpolation) or the profile path (`..`/separator
    //    traversal). The id arrives from the unauthenticated session-bus method.
    if !flatpak::is_valid_app_id(app_id) {
        return Err(install::InstallError::FlatpakFailed(format!(
            "invalid flatpak app_id: {app_id}"
        )));
    }

    // 1. Install via flatpak CLI.
    queue.update_progress(job_id, 20, "installing via flatpak");
    emit_progress(conn, job_id, 20, "installing via flatpak").await;
    flatpak::install_flatpak(app_id, remote).map_err(|e| {
        install::InstallError::FlatpakFailed(e.to_string())
    })?;

    // 2. Create default Arlen permission profile. Flatpak apps run as the user, so
    //    the grant is a user-tier profile at the canonical `~/.config/permissions/`
    //    that the loaders actually read. The old `flatpak-profiles/` directory was
    //    consulted by nothing, so its grants were silently ignored.
    queue.update_progress(job_id, 70, "creating permission profile");
    emit_progress(conn, job_id, 70, "creating permission profile").await;
    let profile_path = arlen_permissions::profile_path(app_id)
        .map_err(|e| install::InstallError::Io(std::io::Error::other(e.to_string())))?;
    // Write the default profile ONLY if none exists (AUTH-CANONICAL §2: user edits to
    // an existing profile are never overwritten). A reinstall must not silently reset
    // grants the user has narrowed.
    if profile_path.exists() {
        tracing::info!("flatpak {app_id} already has a permission profile; leaving it");
    } else {
        // Generate the FLOOR from what the Flatpak already declares (E5), so the
        // app gets exactly its manifest reach, never more; fall back to the
        // conservative default if the declared context is unreadable.
        let profile = match flatpak::get_flatpak_context(app_id) {
            Ok(ctx) => flatpak::floor_profile_from_context(&ctx, app_id),
            Err(e) => {
                tracing::warn!(
                    "flatpak {app_id}: declared permissions unreadable ({e}), using conservative default"
                );
                flatpak::default_permission_profile(app_id)
            }
        };
        if let Some(dir) = profile_path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        std::fs::write(&profile_path, &profile).map_err(install::InstallError::Io)?;
        tracing::info!("wrote Arlen permission profile for flatpak {app_id}");
    }

    // Announce the profile so the knowledge daemon projects the app's declared
    // grants into the LCG now, without waiting for the app to first run (E1: an
    // installed-but-never-run app otherwise has a profile on disk but zero Grant
    // nodes). Idempotent + best-effort.
    crate::event_emit::emit_permission_changed(app_id, true);

    Ok(())
}

/// Execute a Flatpak uninstall job.
async fn run_uninstall_flatpak(
    queue: &JobQueue,
    conn: &Connection,
    job_id: &str,
    app_id: &str,
) -> Result<(), install::InstallError> {
    use crate::flatpak;

    // 1. Uninstall via flatpak CLI.
    queue.update_progress(job_id, 30, "uninstalling via flatpak");
    emit_progress(conn, job_id, 30, "uninstalling via flatpak").await;
    flatpak::uninstall_flatpak(app_id).map_err(|e| {
        install::InstallError::FlatpakFailed(e.to_string())
    })?;

    // 2. Remove Arlen permission profile from the canonical user-tier location it
    //    was written to.
    queue.update_progress(job_id, 70, "removing permission profile");
    emit_progress(conn, job_id, 70, "removing permission profile").await;
    if let Ok(profile_path) = arlen_permissions::profile_path(app_id) {
        if profile_path.exists() {
            let _ = std::fs::remove_file(&profile_path);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The upgrade job carries its own kind so it can be gated. Install refuses
    /// an already-installed app on purpose; routing an update through it would
    /// turn "install" into an unreviewed replace.
    #[test]
    fn an_upgrade_is_its_own_job_kind() {
        let q = JobQueue::new();
        let id = q.enqueue(JobKind::Upgrade {
            path: "/tmp/app.lunpkg".into(),
        });
        assert!(!id.is_empty());
        let (_, state, _) = q.get_status(&id).expect("the job should be queued");
        assert_eq!(state, "pending");
    }

    #[test]
    fn test_enqueue_and_status() {
        let q = JobQueue::new();
        let id = q.enqueue(JobKind::Uninstall {
            app_id: "com.test".into(),
        });
        let (progress, state, _status) = q.get_status(&id).unwrap();
        assert_eq!(progress, 0);
        assert_eq!(state, "pending");
    }

    #[test]
    fn test_update_progress() {
        let q = JobQueue::new();
        let id = q.enqueue(JobKind::Uninstall {
            app_id: "com.test".into(),
        });
        q.update_progress(&id, 50, "halfway");
        let (progress, state, status) = q.get_status(&id).unwrap();
        assert_eq!(progress, 50);
        assert_eq!(state, "running");
        assert_eq!(status, "halfway");
    }

    #[test]
    fn test_mark_completed() {
        let q = JobQueue::new();
        let id = q.enqueue(JobKind::Uninstall {
            app_id: "com.test".into(),
        });
        q.mark_completed(&id);
        let (progress, state, _) = q.get_status(&id).unwrap();
        assert_eq!(progress, 100);
        assert_eq!(state, "completed");
    }

    #[test]
    fn test_mark_failed() {
        let q = JobQueue::new();
        let id = q.enqueue(JobKind::Uninstall {
            app_id: "com.test".into(),
        });
        q.mark_failed(&id, "disk full");
        let (_, state, _) = q.get_status(&id).unwrap();
        assert_eq!(state, "failed");
    }
}

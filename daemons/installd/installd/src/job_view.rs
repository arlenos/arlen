//! Reporting an install to the Activity/Jobs surface (`job-progress-surface.md`).
//!
//! The file manager was the first producer; this is the second, and it closes the
//! other half of the same gap: installing or removing an app currently shows
//! nothing at all while it runs, and then a notification when it is over. A person
//! who pressed Install has no way to tell a slow install from one that did not
//! start.
//!
//! **THE JOB IS INDETERMINATE, DELIBERATELY.** The contract's first rule is that
//! amounts travel in real units and never as a pre-baked percent, and a percent is
//! all this daemon has: the `progress` it already tracks is a stage marker (5
//! "starting", then verify, extract, commit), not a measurement of anything. Sending
//! it as "5 of 100 items" would be exactly the invented number that rule exists to
//! forbid, wearing a unit. So the row says what is happening and that it is running,
//! which is true, and says nothing it did not measure.
//!
//! **NOT KILLABLE AND NOT SUSPENDABLE**, for the reason the file manager is
//! killable and it is not: an install is one transaction with a rollback, there is
//! no cancel path into the worker, and offering a Cancel button that cannot stop
//! anything is worse than offering none.
//!
//! **EVERY FAILURE HERE IS SILENT**, the same promise the file manager makes: an
//! install must not fail, stall or change behaviour because the progress surface is
//! unreachable. No daemon means no row and the install runs exactly as before.

use notification_proto::client::JobViewServerProxy;

use crate::jobs::JobKind;

/// The app id this producer registers under. Not a desktop id - there is no
/// installd window - but the daemon's own name, so a row can be attributed to
/// the thing that is doing the work rather than to whichever app asked.
const APP_ID: &str = "arlen-installd";

/// A registered install job, for as long as the operation runs.
pub struct InstallJob {
    proxy: JobViewServerProxy<'static>,
    id: u64,
}

impl InstallJob {
    /// Register the job, or `None` when the job server is unreachable.
    pub async fn start(title: &str) -> Option<InstallJob> {
        let connection = zbus::Connection::session().await.ok()?;
        let proxy = JobViewServerProxy::new(&connection).await.ok()?;
        let id = proxy
            .register(
                APP_ID, title, "items", 0, false, // indeterminate: see the module note
                false, // not killable: no cancel path into a transactional install
                false, // not suspendable: nor a pause one
                // No egress line. A lunpkg install reads a local file and reaches
                // nothing; a Flatpak install does reach a remote, but what this
                // daemon holds is the remote's NAME ("flathub"), and the field is
                // a HOST that gets shown at a consent moment. A name in a host's
                // place is a small lie in the one field built to be precise.
                "", &[],
            )
            .await
            .ok()?;
        Some(InstallJob { proxy, id })
    }

    /// Say how it ended, then take the row off the list.
    ///
    /// The state carries the message so the zone can say WHY a job stopped; a
    /// failed install that simply vanished would look like one that worked.
    pub async fn finish(self, result: &Result<(), impl std::fmt::Display>) {
        let (state, message) = match result {
            Ok(()) => ("done", String::new()),
            Err(e) => ("error-fatal", e.to_string()),
        };
        let _ = self.proxy.set_state(self.id, state, &message).await;
        let _ = self.proxy.finish(self.id).await;
    }
}

/// What the zone shows for one operation.
///
/// Written here because only the producer knows what it is doing, which is the
/// convention the file manager's producer set and the shell's store records as
/// i18n-foreign: a job title is written by whichever daemon is working, and
/// whether daemons should send an id instead is a design question nobody has
/// answered yet. A `.lunpkg` is named by its file, since an id is only known once
/// the manifest inside it has been read - after the row would have to exist.
#[must_use]
pub fn title_for(kind: &JobKind) -> String {
    match kind {
        JobKind::InstallPackage { path } => format!("Installing {}", package_name(path)),
        JobKind::Upgrade { path } => format!("Updating {}", package_name(path)),
        JobKind::InstallFlatpak { app_id, .. } => format!("Installing {app_id}"),
        JobKind::UpdateFlatpak { app_id } => format!("Updating {app_id}"),
        JobKind::Uninstall { app_id } | JobKind::UninstallFlatpak { app_id } => {
            format!("Removing {app_id}")
        }
    }
}

/// The file's own name, without its directory or the `.lunpkg` suffix. The full
/// path is somebody's filesystem and the row outlives the window it started in.
fn package_name(path: &str) -> String {
    let base = path.rsplit('/').next().unwrap_or(path);
    base.strip_suffix(".lunpkg").unwrap_or(base).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_package_is_named_by_its_file_and_not_by_its_path() {
        assert_eq!(package_name("/home/tim/Downloads/notes-1.2.lunpkg"), "notes-1.2");
        assert_eq!(package_name("notes.lunpkg"), "notes");
        // A path with no suffix and no directory still answers something.
        assert_eq!(package_name("notes"), "notes");
    }

    #[test]
    fn every_kind_says_what_it_is_doing() {
        let cases = [
            (JobKind::InstallPackage { path: "/tmp/a.lunpkg".into() }, "Installing a"),
            (JobKind::Upgrade { path: "/tmp/a.lunpkg".into() }, "Updating a"),
            (
                JobKind::InstallFlatpak { app_id: "org.x.A".into(), remote: "flathub".into() },
                "Installing org.x.A",
            ),
            (JobKind::UpdateFlatpak { app_id: "org.x.A".into() }, "Updating org.x.A"),
            (JobKind::Uninstall { app_id: "org.x.A".into() }, "Removing org.x.A"),
            (JobKind::UninstallFlatpak { app_id: "org.x.A".into() }, "Removing org.x.A"),
        ];
        for (kind, want) in cases {
            assert_eq!(title_for(&kind), want);
        }
    }

    #[test]
    fn a_title_never_carries_the_directory_it_came_from() {
        // The row outlives the window that started it and is visible to anyone
        // looking at the desktop, so it must not spell out where in somebody's
        // home the file was.
        let t = title_for(&JobKind::InstallPackage { path: "/home/tim/secret-dir/x.lunpkg".into() });
        assert!(!t.contains('/'), "{t}");
        assert!(!t.contains("secret-dir"), "{t}");
    }
}

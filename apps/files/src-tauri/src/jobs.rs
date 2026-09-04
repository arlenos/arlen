//! Reporting a long file operation to the Activity/Jobs surface
//! (`job-progress-surface.md`).
//!
//! The file manager is the plan's FIRST producer, and it is first for a reason:
//! copying two hundred photos has shown no progress at all until now, which is
//! the actual gap the whole surface exists to close. Everything the contract
//! asks for is decided here rather than in the shell.
//!
//! WHAT THIS REPORTS, and what it deliberately does not:
//!
//!  - amounts in REAL units - entries, not a percentage. The zone renders "84 of
//!    240" and derives its own bar;
//!  - `killable`, because a copy loop can stop between entries, and NOT
//!    `suspendable`, because it cannot resume: a paused copy would have to hold
//!    an open position in a directory walk across an unbounded wait. The plan
//!    says never promise pause on a copy that cannot resume cleanly, and this is
//!    where that promise would be made;
//!  - no threshold. The job is registered the moment the operation starts and
//!    the SHELL decides whether it has run long enough to be worth a row. A
//!    producer deciding that itself is every producer inventing its own idea of
//!    what is worth showing.
//!
//! EVERY FAILURE HERE IS SILENT. A file operation must not fail, stall or change
//! behaviour because the progress surface is unreachable: the copy is the point,
//! the bar is a courtesy. So a missing daemon means no job and the operation
//! runs exactly as it did before.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// The bus address of the job server.
const SERVICE: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/arlen/JobViewServer";
const INTERFACE: &str = "org.arlen.JobViewServer1";

/// The app id this producer registers under: the desktop id the rest of the
/// system knows the file manager by, so the zone's row and the app's windows
/// name the same app.
pub const APP_ID: &str = "arlen-files";

/// A registered job, for as long as the operation runs.
pub struct JobHandle {
    proxy: zbus::Proxy<'static>,
    id: u64,
    cancelled: Arc<AtomicBool>,
    listener: tokio::task::JoinHandle<()>,
}

impl JobHandle {
    /// Register a job over `total` entries, or `None` if the surface cannot be
    /// reached. The caller carries on either way.
    pub async fn start(title: String, total: u64) -> Option<JobHandle> {
        let connection = zbus::Connection::session().await.ok()?;
        let proxy = zbus::Proxy::new(&connection, SERVICE, PATH, INTERFACE)
            .await
            .ok()?;
        let id: u64 = proxy
            .call(
                "Register",
                &(
                    APP_ID.to_string(),
                    title,
                    "items".to_string(),
                    total,
                    true,  // determinate: the entry count is known up front
                    true,  // killable: the loop can stop between entries
                    false, // suspendable: a copy cannot resume cleanly
                    String::new(), // no egress: these bytes stay on this machine
                ),
            )
            .await
            .ok()?;

        // Listen for the person pressing Cancel. The daemon relays the intent and
        // this is the half that acts on it: a flag the loop reads between
        // entries, so a cancel stops at an entry boundary rather than halfway
        // through a file.
        let cancelled = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&cancelled);
        let watch = proxy.clone();
        let listener = tokio::spawn(async move {
            let Ok(mut stream) = watch.receive_signal("CancelRequested").await else {
                return;
            };
            use futures_util::StreamExt;
            while let Some(message) = stream.next().await {
                if let Ok(asked) = message.body().deserialize::<u64>() {
                    if asked == id {
                        flag.store(true, Ordering::SeqCst);
                        return;
                    }
                }
            }
        });

        Some(JobHandle {
            proxy,
            id,
            cancelled,
            listener,
        })
    }

    /// Report how many entries are done.
    pub async fn advance(&self, processed: u64, total: u64) {
        let _: Result<bool, _> = self
            .proxy
            .call("Update", &(self.id, processed, total, true))
            .await;
    }

    /// Whether the person asked to stop.
    pub fn cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    /// Say why the job is not running any more, then take it off the list.
    ///
    /// A cancelled or failed operation sets a state with a message first, so the
    /// zone can say what happened, and only then finishes. Finishing without it
    /// would make a stopped copy look like a completed one.
    pub async fn finish(self, state: &str, message: &str) {
        let _: Result<bool, _> = self
            .proxy
            .call(
                "SetState",
                &(self.id, state.to_string(), message.to_string()),
            )
            .await;
        let _: Result<bool, _> = self.proxy.call("Finish", &(self.id,)).await;
        self.listener.abort();
    }
}

/// The title the zone shows for one operation.
///
/// Built here rather than in the shell because only the producer knows what it
/// is doing. It names the count and the verb and nothing else: a title that
/// listed the files would be a sentence about somebody's own documents in a
/// surface that outlives the window, and the per-item names belong in the
/// expandable rows the contract asks for instead.
#[must_use]
pub fn title_for(kind: &str, count: u64) -> String {
    let item = if count == 1 { "item" } else { "items" };
    match kind {
        "copy" => format!("Copying {count} {item}"),
        "move" => format!("Moving {count} {item}"),
        "trash" => format!("Moving {count} {item} to the trash"),
        "delete" => format!("Deleting {count} {item}"),
        "duplicate" => format!("Duplicating {count} {item}"),
        other => format!("Working on {count} {item} ({other})"),
    }
}

/// Whether an operation of this kind and size is worth registering at all.
///
/// A single rename or a new folder finishes before anything could render it, and
/// a job registered and finished in the same millisecond is churn on the daemon
/// and the socket for a row nobody sees. This is NOT the visibility threshold -
/// that is the shell's, and it is about time rather than count. This is only
/// about not reporting an operation that has no progress to report.
#[must_use]
pub fn worth_reporting(kind: &str, count: u64) -> bool {
    matches!(kind, "copy" | "move" | "trash" | "delete" | "duplicate") && count > 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_title_counts_and_names_the_verb_without_naming_the_files() {
        assert_eq!(title_for("copy", 240), "Copying 240 items");
        assert_eq!(title_for("trash", 1), "Moving 1 item to the trash");
        assert_eq!(title_for("move", 2), "Moving 2 items");
    }

    #[test]
    fn an_unknown_kind_still_says_what_it_is_doing() {
        let t = title_for("archive", 3);
        assert!(t.contains('3') && t.contains("archive"), "{t}");
    }

    #[test]
    fn a_single_entry_operation_is_not_a_job() {
        assert!(!worth_reporting("copy", 1));
        assert!(worth_reporting("copy", 2));
    }

    #[test]
    fn the_operations_with_nothing_to_count_are_never_jobs() {
        for kind in ["rename", "new_folder"] {
            assert!(!worth_reporting(kind, 50), "{kind} has no per-entry progress");
        }
    }
}

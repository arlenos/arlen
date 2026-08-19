// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: GPL-3.0-only

//! Removing an app from its settings page.
//!
//! Settings does not remove anything itself. `installd` owns installation and
//! removal - it holds the transaction, the staged-delete window and the
//! privileged helper - so this is a client of `org.arlen.InstallDaemon1` and
//! nothing more. Doing the removal here would put a second remover next to the
//! one that knows how to roll back.
//!
//! The daemon answers a removal request with a job id and reports the outcome
//! over `JobCompleted`, so a caller that returns on the id would report success
//! for a removal that then failed. This waits for the job.
//!
//! Two things about that reply are easy to get wrong. An unauthorised request is
//! refused with an EMPTY job id rather than an error, because a method whose
//! result arrives on a signal has no error channel; waiting on it would burn the
//! timeout and then blame the daemon for not answering. And a `JobCompleted` for
//! a different job is not this one's outcome.

use futures_util::StreamExt;
use zbus::Connection;

const BUS_NAME: &str = "org.arlen.InstallDaemon1";
const OBJECT_PATH: &str = "/org/arlen/InstallDaemon1";
const INTERFACE: &str = "org.arlen.InstallDaemon1";

/// How long to wait for a removal before giving up on hearing about it.
///
/// Removal is not a download; a minute is generous for unlinking a tree, and a
/// caller that hangs forever is worse than one that says it does not know.
const JOB_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// Reject anything that is not a plain app id before it reaches the bus.
fn is_safe_app_id(id: &str) -> bool {
    !id.is_empty()
        && id != "."
        && id != ".."
        && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
}

/// Wait for one job's completion signal.
async fn wait_for_job(conn: &Connection, job_id: &str) -> Result<(), String> {
    let proxy = zbus::Proxy::new(conn, BUS_NAME, OBJECT_PATH, INTERFACE)
        .await
        .map_err(|e| format!("proxy creation failed: {e}"))?;
    let mut stream = proxy
        .receive_all_signals()
        .await
        .map_err(|e| format!("signal subscription failed: {e}"))?;

    let deadline = tokio::time::Instant::now() + JOB_TIMEOUT;
    loop {
        match tokio::time::timeout_at(deadline, stream.next()).await {
            Ok(Some(signal)) => {
                let member = signal.header().member().map(|m| m.to_string()).unwrap_or_default();
                if member != "JobCompleted" {
                    continue;
                }
                let Ok((sid, ok, error)) =
                    signal.body().deserialize::<(String, bool, String)>()
                else {
                    continue;
                };
                // Another job's completion is not ours; keep waiting rather than
                // reporting its outcome as this removal's.
                if sid != job_id {
                    continue;
                }
                return if ok { Ok(()) } else { Err(error) };
            }
            // The daemon went away mid-job. Whether the removal happened is
            // genuinely unknown, and saying so is better than guessing either way.
            Ok(None) => return Err("the install daemon stopped responding".to_owned()),
            Err(_) => return Err("the removal did not report back in time".to_owned()),
        }
    }
}

/// Remove an installed app.
/// Say which of the two very different things went wrong.
///
/// `installd` is one of the units the image does not ship yet, so on a running
/// Arlen the overwhelmingly likely failure is that nothing owns the name at all.
/// The previous message called that "the install daemon refused the request",
/// which is a sentence about a daemon that made a decision - and a user reading
/// it has every reason to go looking for the permission they are missing. What is
/// true is that app removal is not available on this system.
///
/// The distinction is on the wire: D-Bus answers a call to an unowned name with
/// `org.freedesktop.DBus.Error.ServiceUnknown`, which is not something a running
/// daemon can produce. Matched by substring for the same reason
/// `tauri-plugin-portal` does it: the error name arrives as an owned string and
/// zbus offers no typed variant to match on.
fn describe_call_failure(err: zbus::Error) -> String {
    match &err {
        zbus::Error::MethodError(name, _, _) if name.as_str().contains("ServiceUnknown") => {
            "removing apps is unavailable on this system: nothing provides the \
             install daemon"
                .to_owned()
        }
        // A refusal the daemon explained: it decides what may be removed - a
        // component of the running desktop is refused by name - and the sentence
        // it wrote is for the person who pressed Remove. Wrapping it in "the
        // install daemon refused the request: org.freedesktop.DBus.Error.
        // AccessDenied: ..." buries the only part they can act on.
        zbus::Error::MethodError(name, Some(detail), _)
            if name.as_str().ends_with("AccessDenied") && !detail.trim().is_empty() =>
        {
            detail.trim().to_owned()
        }
        _ => format!("the install daemon refused the request: {err}"),
    }
}

#[tauri::command]
pub async fn settings_app_uninstall(app_id: String) -> Result<(), String> {
    if !is_safe_app_id(&app_id) {
        return Err("not an app id".to_owned());
    }
    let conn = Connection::session()
        .await
        .map_err(|e| format!("cannot reach the session bus: {e}"))?;
    let reply = conn
        .call_method(
            Some(BUS_NAME),
            OBJECT_PATH,
            Some(INTERFACE),
            "Uninstall",
            &(app_id.clone(),),
        )
        .await
        .map_err(describe_call_failure)?;
    let job_id: String = reply
        .body()
        .deserialize()
        .map_err(|e| format!("unexpected reply from the install daemon: {e}"))?;
    // An empty job id is how the daemon refuses: its result arrives on a signal,
    // so it has no error channel on the method itself. Waiting on it would spend
    // the whole timeout and then report "no answer" for what was a clear no.
    if job_id.is_empty() {
        return Err("the install daemon did not authorise this removal".to_owned());
    }
    wait_for_job(&conn, &job_id).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_app_id_that_is_not_one_never_reaches_the_bus() {
        assert!(is_safe_app_id("org.arlen.files"));
        assert!(!is_safe_app_id(""));
        assert!(!is_safe_app_id(".."));
        assert!(!is_safe_app_id("org.arlen.files; rm -rf /"));
        assert!(!is_safe_app_id("../../etc"));
    }

    /// A refusal the daemon explained reaches the person unwrapped.
    ///
    /// `installd` decides what may be removed and writes the sentence - "X is
    /// part of the desktop itself and cannot be removed". Reporting that as "the
    /// install daemon refused the request:
    /// org.freedesktop.DBus.Error.AccessDenied: X is part of..." buries the only
    /// part of it a reader can act on under two layers of transport.
    #[test]
    fn the_daemons_own_sentence_is_what_the_reader_sees() {
        let denied = zbus::Error::MethodError(
            zbus::names::OwnedErrorName::try_from("org.freedesktop.DBus.Error.AccessDenied")
                .unwrap(),
            Some("dev.arlen.desktop-shell is part of the desktop itself and cannot be removed".into()),
            zbus::Message::method_call("/org/arlen/InstallDaemon1", "Uninstall")
                .unwrap()
                .build(&())
                .unwrap(),
        );
        let said = describe_call_failure(denied);
        assert_eq!(
            said,
            "dev.arlen.desktop-shell is part of the desktop itself and cannot be removed"
        );

        // A denial with nothing to say still gets the generic line rather than an
        // empty string, which would render as a blank error.
        let bare = zbus::Error::MethodError(
            zbus::names::OwnedErrorName::try_from("org.freedesktop.DBus.Error.AccessDenied")
                .unwrap(),
            Some("   ".into()),
            zbus::Message::method_call("/org/arlen/InstallDaemon1", "Uninstall")
                .unwrap()
                .build(&())
                .unwrap(),
        );
        assert!(describe_call_failure(bare).contains("refused"));
    }

    /// The two failures have to read differently, because they send the reader
    /// to different places: one to look for a missing permission, the other to
    /// understand the feature is not on this system.
    #[test]
    fn an_absent_daemon_does_not_read_as_a_refusal() {
        let absent = zbus::Error::MethodError(
            zbus::names::OwnedErrorName::try_from("org.freedesktop.DBus.Error.ServiceUnknown")
                .unwrap(),
            None,
            zbus::Message::method_call("/org/arlen/InstallDaemon1", "Uninstall")
                .unwrap()
                .build(&())
                .unwrap(),
        );
        let said = describe_call_failure(absent);
        assert!(said.contains("unavailable"), "got {said}");
        assert!(!said.contains("refused"), "an absent daemon refused nothing: {said}");

        let refused = zbus::Error::MethodError(
            zbus::names::OwnedErrorName::try_from("org.arlen.InstallDaemon1.Error.Denied")
                .unwrap(),
            None,
            zbus::Message::method_call("/org/arlen/InstallDaemon1", "Uninstall")
                .unwrap()
                .build(&())
                .unwrap(),
        );
        assert!(describe_call_failure(refused).contains("refused"));
    }
}

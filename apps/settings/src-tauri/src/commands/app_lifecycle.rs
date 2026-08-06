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
        .map_err(|e| format!("the install daemon refused the request: {e}"))?;
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
}

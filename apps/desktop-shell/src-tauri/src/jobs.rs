//! The Activity/Jobs zone's backing state (`job-progress-surface.md`).
//!
//! The notification daemon hosts the JobView server and pushes every register,
//! update and finish down the socket this shell already holds; the client
//! forwards each one here AND to the frontend. This module is the shell's live
//! set, so a store that loads after a job started has something to render rather
//! than waiting for that job's next tick - a paused download might not send one
//! for a long time.
//!
//! THE TRANSLATION LIVES HERE, in one place, and it is not cosmetic. The wire
//! spells the error states `error-recoverable` and `error-fatal`, the KDE
//! JobViewV3 tokens the daemon mirrors; the store declares them
//! `error_recoverable` and `error_fatal`. Forwarding the wire spelling verbatim
//! meant a job that failed arrived in a state the zone has no branch for, so the
//! one moment the surface exists to explain rendered as nothing. The shell is
//! the translator between the daemon's vocabulary and the surface's, and this is
//! the only place either name appears.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde::Serialize;

/// One real-unit metric, as the zone renders it ("84 of 240 files").
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct JobMetric {
    pub processed: u64,
    pub total: u64,
    pub unit: String,
}

/// One job in the shape the store declares.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct JobRow {
    pub id: String,
    pub title: String,
    pub app_id: String,
    /// What to call the producing app. The id until the shell can do better:
    /// showing `org.arlen.files` is plain, and inventing a prettier name for an
    /// app nobody looked up would be worse.
    pub app_label: String,
    pub fraction: f64,
    pub state: String,
    pub metrics: Vec<JobMetric>,
    pub killable: bool,
    pub suspendable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub egress_host: Option<String>,
    /// When it began, epoch micros. The SHELL owns the visibility threshold, so
    /// this travels rather than a daemon-side decision about what is worth
    /// showing.
    pub started_at: u64,
}

/// The store's spelling of a wire state token.
///
/// An unknown token is carried through rather than dropped or rounded to
/// `running`: a job in a state this build does not know about is still a job,
/// and claiming it is running would be the surface saying something it was not
/// told.
#[must_use]
pub fn state_token(wire: &str) -> String {
    match wire {
        "error-recoverable" => "error_recoverable".to_string(),
        "error-fatal" => "error_fatal".to_string(),
        other => other.to_string(),
    }
}

/// The zone's row for one job update.
#[must_use]
pub fn row_from_update(
    id: u64,
    app_id: &str,
    title: &str,
    state: &str,
    state_message: &str,
    unit: &str,
    processed: u64,
    determinate: bool,
    total: u64,
    fraction: f64,
    killable: bool,
    suspendable: bool,
    started_at: u64,
    egress_host: &str,
) -> JobRow {
    JobRow {
        id: id.to_string(),
        title: title.to_string(),
        app_id: app_id.to_string(),
        app_label: app_id.to_string(),
        fraction,
        state: state_token(state),
        // A job that does not know its total yet reports no metric at all. The
        // alternative - "84 of 0 files" - is a sentence about a total nobody has
        // counted, and the zone draws an indeterminate bar from the flag instead.
        metrics: if determinate {
            vec![JobMetric {
                processed,
                total,
                unit: unit.to_string(),
            }]
        } else {
            Vec::new()
        },
        killable,
        suspendable,
        error: (!state_message.is_empty()).then(|| state_message.to_string()),
        egress_host: (!egress_host.is_empty()).then(|| egress_host.to_string()),
        started_at,
    }
}

/// The jobs running right now, by id.
pub type LiveJobs = Arc<Mutex<HashMap<u64, JobRow>>>;

/// A fresh, empty live set.
#[must_use]
pub fn new_live_jobs() -> LiveJobs {
    Arc::new(Mutex::new(HashMap::new()))
}

/// Fold one update into the live set. `removed` takes the job out.
pub fn apply(live: &LiveJobs, id: u64, row: JobRow, removed: bool) {
    let Ok(mut map) = live.lock() else {
        return;
    };
    if removed {
        map.remove(&id);
    } else {
        map.insert(id, row);
    }
}

/// The jobs running right now, oldest first.
///
/// Oldest first because the zone is a list somebody reads top down and a job
/// that jumps position as its progress changes is a list nobody can click in.
#[tauri::command]
pub fn list_jobs(live: tauri::State<'_, LiveJobs>) -> Vec<JobRow> {
    let Ok(map) = live.lock() else {
        return Vec::new();
    };
    let mut rows: Vec<JobRow> = map.values().cloned().collect();
    rows.sort_by(|a, b| a.started_at.cmp(&b.started_at).then(a.id.cmp(&b.id)));
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: u64, state: &str, started_at: u64) -> JobRow {
        row_from_update(
            id,
            "org.arlen.files",
            "Copying 240 photos to USB",
            state,
            if state == "running" { "" } else { "The disk is full" },
            "files",
            84,
            true,
            240,
            0.35,
            true,
            false,
            started_at,
            "",
        )
    }

    #[test]
    fn the_error_states_arrive_in_the_spelling_the_zone_branches_on() {
        assert_eq!(state_token("error-recoverable"), "error_recoverable");
        assert_eq!(state_token("error-fatal"), "error_fatal");
        assert_eq!(state_token("running"), "running");
    }

    #[test]
    fn a_state_this_build_does_not_know_is_carried_not_guessed() {
        assert_eq!(state_token("throttled"), "throttled");
    }

    #[test]
    fn a_running_job_carries_no_error_line() {
        let r = row(1, "running", 10);
        assert_eq!(r.error, None);
        assert_eq!(r.state, "running");
        assert_eq!(r.metrics, vec![JobMetric { processed: 84, total: 240, unit: "files".into() }]);
    }

    #[test]
    fn a_stalled_job_says_why() {
        let r = row(1, "impeded", 10);
        assert_eq!(r.error.as_deref(), Some("The disk is full"));
    }

    #[test]
    fn a_job_with_no_total_yet_reports_no_metric_rather_than_a_zero() {
        let r = row_from_update(1, "a", "t", "running", "", "bytes", 500, false, 0, 0.0, false, false, 1, "");
        assert!(r.metrics.is_empty(), "84 of 0 is not a sentence about anything");
    }

    #[test]
    fn a_networked_job_names_the_host_it_reaches() {
        let r = row_from_update(1, "a", "t", "running", "", "bytes", 1, true, 2, 0.5, false, false, 1, "cdn.example");
        assert_eq!(r.egress_host.as_deref(), Some("cdn.example"));
        let local = row(2, "running", 1);
        assert_eq!(local.egress_host, None, "a local job claims no destination");
    }

    #[test]
    fn an_update_replaces_its_job_and_a_finish_removes_it() {
        let live = new_live_jobs();
        apply(&live, 1, row(1, "running", 10), false);
        apply(&live, 1, row(1, "paused", 10), false);
        assert_eq!(live.lock().unwrap().len(), 1);
        assert_eq!(live.lock().unwrap()[&1].state, "paused");
        apply(&live, 1, row(1, "done", 10), true);
        assert!(live.lock().unwrap().is_empty());
    }

    #[test]
    fn the_list_reads_oldest_first_so_a_row_does_not_move_under_the_pointer() {
        let live = new_live_jobs();
        apply(&live, 2, row(2, "running", 200), false);
        apply(&live, 1, row(1, "running", 100), false);
        let map = live.lock().unwrap();
        let mut rows: Vec<JobRow> = map.values().cloned().collect();
        rows.sort_by(|a, b| a.started_at.cmp(&b.started_at).then(a.id.cmp(&b.id)));
        assert_eq!(
            rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            vec!["1", "2"]
        );
    }
}

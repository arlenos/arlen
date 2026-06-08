//! Recent anomaly notices for the agent observability view.
//!
//! The Anomaly Detector daemon appends each dispatched alert to a small
//! `alerts.json` log under its data directory. This command reads that file
//! (the same path the daemon writes) and returns the recent notices for the
//! agent dashboard's "Notices" panel. Read-only and best-effort: a missing or
//! unreadable file means "no notices yet", never an error.

use serde::{Deserialize, Serialize};

/// The on-disk shape written by the daemon's `AlertLog`.
#[derive(Deserialize)]
struct AlertLogFile {
    #[serde(default)]
    alerts: Vec<RecentAlertIn>,
}

/// One alert as the daemon serialises it (snake_case field names).
#[derive(Deserialize)]
struct RecentAlertIn {
    kind: String,
    summary: String,
    body: String,
    critical: bool,
    ts_micros: i64,
}

/// One notice as the frontend consumes it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Notice {
    kind: String,
    summary: String,
    body: String,
    critical: bool,
    ts_micros: i64,
}

/// The notices, plus whether the source was readable. `available` distinguishes
/// "the detector has nothing for us" (a normal empty state, `available = true`)
/// from "the alert log exists but could not be read or parsed" (`available =
/// false`), so a live-refreshing UI does not clear a shown notice and present a
/// degraded source as "all clear".
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NoticesResult {
    available: bool,
    notices: Vec<Notice>,
}

impl NoticesResult {
    /// A readable source (possibly with no notices).
    fn ok(notices: Vec<Notice>) -> Self {
        Self { available: true, notices }
    }
    /// A degraded source: present but unreadable or malformed.
    fn degraded() -> Self {
        Self { available: false, notices: Vec::new() }
    }
}

/// Read the recent anomaly notices from a specific alert-log path. A missing
/// file is a readable "nothing yet" (`available = true`, empty); a present-but-
/// unreadable or malformed file is `available = false` so the UI can flag it.
/// Pure of the data-dir lookup, so the available/degraded distinction is
/// unit-tested against real files.
fn read_notices_at(path: &std::path::Path) -> NoticesResult {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        // Not written yet / detector not installed: a normal empty state.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return NoticesResult::ok(Vec::new()),
        // The file exists but could not be read: degraded, not "all clear".
        Err(_) => return NoticesResult::degraded(),
    };
    let Ok(log) = serde_json::from_slice::<AlertLogFile>(&bytes) else {
        return NoticesResult::degraded();
    };
    NoticesResult::ok(
        log.alerts
            .into_iter()
            .map(|a| Notice {
                kind: a.kind,
                summary: a.summary,
                body: a.body,
                critical: a.critical,
                ts_micros: a.ts_micros,
            })
            .collect(),
    )
}

/// Read the recent anomaly notices (newest first). A missing log or no data dir
/// is a readable "nothing yet" (`available = true`, empty); a present-but-
/// unreadable or malformed log is `available = false` so the UI can flag it.
#[tauri::command]
pub fn ai_notices() -> NoticesResult {
    let Some(path) = dirs::data_dir().map(|d| d.join("arlen/anomaly/alerts.json")) else {
        return NoticesResult::ok(Vec::new());
    };
    read_notices_at(&path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_log_is_available_and_empty() {
        let dir = tempfile::tempdir().unwrap();
        let r = read_notices_at(&dir.path().join("absent.json"));
        assert!(r.available, "a never-written log is a normal empty state");
        assert!(r.notices.is_empty());
    }

    #[test]
    fn a_valid_log_parses_its_alerts() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("alerts.json");
        std::fs::write(
            &path,
            r#"{"alerts":[{"kind":"rate_limit","summary":"s","body":"b","critical":true,"ts_micros":42}]}"#,
        )
        .unwrap();
        let r = read_notices_at(&path);
        assert!(r.available);
        assert_eq!(r.notices.len(), 1);
        assert_eq!(r.notices[0].summary, "s");
        assert!(r.notices[0].critical);
    }

    #[test]
    fn a_malformed_log_is_degraded_not_all_clear() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("alerts.json");
        std::fs::write(&path, "not json at all").unwrap();
        let r = read_notices_at(&path);
        assert!(
            !r.available,
            "a present-but-corrupt log must report degraded, not empty/all-clear"
        );
        assert!(r.notices.is_empty());
    }
}

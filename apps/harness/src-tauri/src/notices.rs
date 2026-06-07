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

/// Read the recent anomaly notices (newest first), or an empty list if the
/// detector has written none yet or is not installed.
#[tauri::command]
pub fn ai_notices() -> Vec<Notice> {
    let Some(path) = dirs::data_dir().map(|d| d.join("arlen/anomaly/alerts.json")) else {
        return Vec::new();
    };
    let Ok(bytes) = std::fs::read(&path) else {
        return Vec::new();
    };
    let Ok(log) = serde_json::from_slice::<AlertLogFile>(&bytes) else {
        return Vec::new();
    };
    log.alerts
        .into_iter()
        .map(|a| Notice {
            kind: a.kind,
            summary: a.summary,
            body: a.body,
            critical: a.critical,
            ts_micros: a.ts_micros,
        })
        .collect()
}

//! Notifications-specific Tauri commands.
//!
//! Most state lives in `~/.config/lunaris/notifications.toml` and is
//! managed via the generic `config_get` / `config_set` commands. This
//! module wraps the few imperative actions that need direct access to
//! the daemon's SQLite history file or the freedesktop `notify-send`
//! tool:
//!
//! * `notifications_get_known_apps` — read distinct `app_name` values
//!   from the daemon's history DB so the per-app rule editor can be
//!   populated without waiting for a given app to send again.
//! * `notifications_clear_history` — delete every row from the daemon
//!   DB. SQLite's file locking handles the concurrent daemon writer.
//! * `notifications_test_notification` — spawn `notify-send` so the
//!   user can preview their toast / DND / per-app rules without
//!   waiting for a real app.
//! * `notifications_set_dnd_temporary` — set a DND mode plus an
//!   `expires_at` ISO-8601 timestamp into `notifications.toml` so the
//!   daemon picks it up via hot-reload. Used by the "1 hour" / "until
//!   tomorrow" Quick Actions.

use std::path::PathBuf;
use std::process::Command;

use chrono::{DateTime, Duration, Local, NaiveTime, TimeZone, Utc};
use rusqlite::{Connection, OpenFlags};
use serde::Serialize;

/// Path to the notification daemon's history database.
fn notifications_db_path() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("lunaris")
        .join("notifications.db")
}

#[derive(Debug, Serialize)]
pub struct AppHistoryEntry {
    pub app_name: String,
    pub last_seen: Option<String>,
    pub count: u32,
}

/// Return the list of distinct app names that have ever sent a
/// notification, plus their most recent timestamp and total count.
/// Sorted by `last_seen` descending so the picker shows the most
/// active apps first.
#[tauri::command]
pub fn notifications_get_known_apps() -> Result<Vec<AppHistoryEntry>, String> {
    let path = notifications_db_path();
    log::info!(
        "notifications_get_known_apps: path={} exists={}",
        path.display(),
        path.exists()
    );
    if !path.exists() {
        log::warn!(
            "notifications_get_known_apps: db not found — is lunaris-notifyd running?"
        );
        return Ok(Vec::new());
    }

    let conn = Connection::open_with_flags(
        &path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| format!("open db: {e}"))?;

    // Diagnostics: log raw row count separately from the grouped query
    // so we can tell whether the DB itself is empty vs the GROUP BY
    // filtering everything out.
    if let Ok(total) = conn.query_row::<i64, _, _>(
        "SELECT COUNT(*) FROM notifications",
        [],
        |row| row.get(0),
    ) {
        log::info!("notifications_get_known_apps: total rows = {total}");
    }

    let mut stmt = conn
        .prepare(
            "SELECT app_name, MAX(timestamp) AS last_seen, COUNT(*) AS cnt \
             FROM notifications \
             WHERE app_name IS NOT NULL AND app_name != '' \
             GROUP BY app_name \
             ORDER BY last_seen DESC",
        )
        .map_err(|e| format!("prepare: {e}"))?;

    let rows = stmt
        .query_map([], |row| {
            Ok(AppHistoryEntry {
                app_name: row.get(0)?,
                last_seen: row.get(1).ok(),
                count: row.get::<_, i64>(2)? as u32,
            })
        })
        .map_err(|e| format!("query: {e}"))?;

    let mut out = Vec::new();
    for row in rows {
        match row {
            Ok(entry) => out.push(entry),
            Err(e) => log::warn!("notifications_get_known_apps: row error: {e}"),
        }
    }
    log::info!("notifications_get_known_apps: returning {} apps", out.len());
    Ok(out)
}

/// Wipe every row from the daemon's history table. The daemon does
/// not need to be told — its next read will simply find nothing. The
/// frontend should also emit its own `notification:cleared` event so
/// the open shell window updates immediately if it is running.
#[tauri::command]
pub fn notifications_clear_history() -> Result<(), String> {
    let path = notifications_db_path();
    if !path.exists() {
        return Ok(());
    }
    let conn = Connection::open(&path).map_err(|e| format!("open db: {e}"))?;
    conn.execute("DELETE FROM notifications", [])
        .map_err(|e| format!("delete: {e}"))?;
    Ok(())
}

/// Spawn `notify-send` to fire a test notification through the normal
/// freedesktop pipeline. Hits the daemon's D-Bus listener exactly the
/// same way a real app would, so all DND / per-app / grouping rules
/// are exercised.
#[tauri::command]
pub fn notifications_test_notification(priority: String) -> Result<(), String> {
    let urgency = match priority.as_str() {
        "critical" => "critical",
        "low" => "low",
        _ => "normal",
    };
    let body = match priority.as_str() {
        "critical" => "Critical priority test — should bypass DND.",
        "low" => "Low priority test.",
        "high" => "High priority test.",
        _ => "Normal priority test.",
    };
    Command::new("notify-send")
        .arg("--urgency")
        .arg(urgency)
        .arg("--app-name")
        .arg("Lunaris Settings")
        .arg("Lunaris Settings test")
        .arg(body)
        .spawn()
        .map_err(|e| format!("spawn notify-send: {e}"))?;
    Ok(())
}

/// Compute an ISO-8601 UTC `expires_at` `seconds` into the future.
/// Used to back the "Enable DND for N hours" Quick Action.
#[tauri::command]
pub fn notifications_dnd_expiry_in(seconds: i64) -> Result<String, String> {
    let when = Utc::now() + Duration::seconds(seconds.max(0));
    Ok(when.to_rfc3339())
}

/// Compute an ISO-8601 UTC `expires_at` for "until tomorrow morning".
/// Resolves to the next 07:00 in the user's local timezone.
#[tauri::command]
pub fn notifications_dnd_expiry_until_morning() -> Result<String, String> {
    let now = Local::now();
    let target_time = NaiveTime::from_hms_opt(7, 0, 0).ok_or("invalid time")?;
    let mut target_date = now.date_naive();
    if now.time() >= target_time {
        target_date = target_date.succ_opt().ok_or("date overflow")?;
    }
    let local_dt = target_date.and_time(target_time);
    let local = Local
        .from_local_datetime(&local_dt)
        .single()
        .ok_or("ambiguous local time")?;
    let utc: DateTime<Utc> = local.with_timezone(&Utc);
    Ok(utc.to_rfc3339())
}

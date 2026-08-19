// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

//! `arlen-calendard` - the session daemon that owns the calendar.
//!
//! The app is a view: closing it changes nothing, because the part that must
//! survive that lives here. `calendar-app.md` section 2 gives the reason in a
//! line - **anything else means reminders die when the window closes** - and
//! section 4 gives the shape: this computes each occurrence's trigger and
//! registers it with `org.arlen.Clock1`, which owns arming and is the only thing
//! that can wake a sleeping machine. It does not run a timer of its own.
//!
//! Everything it decides is decided in the library beside it and tested there:
//! which files, which occurrences, which registrations should exist. What is
//! here is the part that genuinely needs a process - a bus name, a clock client,
//! and a loop that re-derives.

use std::sync::Arc;

use arlen_calendar::registry::{self, Desired};
use arlen_calendar_core::reminders;
use tokio::sync::Mutex;
use tracing::{info, warn};

/// The bus name the app looks for.
const BUS_NAME: &str = "org.arlen.Calendar1";
/// Where the interface lives on it.
const OBJECT_PATH: &str = "/org/arlen/Calendar1";

/// The clock, and where its interface lives.
const CLOCK_NAME: &str = "org.arlen.Clock1";
const CLOCK_PATH: &str = "/org/arlen/Clock1";

/// How far ahead reminders are registered.
///
/// Long enough that a machine left off over a weekend still comes back to its
/// Monday reminders armed, short enough that a decade-long series does not turn
/// into a decade of alarms. The window moves with every re-derivation, so an
/// occurrence beyond it is picked up as it comes into range.
const HORIZON_DAYS: i64 = 14;

/// How often the store is re-read.
///
/// A poll rather than a watch, for now: a watch is the better answer and the
/// honest note is that this is not it yet. At a minute, an event added by hand
/// is armed within a minute, which is far inside any reminder's own lead.
const RE_DERIVE_SECS: u64 = 60;

/// What the daemon holds between derivations.
struct Calendar {
    store: Mutex<arlen_calendar::Store>,
}

/// The interface the app talks to.
struct CalendarInterface {
    calendar: Arc<Calendar>,
}

#[zbus::interface(name = "org.arlen.Calendar1")]
impl CalendarInterface {
    /// Everything the readable calendar files currently hold, as JSON.
    ///
    /// The counts travel with the events because they are different states the
    /// app has to tell apart: no files at all, files holding nothing, and files
    /// that could not be read.
    async fn agenda(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> zbus::fdo::Result<String> {
        admit(&header, connection).await?;
        let store = self.calendar.store.lock().await;
        let events: Vec<serde_json::Value> = store
            .events
            .iter()
            .map(|e| {
                serde_json::json!({
                    "uid": e.uid,
                    "summary": e.summary,
                    "location": e.location,
                    "date": e.start.date().to_string(),
                    "repeats": e.repeats(),
                    "alarms": e.alarms.len(),
                })
            })
            .collect();
        Ok(serde_json::json!({
            "events": events,
            "files": store.files,
            "unreadable": store.unreadable,
        })
        .to_string())
    }
}

/// The callers allowed to read the agenda.
///
/// The calendar app, and nothing else. `arlen-run` binds the session bus into
/// every confined app and the bus is default-allow, so without a gate any app on
/// the machine could read your event titles, locations and who you are meeting -
/// which is a fair amount of somebody's life for a method that looks like a
/// read.
const ADMITTED: &[&str] = &["dev.arlen.calendar"];
/// The same app run from a build tree. Debug only, deliberately, so `just dev`
/// and the screenshot harness reach a daemon a release build refuses.
#[cfg(debug_assertions)]
const DEV_ADMITTED: &[&str] = &["dev.arlen-calendar-app"];

/// The caller's attested app id, or why it could not be established.
///
/// The bus attests the sender and turns it into a pid the caller cannot forge;
/// the start time is read either side of the resolve so a pid recycled
/// underneath it is refused rather than mistaken for the app. The same shape the
/// clock and power daemons use, and duplicated for the same reason: it is four
/// lines of protocol, and a shared helper would put a bus dependency in the
/// permissions crate that every confined app links.
async fn resolve_caller_app_id(
    header: &zbus::message::Header<'_>,
    connection: &zbus::Connection,
) -> Result<String, String> {
    use arlen_permissions::identity::{app_id_from_pid, pid_start_time};
    let sender = header
        .sender()
        .ok_or_else(|| "no sender in message".to_string())?;
    let proxy = zbus::fdo::DBusProxy::new(connection)
        .await
        .map_err(|e| format!("DBusProxy: {e}"))?;
    let pid = proxy
        .get_connection_unix_process_id(sender.clone().into())
        .await
        .map_err(|e| format!("get caller pid: {e}"))?;
    let start_before = pid_start_time(pid).map_err(|e| format!("pid start time: {e}"))?;
    let app_id = app_id_from_pid(pid).map_err(|e| format!("resolve app id: {e}"))?;
    let start_after = pid_start_time(pid).map_err(|e| format!("pid start time: {e}"))?;
    if start_before != start_after {
        return Err("pid recycled during resolution".to_string());
    }
    Ok(app_id)
}

/// Refuse a caller that is not the calendar app.
async fn admit(
    header: &zbus::message::Header<'_>,
    connection: &zbus::Connection,
) -> zbus::fdo::Result<()> {
    let id = resolve_caller_app_id(header, connection).await.map_err(|e| {
        warn!("refused a calendar call from an unresolved caller: {e}");
        zbus::fdo::Error::AccessDenied("unresolved caller".into())
    })?;
    if ADMITTED.contains(&id.as_str()) {
        return Ok(());
    }
    #[cfg(debug_assertions)]
    if DEV_ADMITTED.contains(&id.as_str()) {
        return Ok(());
    }
    warn!(app_id = %id, "refused a calendar call from an app that is not the calendar");
    Err(zbus::fdo::Error::AccessDenied("not the calendar app".into()))
}

/// Read the files, work out the registrations, and tell the clock.
///
/// Errors are logged rather than fatal: a clock that is down means reminders are
/// not armed right now, which the next pass fixes, and stopping the daemon over
/// it would also stop the agenda the app reads.
async fn re_derive(calendar: &Calendar, connection: &zbus::Connection) {
    let Some(dir) = arlen_calendar::calendar_dir() else {
        warn!("no data directory; cannot find the calendars");
        return;
    };
    let store = arlen_calendar::read_dir(&dir);
    let local = local_zone();
    let now = chrono::Utc::now();
    let derived = reminders::registrations(
        &store.events,
        now,
        now + chrono::Duration::days(HORIZON_DAYS),
        local,
    );
    if !derived.unexpanded.is_empty() {
        // Said out loud rather than left as a silent partial: these series have
        // reminders only for the occurrence their file writes.
        warn!(
            uids = ?derived.unexpanded,
            "recurrence rules this machine cannot expand; their later reminders are not armed"
        );
    }
    let desired = registry::desired_from(&derived.due, local);
    *calendar.store.lock().await = store;

    match apply(connection, &desired).await {
        Ok((set, deleted)) => {
            if set + deleted > 0 {
                info!(set, deleted, "reminders registered with the clock");
            }
        }
        Err(e) => warn!("could not register reminders with the clock: {e}"),
    }
}

/// Bring the clock's alarms in line with `desired`.
async fn apply(
    connection: &zbus::Connection,
    desired: &[Desired],
) -> zbus::Result<(usize, usize)> {
    let clock = zbus::Proxy::new(connection, CLOCK_NAME, CLOCK_PATH, CLOCK_NAME).await?;
    let state: String = clock.call("State", &()).await?;
    let existing = existing_alarms(&state);
    let plan = registry::plan(&existing, desired);

    for id in &plan.delete {
        let _: () = clock.call("DeleteAlarm", &(id.as_str())).await?;
    }
    for want in &plan.set {
        let alarm = serde_json::json!({
            "id": want.id,
            "time": want.time,
            "label": want.label,
            "days": serde_json::Value::Array(Vec::new()),
            "enabled": true,
            "fire_late": false,
            "on_date": want.on_date.to_string(),
            "payload": want.payload,
            "next_fire_at": serde_json::Value::Null,
        });
        let _: () = clock.call("SetAlarm", &(alarm.to_string())).await?;
    }
    Ok((plan.set.len(), plan.delete.len()))
}

/// The `(id, payload)` of every alarm the clock holds.
///
/// A malformed answer yields nothing, which makes the plan add its
/// registrations and remove none: the safe direction, since removing on a bad
/// read could delete an alarm that is still wanted.
fn existing_alarms(state_json: &str) -> Vec<(String, Option<String>)> {
    let Ok(state) = serde_json::from_str::<serde_json::Value>(state_json) else {
        return Vec::new();
    };
    state["alarms"]
        .as_array()
        .map(|alarms| {
            alarms
                .iter()
                .filter_map(|a| {
                    let id = a["id"].as_str()?.to_string();
                    let payload = a["payload"].as_str().map(str::to_string);
                    Some((id, payload))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The machine's own zone, which is what a floating or all-day time means.
fn local_zone() -> chrono_tz::Tz {
    std::fs::read_to_string("/etc/timezone")
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(chrono_tz::Tz::UTC)
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let calendar = Arc::new(Calendar { store: Mutex::new(arlen_calendar::Store::default()) });

    let connection = match zbus::connection::Builder::session()
        .and_then(|b| b.name(BUS_NAME))
        .and_then(|b| {
            b.serve_at(OBJECT_PATH, CalendarInterface { calendar: Arc::clone(&calendar) })
        }) {
        Ok(builder) => match builder.build().await {
            Ok(c) => c,
            Err(e) => {
                warn!("could not take {BUS_NAME}: {e}");
                std::process::exit(1);
            }
        },
        Err(e) => {
            warn!("could not build the calendar connection: {e}");
            std::process::exit(1);
        }
    };
    info!("calendar daemon serving {BUS_NAME}");

    // SIGTERM as well as ctrl_c: `systemctl stop` sends the first, and a daemon
    // that only listens for the second is killed wherever it stands. Nothing
    // here is mid-write, so the cost today is a log line rather than damage -
    // but the moment this daemon owns a store it would be the difference.
    let mut term = match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
        Ok(s) => s,
        Err(e) => {
            warn!("could not listen for SIGTERM: {e}");
            return;
        }
    };
    let mut ticks = tokio::time::interval(std::time::Duration::from_secs(RE_DERIVE_SECS));
    loop {
        tokio::select! {
            _ = ticks.tick() => re_derive(&calendar, &connection).await,
            _ = term.recv() => break,
            _ = tokio::signal::ctrl_c() => break,
        }
    }
    info!("shutting down");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_clock_answer_that_is_not_json_removes_nothing() {
        // The safe direction on a bad read: adding a registration twice is
        // harmless because the id is derived, deleting one that is still wanted
        // is not.
        assert!(existing_alarms("not json").is_empty());
        assert!(existing_alarms(r#"{"alarms":"nope"}"#).is_empty());
    }

    #[test]
    fn an_alarm_without_a_payload_reads_back_as_one_nobody_registered() {
        let state = r#"{"alarms":[{"id":"morning","payload":null},
            {"id":"cal:2026-08-26:a","payload":"{\"source\":\"calendar\"}"}]}"#;
        let got = existing_alarms(state);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0], ("morning".to_string(), None));
        assert!(got[1].1.as_deref().is_some_and(|p| p.contains("calendar")));
    }
}

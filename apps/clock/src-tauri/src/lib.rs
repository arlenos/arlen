//! The Arlen clock's Tauri shell.
//!
//! **This app is a view and owns nothing** (`clock-app.md` §1). The alarms, the
//! timers, the focus session and the stopwatch live in the clock daemon; this
//! window renders them and may be closed at any time without changing anything.
//! An alarm that stops existing when a window closes is not an alarm, which is
//! why the state is not here.
//!
//! So every command below is a forward to `org.arlen.Clock1` and nothing else.
//! There is no cache, no optimistic write that outlives the call and no local
//! copy that could disagree with the daemon: each command hands the request over
//! and the frontend re-reads the state afterwards. That is what keeps "the
//! daemon owns it" true in the code rather than only in the design.
//!
//! **A failure is returned, never smoothed over.** When the daemon is not
//! running the call errors, the frontend's own catch path engages, and the app
//! runs on its fixture with `clockMocked` set - which says on screen that
//! nothing is keeping time. A command that swallowed the error would leave a
//! window that looks connected while no alarm will ever ring.

use serde_json::Value;

/// The daemon this window is a view onto.
const CLOCK_SERVICE: &str = "org.arlen.Clock1";
/// Where its interface lives.
const CLOCK_PATH: &str = "/org/arlen/Clock1";

/// A proxy to the clock daemon.
///
/// Connected per call rather than held: these calls are paced by a person
/// pressing things, not by the render loop, so the connection cost buys
/// simplicity and removes any question of a stale handle after the daemon
/// restarts. The 1 Hz tick the surfaces run on derives from anchors already in
/// hand and does not come through here.
async fn clock() -> Result<zbus::Proxy<'static>, String> {
    let conn = zbus::Connection::session()
        .await
        .map_err(|e| format!("session bus: {e}"))?;
    zbus::Proxy::new(&conn, CLOCK_SERVICE, CLOCK_PATH, CLOCK_SERVICE)
        .await
        .map_err(|e| format!("clock daemon unavailable: {e}"))
}

/// The marker a command returns when the clock daemon is not running.
///
/// A token rather than a sentence: the wording belongs to the page, where it is
/// translated. The same shape the knowledge app uses for its own daemon.
pub const NOT_RUNNING: &str = "clock-daemon-not-running";

/// Is anything serving `org.arlen.Clock1`?
///
/// Not D-Bus-activatable (no `.service` file under `dbus-1/services`), so an
/// absent daemon stays absent and a call to it simply fails - which the app used
/// to render as "cannot read your saved clock data right now". True, and the
/// wrong half of the truth: on a machine where nobody started the service there
/// is nothing to read and nothing wrong, and telling a person their data cannot
/// be read sends them looking for a fault that is not there.
async fn daemon_running() -> bool {
    let Ok(conn) = zbus::Connection::session().await else {
        return false;
    };
    let Ok(dbus) = zbus::fdo::DBusProxy::new(&conn).await else {
        return false;
    };
    let Ok(name) = CLOCK_SERVICE.try_into() else {
        return false;
    };
    dbus.name_has_owner(name).await.unwrap_or(false)
}

/// Turn a call failure into the not-running marker when that is what it is.
async fn explain(e: String) -> String {
    if daemon_running().await {
        e
    } else {
        NOT_RUNNING.to_string()
    }
}

/// Call a method that returns nothing.
async fn tell<A>(method: &str, args: &A) -> Result<(), String>
where
    A: serde::Serialize + zbus::zvariant::DynamicType,
{
    clock()
        .await?
        .call::<_, _, ()>(method, args)
        .await
        .map_err(|e| format!("{method}: {e}"))
}

/// Everything the app renders, in one read.
///
/// The daemon serves it as JSON and this parses it into a value, so the
/// frontend receives the object its types describe rather than a string it would
/// have to parse itself. Parsing here also means a malformed answer fails at the
/// boundary instead of somewhere inside a surface.
#[tauri::command]
async fn clock_state() -> Result<Value, String> {
    let raw: String = match clock().await {
        Ok(proxy) => match proxy.call("State", &()).await {
            Ok(raw) => raw,
            Err(e) => return Err(explain(format!("State: {e}")).await),
        },
        Err(e) => return Err(explain(e).await),
    };
    serde_json::from_str(&raw).map_err(|e| format!("clock state is not JSON: {e}"))
}

/// Create or update one alarm. The daemon computes `next_fire_at`.
#[tauri::command]
async fn clock_set_alarm(alarm: Value) -> Result<(), String> {
    tell("SetAlarm", &(alarm.to_string(),)).await
}

/// Arm or disarm one alarm.
#[tauri::command]
async fn clock_toggle_alarm(id: String, enabled: bool) -> Result<(), String> {
    tell("ToggleAlarm", &(id, enabled)).await
}

/// Delete one alarm.
#[tauri::command]
async fn clock_delete_alarm(id: String) -> Result<(), String> {
    tell("DeleteAlarm", &(id,)).await
}

/// Start a countdown.
///
/// The daemon mints the id, so two windows starting a timer in the same moment
/// cannot collide on one.
#[tauri::command]
async fn clock_timer_start(duration_ms: i64) -> Result<String, String> {
    clock()
        .await?
        .call("TimerStart", &(duration_ms,))
        .await
        .map_err(|e| format!("TimerStart: {e}"))
}

/// Pause or resume one timer.
#[tauri::command]
async fn clock_timer_pause(id: String, paused: bool) -> Result<(), String> {
    tell("TimerPause", &(id, paused)).await
}

/// Cancel one timer.
#[tauri::command]
async fn clock_timer_cancel(id: String) -> Result<(), String> {
    tell("TimerCancel", &(id,)).await
}

/// Begin a focus session.
#[tauri::command]
async fn clock_focus_start() -> Result<(), String> {
    tell("FocusStart", &()).await
}

/// End one early.
#[tauri::command]
async fn clock_focus_end() -> Result<(), String> {
    tell("FocusEnd", &()).await
}

/// Change the focus configuration.
#[tauri::command]
async fn clock_focus_config(config: Value) -> Result<(), String> {
    tell("FocusConfig", &(config.to_string(),)).await
}

/// Start or resume the stopwatch.
#[tauri::command]
async fn clock_stopwatch_start() -> Result<(), String> {
    tell("StopwatchStart", &()).await
}

/// Pause it.
#[tauri::command]
async fn clock_stopwatch_pause() -> Result<(), String> {
    tell("StopwatchPause", &()).await
}

/// Record a lap.
#[tauri::command]
async fn clock_stopwatch_lap() -> Result<(), String> {
    tell("StopwatchLap", &()).await
}

/// Back to zero.
#[tauri::command]
async fn clock_stopwatch_reset() -> Result<(), String> {
    tell("StopwatchReset", &()).await
}

/// Show a city, by its id in the shared dataset.
///
/// Only the id crosses: the daemon resolves the name and the zone, because a
/// city whose name arrived from a caller is a city nothing can check.
#[tauri::command]
async fn clock_world_add(id: String) -> Result<(), String> {
    tell("WorldAdd", &(id,)).await
}

/// Stop showing one.
#[tauri::command]
async fn clock_world_remove(id: String) -> Result<(), String> {
    tell("WorldRemove", &(id,)).await
}

/// A structured log line from the frontend into the app's stdout (the shell has
/// no devtools console an operator can open).
#[tauri::command]
fn frontend_log(level: String, message: String) {
    match level.as_str() {
        "error" => log::error!("[frontend] {message}"),
        "warn" => log::warn!("[frontend] {message}"),
        _ => log::info!("[frontend] {message}"),
    }
}

/// Start the window.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Dependencies at warn, this app at info. A blanket `info` also turns on
    // zbus, which logs D-Bus handshake frames WITH their message bytes - and a
    // message body is user content: file paths, query strings, notification
    // text. At info that lands in the journal, readable by anything with
    // journal access and covered by no capability grant, which undoes in a log
    // line what the graph's scoping is for. A byte trace stays available as
    // `RUST_LOG=zbus=trace`, deliberately, rather than by default.
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn,arlen_clock_lib=info")).init();
    tauri::Builder::default()
        .plugin(tauri_plugin_arlen_shell::init())
        .invoke_handler(tauri::generate_handler![
            frontend_log,
            clock_state,
            clock_set_alarm,
            clock_toggle_alarm,
            clock_delete_alarm,
            clock_timer_start,
            clock_timer_pause,
            clock_timer_cancel,
            clock_focus_start,
            clock_focus_end,
            clock_focus_config,
            clock_stopwatch_start,
            clock_stopwatch_pause,
            clock_stopwatch_lap,
            clock_stopwatch_reset,
            clock_world_add,
            clock_world_remove,
        ])
        .run(tauri::generate_context!())
        .expect("error while running the clock");
}

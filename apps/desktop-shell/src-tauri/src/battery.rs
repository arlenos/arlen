/// Battery status via UPower D-Bus.
///
/// Queries `org.freedesktop.UPower` for the display device's battery
/// state. Returns `None` on desktop PCs without a battery.

use std::sync::{Mutex, OnceLock};

use serde::Serialize;
use zbus::blocking::Connection;
use zbus::names::{BusName, InterfaceName};
use zbus::zvariant::{ObjectPath, OwnedValue, Value};

/// Battery status snapshot.
#[derive(Clone, Serialize)]
pub struct BatteryStatus {
    /// Battery level 0-100.
    pub percentage: u8,
    /// True if the battery is charging.
    pub charging: bool,
    /// Estimated minutes remaining (to empty or to full), if available.
    pub time_remaining_minutes: Option<u32>,
}

/// Cached system-bus connection. Previously the command opened a new
/// connection on every call (UPower fires PropertiesChanged every
/// ~30s when charging, and the BatteryIndicator re-queries on each
/// event), which added 20-80ms of sync authentication + socket
/// handshake per call on a Tauri worker thread. The connection stays
/// valid for the process lifetime; if it's ever lost we invalidate
/// the cache on error so the next call reconnects.
static CONN: OnceLock<Mutex<Option<Connection>>> = OnceLock::new();

fn conn_cache() -> &'static Mutex<Option<Connection>> {
    CONN.get_or_init(|| Mutex::new(None))
}

/// Fetch a system-bus connection, reusing the cached one when healthy.
/// The `Mutex` guards the swap; the returned `Connection` is cheap to
/// clone (Arc inside) so callers don't hold the lock across D-Bus I/O.
fn get_connection() -> Option<Connection> {
    let mut guard = conn_cache().lock().ok()?;
    if let Some(ref c) = *guard {
        return Some(c.clone());
    }
    let c = Connection::system().ok()?;
    *guard = Some(c.clone());
    Some(c)
}

/// Drop the cached connection. Called from a method's error path so
/// the next call rebuilds it (e.g. after a dbus-daemon restart).
fn invalidate_connection() {
    if let Ok(mut guard) = conn_cache().lock() {
        *guard = None;
    }
}

/// Returns the current battery status, or None if no battery is
/// present. Async + `spawn_blocking` to keep the sync zbus calls off
/// the tokio runtime — previously this was a sync `pub fn` that
/// pinned a Tauri worker thread for ~50ms each call.
#[tauri::command]
pub async fn get_battery_status() -> Option<BatteryStatus> {
    tauri::async_runtime::spawn_blocking(get_battery_status_blocking)
        .await
        .ok()
        .flatten()
}

fn get_battery_status_blocking() -> Option<BatteryStatus> {
    let conn = get_connection()?;

    let path = "/org/freedesktop/UPower/devices/DisplayDevice";
    let iface = "org.freedesktop.UPower.Device";

    let percentage = match get_property_f64(&conn, path, iface, "Percentage") {
        Some(p) => p,
        None => {
            // Missing property after a successful connect is almost
            // always the dbus-daemon going away. Drop the cached
            // connection so the next call can retry from clean state.
            invalidate_connection();
            return None;
        }
    };
    let state = get_property_u32(&conn, path, iface, "State")?;

    // UPower State enum: 0=Unknown, 1=Charging, 2=Discharging,
    // 3=Empty, 4=FullyCharged, 5=PendingCharge, 6=PendingDischarge
    let charging = state == 1 || state == 5;

    let time_secs = if charging {
        get_property_i64(&conn, path, iface, "TimeToFull").unwrap_or(0)
    } else {
        get_property_i64(&conn, path, iface, "TimeToEmpty").unwrap_or(0)
    };

    let time_remaining_minutes = if time_secs > 0 {
        Some((time_secs / 60) as u32)
    } else {
        None
    };

    // If percentage is 0 and state is unknown, likely no real battery.
    if percentage == 0.0 && state == 0 {
        return None;
    }

    Some(BatteryStatus {
        percentage: percentage.round().clamp(0.0, 100.0) as u8,
        charging,
        time_remaining_minutes,
    })
}

/// Reads a D-Bus property as an OwnedValue.
fn get_property(conn: &Connection, path: &str, iface: &str, prop: &str) -> Option<OwnedValue> {
    let bus: BusName = "org.freedesktop.UPower".try_into().ok()?;
    let obj: ObjectPath = path.try_into().ok()?;
    let props_iface: InterfaceName = "org.freedesktop.DBus.Properties".try_into().ok()?;
    let msg = conn.call_method(Some(&bus), &obj, Some(&props_iface), "Get", &(iface, prop)).ok()?;
    let body = msg.body();
    let variant: OwnedValue = body.deserialize().ok()?;
    Some(variant)
}

fn get_property_f64(conn: &Connection, path: &str, iface: &str, prop: &str) -> Option<f64> {
    let v = get_property(conn, path, iface, prop)?;
    match &*v {
        Value::F64(f) => Some(*f),
        Value::Value(inner) => {
            if let Value::F64(f) = &**inner { Some(*f) } else { None }
        }
        _ => None,
    }
}

fn get_property_u32(conn: &Connection, path: &str, iface: &str, prop: &str) -> Option<u32> {
    let v = get_property(conn, path, iface, prop)?;
    match &*v {
        Value::U32(n) => Some(*n),
        Value::Value(inner) => {
            if let Value::U32(n) = &**inner { Some(*n) } else { None }
        }
        _ => None,
    }
}

fn get_property_i64(conn: &Connection, path: &str, iface: &str, prop: &str) -> Option<i64> {
    let v = get_property(conn, path, iface, prop)?;
    match &*v {
        Value::I64(n) => Some(*n),
        Value::Value(inner) => {
            if let Value::I64(n) = &**inner { Some(*n) } else { None }
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// D-Bus signal monitor
// ---------------------------------------------------------------------------

/// Start monitoring UPower D-Bus signals for battery state changes.
///
/// Emits `battery-changed` Tauri events on PropertiesChanged signals.
pub fn start_monitor(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        if let Err(e) = run_battery_monitor(app).await {
            log::warn!("battery: monitor failed: {e}");
        }
    });
}

async fn run_battery_monitor(app: tauri::AppHandle) -> Result<(), zbus::Error> {
    use futures_util::StreamExt;
    use tauri::Emitter;

    let conn = zbus::Connection::system().await?;

    let proxy = zbus::Proxy::new(
        &conn,
        "org.freedesktop.UPower",
        "/org/freedesktop/UPower/devices/DisplayDevice",
        "org.freedesktop.DBus.Properties",
    )
    .await?;

    let mut stream = proxy.receive_all_signals().await?;

    log::info!("battery: signal monitor started");

    while let Some(_signal) = stream.next().await {
        let _ = app.emit("battery-changed", ());
    }

    Ok(())
}

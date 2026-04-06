/// Battery status via UPower D-Bus.
///
/// Queries `org.freedesktop.UPower` for the display device's battery
/// state. Returns `None` on desktop PCs without a battery.

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

/// Returns the current battery status, or None if no battery is present.
#[tauri::command]
pub fn get_battery_status() -> Option<BatteryStatus> {
    let conn = Connection::system().ok()?;

    let path = "/org/freedesktop/UPower/devices/DisplayDevice";
    let iface = "org.freedesktop.UPower.Device";

    let percentage = get_property_f64(&conn, path, iface, "Percentage")?;
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

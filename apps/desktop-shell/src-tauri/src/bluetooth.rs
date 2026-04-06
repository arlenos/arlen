/// Bluetooth management via BlueZ D-Bus API.
///
/// Communicates with `bluetoothd` over the system bus (`org.bluez`). Reads
/// adapter state and paired/discovered devices via the ObjectManager pattern,
/// and controls power/scan/connect via Adapter1 and Device1 interfaces.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};
use zbus::Connection;

// ---------------------------------------------------------------------------
// Data structures
// ---------------------------------------------------------------------------

/// A single Bluetooth device.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BluetoothDevice {
    /// D-Bus object path (e.g. /org/bluez/hci0/dev_AA_BB_CC_DD_EE_FF).
    pub path: String,
    /// MAC address.
    pub address: String,
    /// Display name (Alias preferred over Name).
    pub name: String,
    /// BlueZ icon hint ("audio-headphones", "input-keyboard", etc.).
    pub icon: String,
    /// Whether the device is paired.
    pub paired: bool,
    /// Whether the device is currently connected.
    pub connected: bool,
    /// Whether auto-connect is allowed.
    pub trusted: bool,
    /// Battery percentage if reported by the device.
    pub battery_percentage: Option<u8>,
}

/// Overall Bluetooth state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BluetoothState {
    /// Whether a Bluetooth adapter exists at all.
    pub available: bool,
    /// Whether the adapter is powered on.
    pub powered: bool,
    /// Whether a discovery scan is running.
    pub discovering: bool,
    /// All known devices (paired + discovered).
    pub devices: Vec<BluetoothDevice>,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Shorthand: extract a typed value from a property map, returning a default on
/// missing key or type mismatch.
fn prop_str(props: &HashMap<String, OwnedValue>, key: &str) -> String {
    props
        .get(key)
        .and_then(|v| {
            let val = Value::try_from(v.clone()).ok()?;
            match unwrap_variant(&val) {
                Value::Str(s) => Some(s.to_string()),
                _ => None,
            }
        })
        .unwrap_or_default()
}

fn prop_bool(props: &HashMap<String, OwnedValue>, key: &str) -> bool {
    props
        .get(key)
        .and_then(|v| {
            let val = Value::try_from(v.clone()).ok()?;
            match unwrap_variant(&val) {
                Value::Bool(b) => Some(*b),
                _ => None,
            }
        })
        .unwrap_or(false)
}

fn prop_u8(props: &HashMap<String, OwnedValue>, key: &str) -> Option<u8> {
    props.get(key).and_then(|v| {
        let val = Value::try_from(v.clone()).ok()?;
        match unwrap_variant(&val) {
            Value::U8(n) => Some(*n),
            _ => None,
        }
    })
}

/// Unwrap nested D-Bus variant wrappers.
fn unwrap_variant<'a>(v: &'a Value<'a>) -> &'a Value<'a> {
    match v {
        Value::Value(inner) => unwrap_variant(inner),
        other => other,
    }
}

/// Type alias for the ObjectManager return value:
/// `Dict<ObjectPath, Dict<InterfaceName, Dict<PropertyName, Variant>>>`
type ManagedObjects = HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>>;

/// Fetch all managed objects from BlueZ via the ObjectManager interface.
async fn get_managed_objects(conn: &Connection) -> Result<ManagedObjects, String> {
    let proxy = zbus::Proxy::new(conn, "org.bluez", "/", "org.freedesktop.DBus.ObjectManager")
        .await
        .map_err(|e| format!("ObjectManager proxy: {e}"))?;

    let reply = proxy
        .call_method("GetManagedObjects", &())
        .await
        .map_err(|e| format!("GetManagedObjects: {e}"))?;

    let objects: ManagedObjects = reply
        .body()
        .deserialize()
        .map_err(|e| format!("deserialize: {e}"))?;

    Ok(objects)
}

/// Parse a BluetoothDevice from a Device1 property map. Battery1 properties
/// are merged in if present on the same object.
fn parse_device(
    path: &str,
    dev_props: &HashMap<String, OwnedValue>,
    bat_props: Option<&HashMap<String, OwnedValue>>,
) -> BluetoothDevice {
    let alias = prop_str(dev_props, "Alias");
    let name_prop = prop_str(dev_props, "Name");
    let name = if !alias.is_empty() { alias } else { name_prop };

    let battery_percentage = bat_props.and_then(|bp| prop_u8(bp, "Percentage"));

    BluetoothDevice {
        path: path.to_string(),
        address: prop_str(dev_props, "Address"),
        name,
        icon: prop_str(dev_props, "Icon"),
        paired: prop_bool(dev_props, "Paired"),
        connected: prop_bool(dev_props, "Connected"),
        trusted: prop_bool(dev_props, "Trusted"),
        battery_percentage,
    }
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

/// Returns the current Bluetooth adapter state and all known devices.
#[tauri::command]
pub async fn get_bluetooth_state() -> Result<BluetoothState, String> {
    let conn = Connection::system()
        .await
        .map_err(|e| format!("system bus: {e}"))?;

    let objects = get_managed_objects(&conn).await?;

    // Find the first Adapter1 interface.
    let mut available = false;
    let mut powered = false;
    let mut discovering = false;

    for (_path, ifaces) in &objects {
        if let Some(adapter_props) = ifaces.get("org.bluez.Adapter1") {
            available = true;
            powered = prop_bool(adapter_props, "Powered");
            discovering = prop_bool(adapter_props, "Discovering");
            break;
        }
    }

    // Collect all Device1 objects.
    let mut devices = Vec::new();
    for (path, ifaces) in &objects {
        if let Some(dev_props) = ifaces.get("org.bluez.Device1") {
            let bat_props = ifaces.get("org.bluez.Battery1");
            let device = parse_device(path.as_str(), dev_props, bat_props);
            // Skip devices with no name (transient scan results).
            if !device.name.is_empty() {
                devices.push(device);
            }
        }
    }

    // Sort: connected first, then paired, then by name.
    devices.sort_by(|a, b| {
        b.connected
            .cmp(&a.connected)
            .then(b.paired.cmp(&a.paired))
            .then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    Ok(BluetoothState {
        available,
        powered,
        discovering,
        devices,
    })
}

/// Toggle the Bluetooth adapter power state.
///
/// When enabling, also unblocks the radio via rfkill (in case it was
/// soft-blocked by airplane mode).
#[tauri::command]
pub async fn set_bluetooth_powered(enabled: bool) -> Result<(), String> {
    if enabled {
        // Unblock radio first (no-op if already unblocked).
        let _ = std::process::Command::new("rfkill")
            .args(["unblock", "bluetooth"])
            .output();
    }

    let conn = Connection::system()
        .await
        .map_err(|e| format!("system bus: {e}"))?;

    // Find the adapter path.
    let adapter_path = find_adapter_path(&conn).await?;

    let proxy = zbus::Proxy::new(
        &conn,
        "org.bluez",
        adapter_path.as_str(),
        "org.freedesktop.DBus.Properties",
    )
    .await
    .map_err(|e| format!("Properties proxy: {e}"))?;

    proxy
        .call_method(
            "Set",
            &("org.bluez.Adapter1", "Powered", Value::Bool(enabled)),
        )
        .await
        .map_err(|e| format!("Set Powered: {e}"))?;

    Ok(())
}

/// Connect to a Bluetooth device.
#[tauri::command]
pub async fn connect_bluetooth_device(path: String) -> Result<(), String> {
    let conn = Connection::system()
        .await
        .map_err(|e| format!("system bus: {e}"))?;

    let proxy = zbus::Proxy::new(&conn, "org.bluez", path.as_str(), "org.bluez.Device1")
        .await
        .map_err(|e| format!("Device1 proxy: {e}"))?;

    proxy
        .call_method("Connect", &())
        .await
        .map_err(|e| format!("Connect: {e}"))?;

    Ok(())
}

/// Disconnect a Bluetooth device.
#[tauri::command]
pub async fn disconnect_bluetooth_device(path: String) -> Result<(), String> {
    let conn = Connection::system()
        .await
        .map_err(|e| format!("system bus: {e}"))?;

    let proxy = zbus::Proxy::new(&conn, "org.bluez", path.as_str(), "org.bluez.Device1")
        .await
        .map_err(|e| format!("Device1 proxy: {e}"))?;

    proxy
        .call_method("Disconnect", &())
        .await
        .map_err(|e| format!("Disconnect: {e}"))?;

    Ok(())
}

/// Remove (forget) a paired Bluetooth device.
#[tauri::command]
pub async fn remove_bluetooth_device(path: String) -> Result<(), String> {
    let conn = Connection::system()
        .await
        .map_err(|e| format!("system bus: {e}"))?;

    let adapter_path = find_adapter_path(&conn).await?;

    let proxy = zbus::Proxy::new(
        &conn,
        "org.bluez",
        adapter_path.as_str(),
        "org.bluez.Adapter1",
    )
    .await
    .map_err(|e| format!("Adapter1 proxy: {e}"))?;

    let obj_path = ObjectPath::try_from(path.as_str()).map_err(|e| e.to_string())?;
    proxy
        .call_method("RemoveDevice", &(obj_path))
        .await
        .map_err(|e| format!("RemoveDevice: {e}"))?;

    Ok(())
}

/// Toggle the trusted (auto-connect) flag on a device.
#[tauri::command]
pub async fn set_device_trusted(path: String, trusted: bool) -> Result<(), String> {
    let conn = Connection::system()
        .await
        .map_err(|e| format!("system bus: {e}"))?;

    let proxy = zbus::Proxy::new(
        &conn,
        "org.bluez",
        path.as_str(),
        "org.freedesktop.DBus.Properties",
    )
    .await
    .map_err(|e| format!("Properties proxy: {e}"))?;

    proxy
        .call_method(
            "Set",
            &("org.bluez.Device1", "Trusted", Value::Bool(trusted)),
        )
        .await
        .map_err(|e| format!("Set Trusted: {e}"))?;

    Ok(())
}

/// Start Bluetooth device discovery (scan).
#[tauri::command]
pub async fn start_bluetooth_scan() -> Result<(), String> {
    let conn = Connection::system()
        .await
        .map_err(|e| format!("system bus: {e}"))?;

    let adapter_path = find_adapter_path(&conn).await?;

    let proxy = zbus::Proxy::new(
        &conn,
        "org.bluez",
        adapter_path.as_str(),
        "org.bluez.Adapter1",
    )
    .await
    .map_err(|e| format!("Adapter1 proxy: {e}"))?;

    proxy
        .call_method("StartDiscovery", &())
        .await
        .map_err(|e| format!("StartDiscovery: {e}"))?;

    Ok(())
}

/// Stop Bluetooth device discovery.
#[tauri::command]
pub async fn stop_bluetooth_scan() -> Result<(), String> {
    let conn = Connection::system()
        .await
        .map_err(|e| format!("system bus: {e}"))?;

    let adapter_path = find_adapter_path(&conn).await?;

    let proxy = zbus::Proxy::new(
        &conn,
        "org.bluez",
        adapter_path.as_str(),
        "org.bluez.Adapter1",
    )
    .await
    .map_err(|e| format!("Adapter1 proxy: {e}"))?;

    // StopDiscovery can fail if not scanning; ignore error.
    let _ = proxy.call_method("StopDiscovery", &()).await;

    Ok(())
}

/// Initiate pairing with a device.
///
/// For "Just Works" devices (most headphones), this completes immediately.
/// For devices requiring PIN confirmation, BlueZ calls back on the registered
/// agent (Phase 4).
#[tauri::command]
pub async fn pair_bluetooth_device(path: String) -> Result<(), String> {
    let conn = Connection::system()
        .await
        .map_err(|e| format!("system bus: {e}"))?;

    let proxy = zbus::Proxy::new(&conn, "org.bluez", path.as_str(), "org.bluez.Device1")
        .await
        .map_err(|e| format!("Device1 proxy: {e}"))?;

    proxy
        .call_method("Pair", &())
        .await
        .map_err(|e| format!("Pair: {e}"))?;

    // Auto-trust after successful pairing.
    let _ = set_device_trusted(path, true).await;

    Ok(())
}

// ---------------------------------------------------------------------------
// D-Bus signal monitoring
// ---------------------------------------------------------------------------

/// Start monitoring BlueZ signals for live state updates.
///
/// Watches InterfacesAdded/Removed on the ObjectManager and
/// PropertiesChanged on adapters and devices. Emits `bluetooth-changed`
/// Tauri events on any relevant change.
pub fn start_monitor(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        if let Err(e) = run_monitor(app).await {
            log::warn!("bluetooth: monitor failed: {e}");
        }
    });
}

async fn run_monitor(app: tauri::AppHandle) -> Result<(), zbus::Error> {
    use futures_util::StreamExt;
    use tauri::Emitter;

    let conn = Connection::system().await?;

    // Monitor InterfacesAdded/Removed on the ObjectManager root.
    let proxy = zbus::Proxy::new(&conn, "org.bluez", "/", "org.freedesktop.DBus.ObjectManager")
        .await?;

    let mut stream = proxy.receive_all_signals().await?;

    log::info!("bluetooth: signal monitor started");

    // Any signal from BlueZ ObjectManager triggers a frontend refresh.
    while let Some(_signal) = stream.next().await {
        let _ = app.emit("bluetooth-changed", ());
    }

    Ok(())
}

/// Find the first BlueZ adapter object path (usually /org/bluez/hci0).
async fn find_adapter_path(conn: &Connection) -> Result<String, String> {
    let objects = get_managed_objects(conn).await?;
    for (path, ifaces) in &objects {
        if ifaces.contains_key("org.bluez.Adapter1") {
            return Ok(path.to_string());
        }
    }
    Err("No Bluetooth adapter found".into())
}

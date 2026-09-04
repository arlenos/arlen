//! The bus half: what NetworkManager and BlueZ say about this machine's radios,
//! and the two one-click remediations that write back.
//!
//! Everything here is allowed to fail. A missing NetworkManager, a machine with
//! no Wi-Fi card, a BlueZ that is not running: each leaves its field unset, and
//! [`crate::read::postures`] turns an unset field into a line that says so. The
//! sentinel is a privacy page, so "we could not look" is a result it has to be
//! able to report rather than a reason to show nothing.
//!
//! THE MAC KEY IS THE STRING ONE. NetworkManager's D-Bus settings dictionary
//! carries the modern policy as `assigned-mac-address` (a string that may be
//! `permanent`, `preserve`, `random`, `stable` or a literal address) beside a
//! deprecated `cloned-mac-address` byte array kept for old clients. Reading or
//! writing the byte array is the documented way to get this wrong, so the string
//! is what is read first and the only thing written.

use std::collections::HashMap;

use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};
use zbus::{Connection, Proxy};

use crate::read::{privacy_from_main_conf, Readings, BLUEZ_MAIN_CONF};

const NM: &str = "org.freedesktop.NetworkManager";
const NM_PATH: &str = "/org/freedesktop/NetworkManager";
const NM_SETTINGS_PATH: &str = "/org/freedesktop/NetworkManager/Settings";
const WIFI_SETTING: &str = "802-11-wireless";
/// NetworkManager's device type for Wi-Fi.
const NM_DEVICE_TYPE_WIFI: u32 = 2;

/// One saved connection's settings, as D-Bus hands them over.
type Settings = HashMap<String, HashMap<String, OwnedValue>>;

fn as_bool(v: &OwnedValue) -> Option<bool> {
    match Value::from(v.clone()) {
        Value::Bool(b) => Some(b),
        _ => None,
    }
}

fn as_string(v: &OwnedValue) -> Option<String> {
    match Value::from(v.clone()) {
        Value::Str(s) => Some(s.to_string()),
        _ => None,
    }
}

/// The MAC policy a saved profile declares, preferring the string key.
#[must_use]
pub fn mac_policy_of(settings: &Settings) -> Option<String> {
    let wifi = settings.get(WIFI_SETTING)?;
    wifi.get("assigned-mac-address").and_then(as_string)
}

/// Whether a saved profile is a Wi-Fi profile.
#[must_use]
pub fn is_wifi(settings: &Settings) -> bool {
    settings.contains_key(WIFI_SETTING)
}

/// Whether a saved profile is marked hidden, which directed-probes its name
/// wherever the machine goes.
#[must_use]
pub fn is_hidden(settings: &Settings) -> bool {
    settings
        .get(WIFI_SETTING)
        .and_then(|w| w.get("hidden"))
        .and_then(as_bool)
        .unwrap_or(false)
}

/// Whether a saved profile sends the machine's hostname over DHCP.
///
/// NetworkManager's default is to send it, so an absent key means yes. That
/// default is the leak: the same hostname handed to every network is a stable
/// identifier no MAC randomization hides.
#[must_use]
pub fn sends_hostname(settings: &Settings) -> bool {
    settings
        .get("ipv4")
        .and_then(|v| v.get("dhcp-send-hostname"))
        .and_then(as_bool)
        .unwrap_or(true)
}

/// Read every surface. Never fails as a whole; each part that cannot be read is
/// left unset.
pub async fn read_host() -> Readings {
    let mut r = Readings::default();
    if let Ok(conn) = Connection::system().await {
        read_network(&conn, &mut r).await;
        read_bluetooth(&conn, &mut r).await;
    }
    r.ble_privacy = std::fs::read_to_string(BLUEZ_MAIN_CONF)
        .ok()
        .as_deref()
        .and_then(privacy_from_main_conf);
    r
}

/// The active Wi-Fi interface's addresses, and what the saved profiles declare.
async fn read_network(conn: &Connection, r: &mut Readings) {
    if let Ok(nm) = Proxy::new(conn, NM, NM_PATH, NM).await {
        if let Ok(devices) = nm.call::<_, _, Vec<OwnedObjectPath>>("GetDevices", &()).await {
            for path in devices {
                let Ok(dev) = Proxy::new(conn, NM, path.clone(), format!("{NM}.Device")).await
                else {
                    continue;
                };
                if dev.get_property::<u32>("DeviceType").await.ok() != Some(NM_DEVICE_TYPE_WIFI) {
                    continue;
                }
                let Ok(wifi) =
                    Proxy::new(conn, NM, path, format!("{NM}.Device.Wireless")).await
                else {
                    continue;
                };
                // Both or neither: comparing an address that read against one
                // that did not would answer "they differ" and report a machine
                // as protected on the strength of a failed read.
                let active = wifi.get_property::<String>("HwAddress").await.ok();
                let permanent = wifi.get_property::<String>("PermHwAddress").await.ok();
                if let (Some(a), Some(p)) = (active, permanent) {
                    r.wifi_active_mac = Some(a);
                    r.wifi_permanent_mac = Some(p);
                    break;
                }
            }
        }
    }

    let Ok(settings) = Proxy::new(conn, NM, NM_SETTINGS_PATH, format!("{NM}.Settings")).await
    else {
        return;
    };
    let Ok(connections) = settings
        .call::<_, _, Vec<OwnedObjectPath>>("ListConnections", &())
        .await
    else {
        return;
    };
    // The list read, so from here on an empty result means this machine has no
    // saved Wi-Fi networks rather than that nobody could look.
    let mut hidden = false;
    let mut hostname = false;
    for path in connections {
        let Ok(c) = Proxy::new(conn, NM, path, format!("{NM}.Settings.Connection")).await else {
            continue;
        };
        let Ok(s) = c.call::<_, _, Settings>("GetSettings", &()).await else {
            continue;
        };
        if !is_wifi(&s) {
            continue;
        }
        r.saved_mac_policies.push(mac_policy_of(&s));
        hidden |= is_hidden(&s);
        hostname |= sends_hostname(&s);
    }
    r.any_hidden_network = Some(hidden);
    r.sends_dhcp_hostname = Some(hostname);
}

/// The Bluetooth adapter's discoverable and pairable flags.
async fn read_bluetooth(conn: &Connection, r: &mut Readings) {
    let Ok(root) = Proxy::new(conn, "org.bluez", "/", "org.freedesktop.DBus.ObjectManager").await
    else {
        return;
    };
    type Managed = HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>>;
    let Ok(objects) = root.call::<_, _, Managed>("GetManagedObjects", &()).await else {
        return;
    };
    for (_, ifaces) in objects {
        let Some(adapter) = ifaces.get("org.bluez.Adapter1") else {
            continue;
        };
        r.bluetooth_discoverable = adapter.get("Discoverable").and_then(as_bool);
        r.bluetooth_pairable = adapter.get("Pairable").and_then(as_bool);
        return;
    }
}

/// Stop the Bluetooth adapter being discoverable.
///
/// One of the two one-click-safe remediations: paired devices keep working, and
/// the change is a property write with nothing to undo but the same write again.
pub async fn stop_being_discoverable() -> Result<(), String> {
    let conn = Connection::system()
        .await
        .map_err(|e| format!("could not reach the system bus: {e}"))?;
    let root = Proxy::new(&conn, "org.bluez", "/", "org.freedesktop.DBus.ObjectManager")
        .await
        .map_err(|e| format!("could not reach BlueZ: {e}"))?;
    type Managed = HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>>;
    let objects = root
        .call::<_, _, Managed>("GetManagedObjects", &())
        .await
        .map_err(|e| format!("could not list the Bluetooth adapters: {e}"))?;
    for (path, ifaces) in objects {
        if !ifaces.contains_key("org.bluez.Adapter1") {
            continue;
        }
        let adapter = Proxy::new(&conn, "org.bluez", path, "org.bluez.Adapter1")
            .await
            .map_err(|e| format!("could not reach the Bluetooth adapter: {e}"))?;
        return adapter
            .set_property("Discoverable", false)
            .await
            .map_err(|e| format!("the Bluetooth adapter refused the change: {e}"));
    }
    Err("this machine has no Bluetooth adapter to quieten".to_string())
}

/// Make every saved Wi-Fi profile join with a per-network address.
///
/// The other one-click-safe remediation. `Update` replaces the whole settings
/// map, so each profile is read, its one key changed and the rest written back
/// verbatim; anything else would silently drop settings this daemon does not
/// know about. The running interface keeps the address it already has until the
/// connection is activated again, which is why the readout keeps the active-MAC
/// line separate from the saved-policy one.
pub async fn randomize_saved_macs() -> Result<(), String> {
    let conn = Connection::system()
        .await
        .map_err(|e| format!("could not reach the system bus: {e}"))?;
    let settings = Proxy::new(&conn, NM, NM_SETTINGS_PATH, format!("{NM}.Settings"))
        .await
        .map_err(|e| format!("could not reach NetworkManager: {e}"))?;
    let connections = settings
        .call::<_, _, Vec<OwnedObjectPath>>("ListConnections", &())
        .await
        .map_err(|e| format!("could not list the saved networks: {e}"))?;

    let mut changed = 0usize;
    let mut refused = 0usize;
    for path in connections {
        let Ok(c) = Proxy::new(&conn, NM, path, format!("{NM}.Settings.Connection")).await else {
            continue;
        };
        let Ok(mut s) = c.call::<_, _, Settings>("GetSettings", &()).await else {
            continue;
        };
        if !is_wifi(&s) {
            continue;
        }
        if mac_policy_of(&s).as_deref() == Some("stable") {
            continue;
        }
        let Ok(stable) = OwnedValue::try_from(Value::from("stable")) else {
            continue;
        };
        if let Some(wifi) = s.get_mut(WIFI_SETTING) {
            wifi.insert("assigned-mac-address".to_string(), stable);
        }
        match c.call::<_, _, ()>("Update", &(s,)).await {
            Ok(()) => changed += 1,
            Err(_) => refused += 1,
        }
    }
    if refused > 0 && changed == 0 {
        return Err("NetworkManager refused to change the saved networks".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wifi_settings(pairs: &[(&str, OwnedValue)]) -> Settings {
        let mut wifi = HashMap::new();
        for (k, v) in pairs {
            wifi.insert((*k).to_string(), v.clone());
        }
        let mut s = HashMap::new();
        s.insert(WIFI_SETTING.to_string(), wifi);
        s
    }

    fn owned(v: Value<'_>) -> OwnedValue {
        OwnedValue::try_from(v).unwrap()
    }

    #[test]
    fn the_string_key_is_the_one_read() {
        let s = wifi_settings(&[("assigned-mac-address", owned(Value::from("stable")))]);
        assert_eq!(mac_policy_of(&s).as_deref(), Some("stable"));
    }

    #[test]
    fn a_profile_with_no_policy_says_so_rather_than_claiming_a_default() {
        let s = wifi_settings(&[]);
        assert_eq!(mac_policy_of(&s), None);
    }

    #[test]
    fn a_profile_without_a_hostname_key_is_read_as_sending_one() {
        let s = wifi_settings(&[]);
        assert!(
            sends_hostname(&s),
            "NetworkManager sends it unless told not to, and that default is the leak"
        );
    }

    #[test]
    fn an_explicit_no_is_honoured() {
        let mut s = wifi_settings(&[]);
        let mut ipv4 = HashMap::new();
        ipv4.insert("dhcp-send-hostname".to_string(), owned(Value::from(false)));
        s.insert("ipv4".to_string(), ipv4);
        assert!(!sends_hostname(&s));
    }

    #[test]
    fn hidden_is_only_true_when_the_profile_says_so() {
        assert!(!is_hidden(&wifi_settings(&[])));
        assert!(is_hidden(&wifi_settings(&[(
            "hidden",
            owned(Value::from(true))
        )])));
    }

    #[test]
    fn a_wired_profile_is_not_a_wifi_profile() {
        let mut s: Settings = HashMap::new();
        s.insert("802-3-ethernet".to_string(), HashMap::new());
        assert!(!is_wifi(&s));
    }
}

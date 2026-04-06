/// Network status via nmcli (NetworkManager).
///
/// Reads the active connection type, name, and signal strength.

use serde::{Deserialize, Serialize};

/// Current network status.
#[derive(Clone, Serialize, Deserialize)]
pub struct NetworkStatus {
    /// "wifi", "ethernet", or "disconnected".
    pub connection_type: String,
    /// Whether any network connection is active.
    pub connected: bool,
    /// Connection name: SSID for WiFi, interface name for Ethernet.
    pub name: Option<String>,
    /// WiFi signal strength 0-100. None for Ethernet/disconnected.
    pub signal_strength: Option<u8>,
    /// Whether a VPN tunnel is active.
    pub vpn_active: bool,
}

/// Returns the current network status.
#[tauri::command]
pub fn get_network_status() -> Result<NetworkStatus, String> {
    let (conn_type, connected, name, signal) = parse_device_status()?;
    let vpn_active = check_vpn();

    Ok(NetworkStatus {
        connection_type: conn_type,
        connected,
        name,
        signal_strength: signal,
        vpn_active,
    })
}

/// Parses `nmcli -t -f TYPE,STATE,CONNECTION device` for the primary connection.
fn parse_device_status() -> Result<(String, bool, Option<String>, Option<u8>), String> {
    let output = std::process::Command::new("nmcli")
        .args(["-t", "-f", "TYPE,STATE,CONNECTION", "device"])
        .output()
        .map_err(|e| format!("nmcli not found: {e}"))?;

    if !output.status.success() {
        return Err("nmcli device failed".into());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Find the first connected wifi or ethernet device.
    let mut wifi_conn: Option<String> = None;
    let mut ethernet_conn: Option<String> = None;

    for line in stdout.lines() {
        let parts: Vec<&str> = line.split(':').collect();
        if parts.len() < 3 {
            continue;
        }
        let dev_type = parts[0];
        let state = parts[1];
        let connection = parts[2];

        if state == "connected" {
            match dev_type {
                "wifi" => {
                    wifi_conn = Some(connection.to_string());
                }
                "ethernet" => {
                    ethernet_conn = Some(connection.to_string());
                }
                _ => {}
            }
        }
    }

    // Prefer WiFi info (more interesting to show).
    if let Some(conn_name) = wifi_conn {
        let signal = get_wifi_signal(&conn_name);
        return Ok(("wifi".into(), true, Some(conn_name), signal));
    }

    if let Some(conn_name) = ethernet_conn {
        return Ok(("ethernet".into(), true, Some(conn_name), None));
    }

    Ok(("disconnected".into(), false, None, None))
}

/// Gets WiFi signal strength for the connected SSID.
fn get_wifi_signal(ssid: &str) -> Option<u8> {
    let output = std::process::Command::new("nmcli")
        .args(["-t", "-f", "IN-USE,SSID,SIGNAL", "dev", "wifi", "list"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let parts: Vec<&str> = line.split(':').collect();
        if parts.len() >= 3 && parts[0] == "*" {
            // Active connection indicated by '*' in IN-USE field.
            let signal: u8 = parts[2].parse().unwrap_or(0);
            return Some(signal);
        }
    }

    // Fallback: match by SSID name.
    for line in stdout.lines() {
        let parts: Vec<&str> = line.split(':').collect();
        if parts.len() >= 3 && parts[1] == ssid {
            let signal: u8 = parts[2].parse().unwrap_or(0);
            return Some(signal);
        }
    }

    None
}

/// A WiFi network visible in the area.
#[derive(Clone, Serialize, Deserialize)]
pub struct WifiNetwork {
    pub ssid: String,
    pub signal: u8,
    pub security: String,
    pub is_connected: bool,
    pub is_known: bool,
}

/// Returns visible WiFi networks, sorted by connected first then signal.
#[tauri::command]
pub fn get_wifi_networks() -> Result<Vec<WifiNetwork>, String> {
    // Trigger rescan (best-effort, non-blocking).
    let _ = std::process::Command::new("nmcli")
        .args(["dev", "wifi", "rescan"])
        .output();

    let output = std::process::Command::new("nmcli")
        .args(["-t", "-f", "SSID,SIGNAL,SECURITY,IN-USE", "dev", "wifi", "list"])
        .output()
        .map_err(|e| format!("nmcli not found: {e}"))?;

    if !output.status.success() {
        return Err("nmcli wifi list failed".into());
    }

    // Collect known connection names.
    let known: std::collections::HashSet<String> = std::process::Command::new("nmcli")
        .args(["-t", "-f", "NAME", "connection", "show"])
        .output()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .map(|s| s.to_string())
                .collect()
        })
        .unwrap_or_default();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut networks = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for line in stdout.lines() {
        let parts: Vec<&str> = line.split(':').collect();
        if parts.len() < 4 {
            continue;
        }
        let ssid = parts[0].to_string();
        if ssid.is_empty() || seen.contains(&ssid) {
            continue;
        }
        seen.insert(ssid.clone());

        networks.push(WifiNetwork {
            signal: parts[1].parse().unwrap_or(0),
            security: parts[2].to_string(),
            is_connected: parts[3] == "*",
            is_known: known.contains(&ssid),
            ssid,
        });
    }

    networks.sort_by(|a, b| {
        b.is_connected
            .cmp(&a.is_connected)
            .then(b.signal.cmp(&a.signal))
    });
    Ok(networks)
}

/// Connects to a known WiFi network by SSID.
#[tauri::command]
pub fn connect_wifi(ssid: String) -> Result<(), String> {
    let output = std::process::Command::new("nmcli")
        .args(["dev", "wifi", "connect", &ssid])
        .output()
        .map_err(|e| format!("nmcli connect failed: {e}"))?;

    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    Ok(())
}

/// Connects to a WiFi network with a password.
#[tauri::command]
pub fn connect_wifi_password(ssid: String, password: String) -> Result<(), String> {
    let output = std::process::Command::new("nmcli")
        .args(["dev", "wifi", "connect", &ssid, "password", &password])
        .output()
        .map_err(|e| format!("nmcli connect failed: {e}"))?;

    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    Ok(())
}

/// Disconnects WiFi by finding the active wifi device.
#[tauri::command]
pub fn disconnect_wifi() -> Result<(), String> {
    // Find the wifi device name.
    let output = std::process::Command::new("nmcli")
        .args(["-t", "-f", "DEVICE,TYPE,STATE", "device"])
        .output()
        .map_err(|e| format!("nmcli failed: {e}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let parts: Vec<&str> = line.split(':').collect();
        if parts.len() >= 3 && parts[1] == "wifi" && parts[2] == "connected" {
            let _ = std::process::Command::new("nmcli")
                .args(["dev", "disconnect", parts[0]])
                .output();
            return Ok(());
        }
    }
    Err("No connected wifi device found".into())
}

/// Returns whether airplane mode is active (all WiFi radios soft-blocked).
#[tauri::command]
pub fn get_airplane_mode() -> Result<bool, String> {
    let output = std::process::Command::new("rfkill")
        .args(["list", "wifi"])
        .output()
        .map_err(|e| format!("rfkill not found: {e}"))?;
    let text = String::from_utf8_lossy(&output.stdout);
    Ok(text.contains("Soft blocked: yes"))
}

/// Toggles airplane mode by blocking or unblocking all wireless radios.
#[tauri::command]
pub fn set_airplane_mode(enabled: bool) -> Result<(), String> {
    let action = if enabled { "block" } else { "unblock" };
    let status = std::process::Command::new("rfkill")
        .args([action, "all"])
        .status()
        .map_err(|e| format!("rfkill {action} failed: {e}"))?;
    if !status.success() {
        return Err(format!("rfkill {action} all returned non-zero"));
    }
    Ok(())
}

/// Checks if any VPN connection is active.
fn check_vpn() -> bool {
    let output = match std::process::Command::new("nmcli")
        .args(["-t", "-f", "TYPE,STATE", "con", "show", "--active"])
        .output()
    {
        Ok(o) => o,
        Err(_) => return false,
    };

    if !output.status.success() {
        return false;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout.lines().any(|line| {
        let parts: Vec<&str> = line.split(':').collect();
        parts.len() >= 2 && parts[0].contains("vpn") && parts[1] == "activated"
    })
}

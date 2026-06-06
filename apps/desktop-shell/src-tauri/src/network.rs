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
pub async fn get_network_status() -> Result<NetworkStatus, String> {
    let (conn_type, connected, name, signal) = parse_device_status().await?;
    let vpn_active = check_vpn().await;

    Ok(NetworkStatus {
        connection_type: conn_type,
        connected,
        name,
        signal_strength: signal,
        vpn_active,
    })
}

/// Parses `nmcli -t -f TYPE,STATE,CONNECTION device` for the primary connection.
async fn parse_device_status() -> Result<(String, bool, Option<String>, Option<u8>), String> {
    let output = tokio::process::Command::new("nmcli")
        .args(["-t", "-f", "TYPE,STATE,CONNECTION", "device"])
        .output()
        .await
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

/// Returns signal strength for the connected SSID, sourced from the
/// `WIFI_CACHE` populated by `get_wifi_networks`.
///
/// Previously this ran a synchronous `nmcli dev wifi list` of its own
/// — which on the first hover after shell start triggers a fresh RF
/// radio sweep (1-3s) and was the dominant cause of the first-hover
/// freeze. The cache is populated whenever `loadNetworks()` runs from
/// the popover (or the network monitor) so by the time
/// `get_network_status` is asked we usually already have a number.
/// Returns `None` on cache miss; the indicator handles that gracefully.
fn get_wifi_signal(ssid: &str) -> Option<u8> {
    let cached = get_wifi_cache()?;
    // Active connection has is_connected == true; fall back to ssid match.
    cached
        .iter()
        .find(|n| n.is_connected)
        .or_else(|| cached.iter().find(|n| n.ssid == ssid))
        .map(|n| n.signal)
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

/// Combined WiFi scan cooldown + result cache. The RF scan and the
/// nmcli subprocess calls are both skipped when the cache is fresh.
static WIFI_CACHE: std::sync::Mutex<Option<(std::time::Instant, Vec<WifiNetwork>)>> =
    std::sync::Mutex::new(None);
const WIFI_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(30);

/// Return the cached WiFi list if it is younger than 30 seconds.
fn get_wifi_cache() -> Option<Vec<WifiNetwork>> {
    let guard = WIFI_CACHE.lock().unwrap();
    match guard.as_ref() {
        Some((ts, list)) if ts.elapsed() < WIFI_CACHE_TTL => Some(list.clone()),
        _ => None,
    }
}

/// Invalidate the WiFi cache. Called after connect/disconnect/forget
/// so the next `get_wifi_networks` does a fresh fetch and the
/// `is_connected` flags reflect the new state. Without this, the
/// shell would show the OLD connected network in the available list
/// for up to 30 s after switching networks.
fn invalidate_wifi_cache() {
    *WIFI_CACHE.lock().unwrap() = None;
}

/// Store a fresh WiFi list in the cache.
fn set_wifi_cache(list: &[WifiNetwork]) {
    *WIFI_CACHE.lock().unwrap() = Some((std::time::Instant::now(), list.to_vec()));
}

/// Whether a new RF scan should be triggered. Only true when the
/// cache has expired.
fn should_rescan_wifi() -> bool {
    let guard = WIFI_CACHE.lock().unwrap();
    match guard.as_ref() {
        None => true,
        Some((ts, _)) if ts.elapsed() > WIFI_CACHE_TTL => true,
        _ => false,
    }
}

/// Returns visible WiFi networks, sorted by connected first then signal.
/// Results are cached for 30 seconds — within that window, no RF scan
/// and no nmcli subprocesses are spawned.
///
/// **Async on purpose.** Earlier this was a blocking sync `pub fn`,
/// which meant the very first popover open (with empty cache) parked
/// a Tauri worker thread for 1-5 s while NetworkManager finished a
/// fresh RF scan. The shell felt frozen because that worker is
/// shared with theme/window-list updates that the topbar polls on a
/// tight cadence. Switching to `pub async fn` plus
/// `tokio::process::Command` makes every nmcli invocation cooperate
/// with the runtime instead of stalling a thread.
#[tauri::command]
pub async fn get_wifi_networks() -> Result<Vec<WifiNetwork>, String> {
    // Return cached list if fresh.
    if let Some(cached) = get_wifi_cache() {
        return Ok(cached);
    }

    // Cache expired — trigger RF scan (best-effort, non-blocking).
    // `tokio::process::Command::spawn` returns immediately; we drop
    // the child and let NetworkManager publish results in its own
    // time. The user will see fresh results on the next poll.
    if should_rescan_wifi() {
        if let Ok(mut child) = tokio::process::Command::new("nmcli")
            .args(["dev", "wifi", "rescan"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
        {
            // Detach: don't await; let NetworkManager finish in the
            // background while we read whatever's already published.
            tokio::spawn(async move {
                let _ = child.wait().await;
            });
        }
    }

    let output = tokio::process::Command::new("nmcli")
        .args(["-t", "-f", "SSID,SIGNAL,SECURITY,IN-USE", "dev", "wifi", "list"])
        .output()
        .await
        .map_err(|e| format!("nmcli not found: {e}"))?;

    if !output.status.success() {
        return Err("nmcli wifi list failed".into());
    }

    // Collect known connection names. Async so the second nmcli
    // invocation also yields to the runtime rather than blocking.
    let known: std::collections::HashSet<String> =
        match tokio::process::Command::new("nmcli")
            .args(["-t", "-f", "NAME", "connection", "show"])
            .output()
            .await
        {
            Ok(o) => String::from_utf8_lossy(&o.stdout)
                .lines()
                .map(|s| s.to_string())
                .collect(),
            Err(_) => Default::default(),
        };

    let stdout = String::from_utf8_lossy(&output.stdout);
    // nmcli emits ONE row per BSSID (access point), so an SSID with
    // a mesh / dual-band setup produces multiple rows. We need to
    // keep the one with IN-USE="*" if any (the BSSID we're actually
    // connected to), otherwise the strongest signal. Dropping by
    // first-occurrence loses the connected flag whenever the active
    // BSSID isn't the first row — which then shows the active SSID
    // in the "Available Networks" list as if it were unconnected.
    use std::collections::HashMap;
    let mut by_ssid: HashMap<String, WifiNetwork> = HashMap::new();
    for line in stdout.lines() {
        let parts: Vec<&str> = line.split(':').collect();
        if parts.len() < 4 {
            continue;
        }
        let ssid = parts[0].to_string();
        if ssid.is_empty() {
            continue;
        }
        let candidate = WifiNetwork {
            signal: parts[1].parse().unwrap_or(0),
            security: parts[2].to_string(),
            is_connected: parts[3] == "*",
            is_known: known.contains(&ssid),
            ssid: ssid.clone(),
        };
        match by_ssid.get(&ssid) {
            Some(existing) => {
                // Prefer connected row; fall back to higher signal.
                let prefer_new = candidate.is_connected
                    || (!existing.is_connected && candidate.signal > existing.signal);
                if prefer_new {
                    by_ssid.insert(ssid, candidate);
                }
            }
            None => {
                by_ssid.insert(ssid, candidate);
            }
        }
    }
    let mut networks: Vec<WifiNetwork> = by_ssid.into_values().collect();

    networks.sort_by(|a, b| {
        b.is_connected
            .cmp(&a.is_connected)
            .then(b.signal.cmp(&a.signal))
    });

    set_wifi_cache(&networks);
    Ok(networks)
}

/// Connects to a known WiFi network by SSID.
#[tauri::command]
pub async fn connect_wifi(ssid: String) -> Result<(), String> {
    let output = tokio::process::Command::new("nmcli")
        .args(["dev", "wifi", "connect", &ssid])
        .output()
        .await
        .map_err(|e| format!("nmcli connect failed: {e}"))?;

    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    invalidate_wifi_cache();
    Ok(())
}

/// Connects to a WiFi network with a password.
#[tauri::command]
pub async fn connect_wifi_password(ssid: String, password: String) -> Result<(), String> {
    let output = tokio::process::Command::new("nmcli")
        .args(["dev", "wifi", "connect", &ssid, "password", &password])
        .output()
        .await
        .map_err(|e| format!("nmcli connect failed: {e}"))?;

    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    invalidate_wifi_cache();
    Ok(())
}

/// Disconnects WiFi by finding the active wifi device.
#[tauri::command]
pub async fn disconnect_wifi() -> Result<(), String> {
    // Find the wifi device name.
    let output = tokio::process::Command::new("nmcli")
        .args(["-t", "-f", "DEVICE,TYPE,STATE", "device"])
        .output()
        .await
        .map_err(|e| format!("nmcli failed: {e}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let parts: Vec<&str> = line.split(':').collect();
        if parts.len() >= 3 && parts[1] == "wifi" && parts[2] == "connected" {
            let result = tokio::process::Command::new("nmcli")
                .args(["dev", "disconnect", parts[0]])
                .output()
                .await
                .map_err(|e| format!("nmcli disconnect: {e}"))?;
            if !result.status.success() {
                return Err(String::from_utf8_lossy(&result.stderr)
                    .trim()
                    .to_string());
            }
            invalidate_wifi_cache();
            return Ok(());
        }
    }
    Err("No connected wifi device found".into())
}

/// Returns whether WiFi radio is enabled.
#[tauri::command]
pub async fn get_wifi_enabled() -> Result<bool, String> {
    let output = tokio::process::Command::new("nmcli")
        .args(["radio", "wifi"])
        .output()
        .await
        .map_err(|e| format!("nmcli radio wifi: {e}"))?;
    let text = String::from_utf8_lossy(&output.stdout);
    Ok(text.trim() == "enabled")
}

/// Enable or disable the WiFi radio via NetworkManager.
#[tauri::command]
pub async fn set_wifi_enabled(enabled: bool) -> Result<(), String> {
    let val = if enabled { "on" } else { "off" };
    let status = tokio::process::Command::new("nmcli")
        .args(["radio", "wifi", val])
        .status()
        .await
        .map_err(|e| format!("nmcli radio wifi {val}: {e}"))?;
    if !status.success() {
        return Err(format!("nmcli radio wifi {val} returned non-zero"));
    }
    Ok(())
}

/// Returns whether airplane mode is active (all WiFi radios soft-blocked).
#[tauri::command]
pub async fn get_airplane_mode() -> Result<bool, String> {
    let output = tokio::process::Command::new("rfkill")
        .args(["list", "wifi"])
        .output()
        .await
        .map_err(|e| format!("rfkill not found: {e}"))?;
    let text = String::from_utf8_lossy(&output.stdout);
    Ok(text.contains("Soft blocked: yes"))
}

/// Toggles airplane mode by blocking or unblocking all wireless radios.
#[tauri::command]
pub async fn set_airplane_mode(enabled: bool) -> Result<(), String> {
    let action = if enabled { "block" } else { "unblock" };
    let status = tokio::process::Command::new("rfkill")
        .args([action, "all"])
        .status()
        .await
        .map_err(|e| format!("rfkill {action} failed: {e}"))?;
    if !status.success() {
        return Err(format!("rfkill {action} all returned non-zero"));
    }
    Ok(())
}

/// Connection details for a known network.
#[derive(Clone, Serialize)]
pub struct ConnectionDetails {
    pub ip: String,
    pub gateway: String,
    pub dns: String,
    pub mac: String,
}

/// VPN connection info.
#[derive(Clone, Serialize)]
pub struct VpnConnection {
    pub name: String,
    pub active: bool,
}

/// Get detailed connection info for a connected/known network.
#[tauri::command]
pub async fn get_connection_details(ssid: String) -> Result<ConnectionDetails, String> {
    let output = tokio::process::Command::new("nmcli")
        .args(["-t", "-f", "IP4.ADDRESS,IP4.GATEWAY,IP4.DNS,GENERAL.HWADDR", "connection", "show", &ssid])
        .output()
        .await
        .map_err(|e| format!("nmcli: {e}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut ip = String::new();
    let mut gateway = String::new();
    let mut dns = String::new();
    let mut mac = String::new();

    for line in stdout.lines() {
        let line = line.trim();
        if let Some(val) = line.strip_prefix("IP4.ADDRESS[1]:") {
            ip = val.trim().to_string();
        } else if let Some(val) = line.strip_prefix("IP4.GATEWAY:") {
            gateway = val.trim().to_string();
        } else if let Some(val) = line.strip_prefix("IP4.DNS[1]:") {
            dns = val.trim().to_string();
        } else if let Some(val) = line.strip_prefix("GENERAL.HWADDR:") {
            mac = val.trim().to_string();
        }
    }

    Ok(ConnectionDetails { ip, gateway, dns, mac })
}

/// Get the saved PSK password for a known WiFi network.
#[tauri::command]
pub async fn get_saved_password(ssid: String) -> Result<Option<String>, String> {
    let output = tokio::process::Command::new("nmcli")
        .args(["-s", "-t", "-f", "802-11-wireless-security.psk", "connection", "show", &ssid])
        .output()
        .await
        .map_err(|e| format!("nmcli: {e}"))?;

    if !output.status.success() {
        return Ok(None);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        if let Some(val) = line.strip_prefix("802-11-wireless-security.psk:") {
            let psk = val.trim().to_string();
            if !psk.is_empty() {
                return Ok(Some(psk));
            }
        }
    }
    Ok(None)
}

/// Delete a saved network connection.
#[tauri::command]
pub async fn forget_network(ssid: String) -> Result<(), String> {
    let status = tokio::process::Command::new("nmcli")
        .args(["connection", "delete", &ssid])
        .status()
        .await
        .map_err(|e| format!("nmcli: {e}"))?;
    if !status.success() {
        return Err(format!("Failed to forget {ssid}"));
    }
    invalidate_wifi_cache();
    Ok(())
}

/// Connect to a hidden WiFi network with SSID and password.
#[tauri::command]
pub async fn connect_hidden_network(ssid: String, password: String) -> Result<(), String> {
    let output = tokio::process::Command::new("nmcli")
        .args([
            "dev", "wifi", "connect", &ssid,
            "password", &password,
            "hidden", "yes",
        ])
        .output()
        .await
        .map_err(|e| format!("nmcli: {e}"))?;

    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    invalidate_wifi_cache();
    Ok(())
}

/// List all VPN connections (active and inactive).
#[tauri::command]
pub async fn get_vpn_connections() -> Result<Vec<VpnConnection>, String> {
    // Get all VPN connections.
    let output = tokio::process::Command::new("nmcli")
        .args(["-t", "-f", "NAME,TYPE", "connection", "show"])
        .output()
        .await
        .map_err(|e| format!("nmcli: {e}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let all_vpns: Vec<String> = stdout
        .lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.split(':').collect();
            if parts.len() >= 2 && parts[1].contains("vpn") {
                Some(parts[0].to_string())
            } else {
                None
            }
        })
        .collect();

    // Get active VPN connections.
    let active_output = tokio::process::Command::new("nmcli")
        .args(["-t", "-f", "NAME,TYPE,STATE", "connection", "show", "--active"])
        .output()
        .await
        .unwrap_or_else(|_| std::process::Output {
            status: std::process::ExitStatus::default(),
            stdout: Vec::new(),
            stderr: Vec::new(),
        });

    let active_stdout = String::from_utf8_lossy(&active_output.stdout);
    let active_vpns: std::collections::HashSet<String> = active_stdout
        .lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.split(':').collect();
            if parts.len() >= 3 && parts[1].contains("vpn") && parts[2] == "activated" {
                Some(parts[0].to_string())
            } else {
                None
            }
        })
        .collect();

    Ok(all_vpns
        .into_iter()
        .map(|name| VpnConnection {
            active: active_vpns.contains(&name),
            name,
        })
        .collect())
}

/// Connect a VPN by name.
#[tauri::command]
pub async fn connect_vpn(name: String) -> Result<(), String> {
    let status = tokio::process::Command::new("nmcli")
        .args(["connection", "up", &name])
        .status()
        .await
        .map_err(|e| format!("nmcli: {e}"))?;
    if !status.success() {
        return Err(format!("Failed to connect VPN {name}"));
    }
    Ok(())
}

/// Disconnect a VPN by name.
#[tauri::command]
pub async fn disconnect_vpn(name: String) -> Result<(), String> {
    let status = tokio::process::Command::new("nmcli")
        .args(["connection", "down", &name])
        .status()
        .await
        .map_err(|e| format!("nmcli: {e}"))?;
    if !status.success() {
        return Err(format!("Failed to disconnect VPN {name}"));
    }
    Ok(())
}

/// Checks if any VPN connection is active.
async fn check_vpn() -> bool {
    let output = match tokio::process::Command::new("nmcli")
        .args(["-t", "-f", "TYPE,STATE", "con", "show", "--active"])
        .output()
        .await
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

// ---------------------------------------------------------------------------
// D-Bus signal monitor
// ---------------------------------------------------------------------------

/// Start monitoring NetworkManager D-Bus signals for live state updates.
///
/// Emits `network-changed` Tauri events when connectivity state changes.
pub fn start_monitor(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        if let Err(e) = run_network_monitor(app).await {
            log::warn!("network: monitor failed: {e}");
        }
    });
}

async fn run_network_monitor(app: tauri::AppHandle) -> Result<(), zbus::Error> {
    use futures_util::StreamExt;
    use tauri::Emitter;

    let conn = zbus::Connection::system().await?;

    // Monitor PropertiesChanged on org.freedesktop.NetworkManager.
    let proxy = zbus::Proxy::new(
        &conn,
        "org.freedesktop.NetworkManager",
        "/org/freedesktop/NetworkManager",
        "org.freedesktop.DBus.Properties",
    )
    .await?;

    let mut stream = proxy.receive_all_signals().await?;

    log::info!("network: signal monitor started");

    while let Some(_signal) = stream.next().await {
        let _ = app.emit("network-changed", ());
    }

    Ok(())
}

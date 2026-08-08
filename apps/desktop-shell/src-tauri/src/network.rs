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

/// NetworkManager device types, from the published enum. Only the two the
/// indicator distinguishes are named.
const NM_DEVICE_TYPE_ETHERNET: u32 = 1;
const NM_DEVICE_TYPE_WIFI: u32 = 2;
/// `NM_DEVICE_STATE_ACTIVATED` - the device is up and carrying traffic. The
/// state enum has fourteen values and this is the only one `nmcli` printed as
/// "connected", so matching it exactly keeps the old behaviour rather than
/// treating, say, `DEACTIVATING` as still connected.
const NM_DEVICE_STATE_ACTIVATED: u32 = 100;

/// The connected wifi or ethernet device, from NetworkManager over the system
/// bus.
///
/// **Behaviour is unchanged from the `nmcli -t -f TYPE,STATE,CONNECTION device`
/// this replaces**, deliberately: wifi is preferred over ethernet when both are
/// up, because that is what the indicator has always shown and is a UI choice
/// rather than an accident of the old parser.
///
/// NetworkManager also publishes `PrimaryConnection`, the one carrying the
/// default route, which is arguably the more truthful answer when both are
/// connected. Switching to it would change what the indicator displays, so it is
/// a decision to make on purpose and not a side effect of dropping a subprocess.
async fn parse_device_status() -> Result<(String, bool, Option<String>, Option<u8>), String> {
    let conn = zbus::Connection::system()
        .await
        .map_err(|e| format!("system bus: {e}"))?;
    let manager = zbus::Proxy::new(
        &conn,
        "org.freedesktop.NetworkManager",
        "/org/freedesktop/NetworkManager",
        "org.freedesktop.NetworkManager",
    )
    .await
    .map_err(|e| format!("NetworkManager unavailable: {e}"))?;
    let devices: Vec<zbus::zvariant::OwnedObjectPath> = manager
        .call("GetDevices", &())
        .await
        .map_err(|e| format!("GetDevices: {e}"))?;

    let mut wifi_conn: Option<String> = None;
    let mut ethernet_conn: Option<String> = None;

    for path in devices {
        let Ok(device) = zbus::Proxy::new(
            &conn,
            "org.freedesktop.NetworkManager",
            path.clone(),
            "org.freedesktop.NetworkManager.Device",
        )
        .await
        else {
            continue;
        };
        // A device that cannot be read is skipped rather than failing the whole
        // status: NetworkManager removes devices while they are being enumerated
        // (a USB tether unplugged mid-loop), and one vanishing device must not
        // black out the indicator.
        let (Ok(state), Ok(kind)) = (
            device.get_property::<u32>("State").await,
            device.get_property::<u32>("DeviceType").await,
        ) else {
            continue;
        };
        if state != NM_DEVICE_STATE_ACTIVATED {
            continue;
        }
        if kind != NM_DEVICE_TYPE_WIFI && kind != NM_DEVICE_TYPE_ETHERNET {
            continue;
        }
        // The human-readable name is the active connection's `Id`, which is what
        // nmcli printed in the CONNECTION column.
        let Ok(active) = device
            .get_property::<zbus::zvariant::OwnedObjectPath>("ActiveConnection")
            .await
        else {
            continue;
        };
        let Ok(active_proxy) = zbus::Proxy::new(
            &conn,
            "org.freedesktop.NetworkManager",
            active,
            "org.freedesktop.NetworkManager.Connection.Active",
        )
        .await
        else {
            continue;
        };
        let Ok(id) = active_proxy.get_property::<String>("Id").await else {
            continue;
        };
        match kind {
            NM_DEVICE_TYPE_WIFI => wifi_conn = Some(id),
            _ => ethernet_conn = Some(id),
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

/// One access point as NetworkManager sees it.
struct SeenAccessPoint {
    ssid: String,
    strength: u8,
    security: String,
    is_active: bool,
}

/// `NM_802_11_AP_FLAGS_PRIVACY` - the network is encrypted at all.
const AP_FLAGS_PRIVACY: u32 = 0x1;

/// The security label for an access point's three flag words.
///
/// **Verified against `nmcli` on a live scan for the WPA2 case only**, which is
/// what this machine can see: `flags=1 wpa=0 rsn=392` renders as `WPA2`, and 392
/// is CCMP pairwise + CCMP group + PSK key management. The other arms come from
/// the published `NM_802_11_AP_SEC_*` constants rather than from a scan, because
/// no WPA3, enterprise or WEP network was in range - stated plainly, since a
/// mapping that is only true where it was written is the failure this kind of
/// code invites.
///
/// The order matters: RSN covers WPA2 and WPA3, so it is asked first and the
/// key-management bits separate them.
fn ap_security(flags: u32, wpa: u32, rsn: u32) -> String {
    /// `NM_802_11_AP_SEC_KEY_MGMT_802_1X`.
    const KEY_MGMT_802_1X: u32 = 0x200;
    /// `NM_802_11_AP_SEC_KEY_MGMT_SAE`, which is what makes an RSN network WPA3.
    const KEY_MGMT_SAE: u32 = 0x400;

    let mut parts: Vec<&str> = Vec::new();
    if rsn != 0 {
        parts.push(if rsn & KEY_MGMT_SAE != 0 {
            "WPA3"
        } else {
            "WPA2"
        });
    } else if wpa != 0 {
        parts.push("WPA1");
    } else if flags & AP_FLAGS_PRIVACY != 0 {
        // Encrypted, but neither WPA nor RSN: the only thing left is WEP.
        parts.push("WEP");
    }
    if (wpa | rsn) & KEY_MGMT_802_1X != 0 {
        parts.push("802.1X");
    }
    parts.join(" ")
}

/// Every access point the wifi devices currently see.
///
/// Replaces the read half of `nmcli -t -f SSID,SIGNAL,SECURITY,IN-USE dev wifi
/// list`. This reads what NetworkManager has already published and never asks
/// for a scan - the forced rescan above is a separate, deliberate action and
/// stays one, so opening the popover cannot trigger an RF sweep through this
/// path.
async fn scan_access_points() -> Result<Vec<SeenAccessPoint>, String> {
    let conn = zbus::Connection::system()
        .await
        .map_err(|e| format!("system bus: {e}"))?;
    let manager = zbus::Proxy::new(
        &conn,
        "org.freedesktop.NetworkManager",
        "/org/freedesktop/NetworkManager",
        "org.freedesktop.NetworkManager",
    )
    .await
    .map_err(|e| format!("NetworkManager unavailable: {e}"))?;
    let devices: Vec<zbus::zvariant::OwnedObjectPath> = manager
        .call("GetDevices", &())
        .await
        .map_err(|e| format!("GetDevices: {e}"))?;

    let mut seen = Vec::new();
    for path in devices {
        let Ok(device) = zbus::Proxy::new(
            &conn,
            "org.freedesktop.NetworkManager",
            path.clone(),
            "org.freedesktop.NetworkManager.Device",
        )
        .await
        else {
            continue;
        };
        if device.get_property::<u32>("DeviceType").await != Ok(NM_DEVICE_TYPE_WIFI) {
            continue;
        }
        let Ok(wireless) = zbus::Proxy::new(
            &conn,
            "org.freedesktop.NetworkManager",
            path,
            "org.freedesktop.NetworkManager.Device.Wireless",
        )
        .await
        else {
            continue;
        };
        // The AP we are associated with, so a row can say so without a second
        // pass. Absent when the radio is up but unassociated.
        let active = wireless
            .get_property::<zbus::zvariant::OwnedObjectPath>("ActiveAccessPoint")
            .await
            .ok();
        let Ok(points): Result<Vec<zbus::zvariant::OwnedObjectPath>, _> =
            wireless.call("GetAllAccessPoints", &()).await
        else {
            continue;
        };
        for ap in points {
            let Ok(proxy) = zbus::Proxy::new(
                &conn,
                "org.freedesktop.NetworkManager",
                ap.clone(),
                "org.freedesktop.NetworkManager.AccessPoint",
            )
            .await
            else {
                continue;
            };
            // An access point that vanishes between the list and the read is
            // skipped, not fatal: they come and go with every beacon interval.
            let (Ok(ssid), Ok(strength), Ok(flags), Ok(wpa), Ok(rsn)) = (
                proxy.get_property::<Vec<u8>>("Ssid").await,
                proxy.get_property::<u8>("Strength").await,
                proxy.get_property::<u32>("Flags").await,
                proxy.get_property::<u32>("WpaFlags").await,
                proxy.get_property::<u32>("RsnFlags").await,
            ) else {
                continue;
            };
            seen.push(SeenAccessPoint {
                // The SSID is bytes, not text: the standard does not require it
                // to be UTF-8. Lossy rather than dropped, so a router with an
                // odd name is still selectable instead of silently missing.
                ssid: String::from_utf8_lossy(&ssid).into_owned(),
                strength,
                security: ap_security(flags, wpa, rsn),
                is_active: active.as_ref() == Some(&ap),
            });
        }
    }
    Ok(seen)
}

/// The names of every saved connection, as NetworkManager holds them.
///
/// Replaces `nmcli -t -f NAME connection show`. One call to list the connection
/// objects and one per object for its settings - twenty-odd round trips on a
/// well-used machine, which is still far cheaper than forking a process, and
/// they are reads that touch nothing.
///
/// An unreadable connection is skipped rather than failing the set: this only
/// decides whether a scanned network is drawn as "known", so a connection that
/// disappears mid-enumeration should cost that one row's badge and not the whole
/// list.
///
/// **This compares a saved connection's `id` against a scanned SSID, which is
/// what the nmcli version did and is not quite right.** The id is a label and
/// usually equals the SSID, but a network saved as "Home" is the same network
/// under another name, and today it draws as unknown. The settings also carry
/// `802-11-wireless.ssid` as the actual bytes, which would answer properly.
/// Left as it was so this stays a transport change; the fix is a behaviour
/// change and deserves to be visible as one.
async fn saved_connection_names() -> std::collections::HashSet<String> {
    let Ok(conn) = zbus::Connection::system().await else {
        return Default::default();
    };
    let Ok(settings) = zbus::Proxy::new(
        &conn,
        "org.freedesktop.NetworkManager",
        "/org/freedesktop/NetworkManager/Settings",
        "org.freedesktop.NetworkManager.Settings",
    )
    .await
    else {
        return Default::default();
    };
    let Ok(paths): Result<Vec<zbus::zvariant::OwnedObjectPath>, _> =
        settings.call("ListConnections", &()).await
    else {
        return Default::default();
    };

    let mut names = std::collections::HashSet::new();
    for (id, _) in saved_connections(&conn, paths).await {
        names.insert(id);
    }
    names
}

/// Every saved connection as `(id, type)`, read once from its settings.
///
/// Split out because two callers want different halves of the same read: which
/// names exist (to badge a scanned network as known) and which of them are VPNs.
/// One walk, so the popover does not pay for the settings twice.
async fn saved_connections(
    conn: &zbus::Connection,
    paths: Vec<zbus::zvariant::OwnedObjectPath>,
) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for path in paths {
        let Ok(proxy) = zbus::Proxy::new(
            conn,
            "org.freedesktop.NetworkManager",
            path,
            "org.freedesktop.NetworkManager.Settings.Connection",
        )
        .await
        else {
            continue;
        };
        type Settings = std::collections::HashMap<
            String,
            std::collections::HashMap<String, zbus::zvariant::OwnedValue>,
        >;
        let Ok(map): Result<Settings, _> = proxy.call("GetSettings", &()).await else {
            continue;
        };
        let field = |name: &str| {
            map.get("connection")
                .and_then(|c| c.get(name))
                .and_then(|v| String::try_from(v.clone()).ok())
        };
        if let Some(id) = field("id") {
            out.push((id, field("type").unwrap_or_default()));
        }
    }
    out
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

    let known = saved_connection_names().await;
    let seen = scan_access_points().await?;

    // NetworkManager publishes ONE access point per BSSID, so an SSID with a
    // mesh / dual-band setup appears several times. Keep the one we are actually
    // connected to if any, otherwise the strongest. Dropping by first occurrence
    // loses the connected flag whenever the active BSSID is not first, which then
    // shows the active SSID in "Available Networks" as if it were unconnected.
    use std::collections::HashMap;
    let mut by_ssid: HashMap<String, WifiNetwork> = HashMap::new();
    for point in seen {
        let ssid = point.ssid;
        // A hidden network broadcasts an empty SSID. There is nothing to show and
        // nothing to click, so it is dropped here exactly as the nmcli parse
        // dropped its blank first column.
        if ssid.is_empty() {
            continue;
        }
        let candidate = WifiNetwork {
            signal: point.strength,
            security: point.security,
            is_connected: point.is_active,
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
///
/// **The password is passed as a command-line argument, where it is readable by
/// every process on the machine for the duration of the call.**
/// `/proc/<pid>/cmdline` is mode 0444 with no `hidepid` in effect, so this is not
/// a same-uid caveat - any process can poll for an `nmcli` and read the secret out
/// of its argv. The window is short and the race has to be won, but a polling loop
/// wins it reliably, and a WiFi PSK is long-lived.
///
/// Not changed here, deliberately. `nmcli --ask` reads the secret from stdin and
/// would close it with a small change, but this is the live WiFi path on a machine
/// I cannot test an actual association against, and a wrong guess about which
/// nmcli versions prompt the same way breaks connecting to networks.
///
/// What IS established, so whoever can test an association does not have to
/// re-derive it: nmcli 1.56.1 accepts `--ask` on `dev wifi connect`, and with a
/// bogus SSID it fails fast on "No network with SSID found" rather than hanging
/// for input - so the flag parses and the no-such-network path is safe. The one
/// unverified step is whether a piped password satisfies the prompt for a network
/// that actually requires one. Do NOT add an argv fallback if it does not: falling
/// back would re-introduce the exposure precisely in the failure case, and
/// silently. The real
/// answer is the NetworkManager D-Bus API, which takes secrets as method
/// arguments over the bus and never puts them in an argv - which is what makes
/// the "replace the shelling-out with real D-Bus" job a security fix rather than
/// a tidiness one.
///
/// The same exposure applies in reverse to [`get_saved_password`], which reads a
/// stored PSK back out through `nmcli -s`: the secret is in that child's OUTPUT
/// rather than its argv, so it is not world-readable, but it does cross a pipe
/// through this process.
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
///
/// **The device lookup this replaces was broken on any non-English desktop.** It
/// matched the state column against the literal "connected", and NetworkManager
/// translates that word - the German catalogue has `verbunden` - so the loop
/// found nothing and the function returned "No connected wifi device found"
/// while WiFi was plainly connected. Matching the state enum instead removes the
/// language from the question, the same fix as the radio toggle.
///
/// The lookup half is the walk used by `parse_device_status` and
/// `scan_access_points`, both checked against a live NetworkManager. The
/// `Disconnect` call itself is not exercised here: it would drop your session's
/// WiFi, so it is written against the documented interface and wants one run on
/// a machine that is not the one being developed on.
#[tauri::command]
pub async fn disconnect_wifi() -> Result<(), String> {
    let conn = zbus::Connection::system()
        .await
        .map_err(|e| format!("system bus: {e}"))?;
    let manager = zbus::Proxy::new(
        &conn,
        "org.freedesktop.NetworkManager",
        "/org/freedesktop/NetworkManager",
        "org.freedesktop.NetworkManager",
    )
    .await
    .map_err(|e| format!("NetworkManager unavailable: {e}"))?;
    let devices: Vec<zbus::zvariant::OwnedObjectPath> = manager
        .call("GetDevices", &())
        .await
        .map_err(|e| format!("GetDevices: {e}"))?;

    for path in devices {
        let Ok(device) = zbus::Proxy::new(
            &conn,
            "org.freedesktop.NetworkManager",
            path,
            "org.freedesktop.NetworkManager.Device",
        )
        .await
        else {
            continue;
        };
        let (Ok(kind), Ok(state)) = (
            device.get_property::<u32>("DeviceType").await,
            device.get_property::<u32>("State").await,
        ) else {
            continue;
        };
        if kind != NM_DEVICE_TYPE_WIFI || state != NM_DEVICE_STATE_ACTIVATED {
            continue;
        }
        device
            .call::<_, _, ()>("Disconnect", &())
            .await
            .map_err(|e| format!("Disconnect: {e}"))?;
        invalidate_wifi_cache();
        return Ok(());
    }
    Err("No connected wifi device found".into())
}

/// Returns whether the WiFi radio is enabled.
///
/// NetworkManager's `WirelessEnabled`, which is the property `nmcli radio wifi`
/// prints as "enabled" or "disabled". Checked side by side on a live session:
/// `enabled` against `b true`.
///
/// **It also fixes a real bug rather than only moving the transport.** The old
/// code compared the output against the literal "enabled", and that word is
/// translated: NetworkManager's shipped German catalogue maps `msgid "enabled"`
/// to `"aktiviert"`. On any non-English desktop the comparison returned false
/// whatever the radio was doing, so the WiFi toggle showed off while WiFi was on.
/// Confirmed from the catalogue with `msgunfmt`, not by running `nmcli` under a
/// German locale - this machine has no de_DE generated, so that test would have
/// printed English and proved nothing. The property is a boolean and has no
/// language.
#[tauri::command]
pub async fn get_wifi_enabled() -> Result<bool, String> {
    let conn = zbus::Connection::system()
        .await
        .map_err(|e| format!("system bus: {e}"))?;
    let manager = zbus::Proxy::new(
        &conn,
        "org.freedesktop.NetworkManager",
        "/org/freedesktop/NetworkManager",
        "org.freedesktop.NetworkManager",
    )
    .await
    .map_err(|e| format!("NetworkManager unavailable: {e}"))?;
    manager
        .get_property::<bool>("WirelessEnabled")
        .await
        .map_err(|e| format!("WirelessEnabled: {e}"))
}

/// Enable or disable the WiFi radio via NetworkManager.
#[tauri::command]
pub async fn set_wifi_enabled(enabled: bool) -> Result<(), String> {
    let conn = zbus::Connection::system()
        .await
        .map_err(|e| format!("system bus: {e}"))?;
    let manager = zbus::Proxy::new(
        &conn,
        "org.freedesktop.NetworkManager",
        "/org/freedesktop/NetworkManager",
        "org.freedesktop.NetworkManager",
    )
    .await
    .map_err(|e| format!("NetworkManager unavailable: {e}"))?;
    // The same property `get_wifi_enabled` reads, and the same authorisation
    // `nmcli radio wifi on|off` needed: NetworkManager asks polkit for
    // network-control either way, so this is not a privilege change.
    manager
        .set_property("WirelessEnabled", enabled)
        .await
        .map_err(|e| format!("WirelessEnabled: {e}"))
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
///
/// Emits `arlen://airplane-changed` on success so the indicators and tiles
/// refresh immediately instead of waiting for their periodic poll.
#[tauri::command]
pub async fn set_airplane_mode(app: tauri::AppHandle, enabled: bool) -> Result<(), String> {
    use tauri::Emitter;
    let action = if enabled { "block" } else { "unblock" };
    let status = tokio::process::Command::new("rfkill")
        .args([action, "all"])
        .status()
        .await
        .map_err(|e| format!("rfkill {action} failed: {e}"))?;
    if !status.success() {
        return Err(format!("rfkill {action} all returned non-zero"));
    }
    let _ = app.emit("arlen://airplane-changed", ());
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

/// Get detailed connection info for a connected network.
///
/// **This replaces a call that never worked.** The nmcli version asked
/// `connection show <name>` for `IP4.ADDRESS,IP4.GATEWAY,IP4.DNS,GENERAL.HWADDR`,
/// and `GENERAL.HWADDR` is a *device* field: nmcli rejects the whole request with
/// `Error: 'connection show': invalid field 'GENERAL...'` and exits 2. The exit
/// status was not checked, so the empty output parsed into four empty strings and
/// the details panel showed blanks for IP, gateway, DNS and MAC - all four,
/// because one name in the list belonged to a different object. Confirmed on this
/// machine, where the same connection has an address, a gateway and a MAC that
/// NetworkManager hands over without complaint.
///
/// Details only exist while a connection is up, which is also what the nmcli
/// version could have shown at best: an inactive saved connection has no IP
/// configuration to report. An unknown or inactive name gives empty fields rather
/// than an error, as before.
#[tauri::command]
pub async fn get_connection_details(ssid: String) -> Result<ConnectionDetails, String> {
    let mut details = ConnectionDetails {
        ip: String::new(),
        gateway: String::new(),
        dns: String::new(),
        mac: String::new(),
    };

    let conn = zbus::Connection::system()
        .await
        .map_err(|e| format!("system bus: {e}"))?;
    let manager = zbus::Proxy::new(
        &conn,
        "org.freedesktop.NetworkManager",
        "/org/freedesktop/NetworkManager",
        "org.freedesktop.NetworkManager",
    )
    .await
    .map_err(|e| format!("NetworkManager unavailable: {e}"))?;
    let Ok(active) = manager
        .get_property::<Vec<zbus::zvariant::OwnedObjectPath>>("ActiveConnections")
        .await
    else {
        return Ok(details);
    };

    for path in active {
        let Ok(proxy) = zbus::Proxy::new(
            &conn,
            "org.freedesktop.NetworkManager",
            path,
            "org.freedesktop.NetworkManager.Connection.Active",
        )
        .await
        else {
            continue;
        };
        if proxy.get_property::<String>("Id").await.as_deref() != Ok(ssid.as_str()) {
            continue;
        }

        // The address list is an array of dictionaries; the first entry is the
        // primary address, which is the one nmcli printed as `IP4.ADDRESS[1]`.
        // Rendered back as `address/prefix` because that is the string the panel
        // has always shown.
        if let Ok(ip4) = proxy
            .get_property::<zbus::zvariant::OwnedObjectPath>("Ip4Config")
            .await
        {
            if let Ok(ip4_proxy) = zbus::Proxy::new(
                &conn,
                "org.freedesktop.NetworkManager",
                ip4,
                "org.freedesktop.NetworkManager.IP4Config",
            )
            .await
            {
                type Entries = Vec<
                    std::collections::HashMap<String, zbus::zvariant::OwnedValue>,
                >;
                let field = |entry: &std::collections::HashMap<
                    String,
                    zbus::zvariant::OwnedValue,
                >,
                             name: &str| {
                    entry
                        .get(name)
                        .and_then(|v| String::try_from(v.clone()).ok())
                };
                if let Ok(addresses) = ip4_proxy.get_property::<Entries>("AddressData").await {
                    if let Some(first) = addresses.first() {
                        let address = field(first, "address").unwrap_or_default();
                        let prefix = first
                            .get("prefix")
                            .and_then(|v| u32::try_from(v.clone()).ok());
                        details.ip = match prefix {
                            Some(p) if !address.is_empty() => format!("{address}/{p}"),
                            _ => address,
                        };
                    }
                }
                if let Ok(gateway) = ip4_proxy.get_property::<String>("Gateway").await {
                    details.gateway = gateway;
                }
                if let Ok(servers) = ip4_proxy.get_property::<Entries>("NameserverData").await {
                    if let Some(first) = servers.first() {
                        details.dns = field(first, "address").unwrap_or_default();
                    }
                }
            }
        }

        // The MAC belongs to the device carrying the connection, which is why
        // asking `connection show` for it could never have worked.
        if let Ok(devices) = proxy
            .get_property::<Vec<zbus::zvariant::OwnedObjectPath>>("Devices")
            .await
        {
            if let Some(device) = devices.first() {
                if let Ok(device_proxy) = zbus::Proxy::new(
                    &conn,
                    "org.freedesktop.NetworkManager",
                    device.clone(),
                    "org.freedesktop.NetworkManager.Device",
                )
                .await
                {
                    if let Ok(mac) = device_proxy.get_property::<String>("HwAddress").await {
                        details.mac = mac;
                    }
                }
            }
        }
        break;
    }

    Ok(details)
}

/// A connection's settings as NetworkManager hands them over: setting name, then
/// key, then a typed value.
type NmSettings =
    std::collections::HashMap<String, std::collections::HashMap<String, zbus::zvariant::OwnedValue>>;

/// The wifi SSID a saved connection is for, as the bytes NetworkManager holds.
///
/// The stored form is a byte array, not a string, which is what the standard
/// says an SSID is. Compared as bytes so a name that is not valid UTF-8 is
/// matched or not matched honestly rather than through a lossy decode.
fn connection_ssid(map: &NmSettings) -> Option<Vec<u8>> {
    let v = map.get("802-11-wireless")?.get("ssid")?;
    Vec::<u8>::try_from(v.clone()).ok()
}

/// The pre-shared key out of a `GetSecrets` reply, if there is one.
fn psk_from_secrets(secrets: &NmSettings) -> Option<String> {
    let v = secrets.get("802-11-wireless-security")?.get("psk")?;
    let psk = String::try_from(v.clone()).ok()?;
    (!psk.is_empty()).then_some(psk)
}

/// Get the saved PSK password for a known WiFi network.
///
/// **This used to return the wrong password.** It read
/// `nmcli -s -t -f 802-11-wireless-security.psk connection show <ssid>`, and
/// terse mode escapes both `:` and `\` with a backslash - `--escape` defaults to
/// yes, per nmcli(1) - while the parse never unescaped. Both characters are legal
/// in a WPA passphrase, so a key of `a:b` was copied to the clipboard as `a\:b`
/// and the user pasted something that could not work. The D-Bus value is typed,
/// so there is no escaping to undo and nothing to get wrong.
///
/// It also takes the secret off a subprocess pipe, which is the exposure
/// [`connect_wifi_password`] notes below for the write direction.
///
/// **And it now matches the SSID rather than the connection's name.** The old
/// argument went to a matcher that accepts an id, a uuid or an object path, so a
/// scanned network - a string chosen by whoever is broadcasting nearby - could
/// name a different saved connection and put THAT connection's key on the
/// clipboard. [`saved_connection_names`] deliberately left the same
/// id-versus-SSID gap alone as a behaviour change worth seeing on its own; for a
/// secret it is not the same call, because the wrong match is the wrong secret.
///
/// A connection we cannot read is an error, never `Ok(None)`: "there is no saved
/// password" and "we could not ask" are different answers and a caller that
/// cannot tell them apart will state the first one.
#[tauri::command]
pub async fn get_saved_password(ssid: String) -> Result<Option<String>, String> {
    let conn = zbus::Connection::system()
        .await
        .map_err(|e| format!("system bus: {e}"))?;
    let settings = zbus::Proxy::new(
        &conn,
        "org.freedesktop.NetworkManager",
        "/org/freedesktop/NetworkManager/Settings",
        "org.freedesktop.NetworkManager.Settings",
    )
    .await
    .map_err(|e| format!("NetworkManager unavailable: {e}"))?;
    let paths: Vec<zbus::zvariant::OwnedObjectPath> = settings
        .call("ListConnections", &())
        .await
        .map_err(|e| format!("ListConnections: {e}"))?;

    for path in paths {
        let Ok(proxy) = zbus::Proxy::new(
            &conn,
            "org.freedesktop.NetworkManager",
            path,
            "org.freedesktop.NetworkManager.Settings.Connection",
        )
        .await
        else {
            continue;
        };
        let Ok(map): Result<NmSettings, _> = proxy.call("GetSettings", &()).await else {
            continue;
        };
        if connection_ssid(&map).as_deref() != Some(ssid.as_bytes()) {
            continue;
        }
        // Only now ask for the secret. GetSecrets is the call that can raise a
        // polkit prompt, so it is made for the one connection that matched and
        // not once per saved network while looking for it.
        let secrets: NmSettings = proxy
            .call("GetSecrets", &("802-11-wireless-security"))
            .await
            .map_err(|e| format!("GetSecrets: {e}"))?;
        return Ok(psk_from_secrets(&secrets));
    }
    // Nothing is saved for this SSID. That is an answer, not a failure.
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
///
/// Carries the same world-readable-argv exposure as
/// [`connect_wifi_password`], for the same reason and with the same fix: see
/// that function's note. Both close together or neither does.
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
    // Saved VPNs, by the same "type contains vpn" test the nmcli version used.
    let conn = zbus::Connection::system()
        .await
        .map_err(|e| format!("system bus: {e}"))?;
    let settings = zbus::Proxy::new(
        &conn,
        "org.freedesktop.NetworkManager",
        "/org/freedesktop/NetworkManager/Settings",
        "org.freedesktop.NetworkManager.Settings",
    )
    .await
    .map_err(|e| format!("NetworkManager unavailable: {e}"))?;
    let paths: Vec<zbus::zvariant::OwnedObjectPath> = settings
        .call("ListConnections", &())
        .await
        .map_err(|e| format!("ListConnections: {e}"))?;
    let all_vpns: Vec<String> = saved_connections(&conn, paths)
        .await
        .into_iter()
        .filter(|(_, kind)| kind.contains("vpn"))
        .map(|(id, _)| id)
        .collect();

    let active_vpns = active_vpn_names(&conn).await;

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

/// The names of the VPN connections that are currently up.
///
/// The same walk `check_vpn` does, kept separate because the popover needs the
/// names and the indicator only needs to know whether any exists. An empty set
/// on a bus error rather than an error: a VPN section that shows nothing active
/// is wrong in a visible way, while failing the whole call would empty the list
/// of saved VPNs too.
async fn active_vpn_names(conn: &zbus::Connection) -> std::collections::HashSet<String> {
    /// `NM_ACTIVE_CONNECTION_STATE_ACTIVATED`.
    const ACTIVE_CONNECTION_ACTIVATED: u32 = 2;

    let Ok(manager) = zbus::Proxy::new(
        conn,
        "org.freedesktop.NetworkManager",
        "/org/freedesktop/NetworkManager",
        "org.freedesktop.NetworkManager",
    )
    .await
    else {
        return Default::default();
    };
    let Ok(active) = manager
        .get_property::<Vec<zbus::zvariant::OwnedObjectPath>>("ActiveConnections")
        .await
    else {
        return Default::default();
    };

    let mut names = std::collections::HashSet::new();
    for path in active {
        let Ok(proxy) = zbus::Proxy::new(
            conn,
            "org.freedesktop.NetworkManager",
            path,
            "org.freedesktop.NetworkManager.Connection.Active",
        )
        .await
        else {
            continue;
        };
        let (Ok(kind), Ok(state), Ok(id)) = (
            proxy.get_property::<String>("Type").await,
            proxy.get_property::<u32>("State").await,
            proxy.get_property::<String>("Id").await,
        ) else {
            continue;
        };
        if kind.contains("vpn") && state == ACTIVE_CONNECTION_ACTIVATED {
            names.insert(id);
        }
    }
    names
}

/// Whether a VPN connection is up.
///
/// Replaces `nmcli -t -f TYPE,STATE con show --active`, matching it exactly: a
/// connection whose type contains "vpn" and whose state is activated. Verified
/// side by side on a live session - the same five active connections, the same
/// type strings, and `State=2` where nmcli printed "activated".
///
/// **Two things this does not catch, neither of them new.** NetworkManager also
/// publishes a `Vpn` boolean on an active connection, which is sturdier than a
/// substring match on a type name; and WireGuard is a device type rather than a
/// VPN plugin, so neither the substring nor that boolean is likely to see it.
/// No VPN and no WireGuard was configured here, so the second is reasoning about
/// NetworkManager's model rather than something I watched fail - said that way
/// on purpose. Both are behaviour questions and this commit is a transport
/// change.
async fn check_vpn() -> bool {
    /// `NM_ACTIVE_CONNECTION_STATE_ACTIVATED`, which is what nmcli printed as
    /// "activated" in the STATE column this replaces.
    const ACTIVE_CONNECTION_ACTIVATED: u32 = 2;

    let Ok(conn) = zbus::Connection::system().await else {
        return false;
    };
    let Ok(manager) = zbus::Proxy::new(
        &conn,
        "org.freedesktop.NetworkManager",
        "/org/freedesktop/NetworkManager",
        "org.freedesktop.NetworkManager",
    )
    .await
    else {
        return false;
    };
    let Ok(active) = manager
        .get_property::<Vec<zbus::zvariant::OwnedObjectPath>>("ActiveConnections")
        .await
    else {
        return false;
    };

    for path in active {
        let Ok(proxy) = zbus::Proxy::new(
            &conn,
            "org.freedesktop.NetworkManager",
            path,
            "org.freedesktop.NetworkManager.Connection.Active",
        )
        .await
        else {
            continue;
        };
        let (Ok(kind), Ok(state)) = (
            proxy.get_property::<String>("Type").await,
            proxy.get_property::<u32>("State").await,
        ) else {
            continue;
        };
        if kind.contains("vpn") && state == ACTIVE_CONNECTION_ACTIVATED {
            return true;
        }
    }
    false
}

// ---------------------------------------------------------------------------
// D-Bus signal monitor
// ---------------------------------------------------------------------------

/// Map a [`NetworkStatus`] to the `network.state` event-bus snapshot. Pure, so
/// the field mapping is unit-tested without nmcli. `ssid` carries the name only
/// for wifi; a disconnected status reports `connection_type = "none"` (the
/// canonical vocabulary); `interface` is left empty (the status read does not
/// query the device name).
fn network_state_payload(status: &NetworkStatus) -> crate::projects::proto::NetworkStatePayload {
    let is_wifi = status.connection_type == "wifi";
    crate::projects::proto::NetworkStatePayload {
        connected: status.connected,
        connection_type: if status.connected {
            status.connection_type.clone()
        } else {
            "none".to_string()
        },
        ssid: if is_wifi {
            status.name.clone().unwrap_or_default()
        } else {
            String::new()
        },
        interface: String::new(),
        signal: status.signal_strength.unwrap_or(0) as u32,
        vpn_active: status.vpn_active,
    }
}

/// Publish the current network snapshot on the event bus (SST-R2). Best-effort:
/// a failed status read simply skips this publish.
async fn emit_network_state() {
    use prost::Message;
    if let Ok(status) = get_network_status().await {
        crate::projects::emit_to_event_bus(
            "network.state",
            network_state_payload(&status).encode_to_vec(),
        );
    }
}

/// Start monitoring NetworkManager D-Bus signals for live state updates.
///
/// Emits `network-changed` Tauri events (for the popover) and publishes a
/// debounced `network.state` snapshot on the event bus (SST-R2, for apps/AI)
/// when connectivity state changes.
pub fn start_monitor(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        if let Err(e) = run_network_monitor(app).await {
            log::warn!("network: monitor failed: {e}");
        }
    });
}

async fn run_network_monitor(app: tauri::AppHandle) -> Result<(), zbus::Error> {
    use futures_util::StreamExt;
    use std::time::{Duration, Instant};
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

    // Publish the current snapshot once on (re)connect so a consumer that
    // subscribed after the last change still gets the present network state
    // without waiting for the next transition (net/audio have no pull fallback,
    // unlike power's org.arlen.Power1).
    emit_network_state().await;

    // NetworkManager fires bursts of PropertiesChanged for a single transition;
    // the frontend `network-changed` is left undebounced (it self-throttles),
    // but the `network.state` publish spawns an nmcli read, so coalesce it to one
    // per 200ms window.
    let mut last_state_emit = Instant::now() - Duration::from_secs(1);

    while let Some(_signal) = stream.next().await {
        let _ = app.emit("network-changed", ());
        let now = Instant::now();
        if now.duration_since(last_state_emit) >= Duration::from_millis(200) {
            emit_network_state().await;
            last_state_emit = now;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One setting, one key, one value: the shape NetworkManager replies in.
    fn setting(name: &str, key: &str, value: zbus::zvariant::Value<'static>) -> NmSettings {
        let mut inner = std::collections::HashMap::new();
        inner.insert(key.to_string(), value.try_into().unwrap());
        let mut outer = std::collections::HashMap::new();
        outer.insert(name.to_string(), inner);
        outer
    }

    /// The characters the old terse-mode parse mangled. A key of `a:b` came back
    /// as `a\:b` and went to the clipboard that way, so this is the regression
    /// that fix exists for: the value arrives verbatim, escaping and all.
    #[test]
    fn a_key_containing_a_colon_or_a_backslash_survives_verbatim() {
        for raw in ["a:b", "a\\b", "pa:ss\\word:", ":", "\\"] {
            let secrets = setting(
                "802-11-wireless-security",
                "psk",
                zbus::zvariant::Value::from(raw),
            );
            assert_eq!(psk_from_secrets(&secrets).as_deref(), Some(raw));
        }
    }

    /// An empty key is not a key. NetworkManager returns the field present and
    /// blank for a connection whose secret is not stored here (agent-owned, or on
    /// a keyring), and copying "" to the clipboard would look like it worked.
    #[test]
    fn a_blank_key_is_no_key() {
        let secrets = setting(
            "802-11-wireless-security",
            "psk",
            zbus::zvariant::Value::from(""),
        );
        assert_eq!(psk_from_secrets(&secrets), None);
        assert_eq!(psk_from_secrets(&NmSettings::new()), None);
    }

    /// The SSID is matched, not the connection's name. A network saved as "Home"
    /// is still that SSID, and a connection NAMED like a nearby SSID is not it -
    /// which is the whole reason this reads `802-11-wireless.ssid` instead of
    /// handing a broadcast string to a matcher that also accepts names.
    #[test]
    fn the_ssid_is_the_bytes_not_the_connection_name() {
        let mut map = setting(
            "802-11-wireless",
            "ssid",
            zbus::zvariant::Value::from(b"Vodafone-1234".to_vec()),
        );
        map.insert(
            "connection".to_string(),
            [(
                "id".to_string(),
                zbus::zvariant::Value::from("Home").try_into().unwrap(),
            )]
            .into_iter()
            .collect(),
        );
        assert_eq!(connection_ssid(&map).as_deref(), Some(&b"Vodafone-1234"[..]));
        assert_ne!(connection_ssid(&map).as_deref(), Some(&b"Home"[..]));
    }

    /// A wired or VPN connection has no wifi setting at all, so it is skipped
    /// rather than mistaken for an SSID-less match.
    #[test]
    fn a_connection_with_no_wifi_setting_has_no_ssid() {
        let map = setting(
            "connection",
            "id",
            zbus::zvariant::Value::from("Wired connection 1"),
        );
        assert_eq!(connection_ssid(&map), None);
    }

    use super::{ap_security, network_state_payload, NetworkStatus};

    /// The one case a live scan could confirm: `flags=1 wpa=0 rsn=392` is what
    /// this machine's access points publish, and `nmcli` renders it `WPA2`. If
    /// this ever fails, the popover has started labelling networks differently
    /// from the tool everyone cross-checks against.
    #[test]
    fn the_flags_a_real_wpa2_network_publishes_read_as_wpa2() {
        assert_eq!(ap_security(1, 0, 392), "WPA2");
    }

    /// The remaining arms, from the published `NM_802_11_AP_SEC_*` constants
    /// rather than from a scan - no WPA3, enterprise or WEP network was in range
    /// to check against, which is said here as well as at the function so a
    /// reader knows which lines are measured and which are read from a spec.
    #[test]
    fn the_other_security_shapes_follow_the_published_constants() {
        // SAE key management is what makes an RSN network WPA3.
        assert_eq!(ap_security(1, 0, 0x400), "WPA3");
        // WPA with no RSN is the original.
        assert_eq!(ap_security(1, 0x100, 0), "WPA1");
        // Encrypted, neither WPA nor RSN: nothing else it can be.
        assert_eq!(ap_security(1, 0, 0), "WEP");
        // No privacy bit at all is an open network, and shows as nothing rather
        // than as the word "none" - the popover renders the empty string as no
        // badge.
        assert_eq!(ap_security(0, 0, 0), "");
        // Enterprise is an addition to the generation, not a replacement.
        assert_eq!(ap_security(1, 0, 392 | 0x200), "WPA2 802.1X");
    }

    #[test]
    fn payload_maps_wifi_fields() {
        let status = NetworkStatus {
            connection_type: "wifi".into(),
            connected: true,
            name: Some("HomeNet".into()),
            signal_strength: Some(72),
            vpn_active: true,
        };
        let p = network_state_payload(&status);
        assert!(p.connected);
        assert_eq!(p.connection_type, "wifi");
        assert_eq!(p.ssid, "HomeNet");
        assert_eq!(p.signal, 72);
        assert!(p.vpn_active);
    }

    #[test]
    fn payload_leaves_ssid_empty_for_ethernet() {
        let status = NetworkStatus {
            connection_type: "ethernet".into(),
            connected: true,
            name: Some("Wired connection 1".into()),
            signal_strength: None,
            vpn_active: false,
        };
        let p = network_state_payload(&status);
        assert_eq!(p.connection_type, "ethernet");
        assert_eq!(p.ssid, "", "ethernet has no ssid");
        assert_eq!(p.signal, 0, "no signal when not wifi");
    }

    #[test]
    fn disconnected_maps_to_none() {
        let status = NetworkStatus {
            connection_type: "disconnected".into(),
            connected: false,
            name: None,
            signal_strength: None,
            vpn_active: false,
        };
        let p = network_state_payload(&status);
        assert!(!p.connected);
        assert_eq!(p.connection_type, "none");
        assert_eq!(p.ssid, "");
    }
}

/// Power profile control.
///
/// The read prefers Arlen's own power daemon (`org.arlen.Power1`), which already
/// tracks the active profile for the battery indicator, and falls back to
/// `powerprofilesctl` when that daemon is not running. The write still shells
/// out: the daemon's `SetProfile` is gated on a `[system.power] set_profile`
/// grant no shell permission profile carries yet, so routing it there today
/// would deny every profile change.

/// The power daemon's well-known name, object and interface (all three coincide).
const POWER_DAEMON: &str = "org.arlen.Power1";
/// The power daemon's object path.
const POWER_PATH: &str = "/org/arlen/Power1";

/// Returns the current power profile ("power-saver", "balanced", "performance").
///
/// Reads Arlen's power daemon first so the shell and the daemon cannot disagree
/// about the active profile, then falls back to `powerprofilesctl` when the
/// daemon is absent or reports `unknown` (it uses that when it cannot resolve a
/// profile itself).
#[tauri::command]
pub async fn get_power_profile() -> Result<String, String> {
    if let Some(profile) = daemon_profile().await {
        return Ok(profile);
    }
    let output = tokio::process::Command::new("powerprofilesctl")
        .arg("get")
        .output()
        .await
        .map_err(|e| format!("powerprofilesctl not found: {e}"))?;

    if !output.status.success() {
        return Err("powerprofilesctl get failed".into());
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// The active profile per the power daemon, or `None` when it is unreachable or
/// does not know. `Profile` is an ungated property (unlike `SetProfile`), so this
/// read needs no capability grant.
async fn daemon_profile() -> Option<String> {
    let conn = zbus::Connection::session().await.ok()?;
    let proxy = zbus::Proxy::new(&conn, POWER_DAEMON, POWER_PATH, POWER_DAEMON)
        .await
        .ok()?;
    let profile: String = proxy.get_property("Profile").await.ok()?;
    // The daemon reports "unknown" when it cannot resolve one; treat that as no
    // answer so the caller falls through rather than showing "unknown" as a mode.
    if profile.is_empty() || profile == "unknown" {
        return None;
    }
    Some(profile)
}

/// Sets the power profile.
#[tauri::command]
pub async fn set_power_profile(profile: String) -> Result<(), String> {
    let status = tokio::process::Command::new("powerprofilesctl")
        .args(["set", &profile])
        .status()
        .await
        .map_err(|e| format!("powerprofilesctl set failed: {e}"))?;

    if !status.success() {
        return Err("powerprofilesctl set returned non-zero".into());
    }
    Ok(())
}

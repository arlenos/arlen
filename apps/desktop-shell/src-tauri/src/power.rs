/// Power profile control via powerprofilesctl.

/// Returns the current power profile ("power-saver", "balanced", "performance").
#[tauri::command]
pub async fn get_power_profile() -> Result<String, String> {
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

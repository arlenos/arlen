//! Brightness Tauri commands for the Settings panel.
//!
//! Settings runs in its own Tauri process and can't reach
//! desktop-shell's `brightness_*` commands directly. We mirror the
//! logic here: enumerate `/sys/class/backlight`, write via
//! `org.freedesktop.login1.Session.SetBrightness`. The shell's
//! QuickSettings slider and this Settings panel slider therefore
//! hit the same D-Bus method on the same login session and stay
//! in lock-step.
//!
//! The enumeration + the perceived-linear (`^2.2` gamma) slider math live in
//! `arlen-settings-core::brightness`, unit-tested in CI; this file is the command
//! layer + the logind `SetBrightness` D-Bus write.

use serde::Serialize;

use arlen_settings_core::brightness::{enumerate_devices, slider_to_raw, BacklightDevice};

async fn set_brightness_logind(device: &str, raw: u32) -> Result<(), String> {
    let conn = zbus::Connection::system()
        .await
        .map_err(|e| format!("system bus: {e}"))?;
    let proxy = zbus::Proxy::new(
        &conn,
        "org.freedesktop.login1",
        "/org/freedesktop/login1/session/auto",
        "org.freedesktop.login1.Session",
    )
    .await
    .map_err(|e| format!("login1 proxy: {e}"))?;
    proxy
        .call::<_, _, ()>("SetBrightness", &("backlight", device, raw))
        .await
        .map_err(|e| format!("SetBrightness: {e}"))?;
    Ok(())
}

/// State of one backlight device PLUS the gamma-adjusted slider
/// fraction for it. Frontend uses the fraction directly so it
/// doesn't need to know about gamma curves.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrightnessSnapshot {
    pub device: BacklightDevice,
    pub fraction: f32,
}

#[tauri::command]
pub async fn brightness_get_devices() -> Vec<BrightnessSnapshot> {
    tokio::task::spawn_blocking(enumerate_devices)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|d| {
            let fraction = d.current_fraction();
            BrightnessSnapshot {
                device: d,
                fraction,
            }
        })
        .collect()
}

#[tauri::command]
pub async fn brightness_set(device: String, value: f32) -> Result<u32, String> {
    let devices = tokio::task::spawn_blocking(enumerate_devices)
        .await
        .map_err(|e| format!("enumerate join: {e}"))?;
    let dev = devices
        .into_iter()
        .find(|d| d.name == device)
        .ok_or_else(|| format!("unknown backlight device '{device}'"))?;
    let raw = slider_to_raw(value, dev.max);
    set_brightness_logind(&dev.name, raw).await?;
    Ok(raw)
}

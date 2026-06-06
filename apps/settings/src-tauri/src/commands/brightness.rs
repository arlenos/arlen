/// Brightness Tauri commands for the Settings panel.
///
/// Settings runs in its own Tauri process and can't reach
/// desktop-shell's `brightness_*` commands directly. We mirror the
/// logic here: enumerate `/sys/class/backlight`, write via
/// `org.freedesktop.login1.Session.SetBrightness`. The shell's
/// QuickSettings slider and this Settings panel slider therefore
/// hit the same D-Bus method on the same login session and stay
/// in lock-step.
///
/// Slider math is also identical (`^2.2` perceived-linear gamma)
/// so what looks like 50 % in QuickSettings looks like 50 % here.

use std::fs;
use std::path::PathBuf;

use serde::Serialize;

const SYSFS_BACKLIGHT: &str = "/sys/class/backlight";
const PERCEIVED_GAMMA: f32 = 2.2;
const MIN_FRACTION: f32 = 0.01;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BacklightDevice {
    pub name: String,
    pub kind: String,
    pub max: u32,
    pub current: u32,
}

impl BacklightDevice {
    fn current_fraction(&self) -> f32 {
        if self.max == 0 {
            return 0.0;
        }
        let linear = self.current as f32 / self.max as f32;
        linear.powf(1.0 / PERCEIVED_GAMMA).clamp(0.0, 1.0)
    }
}

fn slider_to_raw(slider: f32, max: u32) -> u32 {
    if max == 0 {
        return 0;
    }
    let clamped = slider.clamp(MIN_FRACTION, 1.0);
    let linear = clamped.powf(PERCEIVED_GAMMA);
    let raw = (linear * max as f32).round().min(max as f32) as u32;
    raw.max(1)
}

fn enumerate_devices() -> Vec<BacklightDevice> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(SYSFS_BACKLIGHT) else {
        return out;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let dir = entry.path();
        let max = read_u32(&dir.join("max_brightness")).unwrap_or(0);
        let current = read_u32(&dir.join("actual_brightness"))
            .or_else(|| read_u32(&dir.join("brightness")))
            .unwrap_or(0);
        let kind = fs::read_to_string(dir.join("type"))
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|_| "raw".to_string());
        if max > 0 {
            out.push(BacklightDevice {
                name,
                kind,
                max,
                current,
            });
        }
    }
    out.sort_by(|a, b| {
        kind_priority(&a.kind)
            .cmp(&kind_priority(&b.kind))
            .then(a.name.cmp(&b.name))
    });
    out
}

fn kind_priority(kind: &str) -> u8 {
    match kind {
        "firmware" => 0,
        "platform" => 1,
        _ => 2,
    }
}

fn read_u32(path: &PathBuf) -> Option<u32> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slider_to_raw_floor_one_for_low_res_devices() {
        // max=100 with a 0% slider would round to 0 without the
        // floor — we never want a black screen from rounding.
        assert!(slider_to_raw(0.0, 100) >= 1);
        assert!(slider_to_raw(0.0, 7) >= 1);
    }

    #[test]
    fn slider_to_raw_max_is_max() {
        assert_eq!(slider_to_raw(1.0, 65535), 65535);
    }

    #[test]
    fn slider_to_raw_zero_max_returns_zero() {
        assert_eq!(slider_to_raw(0.5, 0), 0);
    }

    #[test]
    fn current_fraction_round_trips() {
        let max = 65535_u32;
        let original = 0.65_f32;
        let raw = slider_to_raw(original, max);
        let dev = BacklightDevice {
            name: "x".into(),
            kind: "firmware".into(),
            max,
            current: raw,
        };
        assert!((dev.current_fraction() - original).abs() < 0.005);
    }
}

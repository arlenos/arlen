//! Backlight enumeration + the perceived-linear slider math the settings
//! brightness commands wrap. Mirrors desktop-shell's `brightness_*` so the
//! QuickSettings slider and the Settings panel slider apply the same `^2.2`
//! gamma curve and stay in lock-step. Pure of Tauri and unit-tested in CI; the
//! commands + the logind `SetBrightness` D-Bus write stay in the host.

use std::fs;
use std::path::PathBuf;

use serde::Serialize;

const SYSFS_BACKLIGHT: &str = "/sys/class/backlight";
const PERCEIVED_GAMMA: f32 = 2.2;
const MIN_FRACTION: f32 = 0.01;

/// One `/sys/class/backlight` device: its name, kind, and raw max/current.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BacklightDevice {
    pub name: String,
    pub kind: String,
    pub max: u32,
    pub current: u32,
}

impl BacklightDevice {
    /// The gamma-corrected slider fraction (0..=1) for this device's current raw
    /// value, so the frontend can render the slider without knowing about gamma.
    pub fn current_fraction(&self) -> f32 {
        if self.max == 0 {
            return 0.0;
        }
        let linear = self.current as f32 / self.max as f32;
        linear.powf(1.0 / PERCEIVED_GAMMA).clamp(0.0, 1.0)
    }
}

/// Map a 0..=1 perceived slider fraction to a raw backlight value, applying the
/// `^2.2` gamma and never returning 0 for a non-zero-max device (a rounding-to-0
/// would black the screen).
pub fn slider_to_raw(slider: f32, max: u32) -> u32 {
    if max == 0 {
        return 0;
    }
    let clamped = slider.clamp(MIN_FRACTION, 1.0);
    let linear = clamped.powf(PERCEIVED_GAMMA);
    let raw = (linear * max as f32).round().min(max as f32) as u32;
    raw.max(1)
}

/// Enumerate the backlight devices, firmware/platform first, each with its raw
/// max/current read from sysfs. Skips devices with a zero max.
pub fn enumerate_devices() -> Vec<BacklightDevice> {
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

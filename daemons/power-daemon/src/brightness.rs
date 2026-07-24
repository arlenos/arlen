//! PWR-R3 dim executor: backlight save/restore for the idle policy.
//!
//! When an idle stage's action is [`Dim`](crate::idle::IdleAction::Dim) the
//! daemon lowers the screen brightness and restores it on the next input.
//! The value math (`percent -> raw`) and the `/sys/class/backlight`
//! enumeration are pure and unit-tested; the write goes through
//! `org.freedesktop.login1.Session.SetBrightness` (the unprivileged path -
//! logind writes sysfs as root on the session's behalf), the same method the
//! Settings brightness slider uses, so the two stay in lock-step.
//!
//! Kept self-contained in the daemon (a small sysfs read + one D-Bus call)
//! rather than depending on the Settings app's core, so the daemon carries no
//! app dependency.

use std::path::{Path, PathBuf};

/// One backlight device's current + maximum raw brightness.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Backlight {
    /// The device name (the `/sys/class/backlight/<name>` directory), passed
    /// to logind's `SetBrightness`.
    pub name: String,
    /// The current raw brightness.
    pub current: u32,
    /// The device maximum (`max_brightness`).
    pub max: u32,
}

/// A failure setting brightness over logind. Non-fatal: a dim that cannot be
/// applied leaves the screen as-is (the caller logs it).
#[derive(Debug, thiserror::Error)]
pub enum BrightnessError {
    /// The logind session proxy could not be built.
    #[error("login1 session proxy: {0}")]
    Proxy(zbus::Error),
    /// The `SetBrightness` call failed.
    #[error("SetBrightness: {0}")]
    Call(zbus::Error),
}

/// The raw value for `percent` of `max`, linear. A non-zero percent yields at
/// least `1` so a dim never fully blanks the screen (that is Blank's job); a
/// `0` percent yields `0`. Clamped to `[.., max]`.
pub fn percent_to_raw(percent: u8, max: u32) -> u32 {
    let pct = percent.min(100) as u64;
    let raw = (pct * max as u64 / 100) as u32;
    let floor = if percent > 0 { 1 } else { 0 };
    raw.clamp(floor, max.max(floor))
}

/// Enumerate the backlight devices under `root` (`/sys/class/backlight` in
/// production). A device missing/garbled `brightness`/`max_brightness` is
/// skipped rather than failing the whole read.
pub fn enumerate_in(root: &Path) -> Vec<Backlight> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for e in entries.flatten() {
        let dir = e.path();
        let name = match e.file_name().to_str() {
            Some(n) => n.to_string(),
            None => continue,
        };
        let read_u32 = |f: &str| -> Option<u32> {
            std::fs::read_to_string(dir.join(f)).ok()?.trim().parse().ok()
        };
        if let (Some(current), Some(max)) = (read_u32("brightness"), read_u32("max_brightness")) {
            if max > 0 {
                out.push(Backlight { name, current, max });
            }
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// The production backlight enumeration (`/sys/class/backlight`).
pub fn enumerate() -> Vec<Backlight> {
    enumerate_in(&PathBuf::from("/sys/class/backlight"))
}

/// Set `device` to `raw` via logind's `SetBrightness` (subsystem `backlight`).
pub async fn set_via_logind(
    conn: &zbus::Connection,
    device: &str,
    raw: u32,
) -> Result<(), BrightnessError> {
    let proxy = zbus::Proxy::new(
        conn,
        "org.freedesktop.login1",
        "/org/freedesktop/login1/session/auto",
        "org.freedesktop.login1.Session",
    )
    .await
    .map_err(BrightnessError::Proxy)?;
    proxy
        .call::<_, _, ()>("SetBrightness", &("backlight", device, raw))
        .await
        .map_err(BrightnessError::Call)?;
    Ok(())
}

/// Holds the pre-dim brightness so a resume restores it. One per idle
/// consumer; a second `dim` while already dimmed keeps the ORIGINAL saved
/// value (it does not overwrite the save with the dimmed reading).
#[derive(Debug, Default)]
pub struct Dimmer {
    /// The (device, raw) brightness captured at the last `dim`, restored on
    /// `restore`. `None` when not currently dimmed.
    saved: Option<Vec<(String, u32)>>,
}

impl Dimmer {
    /// A dimmer holding no saved state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether a dim is currently applied (state is saved for restore).
    pub fn is_dimmed(&self) -> bool {
        self.saved.is_some()
    }

    /// Save the current brightness (unless already dimmed) and set each device
    /// to `to_percent`. Best-effort per device; a device that fails to set is
    /// logged and skipped (its saved value still restores on resume).
    pub async fn dim(&mut self, conn: &zbus::Connection, to_percent: u8) {
        let devices = enumerate();
        if devices.is_empty() {
            return;
        }
        if self.saved.is_none() {
            self.saved = Some(devices.iter().map(|d| (d.name.clone(), d.current)).collect());
        }
        for d in &devices {
            let raw = percent_to_raw(to_percent, d.max);
            if let Err(e) = set_via_logind(conn, &d.name, raw).await {
                tracing::warn!(device = %d.name, "idle dim failed: {e}");
            }
        }
    }

    /// Restore the saved brightness and clear the saved state. A no-op when
    /// not dimmed.
    pub async fn restore(&mut self, conn: &zbus::Connection) {
        let Some(saved) = self.saved.take() else {
            return;
        };
        for (name, raw) in saved {
            if let Err(e) = set_via_logind(conn, &name, raw).await {
                tracing::warn!(device = %name, "idle dim restore failed: {e}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_to_raw_is_linear_with_a_visible_floor() {
        assert_eq!(percent_to_raw(50, 1000), 500);
        assert_eq!(percent_to_raw(30, 100), 30);
        assert_eq!(percent_to_raw(100, 255), 255);
        // A non-zero percent never fully blanks (floor 1), even when rounding
        // would land on 0.
        assert_eq!(percent_to_raw(1, 10), 1);
        assert_eq!(percent_to_raw(5, 10), 1); // 0.5 -> 0 -> floored to 1
        // 0 percent is an explicit off; over-100 clamps.
        assert_eq!(percent_to_raw(0, 1000), 0);
        assert_eq!(percent_to_raw(250, 1000), 1000);
    }

    #[test]
    fn enumerate_reads_devices_and_skips_broken_ones() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        // A good device.
        let good = root.join("intel_backlight");
        std::fs::create_dir(&good).unwrap();
        std::fs::write(good.join("brightness"), "120\n").unwrap();
        std::fs::write(good.join("max_brightness"), "255\n").unwrap();
        // A device with a zero max (skipped).
        let zero = root.join("zero");
        std::fs::create_dir(&zero).unwrap();
        std::fs::write(zero.join("brightness"), "0\n").unwrap();
        std::fs::write(zero.join("max_brightness"), "0\n").unwrap();
        // A device missing max_brightness (skipped).
        let partial = root.join("partial");
        std::fs::create_dir(&partial).unwrap();
        std::fs::write(partial.join("brightness"), "50\n").unwrap();

        let devices = enumerate_in(root);
        assert_eq!(
            devices,
            vec![Backlight {
                name: "intel_backlight".into(),
                current: 120,
                max: 255,
            }]
        );
    }

    #[test]
    fn enumerate_missing_root_is_empty() {
        assert!(enumerate_in(Path::new("/nonexistent/backlight")).is_empty());
    }

    #[test]
    fn a_fresh_dimmer_holds_no_state() {
        assert!(!Dimmer::new().is_dimmed());
    }

    /// Non-intrusive runtime verify of the logind `SetBrightness` path: set
    /// each backlight to its CURRENT value (no visible change) and confirm the
    /// call succeeds, proving the D-Bus write works without touching what the
    /// user sees. `#[ignore]d`: needs a logind session + a real backlight.
    #[tokio::test]
    #[ignore = "needs a logind session and a backlight device"]
    async fn set_via_logind_no_op_succeeds() {
        let devices = enumerate();
        if devices.is_empty() {
            eprintln!("no backlight device; skipping the logind verify");
            return;
        }
        let conn = zbus::Connection::system().await.expect("system bus");
        for d in &devices {
            set_via_logind(&conn, &d.name, d.current)
                .await
                .unwrap_or_else(|e| panic!("SetBrightness no-op on {} failed: {e}", d.name));
        }
    }
}

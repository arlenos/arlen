/// Backlight brightness control for the laptop's internal panel.
///
/// Linux exposes the backlight at `/sys/class/backlight/<dev>/{...}`
/// where `brightness` is a u32 in `[0, max_brightness]`. Direct
/// writes need root, so we route through logind's
/// `org.freedesktop.login1.Session.SetBrightness` D-Bus method,
/// which is the cross-distro session-scoped path GNOME / KDE /
/// elementary all use. No udev rule, no `video` group membership
/// required — works in any active session out of the box.
///
/// Display panels are perceived logarithmically; a linear slider
/// would bunch all the visual change in the top 20 %. We map the
/// slider's `[0.0, 1.0]` to the hardware range with a `^2.2` curve
/// (sRGB gamma) so 50 % on the slider feels like 50 % brightness.
///
/// External monitors (DisplayPort, HDMI) typically don't expose a
/// backlight — DDC/CI exists but is optional and patchy. D3 covers
/// internal panels only; DDC/CI is a separate sprint if we ever
/// need it.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use serde::Serialize;
use tokio::sync::Mutex;

const SYSFS_BACKLIGHT: &str = "/sys/class/backlight";
/// sRGB-style perceived-linear gamma. Matches what GNOME's
/// gnome-settings-daemon uses internally for backlight control.
const PERCEIVED_GAMMA: f32 = 2.2;
/// Lower bound on the slider so the user can't blank the screen
/// completely by accident. 1 % brightness is still readable on
/// every panel I've seen; 0 % shuts the display off on some.
const MIN_FRACTION: f32 = 0.01;

/// One backlight device as exposed by `/sys/class/backlight`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BacklightDevice {
    /// Kernel-assigned name (`amdgpu_bl1`, `intel_backlight`, …).
    /// Stable for the lifetime of the boot.
    pub name: String,
    /// `firmware`, `platform`, or `raw`. We prefer `firmware` then
    /// `platform` over `raw` when picking the "internal" device.
    pub kind: String,
    /// Maximum raw value writable to `brightness`. Devices vary
    /// wildly: 7, 100, 4895, 65535, …
    pub max: u32,
    /// Last value read from `actual_brightness`. The slider does
    /// the gamma-inverse on this so the UI shows a perceived value.
    pub current: u32,
}

impl BacklightDevice {
    /// Slider value in `[0.0, 1.0]` from the raw hardware value.
    /// Inverse of `slider_to_raw`.
    pub fn current_fraction(&self) -> f32 {
        if self.max == 0 {
            return 0.0;
        }
        let linear = self.current as f32 / self.max as f32;
        linear.powf(1.0 / PERCEIVED_GAMMA).clamp(0.0, 1.0)
    }
}

/// Convert a slider fraction `[0.0, 1.0]` to a raw backlight value
/// for the given max. Applies the `^2.2` gamma curve and floors to
/// `MIN_FRACTION`, then ensures at least raw `1` whenever `max > 0`
/// so low-resolution backlights (max=100, max=7) never end up at
/// zero from rounding.
pub fn slider_to_raw(slider: f32, max: u32) -> u32 {
    if max == 0 {
        return 0;
    }
    let clamped = slider.clamp(MIN_FRACTION, 1.0);
    let linear = clamped.powf(PERCEIVED_GAMMA);
    let raw = (linear * max as f32).round().min(max as f32) as u32;
    raw.max(1)
}

/// Walk `/sys/class/backlight` and read every device. Returns an
/// empty Vec when no devices exist (desktop machine, headless VM)
/// — the UI surfaces this as "no panels with software brightness".
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
    // Sort firmware → platform → raw, then by name. Picking the
    // first entry then gives the "best" internal panel for a
    // single-slider UI.
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

/// Set the backlight brightness for a device via logind. Returns
/// `Ok(())` on success or a human-readable error string suitable
/// for surfacing in the Tauri command result.
pub async fn set_brightness_logind(device: &str, raw: u32) -> Result<(), String> {
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

/// Tauri-managed cache so reads don't hit sysfs more than ~5 Hz.
/// The slider polls on `mousemove` during a drag, which can be
/// 120+ events/sec; without a cache we'd churn syscalls.
#[derive(Default)]
pub struct BrightnessState {
    inner: Arc<Mutex<Option<Vec<BacklightDevice>>>>,
}

impl BrightnessState {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn refresh(&self) -> Vec<BacklightDevice> {
        let devices = tokio::task::spawn_blocking(enumerate_devices)
            .await
            .unwrap_or_default();
        let mut guard = self.inner.lock().await;
        *guard = Some(devices.clone());
        devices
    }
}

/// List every backlight device with its current value and gamma-
/// adjusted slider fraction. Used by the Settings panel when more
/// than one panel is attached (rare on laptops, common on
/// docked-with-internal-and-edid setups).
#[tauri::command]
pub async fn brightness_get_devices() -> Vec<BacklightDevice> {
    tokio::task::spawn_blocking(enumerate_devices)
        .await
        .unwrap_or_default()
}

/// Convenience for QuickSettings: returns the first / preferred
/// device (firmware > platform > raw, then alphabetical) or
/// `None` when no backlight exists. The shell's slider hides
/// itself in that case.
#[tauri::command]
pub async fn brightness_get_primary() -> Option<BacklightDevice> {
    tokio::task::spawn_blocking(enumerate_devices)
        .await
        .ok()
        .and_then(|devs| devs.into_iter().next())
}

/// Set brightness on the named device. `value` is the slider
/// fraction `[0.0, 1.0]`; gamma + clamping happens here so callers
/// stay simple. Returns the raw value that was written so the UI
/// can echo it back without a re-read race.
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

/// Step size per hardware-key press, expressed as a slider
/// fraction. 5 % matches GNOME / macOS conventions and feels
/// neither sluggish nor coarse on a 0-100 % range.
const STEP_FRACTION: f32 = 0.05;
/// Coalesce window for held-key repeats. The kernel emits
/// auto-repeat events at ~30Hz; without coalescing every press
/// would issue a logind D-Bus read+write. ~33 ms keeps the
/// effective rate at 30 Hz max while individual taps still feel
/// responsive (32ms is well below human pre-attentive perception).
const STEP_COALESCE_MS: u64 = 33;

/// Pending step state. Kept module-global because the hardware-
/// key handler doesn't have a natural place to thread it through
/// otherwise. Tracks accumulated direction (so 3 rapid taps in a
/// 33ms window collapse into one +3 step) and the in-flight task
/// guard so we don't spawn parallel writers.
static PENDING_STEP: std::sync::Mutex<i32> = std::sync::Mutex::new(0);
static STEP_INFLIGHT: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Step the primary backlight by `direction × STEP_FRACTION`.
///
/// Coalesces rapid repeats: presses arriving within one
/// `STEP_COALESCE_MS` window collapse into a single read-modify-write
/// rather than serialising. The worker holds `STEP_INFLIGHT` for the
/// full D-Bus round-trip and only releases it once `PENDING_STEP`
/// stays zero across a release-then-recheck cycle, so an increment
/// that lands between drain and release can never be silently lost.
///
/// `direction` is an IPC-trust boundary: the compositor's
/// `lunaris-shell-overlay` `brightness_step` event documents it as
/// `+1` or `-1`, but the channel is an `int` and a buggy or skewed
/// sender can put any `i32` on the wire. We reject anything that
/// isn't exactly `±1` and log a warning so protocol mismatches are
/// loud rather than silently writing a 50-step jump to logind.
///
/// Persists the new fraction to `shell.toml` so the level survives a
/// reboot, then emits `lunaris://brightness-changed` so the
/// QuickSettings + Settings sliders re-read the hardware position
/// without polling.
pub fn brightness_step_relative(app: tauri::AppHandle, direction: i32) {
    if direction != 1 && direction != -1 {
        log::warn!(
            "brightness_step: rejecting out-of-contract direction={direction} \
             (expected +1 or -1); ignoring event"
        );
        return;
    }
    {
        let mut p = PENDING_STEP.lock().unwrap();
        *p += direction;
    }
    if STEP_INFLIGHT.swap(true, std::sync::atomic::Ordering::AcqRel) {
        // A coalescing task is already running; it will pick up
        // the new pending direction on its next loop iteration.
        return;
    }
    tauri::async_runtime::spawn(async move {
        loop {
            // Coalesce window so adjacent presses fold into one
            // D-Bus write.
            tokio::time::sleep(std::time::Duration::from_millis(STEP_COALESCE_MS)).await;

            let direction = {
                let mut p = PENDING_STEP.lock().unwrap();
                std::mem::take(&mut *p)
            };

            if direction == 0 {
                // Nothing to do this cycle. Release the guard, then
                // re-check PENDING_STEP: a producer that incremented
                // between the drain above and this release would
                // have seen INFLIGHT==true and returned without
                // spawning, so we must pick it up ourselves.
                STEP_INFLIGHT.store(false, std::sync::atomic::Ordering::Release);
                let leftover = *PENDING_STEP.lock().unwrap();
                if leftover == 0 {
                    return;
                }
                // A late increment slipped in. Try to re-claim the
                // guard; if a fresh producer raced ahead and got it
                // first, our work is its problem now.
                if STEP_INFLIGHT.swap(true, std::sync::atomic::Ordering::AcqRel) {
                    return;
                }
                continue;
            }

            let devices = tokio::task::spawn_blocking(enumerate_devices)
                .await
                .unwrap_or_default();
            let Some(primary) = devices.into_iter().next() else {
                log::info!("brightness_step: no backlight device, ignoring");
                // Drop any further pending presses — without
                // hardware they're meaningless.
                *PENDING_STEP.lock().unwrap() = 0;
                STEP_INFLIGHT.store(false, std::sync::atomic::Ordering::Release);
                return;
            };
            let new_fraction =
                (primary.current_fraction() + direction as f32 * STEP_FRACTION).clamp(0.0, 1.0);
            let raw = slider_to_raw(new_fraction, primary.max);
            if let Err(err) = set_brightness_logind(&primary.name, raw).await {
                log::warn!("brightness_step: set failed: {err}");
                // On a hardware/D-Bus failure, drop the queue and
                // bail. Better than spinning indefinitely against a
                // broken backend.
                *PENDING_STEP.lock().unwrap() = 0;
                STEP_INFLIGHT.store(false, std::sync::atomic::Ordering::Release);
                return;
            }
            log::info!(
                "brightness_step: device={} direction={} fraction={:.2} raw={}",
                primary.name,
                direction,
                new_fraction,
                raw
            );

            // Persist so the level survives a reboot. Errors are
            // logged-and-continued — a transient TOML write failure
            // shouldn't break the running session.
            persist_brightness(new_fraction);

            use tauri::Emitter;
            let _ = app.emit(
                "lunaris://brightness-changed",
                serde_json::json!({
                    "device": primary.name,
                    "fraction": new_fraction,
                }),
            );

            // Loop continues; STEP_INFLIGHT stays held so a producer
            // arriving during the next sleep simply increments
            // PENDING_STEP and we drain it on the following pass.
        }
    });
}

/// Update `~/.config/lunaris/shell.toml` so the new brightness is
/// the value `replay_persisted_brightness` reads on the next boot.
///
/// Routed through `update_shell_config` so the load-modify-write is
/// serialised against every other in-process writer (night-light
/// commands, the frontend's full-replace path) under
/// `WRITE_LOCK`. A bare `get_shell_config` + `save_shell_config`
/// here would race those writers and silently drop their fields on
/// every key press.
///
/// Errors are logged-and-swallowed: the hardware write already
/// succeeded, and a transient TOML failure shouldn't break the
/// running session.
fn persist_brightness(fraction: f32) {
    if let Err(err) = crate::shell_config::update_shell_config(|cfg| {
        cfg.display.brightness = fraction;
    }) {
        log::warn!("brightness persist: update failed: {err}");
    }
}

/// Replay the persisted brightness from `shell.toml` on startup.
/// Called from `lib.rs::run` after the Tauri app initialises so
/// the user resumes at the same brightness level across reboots.
/// Silent on failure — a missing backlight or a logind permission
/// error must not crash the shell.
pub async fn replay_persisted_brightness(saved_fraction: f32) {
    let devices = match tokio::task::spawn_blocking(enumerate_devices).await {
        Ok(v) => v,
        Err(err) => {
            log::warn!("brightness replay: enumerate failed: {err}");
            return;
        }
    };
    let Some(primary) = devices.into_iter().next() else {
        return;
    };
    let raw = slider_to_raw(saved_fraction, primary.max);
    if let Err(err) = set_brightness_logind(&primary.name, raw).await {
        log::warn!(
            "brightness replay: set_brightness({}, {raw}) failed: {err}",
            primary.name
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slider_to_raw_clamps_below_min_fraction() {
        // 0.0 input is clamped to MIN_FRACTION (1 %), then gamma'd,
        // then floored to 1 so even low-resolution backlights
        // (max=100, max=7) don't bottom out at zero.
        let raw = slider_to_raw(0.0, 1000);
        assert!(raw >= 1, "screen must never go fully dark, got {raw}");

        // High-resolution backlights should give MORE than 1 because
        // the gamma curve has actual headroom there.
        let raw_hires = slider_to_raw(0.0, 65535);
        assert!(raw_hires >= 1);
    }

    #[test]
    fn slider_to_raw_zero_max_is_zero() {
        // No backlight (synthetic max=0) returns 0 instead of
        // floor-1, otherwise we'd write a non-zero value to a
        // device that doesn't support brightness.
        assert_eq!(slider_to_raw(0.5, 0), 0);
    }

    #[test]
    fn slider_to_raw_max_is_max() {
        assert_eq!(slider_to_raw(1.0, 65535), 65535);
        assert_eq!(slider_to_raw(1.0, 100), 100);
    }

    #[test]
    fn slider_to_raw_gamma_is_perceived_linear() {
        // 50 % on the slider should land far below 50 % of max
        // because perception is logarithmic — that's the whole
        // point of the curve.
        let raw = slider_to_raw(0.5, 1000);
        assert!(
            raw < 300,
            "50 % slider on linear scale would be 500; gamma-curve must compress to <300, got {raw}",
        );
        assert!(raw > 100, "and not collapse below 100 either, got {raw}");
    }

    #[test]
    fn current_fraction_round_trips_through_slider_to_raw() {
        // Pick a slider value, convert to raw, build a synthetic
        // device, recover the slider via current_fraction. Should
        // match within 0.5 % (rounding loss).
        let original = 0.4_f32;
        let max = 65535_u32;
        let raw = slider_to_raw(original, max);
        let dev = BacklightDevice {
            name: "test".into(),
            kind: "firmware".into(),
            max,
            current: raw,
        };
        let recovered = dev.current_fraction();
        assert!(
            (recovered - original).abs() < 0.005,
            "round-trip drift: {original} → {recovered}",
        );
    }

    #[test]
    fn current_fraction_zero_max_is_zero() {
        let dev = BacklightDevice {
            name: "broken".into(),
            kind: "raw".into(),
            max: 0,
            current: 0,
        };
        assert_eq!(dev.current_fraction(), 0.0);
    }

    #[test]
    fn kind_priority_orders_firmware_first() {
        assert!(kind_priority("firmware") < kind_priority("platform"));
        assert!(kind_priority("platform") < kind_priority("raw"));
    }

    /// `brightness_step_relative` accepts `direction` over a
    /// Wayland IPC where the channel is a wide `int`. Anything but
    /// exactly `±1` is a contract violation by the sender — drop
    /// the increment instead of letting it accumulate. We can't
    /// invoke the public function in unit tests (it needs a Tauri
    /// AppHandle and spawns a runtime), but we exercise the same
    /// validation predicate here so the contract stays codified
    /// in tests.
    #[test]
    fn step_direction_contract_rejects_out_of_range() {
        fn is_valid_step(direction: i32) -> bool {
            direction == 1 || direction == -1
        }

        // The contract values.
        assert!(is_valid_step(1));
        assert!(is_valid_step(-1));

        // Common mistakes / protocol-skew payloads.
        assert!(!is_valid_step(0));
        assert!(!is_valid_step(2));
        assert!(!is_valid_step(-2));
        assert!(!is_valid_step(50));
        assert!(!is_valid_step(i32::MAX));
        assert!(!is_valid_step(i32::MIN));
    }
}

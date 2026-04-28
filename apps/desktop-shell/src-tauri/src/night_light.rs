/// Tauri commands for the night-light backend.
///
/// Each command persists the change to `shell.toml` AND dispatches
/// the corresponding `lunaris-shell-overlay` request so the
/// compositor's gamma engine reflects the new state without a
/// reload. The compositor is the source of truth for "warm tint
/// is right now active"; `shell.toml` is the source of truth for
/// the user's intent and survives reboots.

use std::sync::Arc;

use tauri::State;

use crate::shell_config::{
    NightLightSchedule, ShellConfig, get_shell_config, save_shell_config,
};
use crate::shell_overlay_client::ShellOverlaySender;

/// Toggle night light on/off and update the target temperature.
/// The temperature is preserved in shell.toml even when disabled
/// so the next time the user flips the toggle on, it resumes at
/// the same value.
#[tauri::command]
pub fn night_light_set(
    enabled: bool,
    temperature: u16,
    sender: State<'_, Arc<ShellOverlaySender>>,
) -> Result<(), String> {
    let mut cfg = get_shell_config().unwrap_or_default();
    cfg.night_light.enabled = enabled;
    cfg.night_light.temperature = temperature;
    save_shell_config(cfg.clone())?;
    sender.set_night_light(enabled, temperature as u32);
    Ok(())
}

/// Update the schedule mode and (for custom mode) the start/end
/// times. `schedule` is one of `"manual" | "sunset_sunrise" | "custom"`.
/// Times are minutes-since-midnight. The compositor re-evaluates
/// immediately, so a custom-mode change can flip the current
/// effective state.
#[tauri::command]
pub fn night_light_set_schedule(
    schedule: String,
    custom_start: u32,
    custom_end: u32,
    sender: State<'_, Arc<ShellOverlaySender>>,
) -> Result<(), String> {
    let parsed = match schedule.as_str() {
        "manual" => NightLightSchedule::Manual,
        "sunset_sunrise" => NightLightSchedule::SunsetSunrise,
        "custom" => NightLightSchedule::Custom,
        other => return Err(format!("unknown schedule '{other}'")),
    };
    let mut cfg = get_shell_config().unwrap_or_default();
    cfg.night_light.schedule = parsed;
    cfg.night_light.custom_start = custom_start;
    cfg.night_light.custom_end = custom_end;
    save_shell_config(cfg.clone())?;
    sender.set_night_light_schedule(parsed.to_protocol(), custom_start, custom_end);
    Ok(())
}

/// Update the user's geographic location. Used by the
/// `sunset_sunrise` schedule mode. `(0.0, 0.0)` is treated as
/// "unset" by the compositor.
#[tauri::command]
pub fn night_light_set_location(
    latitude: f64,
    longitude: f64,
    sender: State<'_, Arc<ShellOverlaySender>>,
) -> Result<(), String> {
    let mut cfg = get_shell_config().unwrap_or_default();
    cfg.night_light.latitude = latitude;
    cfg.night_light.longitude = longitude;
    save_shell_config(cfg.clone())?;
    sender.set_night_light_location(latitude, longitude);
    Ok(())
}

/// Push the persisted night-light state to the compositor on
/// startup so the gamma engine matches what shell.toml said at the
/// last shutdown. Spawns a background thread that waits up to 5
/// seconds for the shell-overlay sender's Wayland proxy to bind,
/// then replays the location → schedule → enabled triple. Calls on
/// an unbound proxy are silent no-ops, so a slightly-too-early
/// invocation just gets retried in the next loop.
pub fn replay_persisted_state(sender: Arc<ShellOverlaySender>) {
    std::thread::spawn(move || {
        let cfg: ShellConfig = match get_shell_config() {
            Ok(c) => c,
            Err(err) => {
                log::warn!("night_light replay: read shell.toml failed: {err}");
                return;
            }
        };
        // Poll up to 5s for the overlay proxy to bind. The first
        // tick after connect succeeds; the others are no-ops.
        for _ in 0..50 {
            if sender.is_bound() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        sender.set_night_light_location(
            cfg.night_light.latitude,
            cfg.night_light.longitude,
        );
        sender.set_night_light_schedule(
            cfg.night_light.schedule.to_protocol(),
            cfg.night_light.custom_start,
            cfg.night_light.custom_end,
        );
        sender.set_night_light(
            cfg.night_light.enabled,
            cfg.night_light.temperature as u32,
        );
        log::info!(
            "night_light: replayed persisted state (enabled={}, temp={}K, schedule={:?})",
            cfg.night_light.enabled,
            cfg.night_light.temperature,
            cfg.night_light.schedule,
        );
    });
}

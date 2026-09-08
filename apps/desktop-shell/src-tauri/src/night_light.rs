//! Tauri commands for the night-light backend.
//!
//! Each command persists the change to `shell.toml` AND dispatches
//! the corresponding `arlen-shell-overlay` request so the
//! compositor's gamma engine reflects the new state without a
//! reload. The compositor is the source of truth for "warm tint
//! is right now active"; `shell.toml` is the source of truth for
//! the user's intent and survives reboots.

use std::sync::Arc;

use tauri::State;

use crate::shell_config::{
    ShellConfig, get_shell_config, update_shell_config,
};
use crate::shell_overlay_client::ShellOverlaySender;

/// Toggle night light on/off and update the target temperature.
/// The temperature is preserved in shell.toml even when disabled
/// so the next time the user flips the toggle on, it resumes at
/// the same value.
///
/// NOT a `#[tauri::command]`, and it stopped being one on 8 September. Nothing
/// in this app's frontend invoked it: the badge and the launcher both go through
/// `quick_action_run` with `qa.toggle_night_light`, which lands here. Settings
/// cannot reach it either - a Tauri command does not cross an app boundary, which
/// is why Settings writes `shell.toml` and the watcher in `shell_config` relays
/// the change to the compositor. So the registration said the frontend may reach
/// this, and no frontend could.
pub fn night_light_set(
    enabled: bool,
    temperature: u16,
    sender: State<'_, Arc<ShellOverlaySender>>,
) -> Result<(), String> {
    update_shell_config(|cfg| {
        cfg.night_light.enabled = enabled;
        cfg.night_light.temperature = temperature;
    })?;
    sender.set_night_light(enabled, temperature as u32);
    Ok(())
}

// The schedule and location setters lived here and are deleted. They were
// commands with no caller in either direction: this app's frontend never invoked
// them, and Settings - which owns the schedule and location controls - could not,
// because a Tauri command does not cross an app boundary. Settings writes
// `shell.toml` and `shell_config`'s watcher calls `replay_persisted_state` below,
// which pushes location, schedule and enabled to the compositor on every change.
// That is the live path and it covers what these two did, so they were a second
// writer nothing wrote through rather than a faster one.

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

/// Tauri commands for the Display panel.
///
/// Handlers go through `WaylandHandle` (managed Tauri state set up
/// in `lib.rs::run`). The wayland thread owns the live state; we
/// snapshot it for read commands and post commands for writes.
///
/// Apply pattern (mirrors `display-system.md` §A4):
///   1. Frontend calls `display_apply_config(new_config)`. We
///      build a snapshot of the current state, send the new config
///      to the compositor, and return the request id + snapshot.
///   2. Compositor replies with succeeded / failed / cancelled,
///      emitted to the frontend as `displays:apply-result`.
///   3. Frontend's revert modal counts down 15 s. On confirm:
///      `display_save_current()` persists. On timeout / cancel:
///      `display_apply_config(snapshot)` with the saved old state.

use std::sync::Arc;

use tauri::{AppHandle, State};

use crate::displays::{
    types::{Monitor, MonitorConfig},
    wayland_client::{WaylandCommand, WaylandHandle},
};

/// Convenience alias so callers don't have to know about the inner
/// Arc / Mutex shape.
pub type ManagedWaylandHandle = Arc<WaylandHandle>;

/// Return the currently-published monitor list from the wayland
/// thread. Called by the frontend on first mount and again when a
/// `displays:changed` event fires.
#[tauri::command]
pub fn display_get_monitors(
    handle: State<'_, ManagedWaylandHandle>,
) -> Result<Vec<Monitor>, String> {
    let state = handle
        .state
        .lock()
        .map_err(|e| format!("display state lock poisoned: {e}"))?;
    let mons = state.monitors.clone();
    log::info!(
        "displays: display_get_monitors -> {} monitor(s) (serial={})",
        mons.len(),
        state.serial,
    );
    Ok(mons)
}

/// Send a new configuration to the compositor. Returns the request
/// id; the compositor's reply arrives as a `displays:apply-result`
/// Tauri event tagged with the same id.
#[tauri::command]
pub fn display_apply_config(
    config: Vec<MonitorConfig>,
    handle: State<'_, ManagedWaylandHandle>,
) -> Result<DisplayApplyHandle, String> {
    let request_id = generate_request_id();

    // Snapshot the live state before we send the new config. The
    // frontend hands this snapshot back to `display_revert` if the
    // user cancels or the 15-s timer fires.
    let snapshot: Vec<MonitorConfig> = {
        let state = handle
            .state
            .lock()
            .map_err(|e| format!("display state lock poisoned: {e}"))?;
        state.monitors.iter().map(monitor_to_config).collect()
    };

    handle
        .sender
        .send(WaylandCommand::Apply {
            request_id: request_id.clone(),
            config,
        })
        .map_err(|e| format!("wayland thread closed: {e}"))?;

    Ok(DisplayApplyHandle {
        request_id,
        snapshot,
    })
}

/// Apply a previously-captured snapshot. Used by the revert modal.
/// Re-uses `display_apply_config` semantics but does not return a
/// new snapshot — by the time the revert fires we already have the
/// authoritative pre-change state.
#[tauri::command]
pub fn display_revert(
    snapshot: Vec<MonitorConfig>,
    handle: State<'_, ManagedWaylandHandle>,
) -> Result<String, String> {
    let request_id = generate_request_id();
    handle
        .sender
        .send(WaylandCommand::Apply {
            request_id: request_id.clone(),
            config: snapshot,
        })
        .map_err(|e| format!("wayland thread closed: {e}"))?;
    Ok(request_id)
}

/// Persist the current monitor topology to
/// `~/.config/lunaris/compositor.d/displays.toml` as a single
/// matching profile. The compositor itself rewrites the same file
/// on every applied change, but Settings calls this explicitly
/// after the user confirms the revert dialog so the persisted
/// profile is *exactly* what the user just kept (no race with
/// concurrent re-saves from the compositor's PersistenceGuard).
///
/// Hot-plug-profile management lands in D4; for now the call writes
/// a single profile keyed by the current `output_set` and overwrites
/// any existing entry for the same set.
#[tauri::command]
pub fn display_save_current(
    handle: State<'_, ManagedWaylandHandle>,
    _app: AppHandle,
) -> Result<(), String> {
    use cosmic_comp_config::output::comp::{
        AdaptiveSync, OutputConfig, OutputInfo, OutputState, OutputsConfig, TransformDef,
    };
    use cosmic_comp_config::output::displays_toml;

    let monitors = {
        let state = handle
            .state
            .lock()
            .map_err(|e| format!("display state lock poisoned: {e}"))?;
        state.monitors.clone()
    };

    if monitors.is_empty() {
        // Nothing to save: avoid clobbering the file with an empty
        // profile that would lose the user's other layouts.
        return Ok(());
    }

    let mut infos: Vec<OutputInfo> = monitors
        .iter()
        .map(|m| OutputInfo {
            connector: m.connector.clone(),
            make: if m.make.is_empty() {
                "Unknown".into()
            } else {
                m.make.clone()
            },
            model: if m.model.is_empty() {
                "Unknown".into()
            } else {
                m.model.clone()
            },
        })
        .collect();
    infos.sort();

    let mut by_connector: std::collections::HashMap<String, OutputConfig> = monitors
        .iter()
        .map(|m| {
            let mode = m.current_mode.and_then(|i| m.modes.get(i)).cloned();
            let conf = OutputConfig {
                mode: mode
                    .map(|md| ((md.width, md.height), Some(md.refresh_mhz)))
                    .unwrap_or(((0, 0), None)),
                vrr: match m.vrr {
                    crate::displays::types::VrrState::Enabled => AdaptiveSync::Enabled,
                    crate::displays::types::VrrState::Disabled => AdaptiveSync::Disabled,
                    crate::displays::types::VrrState::Force => AdaptiveSync::Force,
                },
                scale: m.scale,
                transform: transform_to_def(m.transform),
                position: (m.position.x.max(0) as u32, m.position.y.max(0) as u32),
                enabled: if !m.enabled {
                    OutputState::Disabled
                } else if let Some(target) = &m.mirroring {
                    OutputState::Mirroring(target.clone())
                } else {
                    OutputState::Enabled
                },
                max_bpc: if m.max_bpc == 0 {
                    None
                } else {
                    Some(m.max_bpc)
                },
                xwayland_primary: m.primary,
            };
            (m.connector.clone(), conf)
        })
        .collect();

    let outputs: Vec<OutputConfig> = infos
        .iter()
        .filter_map(|info| by_connector.remove(&info.connector))
        .collect();

    let mut cfg = displays_toml::load(&displays_toml_path()?);
    cfg.config.insert(infos, outputs);

    displays_toml::save(&displays_toml_path()?, &cfg).map_err(|e| e.to_string())
}

fn displays_toml_path() -> Result<std::path::PathBuf, String> {
    let home = std::env::var("HOME").map_err(|_| "HOME not set".to_string())?;
    Ok(std::path::PathBuf::from(home)
        .join(".config/lunaris/compositor.d/displays.toml"))
}

fn transform_to_def(
    t: crate::displays::types::Transform,
) -> cosmic_comp_config::output::comp::TransformDef {
    use cosmic_comp_config::output::comp::TransformDef as D;
    use crate::displays::types::Transform as T;
    match t {
        T::Normal => D::Normal,
        T::Rotate90 => D::_90,
        T::Rotate180 => D::_180,
        T::Rotate270 => D::_270,
        T::Flipped => D::Flipped,
        T::Flipped90 => D::Flipped90,
        T::Flipped180 => D::Flipped180,
        T::Flipped270 => D::Flipped270,
    }
}

fn monitor_to_config(m: &Monitor) -> MonitorConfig {
    use crate::displays::types::EnabledKind;
    MonitorConfig {
        connector: m.connector.clone(),
        mode_index: m.current_mode,
        position: m.position,
        scale: m.scale,
        transform: m.transform,
        enabled: if !m.enabled {
            EnabledKind::Disabled
        } else if let Some(target) = &m.mirroring {
            EnabledKind::Mirror(target.clone())
        } else {
            EnabledKind::Active
        },
        vrr: m.vrr,
        primary: m.primary,
        max_bpc: m.max_bpc,
    }
}

fn generate_request_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(1);
    format!("apply-{}", SEQ.fetch_add(1, Ordering::Relaxed))
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DisplayApplyHandle {
    /// Used to correlate the eventual `displays:apply-result` event.
    pub request_id: String,
    /// Pre-apply state, suitable for `display_revert`.
    pub snapshot: Vec<MonitorConfig>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::displays::types::{
        EnabledKind, Monitor, MonitorMode, Position, Transform, VrrState,
    };

    fn sample_monitor() -> Monitor {
        Monitor {
            connector: "DP-1".into(),
            make: "Dell".into(),
            model: "U2719D".into(),
            serial: String::new(),
            physical_size_mm: (597, 336),
            modes: vec![MonitorMode {
                width: 2560,
                height: 1440,
                refresh_mhz: 60_000,
                preferred: true,
            }],
            current_mode: Some(0),
            preferred_mode: Some(0),
            position: Position { x: 0, y: 0 },
            scale: 1.0,
            transform: Transform::Normal,
            enabled: true,
            mirroring: None,
            vrr: VrrState::Disabled,
            primary: true,
            max_bpc: 0,
        }
    }

    #[test]
    fn monitor_to_config_round_trips_basics() {
        let m = sample_monitor();
        let c = monitor_to_config(&m);
        assert_eq!(c.connector, "DP-1");
        assert_eq!(c.mode_index, Some(0));
        assert_eq!(c.scale, 1.0);
        assert!(matches!(c.enabled, EnabledKind::Active));
        assert!(c.primary);
    }

    #[test]
    fn monitor_to_config_disables_when_not_enabled() {
        let mut m = sample_monitor();
        m.enabled = false;
        let c = monitor_to_config(&m);
        assert!(matches!(c.enabled, EnabledKind::Disabled));
    }

    #[test]
    fn monitor_to_config_preserves_mirror_target() {
        let mut m = sample_monitor();
        m.mirroring = Some("HDMI-1".into());
        let c = monitor_to_config(&m);
        match c.enabled {
            EnabledKind::Mirror(t) => assert_eq!(t, "HDMI-1"),
            other => panic!("expected mirror, got {other:?}"),
        }
    }

    #[test]
    fn request_ids_are_unique() {
        let a = generate_request_id();
        let b = generate_request_id();
        assert_ne!(a, b);
    }
}

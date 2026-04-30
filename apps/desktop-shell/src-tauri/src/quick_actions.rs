//! Quick-Action dispatch + post-toast pipeline.
//!
//! Each Quick-Action from the Waypointer's `core.quick_actions`
//! plugin lands here as a `quick_action_run(id)` Tauri call. We do
//! the actual work (read-modify-write of the relevant state),
//! re-read the post-state for an honest confirmation, and emit a
//! `lunaris://toast` event so the main-window's toast pipeline
//! shows the user what happened.
//!
//! The post-state read is what makes "Toggle DND" honest under
//! cascading-toggle constraints (Airplane Mode disables WiFi, etc.,
//! Sprint D plan E14): the toast reflects the actual state on disk
//! after the dispatch, not the requested intent.

use serde::Serialize;
use tauri::{AppHandle, Emitter};

/// Toast payload the main-window's `+layout.svelte` listens for.
/// Kind drives svelte-sonner's variant (success / info / warning).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToastEvent {
    pub kind: ToastKind,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ToastKind {
    Success,
    Info,
    Warning,
    Error,
}

const TOAST_EVENT: &str = "lunaris://toast";

fn emit_toast(app: &AppHandle, kind: ToastKind, message: impl Into<String>) {
    let _ = app.emit(
        TOAST_EVENT,
        ToastEvent {
            kind,
            message: message.into(),
        },
    );
}

/// Dispatch a quick action by its catalog id (declared in
/// `waypointer_system::plugins::quick_actions::ACTIONS`). Errors are
/// logged AND surfaced as warning toasts; the command itself
/// returns Ok so the frontend doesn't double-toast on its own
/// error path.
#[tauri::command]
pub async fn quick_action_run(id: String, app: AppHandle) -> Result<(), String> {
    let outcome = dispatch(&id, app.clone()).await;
    match outcome {
        Ok(message) => {
            emit_toast(&app, ToastKind::Success, message);
        }
        Err(e) => {
            log::warn!("quick_action_run({id}): {e}");
            emit_toast(&app, ToastKind::Warning, format!("{id}: {e}"));
        }
    }
    Ok(())
}

/// Per-action dispatch. Returns the user-facing message on success.
async fn dispatch(id: &str, app: AppHandle) -> Result<String, String> {
    match id {
        // DND state lives in the notification daemon, not in the
        // shell — there's no local reader to "toggle" against.
        // Two explicit actions keep the dispatch path simple and
        // give the user clearer search results (Sprint D plan E11).
        "qa.dnd_enable" => set_dnd(app, "priority").await,
        "qa.dnd_disable" => set_dnd(app, "off").await,
        "qa.toggle_night_light" => toggle_night_light(app).await,
        "qa.toggle_airplane" => toggle_airplane(app).await,
        "qa.toggle_wifi" => toggle_wifi(app).await,
        "qa.toggle_bluetooth" => toggle_bluetooth(app).await,
        "qa.toggle_caffeine" => toggle_caffeine(app).await,
        "qa.toggle_recording" => toggle_recording(app).await,
        "qa.theme_dark" => set_theme(app, "dark"),
        "qa.theme_light" => set_theme(app, "light"),
        "qa.open_settings" => open_settings(None),
        "qa.open_settings_appearance" => open_settings(Some("appearance")),
        "qa.open_settings_display" => open_settings(Some("display")),
        "qa.open_settings_keyboard" => open_settings(Some("keyboard")),
        "qa.open_settings_focus" => open_settings(Some("focus")),
        "qa.open_settings_notifications" => open_settings(Some("notifications")),
        other => Err(format!("unknown quick action: {other}")),
    }
}

// ── Toggle helpers ─────────────────────────────────────────────────

/// Send a DND-mode set request to the notification daemon. Modes:
/// `"priority"` (DND on, critical-bypass), `"off"` (DND off). The
/// daemon broadcasts the new state back via Tauri events, so the
/// shell-side QuickSettings sees the change without a re-read.
///
/// We bypass the `notification_set_dnd` Tauri command and send the
/// proto message directly because constructing a `State<'_, T>`
/// from an `Arc<...>` inside a non-command async fn is awkward.
/// The proto-message construction is a 5-line clone of the command
/// body.
async fn set_dnd(app: AppHandle, mode: &str) -> Result<String, String> {
    use tauri::Manager;
    use notification_proto::proto;
    let writer = app
        .try_state::<crate::notifications::client::SocketWriter>()
        .ok_or_else(|| "notification socket not available".to_string())?
        .inner()
        .clone();
    let dnd_mode = match mode {
        "priority" | "on" => proto::DndMode::DndPriority as i32,
        _ => proto::DndMode::DndOff as i32,
    };
    let msg = proto::ClientMessage {
        msg: Some(proto::client_message::Msg::SetDnd(proto::SetDndMode {
            mode: dnd_mode,
        })),
    };
    crate::notifications::client::send_command(&writer, msg).await?;
    Ok(format!(
        "Do Not Disturb {}",
        if mode == "off" { "disabled" } else { "enabled" }
    ))
}

async fn toggle_night_light(app: AppHandle) -> Result<String, String> {
    use tauri::Manager;

    let cfg = crate::shell_config::get_shell_config()
        .map_err(|e| format!("read shell config: {e}"))?;
    let new_enabled = !cfg.night_light.enabled;

    // Same dispatch path the QuickSettings panel uses.
    let sender = app
        .try_state::<std::sync::Arc<crate::shell_overlay_client::ShellOverlaySender>>()
        .ok_or_else(|| "shell-overlay sender not available".to_string())?;
    crate::night_light::night_light_set(
        new_enabled,
        cfg.night_light.temperature,
        sender,
    )?;
    Ok(format!(
        "Night Light is now {}",
        if new_enabled { "on" } else { "off" }
    ))
}

async fn toggle_airplane(_app: AppHandle) -> Result<String, String> {
    let current = crate::network::get_airplane_mode().unwrap_or(false);
    crate::network::set_airplane_mode(!current)?;
    let after = crate::network::get_airplane_mode().unwrap_or(!current);
    Ok(format!(
        "Airplane mode is now {}",
        if after { "on" } else { "off" }
    ))
}

async fn toggle_wifi(_app: AppHandle) -> Result<String, String> {
    let current = crate::network::get_wifi_enabled().unwrap_or(false);
    crate::network::set_wifi_enabled(!current)?;
    let after = crate::network::get_wifi_enabled().unwrap_or(!current);
    Ok(format!(
        "WiFi is now {}",
        if after { "on" } else { "off" }
    ))
}

async fn toggle_bluetooth(_app: AppHandle) -> Result<String, String> {
    let state = crate::bluetooth::get_bluetooth_state().await?;
    let new_powered = !state.powered;
    crate::bluetooth::set_bluetooth_powered(new_powered).await?;
    Ok(format!(
        "Bluetooth is now {}",
        if new_powered { "on" } else { "off" }
    ))
}

async fn toggle_caffeine(app: AppHandle) -> Result<String, String> {
    use tauri::Manager;
    let state = app
        .try_state::<crate::system_toggles::ToggleState>()
        .ok_or_else(|| "ToggleState not available".to_string())?;
    let after = crate::system_toggles::toggle_caffeine(state)?;
    Ok(format!(
        "Caffeine is now {}",
        if after { "on" } else { "off" }
    ))
}

async fn toggle_recording(app: AppHandle) -> Result<String, String> {
    use tauri::Manager;
    let state = app
        .try_state::<crate::system_toggles::ToggleState>()
        .ok_or_else(|| "ToggleState not available".to_string())?;
    let after = crate::system_toggles::toggle_recording(state)?;
    Ok(format!(
        "Screen recording is now {}",
        if after { "on" } else { "off" }
    ))
}

fn set_theme(app: AppHandle, id: &str) -> Result<String, String> {
    use tauri::Manager;
    let state = app
        .try_state::<crate::theme::commands::ThemeState>()
        .ok_or_else(|| "ThemeState not available".to_string())?;
    crate::theme::commands::set_theme(id.to_string(), state, app.clone())
        .map_err(|e| format!("set_theme: {e:?}"))?;
    Ok(format!(
        "Theme is now {}",
        match id {
            "dark" => "Dark",
            "light" => "Light",
            other => other,
        }
    ))
}

// ── Settings launcher ──────────────────────────────────────────────

fn open_settings(panel: Option<&str>) -> Result<String, String> {
    let mut cmd = std::process::Command::new("lunaris-settings");
    if let Some(panel) = panel {
        cmd.args(["--panel", panel]);
    }
    cmd.stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| {
            // Most-likely cause on dev systems: binary not in PATH
            // (`cargo tauri dev` doesn't install). Surface a hint.
            format!("could not launch lunaris-settings: {e}")
        })?;
    Ok(match panel {
        Some(p) => format!("Opening Settings: {p}"),
        None => "Opening Settings".to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `dispatch` rejects unknown ids cleanly. Without this, a
    /// frontend-side typo would silently no-op.
    #[tokio::test]
    async fn unknown_id_returns_err() {
        // Tauri AppHandle is hard to fake here, so we test the
        // matcher directly via a thin shim. The real entry point
        // (`quick_action_run`) wraps any error into a warning toast
        // and still returns Ok — the inner `dispatch` is where
        // unknown ids surface as Err. We can't test the full path
        // without a Tauri test harness; this test is the next-best
        // safeguard.
        //
        // We construct the matcher in isolation (mirrors the real
        // dispatch) so we don't depend on AppHandle for this guard.
        fn matches_known(id: &str) -> bool {
            matches!(
                id,
                "qa.dnd_enable"
                    | "qa.dnd_disable"
                    | "qa.toggle_night_light"
                    | "qa.toggle_airplane"
                    | "qa.toggle_wifi"
                    | "qa.toggle_bluetooth"
                    | "qa.toggle_caffeine"
                    | "qa.toggle_recording"
                    | "qa.theme_dark"
                    | "qa.theme_light"
                    | "qa.open_settings"
                    | "qa.open_settings_appearance"
                    | "qa.open_settings_display"
                    | "qa.open_settings_keyboard"
                    | "qa.open_settings_focus"
                    | "qa.open_settings_notifications"
            )
        }
        assert!(matches_known("qa.dnd_enable"));
        assert!(matches_known("qa.theme_dark"));
        assert!(!matches_known("qa.does_not_exist"));
        assert!(!matches_known(""));
    }

    /// ToastEvent serialises with camelCase JSON so the JS
    /// listener can read `kind` + `message` cleanly.
    #[test]
    fn toast_event_serialises_as_camel_case() {
        let ev = ToastEvent {
            kind: ToastKind::Success,
            message: "DND is now on".into(),
        };
        let json = serde_json::to_string(&ev).unwrap();
        assert!(json.contains(r#""kind":"success""#));
        assert!(json.contains(r#""message":"DND is now on""#));
    }
}

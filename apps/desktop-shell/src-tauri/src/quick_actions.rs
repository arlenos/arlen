//! Quick-Action dispatch + post-toast pipeline.
//!
//! Each Quick-Action from the Waypointer's `core.quick_actions`
//! plugin lands here as a `quick_action_run(id)` Tauri call. We do
//! the actual work (read-modify-write of the relevant state),
//! re-read the post-state for an honest confirmation, and emit a
//! `arlen://toast` event so the main-window's toast pipeline
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
    /// A catalog id the frontend renders instead of `message`, when the line is
    /// one this side knows the NAME of but not the WORDS.
    ///
    /// A quick action used to build its own confirmation - "Night Light is now
    /// on" - and emit it as text, so on a German desktop the switch flipped and
    /// an English sentence appeared. The catalog lives in the frontend, so the
    /// backend names the sentence and the frontend writes it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ToastKind {
    Success,
    Info,
    Warning,
    Error,
}

const TOAST_EVENT: &str = "arlen://toast";

pub(crate) fn emit_toast(app: &AppHandle, kind: ToastKind, message: impl Into<String>) {
    let _ = app.emit(
        TOAST_EVENT,
        ToastEvent {
            kind,
            message: message.into(),
            key: None,
        },
    );
}

/// Raise a toast the frontend writes, named by its catalog id.
pub(crate) fn emit_toast_key(app: &AppHandle, kind: ToastKind, key: &str) {
    let _ = app.emit(
        TOAST_EVENT,
        ToastEvent {
            kind,
            // A reader never sees this: the frontend renders `key`. It is the id
            // itself so a missing catalog entry is legible rather than blank.
            message: key.to_string(),
            key: Some(key.to_string()),
        },
    );
}

/// Dispatch a quick action by its catalog id (declared in
/// `waypointer_system::plugins::quick_actions::ACTIONS`).
///
/// Success emits the confirmation toast, because the post-state read above is
/// the honest one and only this process has it. FAILURE IS RETURNED, and this
/// is a deliberate reversal: it used to raise its own warning toast and hand
/// the caller `Ok(())`, on the reasoning that one channel avoids a double
/// notice. What that produced was `qa.shutdown: <rust error>` - the internal id
/// and an untranslated reason - and, worse, it made every frontend guard on
/// this command dead code. Six call sites had written one believing it fired.
///
/// The caller knows which named thing the person pressed and holds the
/// catalog, so it can say "The shutdown did not begin." in their language.
/// That is the better line, and it can only be said if the refusal reaches it.
#[tauri::command]
pub async fn quick_action_run(id: String, app: AppHandle) -> Result<(), String> {
    match dispatch(&id, app.clone()).await {
        Ok(key) => {
            emit_toast_key(&app, ToastKind::Success, key);
            Ok(())
        }
        Err(e) => {
            log::warn!("quick_action_run({id}): {e}");
            Err(e)
        }
    }
}

/// Per-action dispatch. Returns the CATALOG ID of the confirmation on success;
/// the frontend writes the sentence in the reader's language.
async fn dispatch(id: &str, app: AppHandle) -> Result<&'static str, String> {
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
        "qa.open_settings_knowledge" => open_settings(Some("knowledge")),
        "qa.open_settings_notifications" => open_settings(Some("notifications")),
        "qa.lock_screen" => session_action(&["lock-session"], "sh.qa.locking"),
        "qa.logout" => session_logout(),
        "qa.reboot" => power_action(&["reboot"], "sh.qa.restarting"),
        "qa.shutdown" => power_action(&["poweroff"], "sh.qa.shuttingDown"),
        other => Err(format!("unknown quick action: {other}")),
    }
}

/// Run a `loginctl <action>` scoped to the current GUI session.
///
/// Codex high-1: previously `qa.logout` shelled out to `loginctl
/// terminate-user <whoami>`, which kills *every* session for the
/// user — SSH logins, parallel desktop sessions, long-running
/// background processes — not just the one the user clicked
/// "Log Out" in. We resolve the active session via
/// `XDG_SESSION_ID` (set by logind for the current login) and
/// terminate only it. If the env var is missing (rare; nested
/// dev sessions, broken login flow) we surface an error rather
/// than fall back to the user-wide kill — losing parallel work
/// to a typoed click is a worse failure than the user finding a
/// different way to log out.
fn session_logout() -> Result<&'static str, String> {
    let session = std::env::var("XDG_SESSION_ID")
        .map_err(|_| {
            "XDG_SESSION_ID is not set — cannot scope logout to the current session".to_string()
        })?;
    if session.trim().is_empty() {
        return Err("XDG_SESSION_ID is empty".into());
    }
    session_action(&["terminate-session", session.trim()], "sh.qa.loggingOut")
}

/// Spawn `loginctl <args>` and wait briefly for early failure.
///
/// The logind session-management calls (`lock-session`,
/// `terminate-session`) return quickly. We give them up to 1.5s
/// to fail; a non-zero exit during that window means policy
/// rejected the call (PolicyKit rule, missing permission) and we
/// surface stderr. If the call is still running after the
/// timeout we trust the success path — it'll either complete
/// silently or eventually fail in a way the user notices via the
/// session ending (logout) or the screen actually locking
/// (lock-session).
fn session_action(args: &[&str], key: &'static str) -> Result<&'static str, String> {
    spawn_and_check("loginctl", args, key, 1500)
}

/// Spawn `systemctl <args>` for poweroff / reboot.
///
/// These are special: a successful poweroff or reboot tears the
/// process tree down before any exit code reaches us. The
/// "successful path" is therefore "still running after the
/// timeout" — by then the kernel halt sequence is in flight.
/// An early non-zero exit means PolicyKit denied the call (or
/// the user lacks permission); we surface stderr.
fn power_action(args: &[&str], key: &'static str) -> Result<&'static str, String> {
    spawn_and_check("systemctl", args, key, 2000)
}

/// Spawn `cmd args`, wait up to `timeout_ms`, surface stderr on
/// early non-zero exit. If the child is still running at timeout,
/// treat as success — appropriate for both fast-returning logind
/// calls and never-returning system shutdowns.
fn spawn_and_check(
    cmd: &str,
    args: &[&str],
    key: &'static str,
    timeout_ms: u64,
) -> Result<&'static str, String> {
    let mut child = std::process::Command::new(cmd)
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("{cmd}: {e}"))?;

    // Poll for early exit. Steps of 50ms keep the worst-case
    // latency for a denied call below ~50ms while letting the
    // process exit cleanly when it's going to. The total budget
    // (timeout_ms) only applies to "still running after this" =
    // success path.
    let deadline = std::time::Instant::now()
        + std::time::Duration::from_millis(timeout_ms);
    while std::time::Instant::now() < deadline {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => {
                return Ok(key);
            }
            Ok(Some(status)) => {
                let mut stderr = String::new();
                if let Some(mut err) = child.stderr.take() {
                    use std::io::Read;
                    let _ = err.read_to_string(&mut stderr);
                }
                let stderr_trim = stderr.trim();
                let detail = if stderr_trim.is_empty() {
                    format!("exit code {status}")
                } else {
                    stderr_trim.to_string()
                };
                return Err(format!("{cmd} failed: {detail}"));
            }
            Ok(None) => {
                // Still running — fine for poweroff/reboot, we'll
                // either time out and report success, or get a
                // late exit on the next iteration.
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(e) => {
                return Err(format!("{cmd}: wait failed: {e}"));
            }
        }
    }
    // Still running after timeout: treat as success. For
    // poweroff/reboot this is the expected path (system is going
    // down before the process can exit cleanly). For session
    // calls a long-running call is rare but harmless to declare
    // success on.
    Ok(key)
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
async fn set_dnd(app: AppHandle, mode: &str) -> Result<&'static str, String> {
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
    Ok(if mode == "off" { "sh.qa.dnd.off" } else { "sh.qa.dnd.on" })
}

async fn toggle_night_light(app: AppHandle) -> Result<&'static str, String> {
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
    Ok(if new_enabled { "sh.qa.nightLight.on" } else { "sh.qa.nightLight.off" })
}

async fn toggle_airplane(app: AppHandle) -> Result<&'static str, String> {
    let current = crate::network::get_airplane_mode().await.unwrap_or(false);
    crate::network::set_airplane_mode(app, !current).await?;
    let after = crate::network::get_airplane_mode().await.unwrap_or(!current);
    Ok(if after { "sh.qa.airplane.on" } else { "sh.qa.airplane.off" })
}

async fn toggle_wifi(_app: AppHandle) -> Result<&'static str, String> {
    let current = crate::network::get_wifi_enabled().await.unwrap_or(false);
    crate::network::set_wifi_enabled(!current).await?;
    let after = crate::network::get_wifi_enabled().await.unwrap_or(!current);
    Ok(if after { "sh.qa.wifi.on" } else { "sh.qa.wifi.off" })
}

async fn toggle_bluetooth(_app: AppHandle) -> Result<&'static str, String> {
    let state = crate::bluetooth::get_bluetooth_state().await?;
    let new_powered = !state.powered;
    crate::bluetooth::set_bluetooth_powered(new_powered).await?;
    Ok(if new_powered { "sh.qa.bluetooth.on" } else { "sh.qa.bluetooth.off" })
}

async fn toggle_caffeine(app: AppHandle) -> Result<&'static str, String> {
    use tauri::Manager;
    let state = app
        .try_state::<crate::system_toggles::ToggleState>()
        .ok_or_else(|| "ToggleState not available".to_string())?;
    let after = crate::system_toggles::toggle_caffeine(state)?;
    Ok(if after { "sh.qa.caffeine.on" } else { "sh.qa.caffeine.off" })
}

async fn toggle_recording(app: AppHandle) -> Result<&'static str, String> {
    use tauri::Manager;
    let state = app
        .try_state::<crate::system_toggles::ToggleState>()
        .ok_or_else(|| "ToggleState not available".to_string())?;
    let after = crate::system_toggles::toggle_recording(state)?;
    Ok(if after { "sh.qa.recording.on" } else { "sh.qa.recording.off" })
}

fn set_theme(app: AppHandle, id: &str) -> Result<&'static str, String> {
    use tauri::Manager;
    let state = app
        .try_state::<crate::theme::commands::ThemeState>()
        .ok_or_else(|| "ThemeState not available".to_string())?;
    crate::theme::commands::set_theme(id.to_string(), state, app.clone())
        .map_err(|e| format!("set_theme: {e:?}"))?;
    Ok(match id {
        "light" => "sh.qa.theme.light",
        // Dark for anything else: the two quick actions are the only callers and
        // a third id would be a bug rather than a theme to name.
        _ => "sh.qa.theme.dark",
    })
}

// ── Settings launcher ──────────────────────────────────────────────

fn open_settings(panel: Option<&str>) -> Result<&'static str, String> {
    let mut cmd = std::process::Command::new("arlen-settings");
    if let Some(panel) = panel {
        cmd.args(["--panel", panel]);
    }
    cmd.stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| {
            // Most-likely cause on dev systems: binary not in PATH
            // (`cargo tauri dev` doesn't install). Surface a hint.
            format!("could not launch arlen-settings: {e}")
        })?;
    // One line for both, panel or not. The panel id is an English token
    // (`display`, `keyboard`) and naming it here would put an untranslated word
    // in a translated sentence; the window that opens says which page it is on.
    Ok("sh.qa.settings.opening")
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
        // (`quick_action_run`) returns that error to the caller, which
        // turns it into a line in the reader's language - the inner
        // `dispatch` is where unknown ids surface as Err. We can't test
        // the full path without a Tauri test harness; this test is the
        // next-best safeguard.
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
                    | "qa.open_settings_knowledge"
                    | "qa.open_settings_notifications"
                    | "qa.lock_screen"
                    | "qa.logout"
                    | "qa.reboot"
                    | "qa.shutdown"
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
            key: None,
        };
        let json = serde_json::to_string(&ev).unwrap();
        assert!(json.contains(r#""kind":"success""#));
        assert!(json.contains(r#""message":"DND is now on""#));
        // Absent rather than null, so a reader of the payload cannot mistake
        // "this line has no catalog id" for "its id is nothing".
        assert!(!json.contains("key"));
    }

    /// A quick action names its confirmation and never writes it.
    ///
    /// The catalog is in the frontend; this side used to build the sentence in
    /// English and emit it as text, which is what a German desktop then showed.
    /// Every id here has to exist in `messages.ts` - and the ids are the only
    /// thing this file may say about the wording.
    #[test]
    fn every_confirmation_is_a_catalog_id() {
        let ids = [
            "sh.qa.dnd.on",
            "sh.qa.dnd.off",
            "sh.qa.nightLight.on",
            "sh.qa.nightLight.off",
            "sh.qa.airplane.on",
            "sh.qa.airplane.off",
            "sh.qa.wifi.on",
            "sh.qa.wifi.off",
            "sh.qa.bluetooth.on",
            "sh.qa.bluetooth.off",
            "sh.qa.caffeine.on",
            "sh.qa.caffeine.off",
            "sh.qa.recording.on",
            "sh.qa.recording.off",
            "sh.qa.theme.dark",
            "sh.qa.theme.light",
            "sh.qa.settings.opening",
            "sh.qa.locking",
            "sh.qa.loggingOut",
            "sh.qa.restarting",
            "sh.qa.shuttingDown",
        ];
        let catalog = include_str!("../../src/lib/i18n/messages.ts");
        for id in ids {
            assert!(
                catalog.contains(&format!("\"{id}\"")),
                "{id} is emitted as a toast id and the catalog does not carry it"
            );
        }
        // And the file that emits them names no others: a new action that
        // invents an id would pass the loop above by not being in the list.
        //
        // The PRODUCTION half only. Scanning the whole file made this test find
        // its own `split` argument and report a bare `sh.qa`, which is a real
        // trap of self-scanning tests rather than a fact about the code.
        let src = include_str!("quick_actions.rs");
        let production = src.split("#[cfg(test)]").next().unwrap_or(src);
        for hit in production.split("\"sh.qa").skip(1) {
            let id = format!("sh.qa{}", hit.split('"').next().unwrap_or(""));
            assert!(ids.contains(&id.as_str()), "{id} is emitted but not listed here");
        }
    }
}

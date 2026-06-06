//! AI authorization prompt bridge.
//!
//! When the AI daemon needs the user to authorize an MCP action
//! scope it emits an `AuthorizationPrompt` signal on
//! `org.lunaris.AI1`. This module listens for that signal, relays it
//! to the shell UI as a Tauri event, and relays the user's decision
//! back to the daemon through the `respond_authorization` method.
//!
//! The shell holds no authorization state of its own: the daemon
//! owns the pending prompts and the grants. This module is a thin
//! signal-to-UI and UI-to-method relay.

use futures_util::StreamExt;
use serde::Serialize;
use tauri::{AppHandle, Emitter};
use zbus::Connection;

/// AI daemon bus name.
const AI_BUS_NAME: &str = "org.lunaris.AI1";
/// AI daemon object path.
const AI_OBJECT_PATH: &str = "/org/lunaris/AI1";

/// Payload sent to the frontend when the daemon asks for an
/// authorization decision.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AuthorizationPromptDto {
    /// Opaque prompt identifier; echoed back in the decision.
    prompt_id: String,
    /// Scope label, shown to the user in plain language.
    scope: String,
}

/// Spawn the `AuthorizationPrompt` signal listener. Survives a
/// missing daemon: if the bus or the daemon is unavailable the
/// listener simply logs and exits, and the shell still runs.
pub fn spawn(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        if let Err(err) = run(app).await {
            log::warn!("[ai-authz] prompt listener stopped: {err}");
        }
    });
}

async fn run(app: AppHandle) -> zbus::Result<()> {
    let connection = Connection::session().await?;
    let proxy = zbus::Proxy::new(
        &connection,
        AI_BUS_NAME,
        AI_OBJECT_PATH,
        AI_BUS_NAME,
    )
    .await?;

    let mut signals = proxy.receive_signal("AuthorizationPrompt").await?;
    log::info!("[ai-authz] listening for authorization prompts");
    while let Some(message) = signals.next().await {
        match message.body().deserialize::<(String, String)>() {
            Ok((prompt_id, scope)) => {
                let _ = app.emit(
                    "ai://authorization-prompt",
                    AuthorizationPromptDto { prompt_id, scope },
                );
            }
            Err(err) => {
                log::warn!("[ai-authz] malformed AuthorizationPrompt signal: {err}");
            }
        }
    }
    Ok(())
}

/// Relay the user's authorization decision back to the AI daemon.
/// Returns whether the daemon still had a matching pending prompt.
#[tauri::command]
pub async fn ai_respond_authorization(
    prompt_id: String,
    granted: bool,
) -> Result<bool, String> {
    let connection = Connection::session()
        .await
        .map_err(|e| format!("session bus: {e}"))?;
    let proxy = zbus::Proxy::new(
        &connection,
        AI_BUS_NAME,
        AI_OBJECT_PATH,
        AI_BUS_NAME,
    )
    .await
    .map_err(|e| format!("ai daemon proxy: {e}"))?;
    proxy
        .call("respond_authorization", &(prompt_id.as_str(), granted))
        .await
        .map_err(|e| format!("respond_authorization: {e}"))
}

//! AI layer status command (Phase 9-α S7).
//!
//! The AI daemon and proxy are D-Bus services, not socket daemons,
//! so liveness is probed by asking the session bus whether their
//! well-known names currently have an owner. This is the D-Bus
//! analogue of the socket-existence checks the About / Knowledge
//! pages use.
//!
//! The `enabled` / `provider` settings are not read here: the AI
//! page already gets those through the generic `ai.toml` config
//! store. This command answers only "is the daemon process alive".

use std::time::Duration;

use serde::Serialize;

/// AI daemon name on the session bus.
const AI_DAEMON_NAME: &str = "org.arlen.AI1";
/// AI daemon object path.
const AI_OBJECT_PATH: &str = "/org/arlen/AI1";
/// AI proxy name on the session bus.
const AI_PROXY_NAME: &str = "org.arlen.AIProxy1";
/// Upper bound on the explanation call; the daemon reads the graph and calls
/// the provider, so allow a generous window but never hang the page.
const EXPLAIN_TIMEOUT: Duration = Duration::from_secs(90);

/// Liveness of the AI layer's two daemons.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiStatus {
    /// `org.arlen.AI1` has an owner on the session bus.
    pub daemon_running: bool,
    /// `org.arlen.AIProxy1` has an owner on the session bus.
    pub proxy_running: bool,
}

/// Probe whether the AI daemon and proxy are running.
#[tauri::command]
pub async fn ai_status() -> Result<AiStatus, String> {
    let connection = match zbus::Connection::session().await {
        Ok(c) => c,
        Err(e) => {
            // No session bus at all — report both as down rather
            // than failing the command, so the page still renders.
            log::warn!("[ai] session bus unavailable: {e}");
            return Ok(AiStatus {
                daemon_running: false,
                proxy_running: false,
            });
        }
    };
    let dbus = zbus::fdo::DBusProxy::new(&connection)
        .await
        .map_err(|e| format!("DBusProxy: {e}"))?;

    Ok(AiStatus {
        daemon_running: name_has_owner(&dbus, AI_DAEMON_NAME).await,
        proxy_running: name_has_owner(&dbus, AI_PROXY_NAME).await,
    })
}

/// Ask the AI daemon for a plain-language summary of what the computer is
/// doing right now (Foundation §5.8 System Explanation Mode). A single bounded
/// D-Bus call to `org.arlen.AI1`; errors (daemon down, disabled, insufficient
/// scope, timeout) come back as a readable string the page shows.
#[tauri::command]
pub async fn ai_explain() -> Result<String, String> {
    let connection = zbus::Connection::session()
        .await
        .map_err(|e| format!("session bus: {e}"))?;
    let proxy = zbus::Proxy::new(&connection, AI_DAEMON_NAME, AI_OBJECT_PATH, AI_DAEMON_NAME)
        .await
        .map_err(|e| format!("ai daemon unavailable: {e}"))?;
    match tokio::time::timeout(
        EXPLAIN_TIMEOUT,
        proxy.call::<_, _, String>("explain_system", &()),
    )
    .await
    {
        Ok(Ok(summary)) => Ok(summary),
        Ok(Err(zbus::Error::MethodError(_, detail, _))) => {
            Err(detail.unwrap_or_else(|| "explanation failed".to_string()))
        }
        Ok(Err(e)) => Err(format!("explanation failed: {e}")),
        Err(_) => Err("the explanation timed out".to_string()),
    }
}

/// Call a String-returning member on the AI daemon, returning `fallback` on any
/// connection or call failure (the manager reads are advisory - a down daemon
/// shows an empty surface rather than erroring the page).
async fn ai_call_string(member: &str, fallback: &str) -> String {
    let Ok(connection) = zbus::Connection::session().await else {
        return fallback.to_string();
    };
    let Ok(proxy) =
        zbus::Proxy::new(&connection, AI_DAEMON_NAME, AI_OBJECT_PATH, AI_DAEMON_NAME).await
    else {
        return fallback.to_string();
    };
    proxy
        .call::<_, _, String>(member, &())
        .await
        .unwrap_or_else(|_| fallback.to_string())
}

/// The catalogued providers for the Settings AI-providers manager
/// (`ai_providers_list`): a JSON array of `{ id, name, kind, enabled,
/// configured, status }`. Empty array if the daemon is unreachable.
#[tauri::command]
pub async fn ai_providers_list() -> String {
    ai_call_string("ai_providers_list", "[]").await
}

/// The configured default provider/model + ranked fallback (`ai_defaults_get`),
/// as `{ provider, model, ranking }`, for the manager's Default-Models page.
/// Empty object if the daemon is unreachable.
#[tauri::command]
pub async fn ai_defaults_get() -> String {
    ai_call_string("ai_defaults_get", "{}").await
}

/// Enable or disable a catalogued provider (`ai_provider_set_enabled`). Returns
/// the daemon's `ok` / `error: ...` status; a transport failure maps to an
/// `error:` string so the manager surfaces it.
#[tauri::command]
pub async fn ai_provider_set_enabled(id: String, enabled: bool) -> String {
    let Ok(connection) = zbus::Connection::session().await else {
        return "error: session bus unavailable".to_string();
    };
    let Ok(proxy) =
        zbus::Proxy::new(&connection, AI_DAEMON_NAME, AI_OBJECT_PATH, AI_DAEMON_NAME).await
    else {
        return "error: AI daemon unavailable".to_string();
    };
    proxy
        .call::<_, _, String>("ai_provider_set_enabled", &(id.as_str(), enabled))
        .await
        .unwrap_or_else(|e| format!("error: {e}"))
}

/// Test a catalogued provider's connectivity (`ai_provider_test`). Returns the
/// daemon's verdict JSON `{ ok, httpStatus?, network? }`; the daemon GETs the
/// provider's catalogued model-list endpoint through the proxy (no caller URL,
/// so no egress-consent step). A transport failure maps to a `network` verdict
/// so the manager always gets the uniform shape.
#[tauri::command]
pub async fn ai_provider_test(id: String) -> String {
    let network = |reason: &str| format!(r#"{{"ok":false,"network":"{reason}"}}"#);
    let Ok(connection) = zbus::Connection::session().await else {
        return network("session bus unavailable");
    };
    let Ok(proxy) =
        zbus::Proxy::new(&connection, AI_DAEMON_NAME, AI_OBJECT_PATH, AI_DAEMON_NAME).await
    else {
        return network("AI daemon unavailable");
    };
    proxy
        .call::<_, _, String>("ai_provider_test", &(id.as_str(),))
        .await
        .unwrap_or_else(|_| network("test failed"))
}

async fn name_has_owner(dbus: &zbus::fdo::DBusProxy<'_>, name: &str) -> bool {
    let Ok(bus_name) = zbus::names::BusName::try_from(name) else {
        return false;
    };
    dbus.name_has_owner(bus_name).await.unwrap_or(false)
}

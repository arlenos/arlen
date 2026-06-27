//! Harness Tauri bridges for the AI provider/model picker, the cost feed, and
//! the autonomy-dial state (arlen-ui flagged these as the missing coder-lane
//! `#[tauri::command]` wrappers: the daemon D-Bus members exist on
//! `org.arlen.AI1` / `org.arlen.AIAgent1`, but the frontend `invoke` had nothing
//! to call). Each is a thin wrapper: open the session bus, call the member,
//! return its JSON string. Reads are advisory - an unreachable daemon yields a
//! fail-safe empty value rather than erroring the UI; the one mutating call
//! (`ai_set_active`) surfaces a real failure so the picker can report it.

use zbus::{Connection, Proxy};

/// The AI daemon: provider/model picker + cost.
const AI_BUS: &str = "org.arlen.AI1";
const AI_PATH: &str = "/org/arlen/AI1";
/// The AI agent: the autonomy-dial state.
const AGENT_BUS: &str = "org.arlen.AIAgent1";
const AGENT_PATH: &str = "/org/arlen/AIAgent1";

/// Call a String-returning member on `(bus, path, bus)`, returning `fallback`
/// on any connection or call failure (the read commands are advisory).
async fn call_string(bus: &str, path: &str, member: &str, fallback: &str) -> String {
    let Ok(connection) = Connection::session().await else {
        return fallback.to_string();
    };
    let Ok(proxy) = Proxy::new(&connection, bus, path, bus).await else {
        return fallback.to_string();
    };
    proxy
        .call(member, &())
        .await
        .unwrap_or_else(|_| fallback.to_string())
}

/// The model catalog for the in-chat picker (`ai_models_list`): a JSON array of
/// `{ provider, model, contextWindow, kind, available }`. Empty array if the
/// daemon is unreachable.
#[tauri::command]
pub async fn ai_models_list() -> String {
    call_string(AI_BUS, AI_PATH, "ai_models_list", "[]").await
}

/// The current live selection (`ai_active`): `{ provider, model }`. Empty object
/// if the daemon is unreachable.
#[tauri::command]
pub async fn ai_active() -> String {
    call_string(AI_BUS, AI_PATH, "ai_active", "{}").await
}

/// Cumulative token usage (`ai_usage`): `{ inputTokens, outputTokens,
/// totalTokens }` for the transparency-drawer Cost feed. Zeros if unreachable.
#[tauri::command]
pub async fn ai_usage() -> String {
    call_string(
        AI_BUS,
        AI_PATH,
        "ai_usage",
        r#"{"inputTokens":0,"outputTokens":0,"totalTokens":0}"#,
    )
    .await
}

/// The catalogued providers for the manager surface (`ai_providers_list`): a JSON
/// array of `{ id, name, kind, enabled, configured, status }`. Empty array if
/// unreachable.
#[tauri::command]
pub async fn ai_providers_list() -> String {
    call_string(AI_BUS, AI_PATH, "ai_providers_list", "[]").await
}

/// The autonomy-dial state (`action_state` on the agent): `{ action_mode,
/// autonomous_apps, executor_live }`. The safe inert shape if the agent is
/// unreachable (suggest / none / off).
#[tauri::command]
pub async fn action_state() -> String {
    call_string(
        AGENT_BUS,
        AGENT_PATH,
        "action_state",
        r#"{"action_mode":"suggest","autonomous_apps":[],"executor_live":false}"#,
    )
    .await
}

/// Live-swap the active provider+model (`ai_set_active`). Returns the new
/// `{ provider, model }` on success, or `Err(message)` on a refused swap
/// (unknown/unallowlisted provider, proxy unreachable) so the picker can report
/// it rather than silently keep the old selection.
#[tauri::command]
pub async fn ai_set_active(provider: String, model: String) -> Result<String, String> {
    let connection = Connection::session()
        .await
        .map_err(|e| format!("session bus unavailable: {e}"))?;
    let proxy = Proxy::new(&connection, AI_BUS, AI_PATH, AI_BUS)
        .await
        .map_err(|e| format!("AI daemon unavailable: {e}"))?;
    proxy
        .call("ai_set_active", &(provider.as_str(), model.as_str()))
        .await
        .map_err(|e| e.to_string())
}

/// Enable or disable a catalogued provider (`ai_provider_set_enabled`). Returns
/// the daemon's `ok` / `error: ...` status string; a transport failure maps to
/// an `error:` string so the manager surfaces it.
#[tauri::command]
pub async fn ai_provider_set_enabled(id: String, enabled: bool) -> String {
    let Ok(connection) = Connection::session().await else {
        return "error: session bus unavailable".to_string();
    };
    let Ok(proxy) = Proxy::new(&connection, AI_BUS, AI_PATH, AI_BUS).await else {
        return "error: AI daemon unavailable".to_string();
    };
    proxy
        .call("ai_provider_set_enabled", &(id.as_str(), enabled))
        .await
        .unwrap_or_else(|e| format!("error: {e}"))
}

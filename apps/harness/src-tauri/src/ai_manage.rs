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

/// The configured default provider/model for the manager's Default-Models page
/// (`ai_defaults_get`): `{ provider, model, ranking }`. Empty object if the
/// daemon is unreachable.
#[tauri::command]
pub async fn ai_defaults_get() -> String {
    call_string(AI_BUS, AI_PATH, "ai_defaults_get", "{}").await
}

/// The agent's pending gate proposals (`pending_proposals`): a JSON array the
/// harness renders as inline gate cards (each `{ id, summary, reason, effects }`),
/// oldest first. Empty array if the agent is unreachable or nothing is pending.
#[tauri::command]
pub async fn pending_proposals() -> String {
    call_string(AGENT_BUS, AGENT_PATH, "pending_proposals", "[]").await
}

/// The agent's recently-completed (silent-done) actions (`completed_actions`): a
/// JSON array the harness renders as quiet done-lines each with an `[Undo]`,
/// oldest first. Each entry carries the correlation id the `compensate` undo
/// keys off. Empty array if unreachable or nothing has executed.
#[tauri::command]
pub async fn completed_actions() -> String {
    call_string(AGENT_BUS, AGENT_PATH, "completed_actions", "[]").await
}

/// Dismiss a pending gate proposal (`deny`): the user declined the confirmation.
/// Returns the agent's `denied` / `no-such-proposal` / `error: ...` status; a
/// transport failure maps to an `error:` string so the gate card surfaces it.
/// Deny is purely local and safe in any mode (it forgoes an action), so it is
/// always available.
#[tauri::command]
pub async fn deny(id: u64) -> String {
    let Ok(connection) = Connection::session().await else {
        return "error: session bus unavailable".to_string();
    };
    let Ok(proxy) = Proxy::new(&connection, AGENT_BUS, AGENT_PATH, AGENT_BUS).await else {
        return "error: AI agent unavailable".to_string();
    };
    proxy
        .call("deny", &(id,))
        .await
        .unwrap_or_else(|e| format!("error: {e}"))
}

/// The agent's working-set shape (`working_set` on the agent): the shape-only
/// introspection of what the agent currently has in scope (AIT-R1), for the
/// transparency drawer's working-set section. Identity/shape only, never user
/// data. Empty object if the agent is unreachable.
#[tauri::command]
pub async fn ai_working_set() -> String {
    call_string(AGENT_BUS, AGENT_PATH, "working_set", "{}").await
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

/// Set the baseline autonomy mode (`ai_set_action_mode` on the agent): `"suggest"`
/// or `"supervised"`, live with no restart. Returns the agent's `ok` / `error: ...`
/// status; a transport failure maps to an `error:` string so the dial surfaces it.
/// `executor_live` stays the orthogonal Tim-gated master (the dial shows the inert
/// state while it is off).
#[tauri::command]
pub async fn ai_set_action_mode(mode: String) -> String {
    let Ok(connection) = Connection::session().await else {
        return "error: session bus unavailable".to_string();
    };
    let Ok(proxy) = Proxy::new(&connection, AGENT_BUS, AGENT_PATH, AGENT_BUS).await else {
        return "error: AI agent unavailable".to_string();
    };
    proxy
        .call("ai_set_action_mode", &(mode.as_str(),))
        .await
        .unwrap_or_else(|e| format!("error: {e}"))
}

/// Grant or revoke an app's autonomy (`ai_set_autonomous_app` on the agent):
/// add/remove `app_id` from `[ai] autonomous_apps`, the dial's per-app "More"
/// control, live with no restart. Returns the agent's `ok` / `error: ...` status;
/// a transport failure maps to an `error:` string so the dial surfaces it.
#[tauri::command]
pub async fn ai_set_autonomous_app(app_id: String, enabled: bool) -> String {
    let Ok(connection) = Connection::session().await else {
        return "error: session bus unavailable".to_string();
    };
    let Ok(proxy) = Proxy::new(&connection, AGENT_BUS, AGENT_PATH, AGENT_BUS).await else {
        return "error: AI agent unavailable".to_string();
    };
    proxy
        .call("ai_set_autonomous_app", &(app_id.as_str(), enabled))
        .await
        .unwrap_or_else(|e| format!("error: {e}"))
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

/// Test a catalogued provider's connectivity (`ai_provider_test`). Returns the
/// daemon's verdict JSON `{ ok, httpStatus?, network? }`; the daemon GETs the
/// provider's catalogued model-list endpoint through the proxy (no caller URL).
/// A transport failure maps to a `network` verdict so the manager gets the
/// uniform shape rather than an error.
#[tauri::command]
pub async fn ai_provider_test(id: String) -> String {
    let network = |reason: &str| format!(r#"{{"ok":false,"network":"{reason}"}}"#);
    let Ok(connection) = Connection::session().await else {
        return network("session bus unavailable");
    };
    let Ok(proxy) = Proxy::new(&connection, AI_BUS, AI_PATH, AI_BUS).await else {
        return network("AI daemon unavailable");
    };
    proxy
        .call("ai_provider_test", &(id.as_str(),))
        .await
        .unwrap_or_else(|_| network("test failed"))
}

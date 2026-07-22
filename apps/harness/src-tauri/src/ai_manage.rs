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
/// The egress proxy: the measured token usage lives here (the ledger that meters
/// every forward), exposed un-gated as read-only display data for exactly this
/// transparency feed.
const PROXY_BUS: &str = "org.arlen.AIProxy1";
const PROXY_PATH: &str = "/org/arlen/AIProxy1";

/// Call a String-returning member on `(bus, path, bus)`, returning `fallback`
/// on any connection or call failure (the read commands are advisory).
/// Like [`call_string`] but SIGNALS failure instead of substituting a value.
///
/// Needed wherever an empty result would be read as a fact about the system
/// rather than as "could not read" - a grant list being the clear case, since an
/// empty one states "nothing has access".
async fn try_call_string(bus: &str, path: &str, member: &str) -> Option<String> {
    let connection = Connection::session().await.ok()?;
    let proxy = Proxy::new(&connection, bus, path, bus).await.ok()?;
    proxy.call(member, &()).await.ok()
}

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

/// Sum ai-proxy's per-provider usage report into the Cost feed's shape.
///
/// The proxy meters every forward per provider over the current window; the Cost
/// section wants one cumulative `{ inputTokens, outputTokens, totalTokens }`, so
/// this folds the providers together. `None` on any parse failure, which the
/// caller maps to `null` - the honest "not measured", never a fabricated zero.
/// Genuinely-zero usage (the ledger is reachable but nothing has been spent) is a
/// real measured value and returns `{...: 0}`, distinct from `None`.
fn sum_usage(report_json: &str) -> Option<String> {
    let report: serde_json::Value = serde_json::from_str(report_json).ok()?;
    let providers = report.get("providers")?.as_array()?;
    let (mut input, mut output, mut total) = (0u64, 0u64, 0u64);
    for p in providers {
        let usage = p.get("usage")?;
        input += usage.get("promptTokens").and_then(|v| v.as_u64()).unwrap_or(0);
        output += usage.get("completionTokens").and_then(|v| v.as_u64()).unwrap_or(0);
        total += usage.get("totalTokens").and_then(|v| v.as_u64()).unwrap_or(0);
    }
    Some(
        serde_json::json!({
            "inputTokens": input,
            "outputTokens": output,
            "totalTokens": total,
        })
        .to_string(),
    )
}

/// Cumulative token usage (`ai_usage`): `{ inputTokens, outputTokens,
/// totalTokens }` for the transparency-drawer Cost feed.
///
/// Reads the MEASURED usage from ai-proxy's `list_provider_usage` (the ledger
/// that meters every forward) and folds the providers into one total. Nothing
/// serves this on `org.arlen.AI1`, which is why the feed read "not measured"
/// despite the data existing one hop away on the proxy.
///
/// Unreachable or unparseable yields `null`, NOT zeros. This is the transparency
/// surface, so "0 tokens used so far" must mean measured-and-zero; reporting zeros
/// for an unreadable proxy states as fact that the assistant cost nothing. The
/// drawer already renders a "not measured" tag for a null usage - fabricating
/// zeros here is what made that branch unreachable.
#[tauri::command]
pub async fn ai_usage() -> String {
    match try_call_string(PROXY_BUS, PROXY_PATH, "list_provider_usage").await {
        Some(report) => sum_usage(&report).unwrap_or_else(|| "null".to_string()),
        None => "null".to_string(),
    }
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

/// Approve a pending gate-card proposal (`approve`): the user confirmed the
/// action, so the agent performs it. The agent re-runs the full trusted proof
/// against the current graph and audits fail-closed before the write, so the
/// approve authorises the act but never bypasses revalidation. Returns the
/// agent's status (`executed` / `nothing-to-execute` / `not-enabled` in suggest
/// mode / `no-such-proposal` / `error: ...`); a transport failure maps to an
/// `error:` string the gate card surfaces.
#[tauri::command]
pub async fn approve(id: u64) -> String {
    let Ok(connection) = Connection::session().await else {
        return "error: session bus unavailable".to_string();
    };
    let Ok(proxy) = Proxy::new(&connection, AGENT_BUS, AGENT_PATH, AGENT_BUS).await else {
        return "error: AI agent unavailable".to_string();
    };
    proxy
        .call("approve", &(id,))
        .await
        .unwrap_or_else(|e| format!("error: {e}"))
}

/// Undo a completed action (`compensate`): the user pressed `[Undo]` on a
/// silent-done line, keyed by the action's correlation id (the `id` on a
/// `completed_actions` entry). The agent retracts the write, re-running the
/// audit fail-closed first. Returns the agent's status (`retracted` /
/// `nothing-to-undo` / `no-such-receipt` / `not-enabled` / `error: ...`); a
/// transport failure maps to an `error:` string. Only functions when the
/// executor is live; in suggest mode nothing was written, so the agent answers
/// `not-enabled`.
#[tauri::command]
pub async fn undo_action(id: String) -> String {
    let Ok(connection) = Connection::session().await else {
        return "error: session bus unavailable".to_string();
    };
    let Ok(proxy) = Proxy::new(&connection, AGENT_BUS, AGENT_PATH, AGENT_BUS).await else {
        return "error: AI agent unavailable".to_string();
    };
    proxy
        .call("compensate", &(id,))
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

/// The AI's capability grants for the transparency drawer's Grants feed
/// (`access_grants` on both AI principals): the Living Capability Graph
/// projection of what the assistant (`org.arlen.AI1`) and the background agent
/// (`org.arlen.AIAgent1`) are each allowed to read. Each daemon reports its OWN
/// grants - the knowledge daemon's `access_grants` op is caller-scoped, so the
/// principal is correct by construction - and this merges the two into the one
/// AI-scoped array `readGrants()` renders, each labelled by its `app_id`. A
/// daemon that is unreachable or holds no grant contributes nothing, so a
/// partial view is honest rather than an error. Returns a JSON array (the
/// frontend invokes it as `GrantView[]`); empty when neither principal answers.
#[tauri::command]
pub async fn ai_access_grants() -> serde_json::Value {
    // Null - NOT an empty array - if EITHER principal cannot be read. The reader
    // (`transparency.ts::readGrants`) documents that a failed read must render
    // honestly and "never as 'no access'", but an `[]` fallback per principal
    // defeated that: an unreachable daemon contributed nothing and the merged
    // result came back as a successful empty list, i.e. "nothing has access".
    // A PARTIAL merge is wrong for the same reason - it under-reports reach while
    // looking complete - so any failure yields unknown rather than a short list.
    let mut grants: Vec<serde_json::Value> = Vec::new();
    for (bus, path) in [(AGENT_BUS, AGENT_PATH), (AI_BUS, AI_PATH)] {
        let Some(json) = try_call_string(bus, path, "access_grants").await else {
            return serde_json::Value::Null;
        };
        let Ok(serde_json::Value::Array(items)) = serde_json::from_str::<serde_json::Value>(&json)
        else {
            return serde_json::Value::Null;
        };
        grants.extend(items);
    }
    serde_json::Value::Array(grants)
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

/// Open the Settings app to the AI panel (the transparency off-switch's "manage
/// AI in Settings" link). Launches `arlen-settings --panel ai`, the deep-link
/// Settings parses at startup to land on its AI page. Errors if the binary can
/// not be spawned (not installed / not on PATH).
#[tauri::command]
pub fn open_ai_settings() -> Result<(), String> {
    std::process::Command::new("arlen-settings")
        .args(["--panel", "ai"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("launch settings: {e}"))
}

#[cfg(test)]
mod tests {
    use super::sum_usage;

    #[test]
    fn folds_providers_into_one_total() {
        // The proxy meters per provider over a window; the Cost feed wants one
        // cumulative figure, so two providers sum.
        let report = r#"{
            "windowResetsInSecs": 3600,
            "providers": [
                {"id":"ollama","usage":{"promptTokens":100,"completionTokens":40,"totalTokens":140,"requests":3},"cap":null},
                {"id":"openai","usage":{"promptTokens":10,"completionTokens":5,"totalTokens":15,"requests":1},"cap":1000}
            ]
        }"#;
        let out: serde_json::Value = serde_json::from_str(&sum_usage(report).unwrap()).unwrap();
        assert_eq!(out["inputTokens"], 110);
        assert_eq!(out["outputTokens"], 45);
        assert_eq!(out["totalTokens"], 155);
    }

    #[test]
    fn measured_zero_is_a_real_value_not_none() {
        // A reachable ledger with no spend is measured-and-zero - distinct from an
        // unreadable proxy (None -> the caller's null -> "not measured"). Reporting
        // this as null would hide that the assistant genuinely cost nothing yet.
        let report = r#"{"windowResetsInSecs":3600,"providers":[]}"#;
        let out: serde_json::Value = serde_json::from_str(&sum_usage(report).unwrap()).unwrap();
        assert_eq!(out["totalTokens"], 0);
        assert_eq!(out["inputTokens"], 0);
    }

    #[test]
    fn malformed_report_is_none_so_the_feed_says_not_measured() {
        // The honesty rule: an unparseable report must not become a fabricated
        // zero. None flows to the caller's null, which the drawer renders as "not
        // measured" rather than "0 tokens used so far".
        assert!(sum_usage("not json").is_none());
        assert!(sum_usage("{}").is_none()); // no providers array
        assert!(sum_usage(r#"{"providers":"x"}"#).is_none());
    }
}

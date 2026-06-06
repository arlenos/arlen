//! Conversation client for the AI daemon (`org.lunaris.AI1`).
//!
//! A2 conversation MVP (ai-app.md §2.1): submit a query and poll it to
//! completion, returning the assistant's answer. The daemon's query
//! path is poll-based — `query` returns a `(query_id, retrieval_token)`
//! handle, and `take_result` is polled until a terminal status — and it
//! authorises result retrieval by the caller's D-Bus connection, so the
//! submit and every poll run on **one** connection held for the call.
//!
//! Token-by-token streaming (the `QueryProgress` signal) is a later
//! step; this MVP returns the full answer when it is ready and surfaces
//! a "thinking" state in the UI meanwhile. Nothing is faked: a missing
//! or disabled daemon surfaces as an error the conversation renders.

use std::time::Duration;

use serde::Serialize;
use zbus::{Connection, Proxy};

/// AI daemon bus name, object path, interface.
const AI_BUS_NAME: &str = "org.lunaris.AI1";
const AI_OBJECT_PATH: &str = "/org/lunaris/AI1";

/// How long to poll a single query before giving up and cancelling it.
const QUERY_TIMEOUT: Duration = Duration::from_secs(90);
/// Delay between `take_result` polls.
const POLL_INTERVAL: Duration = Duration::from_millis(250);

/// The outcome of a conversation turn, returned to the frontend.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryReply {
    /// The assistant's answer text.
    pub answer: String,
}

/// Submit `prompt` to the AI daemon and poll until it completes,
/// returning the answer. Errors (daemon down, disabled, no graph
/// access, query failure, timeout) come back as a readable string the
/// UI shows in the conversation.
#[tauri::command]
pub async fn ai_query(prompt: String) -> Result<QueryReply, String> {
    // One connection for the whole turn: the daemon authorises
    // `take_result` against the submitting connection's unique name, so
    // a fresh connection per poll would be rejected.
    let connection = Connection::session()
        .await
        .map_err(|e| format!("session bus: {e}"))?;
    let proxy = Proxy::new(&connection, AI_BUS_NAME, AI_OBJECT_PATH, AI_BUS_NAME)
        .await
        .map_err(|e| format!("ai daemon unavailable: {e}"))?;

    // Submit. `context_hints` is unused by the daemon today (empty).
    let handle_json: String = proxy
        .call("query", &(prompt.as_str(), ""))
        .await
        .map_err(map_call_error)?;
    let handle: serde_json::Value = serde_json::from_str(&handle_json)
        .map_err(|e| format!("malformed query handle: {e}"))?;
    let query_id = handle
        .get("query_id")
        .and_then(|v| v.as_str())
        .ok_or("query handle missing query_id")?
        .to_string();
    let token = handle
        .get("retrieval_token")
        .and_then(|v| v.as_str())
        .ok_or("query handle missing retrieval_token")?
        .to_string();

    let deadline = tokio::time::Instant::now() + QUERY_TIMEOUT;
    loop {
        if tokio::time::Instant::now() >= deadline {
            // Best-effort cancel so the daemon does not keep working on
            // an answer no one will read.
            let _: Result<bool, _> = proxy.call("cancel", &(query_id.as_str(), token.as_str())).await;
            return Err("the assistant took too long to respond".to_string());
        }

        let outcome_json: String = proxy
            .call("take_result", &(query_id.as_str(), token.as_str()))
            .await
            .map_err(map_call_error)?;
        let outcome: serde_json::Value = serde_json::from_str(&outcome_json)
            .map_err(|e| format!("malformed result envelope: {e}"))?;

        match outcome.get("status").and_then(|v| v.as_str()) {
            Some("completed") => {
                let answer = outcome
                    .get("result")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                return Ok(QueryReply { answer });
            }
            Some("failed") => {
                let reason = outcome
                    .get("reason")
                    .and_then(|v| v.as_str())
                    .unwrap_or("the query failed");
                return Err(reason.to_string());
            }
            Some("cancelled") => return Err("the query was cancelled".to_string()),
            Some("drained") => return Err("the result was already consumed".to_string()),
            // pending / in-progress / unknown: keep polling.
            _ => tokio::time::sleep(POLL_INTERVAL).await,
        }
    }
}

/// Map a zbus method-call error to a readable message. The daemon
/// surfaces its gate refusals as D-Bus errors (disabled, no graph
/// access, capacity), so the text it carries is the useful part.
fn map_call_error(err: zbus::Error) -> String {
    match err {
        zbus::Error::MethodError(_, detail, _) => {
            detail.unwrap_or_else(|| "the AI daemon rejected the request".to_string())
        }
        other => format!("AI daemon error: {other}"),
    }
}

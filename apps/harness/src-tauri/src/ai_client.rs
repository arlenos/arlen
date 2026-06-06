//! Conversation client for the AI daemon (`org.lunaris.AI1`).
//!
//! A2 conversation MVP (ai-app.md §2.1): submit a query and poll it to
//! completion, returning the assistant's answer. The daemon's query
//! path is poll-based — `query` returns a `(query_id, retrieval_token)`
//! handle, and `take_result` is polled until a terminal status — and it
//! authorises result retrieval by the caller's D-Bus connection, so the
//! submit and every poll run on **one** connection held for the call.
//!
//! Each turn is a single, independent query: the daemon query path is
//! NL → validated Cypher → graph → NL, with no conversation memory
//! today (`context_hints` is unused server-side), so prior turns are not
//! carried. The UI says so. Threaded context and token-by-token
//! streaming (the `QueryProgress` signal) are later steps. Nothing is
//! faked: a missing or disabled daemon surfaces as an error the
//! conversation renders.

use std::time::Duration;

use serde::Serialize;
use serde_json::Value;
use tokio::time::{timeout, timeout_at, Instant};
use zbus::{Connection, Proxy};

/// AI daemon bus name, object path, interface.
const AI_BUS_NAME: &str = "org.lunaris.AI1";
const AI_OBJECT_PATH: &str = "/org/lunaris/AI1";

/// How long a whole turn (submit + every poll + the waits between) may
/// take before it is abandoned and the query cancelled.
const QUERY_TIMEOUT: Duration = Duration::from_secs(90);
/// Delay between `take_result` polls.
const POLL_INTERVAL: Duration = Duration::from_millis(250);
/// Bound on the best-effort cancel so cancellation itself cannot hang.
const CANCEL_TIMEOUT: Duration = Duration::from_secs(5);
/// User-facing message when the turn exceeds [`QUERY_TIMEOUT`].
const TIMEOUT_MSG: &str = "the assistant took too long to respond";

/// The outcome of a conversation turn, returned to the frontend.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryReply {
    /// The assistant's answer text.
    pub answer: String,
}

/// The classification of one `take_result` poll envelope.
#[derive(Debug, PartialEq, Eq)]
enum Poll {
    /// Terminal: the answer.
    Completed(String),
    /// Terminal: a failure reason to surface.
    Failed(String),
    /// Terminal: the query was cancelled.
    Cancelled,
    /// Terminal: the result was already consumed.
    Drained,
    /// Non-terminal (pending / in-progress / unknown): keep polling.
    Working,
}

/// Classify a `take_result` JSON envelope. Pure, so the status mapping
/// is unit-tested without a live daemon.
fn classify(outcome: &Value) -> Poll {
    match outcome.get("status").and_then(Value::as_str) {
        Some("completed") => Poll::Completed(
            outcome
                .get("result")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        ),
        Some("failed") => Poll::Failed(
            outcome
                .get("reason")
                .and_then(Value::as_str)
                .unwrap_or("the query failed")
                .to_string(),
        ),
        Some("cancelled") => Poll::Cancelled,
        Some("drained") => Poll::Drained,
        _ => Poll::Working,
    }
}

/// Submit `prompt` to the AI daemon and poll until it completes,
/// returning the answer. Every awaited D-Bus call is bounded by the
/// turn deadline (zbus has no default method timeout), so a stalled
/// daemon cannot hang the turn; on timeout the query is cancelled
/// best-effort. Errors (daemon down, disabled, no graph access, query
/// failure, timeout) come back as a readable string the UI shows.
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

    let deadline = Instant::now() + QUERY_TIMEOUT;

    // Submit (bounded). `context_hints` is unused by the daemon (empty).
    let handle_json: String = match timeout_at(deadline, proxy.call("query", &(prompt.as_str(), "")))
        .await
    {
        Ok(r) => r.map_err(map_call_error)?,
        Err(_) => return Err(TIMEOUT_MSG.to_string()),
    };
    let handle: Value =
        serde_json::from_str(&handle_json).map_err(|e| format!("malformed query handle: {e}"))?;
    let query_id = handle
        .get("query_id")
        .and_then(Value::as_str)
        .ok_or("query handle missing query_id")?
        .to_string();
    let token = handle
        .get("retrieval_token")
        .and_then(Value::as_str)
        .ok_or("query handle missing retrieval_token")?
        .to_string();

    loop {
        let outcome_json: String =
            match timeout_at(deadline, proxy.call("take_result", &(query_id.as_str(), token.as_str())))
                .await
            {
                Ok(r) => r.map_err(map_call_error)?,
                Err(_) => {
                    bounded_cancel(&proxy, &query_id, &token).await;
                    return Err(TIMEOUT_MSG.to_string());
                }
            };
        let outcome: Value = serde_json::from_str(&outcome_json)
            .map_err(|e| format!("malformed result envelope: {e}"))?;

        match classify(&outcome) {
            Poll::Completed(answer) => return Ok(QueryReply { answer }),
            Poll::Failed(reason) => return Err(reason),
            Poll::Cancelled => return Err("the query was cancelled".to_string()),
            Poll::Drained => return Err("the result was already consumed".to_string()),
            Poll::Working => {
                // Wait before the next poll, but never past the deadline.
                if timeout_at(deadline, tokio::time::sleep(POLL_INTERVAL))
                    .await
                    .is_err()
                {
                    bounded_cancel(&proxy, &query_id, &token).await;
                    return Err(TIMEOUT_MSG.to_string());
                }
            }
        }
    }
}

/// Best-effort cancel, itself time-bounded so cancellation cannot become
/// the hang. Errors are ignored: the turn is already being abandoned.
async fn bounded_cancel(proxy: &Proxy<'_>, query_id: &str, token: &str) {
    let _ = timeout(CANCEL_TIMEOUT, async {
        let _: Result<bool, zbus::Error> = proxy.call("cancel", &(query_id, token)).await;
    })
    .await;
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn classify_maps_each_terminal_status() {
        assert_eq!(
            classify(&json!({"status": "completed", "result": "hi"})),
            Poll::Completed("hi".to_string())
        );
        assert_eq!(
            classify(&json!({"status": "failed", "reason": "boom"})),
            Poll::Failed("boom".to_string())
        );
        assert_eq!(classify(&json!({"status": "cancelled"})), Poll::Cancelled);
        assert_eq!(classify(&json!({"status": "drained"})), Poll::Drained);
    }

    #[test]
    fn classify_treats_pending_in_progress_and_unknown_as_working() {
        assert_eq!(classify(&json!({"status": "pending"})), Poll::Working);
        assert_eq!(classify(&json!({"status": "in-progress"})), Poll::Working);
        assert_eq!(classify(&json!({"status": "weird"})), Poll::Working);
        assert_eq!(classify(&json!({})), Poll::Working);
    }

    #[test]
    fn classify_completed_without_result_is_empty_not_a_panic() {
        assert_eq!(
            classify(&json!({"status": "completed"})),
            Poll::Completed(String::new())
        );
    }

    #[test]
    fn classify_failed_without_reason_has_a_default() {
        assert_eq!(
            classify(&json!({"status": "failed"})),
            Poll::Failed("the query failed".to_string())
        );
    }
}

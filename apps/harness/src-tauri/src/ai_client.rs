//! Conversation client for the AI daemon (`org.arlen.AI1`).
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

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::time::{timeout, timeout_at, Instant};
use zbus::{Connection, Proxy};

/// AI daemon bus name, object path, interface.
const AI_BUS_NAME: &str = "org.arlen.AI1";
const AI_OBJECT_PATH: &str = "/org/arlen/AI1";

/// How long a whole turn (submit + every poll + the waits between) may
/// take before it is abandoned and the query cancelled.
const QUERY_TIMEOUT: Duration = Duration::from_secs(90);
/// Delay between `take_result` polls.
const POLL_INTERVAL: Duration = Duration::from_millis(250);
/// Bound on the best-effort cancel so cancellation itself cannot hang.
const CANCEL_TIMEOUT: Duration = Duration::from_secs(5);
/// Fresh budget for fetching the tool trace after the answer arrives. Its
/// own timeout, not the (possibly near-expired) query deadline, so a slow
/// tool-using turn still gets a fair chance to retrieve its trace.
const TRACE_TIMEOUT: Duration = Duration::from_secs(5);
/// User-facing message when the turn exceeds [`QUERY_TIMEOUT`].
const TIMEOUT_MSG: &str = "the assistant took too long to respond";

/// Whether a recorded tool call succeeded (harness-redesign emit seam 1). The
/// daemon's trace carries `done` / `failed`; `running` is the in-flight state the
/// frontend shows for a call before its trace entry lands. Drives the tool-call
/// card's `◷ / ✓ / ✕`.
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum ToolStatus {
    /// In flight (set by the frontend; never emitted in a completed trace).
    Running,
    /// The tool returned a result.
    Done,
    /// The tool call failed.
    Failed,
}

/// One tool call the daemon made while answering, as the trace records it
/// (A3, ai-app.md §2.1). Fields are the daemon's `take_trace` shape; the
/// frontend renders each as a collapsible card so no action is hidden.
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ToolCall {
    /// The MCP server the tool belongs to (e.g. `system.graph`).
    pub server: String,
    /// The tool name (e.g. `query`).
    pub tool: String,
    /// The arguments the model passed, as recorded (already capped daemon-side).
    pub arguments: String,
    /// The tool result the model saw (already capped daemon-side).
    pub result: String,
    /// Whether the call succeeded, from the daemon's trace.
    pub status: ToolStatus,
}

/// The outcome of a conversation turn, returned to the frontend.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryReply {
    /// The assistant's answer text.
    pub answer: String,
    /// The tool calls made while answering, oldest first. Empty when the
    /// query took the direct path (no tool loop): the trace was fetched and
    /// was genuinely empty.
    pub tool_calls: Vec<ToolCall>,
    /// True when the trace could not be retrieved (fetch failed, timed out,
    /// or was malformed) as opposed to being genuinely empty. The UI says so
    /// rather than implying no tools ran, so a slow tool-using turn cannot
    /// masquerade as a direct answer (transparency-first).
    pub trace_unavailable: bool,
    /// The rich-object artifacts the turn produced (the harness redesign's
    /// `Artifact[]` path; the frontend renders them as `Message.artifacts`). The
    /// transport seam: a turn carries its artifacts here. Empty today - no producer
    /// mints them yet (the agent/daemon artifact-emit is a separate AI-layer
    /// slice); when one lands, the artifacts flow end to end with no further
    /// harness/frontend wiring, since the frontend consumer already renders this.
    pub artifacts: Vec<arlen_artifact::Artifact>,
}

/// The result of fetching the tool trace: the calls plus whether retrieval
/// failed. A direct-path answer yields `{ calls: [], unavailable: false }`;
/// a failed/timed-out/malformed fetch yields `{ calls: [], unavailable: true }`.
struct Trace {
    calls: Vec<ToolCall>,
    unavailable: bool,
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
            Poll::Completed(answer) => {
                // The answer is in hand; fetching the tool-call trace is
                // best-effort (it never fails the turn) but its failure is
                // surfaced, not hidden, so a slow tool-using turn is never
                // shown as a direct answer.
                let trace = fetch_trace(&proxy, &query_id, &token).await;
                return Ok(QueryReply {
                    answer,
                    tool_calls: trace.calls,
                    trace_unavailable: trace.unavailable,
                    // No artifact producer in the turn path yet; the transport is
                    // wired so a future agent/daemon emit flows straight through.
                    artifacts: Vec::new(),
                });
            }
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

/// Run System Explanation Mode (Foundation §5.8): ask the daemon for a
/// plain-language summary of what the computer is doing right now. A single
/// bounded call, no poll cycle, since the daemon returns the summary directly.
/// Errors (daemon down, disabled, insufficient scope, timeout) come back as a
/// readable string the UI shows.
#[tauri::command]
pub async fn ai_explain() -> Result<String, String> {
    let connection = Connection::session()
        .await
        .map_err(|e| format!("session bus: {e}"))?;
    let proxy = Proxy::new(&connection, AI_BUS_NAME, AI_OBJECT_PATH, AI_BUS_NAME)
        .await
        .map_err(|e| format!("ai daemon unavailable: {e}"))?;
    let deadline = Instant::now() + QUERY_TIMEOUT;
    match timeout_at(deadline, proxy.call::<_, _, String>("explain_system", &())).await {
        Ok(r) => r.map_err(map_call_error),
        Err(_) => Err(TIMEOUT_MSG.to_string()),
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

/// Fetch the tool-call transcript for a completed query via `take_trace`
/// (single-shot, authorised by the submitting connection like `take_result`),
/// on its own [`TRACE_TIMEOUT`] budget so a near-deadline answer still gets a
/// fair attempt. A successful empty array means the query took the direct path
/// (no tools, `unavailable: false`); a failed, timed-out, or malformed fetch
/// is reported `unavailable: true` so the UI never implies no tools ran when it
/// simply could not read the trace.
///
/// Residual (accepted, not a defect here): `take_trace` is single-shot server
/// side (`std::mem::take`), so if the call reaches the daemon and the trace is
/// moved out but the response is lost to this timeout, a retry returns empty.
/// The timeout is kept deliberately: it must not let a stalled fetch block the
/// already-computed answer from reaching the UI. The trace is daemon-capped and
/// the server op is in-memory and instant, so this timeout effectively only
/// fires on a wedged bus, where the turn itself already failed. Full retry
/// safety would need a peek-until-ack daemon API (a separate change to the
/// settled take_trace contract), not this rendering increment.
async fn fetch_trace(proxy: &Proxy<'_>, query_id: &str, token: &str) -> Trace {
    // Bind the args so the tuple outlives the borrowed call future; pin the
    // reply type to String so the deserialize target is known.
    let args = (query_id, token);
    let json = match timeout(TRACE_TIMEOUT, proxy.call::<_, _, String>("take_trace", &args)).await {
        Ok(Ok(json)) => json,
        _ => return Trace { calls: Vec::new(), unavailable: true },
    };
    match parse_trace(&json) {
        Some(calls) => Trace { calls, unavailable: false },
        None => Trace { calls: Vec::new(), unavailable: true },
    }
}

/// Parse the `take_trace` JSON array into tool calls. Pure, so the shape
/// mapping is unit-tested without a live daemon. `Some(vec)` (possibly empty)
/// is a readable array; `None` is a malformed/non-array payload, which the
/// caller treats as the trace being unavailable, distinct from empty.
fn parse_trace(json: &str) -> Option<Vec<ToolCall>> {
    serde_json::from_str::<Vec<ToolCall>>(json).ok()
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

    #[test]
    fn parse_trace_reads_the_tool_calls_in_order() {
        let json = json!([
            {"server": "system.graph", "tool": "query", "arguments": "MATCH ...", "result": "rows", "status": "done"},
            {"server": "system.knowledge", "tool": "describe_schema", "arguments": "", "result": "err", "status": "failed"}
        ])
        .to_string();
        let calls = parse_trace(&json).expect("valid array parses");
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].server, "system.graph");
        assert_eq!(calls[0].tool, "query");
        assert_eq!(calls[0].status, ToolStatus::Done);
        assert_eq!(calls[1].tool, "describe_schema");
        assert_eq!(calls[1].status, ToolStatus::Failed);
    }

    #[test]
    fn parse_trace_distinguishes_empty_from_unavailable() {
        // A valid empty array is genuinely "no tools" (Some, the direct
        // path); malformed / non-array / missing-field payloads are
        // unavailable (None), so the UI can say so instead of implying no
        // tools ran. This is the transparency distinction.
        assert_eq!(parse_trace("[]"), Some(Vec::new()));
        assert_eq!(parse_trace("not json"), None);
        assert_eq!(parse_trace(r#"{"status":"ok"}"#), None);
        assert_eq!(parse_trace(r#"[{"server":"s","tool":"t"}]"#), None);
    }
}

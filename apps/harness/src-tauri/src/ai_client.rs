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
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::time::{timeout, timeout_at, Instant};
use zbus::{Connection, Proxy};

/// AI daemon bus name, object path, interface.
// `org.arlen.AI1` is now served by the ai-engine-daemon (pi), the drop-in
// replacement for the retired ai-daemon: it owns the name and serves
// `explain_system` (System Explanation Mode as a pi skill).
const AI_BUS_NAME: &str = "org.arlen.AI1";
const AI_OBJECT_PATH: &str = "/org/arlen/AI1";

/// How long a whole turn (submit + every poll + the waits between) may
/// take before it is abandoned and the query cancelled.
const QUERY_TIMEOUT: Duration = Duration::from_secs(90);
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

/// One assistant turn, folded out of the drive stream.
///
/// Pure so the whole mapping is testable without a daemon: the stream is
/// newline-delimited JSON and this is the only thing that interprets it.
#[derive(Debug, Default, PartialEq, Eq)]
struct Turn {
    answer: String,
    calls: Vec<ToolCall>,
    /// The turn ended on `turn_end` rather than on EOF or the deadline. False
    /// means the events seen may be a prefix, which is what `trace_unavailable`
    /// reports: the tools listed are the ones observed, not necessarily all.
    complete: bool,
}

/// Fold one pi agent event into the turn.
///
/// The vocabulary is pi's, not ours - the daemon relays its events byte for
/// byte on purpose, because reshaping trusted model output would be a
/// prompt-injection surface. So this reads pi's documented shapes
/// (`packages/agent/src/types.ts`):
///
///   message_update + assistantMessageEvent.text_delta   the answer, in pieces
///   tool_execution_start { toolCallId, toolName, args }  a call beginning
///   tool_execution_end { toolCallId, result, isError }   its outcome
///   turn_end                                             the turn is done
///
/// Unknown records are ignored rather than refused: pi emits thinking deltas,
/// message boundaries and usage records this surface has no use for, and a
/// client that failed on an unfamiliar event would break every time pi grew one.
fn fold_event(turn: &mut Turn, record: &Value) {
    match record.get("type").and_then(Value::as_str) {
        Some("message_update") => {
            let ev = record.get("assistantMessageEvent");
            if ev.and_then(|e| e.get("type")).and_then(Value::as_str) == Some("text_delta") {
                if let Some(d) = ev.and_then(|e| e.get("delta")).and_then(Value::as_str) {
                    turn.answer.push_str(d);
                }
            }
        }
        Some("tool_execution_start") => {
            let name = record
                .get("toolName")
                .and_then(Value::as_str)
                .unwrap_or_default();
            // pi names a tool `server.tool` where a server exists; the old trace
            // carried the two separately and the surface renders both, so split
            // on the first dot and leave the server empty when there is none
            // rather than inventing one.
            let (server, tool) = match name.split_once('.') {
                Some((s, t)) => (s.to_string(), t.to_string()),
                None => (String::new(), name.to_string()),
            };
            turn.calls.push(ToolCall {
                server,
                tool,
                arguments: compact(record.get("args")),
                result: String::new(),
                // Running until its `tool_execution_end` lands; the surface renders
                // that as in-flight, which is what it is while the stream is open.
                status: ToolStatus::Running,
            });
        }
        Some("tool_execution_end") => {
            let id = record.get("toolCallId").and_then(Value::as_str);
            let name = record.get("toolName").and_then(Value::as_str);
            // Match the end to its start by tool name, newest first: the id is
            // pi's and this surface does not keep it, and a turn does not run
            // two calls of one tool concurrently.
            if let Some(call) = turn.calls.iter_mut().rev().find(|c| {
                name.is_none_or(|n| n == format!("{}{}{}", c.server, if c.server.is_empty() { "" } else { "." }, c.tool))
            }) {
                call.result = compact(record.get("result"));
                call.status = if record.get("isError").and_then(Value::as_bool) == Some(true) {
                    ToolStatus::Failed
                } else {
                    ToolStatus::Done
                };
            }
            let _ = id;
        }
        Some("turn_end") => turn.complete = true,
        _ => {}
    }
}

/// A JSON value as the surface shows it: a string verbatim, anything else
/// compact-encoded. Absent is empty rather than the literal `null`, which would
/// render as a word the model never said.
fn compact(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
    }
}

/// This process's uid, for the fallback runtime path.
fn uid() -> u32 {
    // SAFETY: getuid never fails.
    unsafe { libc::getuid() }
}

/// The drive socket: `$XDG_RUNTIME_DIR/arlen/ai-engine-drive.sock`, else
/// `/run/user/<uid>/arlen/...`. The daemon binds it 0600, so the same-uid
/// boundary is the socket's own permissions - this channel carries prompts to
/// the user's OWN pi.
fn drive_socket_path() -> std::path::PathBuf {
    let base = std::env::var("XDG_RUNTIME_DIR")
        .unwrap_or_else(|_| format!("/run/user/{}", uid()));
    std::path::PathBuf::from(base)
        .join("arlen")
        .join("ai-engine-drive.sock")
}

/// Submit `prompt` and read the turn back off the drive socket.
///
/// **This used to be a D-Bus submit-poll-take against `org.arlen.AI1`, and those
/// methods no longer exist.** The ai-engine-daemon replaced the retired
/// ai-daemon, took the name, and serves only `ExplainSystem`; `query`,
/// `take_result`, `take_trace` and `cancel` went with the daemon that had them,
/// so every turn through here failed with `UnknownMethod`. The file recorded the
/// handover in a comment and kept calling the old API - a comment that documents
/// a migration is not a migration.
///
/// The replacement is a stream, not a poll: one `{"type":"prompt"}` line in,
/// newline-delimited pi agent events out until `turn_end`. Cancelling is
/// dropping the connection, and the turn budget is a deadline on the read loop.
/// The tool calls arrive inline, so there is no second fetch that can fail
/// separately - `trace_unavailable` now means the stream ended before `turn_end`
/// and the calls listed may be a prefix.
#[tauri::command]
pub async fn ai_query(prompt: String) -> Result<QueryReply, String> {
    let path = drive_socket_path();
    let mut stream = UnixStream::connect(&path).await.map_err(|e| {
        format!("the assistant is not reachable: {e} (arlen-ai-engine-daemon)")
    })?;

    let command = serde_json::json!({ "type": "prompt", "message": prompt }).to_string();
    stream
        .write_all(format!("{command}\n").as_bytes())
        .await
        .map_err(|e| format!("could not send the prompt: {e}"))?;

    let deadline = Instant::now() + QUERY_TIMEOUT;
    let mut turn = Turn::default();
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];

    while !turn.complete {
        let read = timeout_at(deadline, stream.read(&mut chunk)).await;
        let n = match read {
            // A deadline or a closed peer both end the turn with what we have;
            // the answer so far is worth more than an error, and `complete`
            // stays false so the surface can say the trace may be partial.
            Err(_) => break,
            Ok(Ok(0)) => break,
            Ok(Ok(n)) => n,
            Ok(Err(e)) => return Err(format!("the assistant connection failed: {e}")),
        };
        buf.extend_from_slice(&chunk[..n]);

        while let Some(pos) = buf.iter().position(|b| *b == b'\n') {
            let line = String::from_utf8_lossy(&buf[..pos]).to_string();
            buf.drain(..=pos);
            if line.trim().is_empty() {
                continue;
            }
            if let Ok(record) = serde_json::from_str::<Value>(&line) {
                fold_event(&mut turn, &record);
            }
        }
    }

    if turn.answer.is_empty() && !turn.complete {
        return Err(TIMEOUT_MSG.to_string());
    }

    Ok(QueryReply {
        answer: turn.answer,
        tool_calls: turn.calls,
        trace_unavailable: !turn.complete,
        // No artifact producer in the turn path yet; the transport is wired so a
        // future agent/daemon emit flows straight through.
        artifacts: Vec::new(),
    })
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
        .map_err(|e| format!("ai engine unavailable: {e}"))?;
    let deadline = Instant::now() + QUERY_TIMEOUT;
    match timeout_at(deadline, proxy.call::<_, _, String>("explain_system", &())).await {
        Ok(r) => r.map_err(map_call_error),
        Err(_) => Err(TIMEOUT_MSG.to_string()),
    }
}

/// Map a zbus method-call error to a readable message. The daemon surfaces its
/// gate refusals as D-Bus errors (disabled, no graph access, capacity), so the
/// text it carries is the useful part. Still used by `ai_explain`, which is the
/// one call on this interface that still exists.
fn map_call_error(err: zbus::Error) -> String {
    match err {
        zbus::Error::MethodError(_, detail, _) => {
            detail.unwrap_or_else(|| "the AI engine rejected the request".to_string())
        }
        other => format!("AI engine error: {other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fold(records: &[serde_json::Value]) -> Turn {
        let mut turn = Turn::default();
        for r in records {
            fold_event(&mut turn, r);
        }
        turn
    }

    /// The answer arrives in pieces and is one string by the end.
    #[test]
    fn text_deltas_concatenate_in_order() {
        let t = fold(&[
            json!({"type": "message_update", "assistantMessageEvent": {"type": "text_delta", "delta": "Hel"}}),
            json!({"type": "message_update", "assistantMessageEvent": {"type": "text_delta", "delta": "lo"}}),
            json!({"type": "turn_end"}),
        ]);
        assert_eq!(t.answer, "Hello");
        assert!(t.complete);
    }

    /// A tool call is a start and an end, and both halves land on one record.
    #[test]
    fn a_tool_call_pairs_its_start_with_its_end() {
        let t = fold(&[
            json!({"type": "tool_execution_start", "toolCallId": "1", "toolName": "graph.read", "args": {"q": "x"}}),
            json!({"type": "tool_execution_end", "toolCallId": "1", "toolName": "graph.read", "result": "3 rows", "isError": false}),
            json!({"type": "turn_end"}),
        ]);
        assert_eq!(t.calls.len(), 1);
        assert_eq!(t.calls[0].server, "graph");
        assert_eq!(t.calls[0].tool, "read");
        assert_eq!(t.calls[0].arguments, r#"{"q":"x"}"#);
        assert_eq!(t.calls[0].result, "3 rows");
        assert_eq!(t.calls[0].status, ToolStatus::Done);
    }

    /// A failing tool is `failed`, which is what the card's ✕ reads from.
    #[test]
    fn an_erroring_tool_call_is_marked_failed() {
        let t = fold(&[
            json!({"type": "tool_execution_start", "toolCallId": "1", "toolName": "fs.write", "args": {}}),
            json!({"type": "tool_execution_end", "toolCallId": "1", "toolName": "fs.write", "result": "denied", "isError": true}),
        ]);
        assert_eq!(t.calls[0].status, ToolStatus::Failed);
    }

    /// A call whose end never arrives stays in flight rather than reading as done.
    #[test]
    fn a_call_without_an_end_stays_running() {
        let t = fold(&[
            json!({"type": "tool_execution_start", "toolCallId": "1", "toolName": "slow.thing", "args": {}}),
        ]);
        assert_eq!(t.calls[0].status, ToolStatus::Running);
        assert!(!t.complete, "no turn_end means the turn may be a prefix");
    }

    /// pi emits thinking deltas, message boundaries and usage records this
    /// surface has no use for. Ignoring them is deliberate: a client that
    /// refused an unfamiliar event would break every time pi grew one.
    #[test]
    fn unknown_records_are_ignored_rather_than_refused() {
        let t = fold(&[
            json!({"type": "agent_start"}),
            json!({"type": "message_update", "assistantMessageEvent": {"type": "thinking_delta", "delta": "hmm"}}),
            json!({"type": "something_pi_grew_later"}),
            json!({"type": "message_update", "assistantMessageEvent": {"type": "text_delta", "delta": "hi"}}),
        ]);
        assert_eq!(t.answer, "hi", "thinking must not leak into the answer");
        assert!(t.calls.is_empty());
    }

    /// A tool with no server prefix keeps an empty server rather than an
    /// invented one; the card renders what pi actually said.
    #[test]
    fn a_bare_tool_name_has_no_server() {
        let t = fold(&[
            json!({"type": "tool_execution_start", "toolCallId": "1", "toolName": "bash", "args": {}}),
        ]);
        assert_eq!(t.calls[0].server, "");
        assert_eq!(t.calls[0].tool, "bash");
    }
}

//! The pi drive client (`pi-agent-adoption.md` step 6): the harness's conversation
//! path to the pi ENGINE, replacing the old poll-based ai-daemon query. It connects
//! to the engine's drive socket (which raw-relays to pi's `--mode rpc`
//! stdin/stdout), submits the user turn as pi's `prompt` command, and streams pi's
//! JSON-line session-event stream back to the A7 UI as `pi://event` Tauri events
//! until the turn ends (pi's `agent_end` event).

use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

/// Resolve the engine drive socket path from `xdg_runtime_dir`
/// (`<dir>/arlen/ai-engine-drive.sock`, else the system `/run/arlen/...`), matching
/// the engine daemon's own `drive_socket_path`.
fn resolve_drive_path(xdg_runtime_dir: Option<&str>) -> std::path::PathBuf {
    match xdg_runtime_dir {
        Some(dir) if !dir.is_empty() => {
            std::path::PathBuf::from(dir).join("arlen").join("ai-engine-drive.sock")
        }
        _ => std::path::PathBuf::from("/run/arlen/ai-engine-drive.sock"),
    }
}

/// The engine drive socket path for this session.
fn drive_socket_path() -> std::path::PathBuf {
    resolve_drive_path(std::env::var("XDG_RUNTIME_DIR").ok().as_deref())
}

/// pi's `prompt` command (its `--mode rpc` submit): one JSON line written to the
/// drive socket. Pure, so the wire shape is unit-tested.
fn prompt_command(message: &str) -> serde_json::Value {
    serde_json::json!({ "type": "prompt", "message": message })
}

/// pi's `get_last_assistant_text` command, tagged with a fixed correlation id so
/// its response is matched unambiguously.
fn get_last_text_command() -> serde_json::Value {
    serde_json::json!({ "type": "get_last_assistant_text", "id": GET_TEXT_ID })
}

/// Correlation id for the `get_last_assistant_text` response.
const GET_TEXT_ID: &str = "arlen-get-last-text";

/// The pi session-event type that ends a turn (the agent finished its response).
const TURN_END_EVENT: &str = "agent_end";

/// Whether `event` is the response to our `get_last_assistant_text` request (pi
/// correlates responses by `id`).
fn is_get_text_response(event: &serde_json::Value) -> bool {
    event.get("type").and_then(|t| t.as_str()) == Some("response")
        && event.get("command").and_then(|c| c.as_str()) == Some("get_last_assistant_text")
        && event.get("id").and_then(|i| i.as_str()) == Some(GET_TEXT_ID)
}

/// The assistant text carried by a `get_last_assistant_text` response
/// (`data.text`); a null or absent text is the empty string.
fn assistant_text_of(event: &serde_json::Value) -> String {
    event
        .get("data")
        .and_then(|d| d.get("text"))
        .and_then(|t| t.as_str())
        .unwrap_or("")
        .to_string()
}

/// Write one JSON command as a line to the drive socket.
async fn write_command(
    write: &mut (impl AsyncWriteExt + Unpin),
    command: &serde_json::Value,
) -> Result<(), String> {
    let mut line = serde_json::to_vec(command).map_err(|e| e.to_string())?;
    line.push(b'\n');
    write.write_all(&line).await.map_err(|e| format!("engine write failed: {e}"))?;
    write.flush().await.map_err(|e| e.to_string())
}

/// Submit a user turn to the pi engine, stream its session events to the frontend
/// as `pi://event` Tauri events, and return the assistant's answer. The turn runs
/// in two phases on one connection: (1) submit the `prompt` and forward every
/// session event (the A7 components render tool calls / transparency) until pi's
/// `agent_end`; (2) fetch `get_last_assistant_text` and return it as the answer
/// (matching the old poll path's answer-returning shape, so the store swap is
/// trivial). A malformed line is skipped rather than aborting the turn.
/// A debug/verification hook: the prompt to auto-drive once on load, from
/// `ARLEN_HARNESS_AUTODRIVE` (unset or empty = none). This lets the headless
/// screenshot pipeline drive a real pi turn WITHOUT synthetic keyboard/mouse
/// input, which is unreliable under a headless compositor. Off by default - a
/// normal run sets no such env var, so this returns `None`.
#[tauri::command]
pub fn pi_autodrive_prompt() -> Option<String> {
    std::env::var("ARLEN_HARNESS_AUTODRIVE")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// A debug/verification hook: a route to navigate to once on load, AFTER any
/// auto-driven prompt has run, from `ARLEN_HARNESS_ROUTE` (unset or empty =
/// none). This lets the headless screenshot pipeline capture a non-default
/// route - e.g. the `/agent` activity timeline showing the just-driven turn's
/// audited actions - without synthetic keyboard/mouse input. Off by default.
#[tauri::command]
pub fn pi_autodrive_route() -> Option<String> {
    std::env::var("ARLEN_HARNESS_ROUTE")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

#[tauri::command]
pub async fn pi_prompt(app: AppHandle, prompt: String) -> Result<String, String> {
    let stream = UnixStream::connect(drive_socket_path())
        .await
        .map_err(|e| format!("could not reach the AI engine: {e}"))?;
    let (read, mut write) = stream.into_split();
    let mut lines = BufReader::new(read).lines();

    // Phase 1: submit + stream events until the turn ends. A per-turn event count
    // is logged (WebView DevTools are not always reachable, so drive diagnostics go
    // through the Rust logger): a turn that ends before `agent_end` reports how many
    // events arrived, which distinguishes a dropped relay from pi ending its own run.
    write_command(&mut write, &prompt_command(&prompt)).await?;
    let mut seen = 0usize;
    loop {
        let Some(text) =
            lines.next_line().await.map_err(|e| format!("engine read failed: {e}"))?
        else {
            log::warn!("pi drive: engine closed after {seen} events, before the turn finished");
            return Err("the AI engine closed before the turn finished".to_string());
        };
        let trimmed = text.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(event) = serde_json::from_str::<serde_json::Value>(trimmed) else {
            continue;
        };
        seen += 1;
        let is_end = event.get("type").and_then(|t| t.as_str()) == Some(TURN_END_EVENT);
        let _ = app.emit("pi://event", &event);
        if is_end {
            log::info!("pi drive: turn finished after {seen} events");
            break;
        }
    }

    // Phase 2: fetch the final assistant text (forwarding any interleaved events).
    write_command(&mut write, &get_last_text_command()).await?;
    loop {
        let Some(text) =
            lines.next_line().await.map_err(|e| format!("engine read failed: {e}"))?
        else {
            return Ok(String::new()); // engine went away; empty answer, not an error
        };
        let trimmed = text.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(event) = serde_json::from_str::<serde_json::Value>(trimmed) else {
            continue;
        };
        if is_get_text_response(&event) {
            return Ok(assistant_text_of(&event));
        }
        let _ = app.emit("pi://event", &event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_command_is_pis_rpc_submit_shape() {
        let c = prompt_command("hello there");
        assert_eq!(c["type"], "prompt");
        assert_eq!(c["message"], "hello there");
    }

    #[test]
    fn drive_path_uses_the_runtime_dir_when_present() {
        assert_eq!(
            resolve_drive_path(Some("/run/user/1000")),
            std::path::PathBuf::from("/run/user/1000/arlen/ai-engine-drive.sock")
        );
    }

    #[test]
    fn drive_path_falls_back_to_the_system_run_dir() {
        assert_eq!(
            resolve_drive_path(None),
            std::path::PathBuf::from("/run/arlen/ai-engine-drive.sock")
        );
        assert_eq!(
            resolve_drive_path(Some("")),
            std::path::PathBuf::from("/run/arlen/ai-engine-drive.sock")
        );
    }

    #[test]
    fn get_text_command_is_id_tagged() {
        let c = get_last_text_command();
        assert_eq!(c["type"], "get_last_assistant_text");
        assert_eq!(c["id"], GET_TEXT_ID);
    }

    #[test]
    fn recognises_the_correlated_get_text_response() {
        let ok = serde_json::json!({
            "type": "response", "command": "get_last_assistant_text",
            "id": GET_TEXT_ID, "data": { "text": "the answer" }
        });
        assert!(is_get_text_response(&ok));
        assert_eq!(assistant_text_of(&ok), "the answer");
    }

    #[test]
    fn rejects_a_response_with_the_wrong_id_or_command() {
        let wrong_id = serde_json::json!({
            "type": "response", "command": "get_last_assistant_text", "id": "other"
        });
        assert!(!is_get_text_response(&wrong_id));
        let wrong_cmd = serde_json::json!({
            "type": "response", "command": "get_state", "id": GET_TEXT_ID
        });
        assert!(!is_get_text_response(&wrong_cmd));
        // An event that is not a response at all.
        assert!(!is_get_text_response(&serde_json::json!({ "type": "agent_end" })));
    }

    #[test]
    fn a_null_or_absent_text_is_the_empty_string() {
        let null_text = serde_json::json!({ "data": { "text": null } });
        assert_eq!(assistant_text_of(&null_text), "");
        assert_eq!(assistant_text_of(&serde_json::json!({})), "");
    }
}

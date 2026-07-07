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

/// The pi session-event type that ends a turn (the agent finished its response).
const TURN_END_EVENT: &str = "agent_end";

/// Submit a user turn to the pi engine and stream its session events to the
/// frontend as `pi://event` Tauri events, returning when the turn ends. Each event
/// is pi's raw session-event JSON, which the A7 components interpret (`text` deltas,
/// tool calls, ...). The loop ends on pi's `agent_end` event (turn complete) or on
/// socket EOF. A malformed line is skipped defensively rather than aborting the turn.
#[tauri::command]
pub async fn pi_prompt(app: AppHandle, prompt: String) -> Result<(), String> {
    let stream = UnixStream::connect(drive_socket_path())
        .await
        .map_err(|e| format!("could not reach the AI engine: {e}"))?;
    let (read, mut write) = stream.into_split();

    let mut line = serde_json::to_vec(&prompt_command(&prompt)).map_err(|e| e.to_string())?;
    line.push(b'\n');
    write.write_all(&line).await.map_err(|e| format!("engine write failed: {e}"))?;
    write.flush().await.map_err(|e| e.to_string())?;

    let mut lines = BufReader::new(read).lines();
    while let Some(text) =
        lines.next_line().await.map_err(|e| format!("engine read failed: {e}"))?
    {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(event) = serde_json::from_str::<serde_json::Value>(trimmed) else {
            continue; // skip a malformed line, never abort the turn on it
        };
        let is_end = event.get("type").and_then(|t| t.as_str()) == Some(TURN_END_EVENT);
        let _ = app.emit("pi://event", &event);
        if is_end {
            break;
        }
    }
    Ok(())
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
}

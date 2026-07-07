//! An internal pi `--mode rpc` driver: submit one prompt to a connected pi and
//! return its assistant answer. The daemon uses this to drive an EPHEMERAL pi run
//! it spawned itself (e.g. System Explanation Mode, `pi-agent-adoption.md` §D) -
//! distinct from the harness driving the persistent conversation pi over the drive
//! socket. It speaks the same pi rpc protocol (JSON lines): submit a `prompt`,
//! stream session events until `agent_end`, then fetch `get_last_assistant_text`.

use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};

/// Correlation id for the `get_last_assistant_text` response.
const GET_TEXT_ID: &str = "arlen-engine-get-last-text";

/// The pi session-event type that ends a turn.
const TURN_END_EVENT: &str = "agent_end";

/// pi's `prompt` command (the rpc submit): one JSON line.
fn prompt_command(message: &str) -> serde_json::Value {
    serde_json::json!({ "type": "prompt", "message": message })
}

/// pi's `get_last_assistant_text` command, id-tagged so its response is matched.
fn get_last_text_command() -> serde_json::Value {
    serde_json::json!({ "type": "get_last_assistant_text", "id": GET_TEXT_ID })
}

/// Whether `event` is the response to our `get_last_assistant_text` request.
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

/// Write one JSON command as a line.
async fn write_command<W: AsyncWrite + Unpin>(
    write: &mut W,
    command: &serde_json::Value,
) -> Result<(), String> {
    let mut line = serde_json::to_vec(command).map_err(|e| e.to_string())?;
    line.push(b'\n');
    write.write_all(&line).await.map_err(|e| format!("engine write failed: {e}"))?;
    write.flush().await.map_err(|e| e.to_string())
}

/// Submit `prompt` to a connected pi (over its rpc `read`/`write` stdio) and return
/// the assistant's answer: submit, drain session events until `agent_end`, then
/// fetch `get_last_assistant_text`. A malformed line is skipped rather than
/// aborting; the engine closing before the turn ends is an error.
pub async fn drive_for_answer<R, W>(read: R, mut write: W, prompt: &str) -> Result<String, String>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut lines = BufReader::new(read).lines();

    // Phase 1: submit + drain until the turn ends.
    write_command(&mut write, &prompt_command(prompt)).await?;
    loop {
        let Some(text) = lines.next_line().await.map_err(|e| format!("engine read failed: {e}"))?
        else {
            return Err("the engine closed before the turn finished".to_string());
        };
        let trimmed = text.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(event) = serde_json::from_str::<serde_json::Value>(trimmed) else {
            continue;
        };
        if event.get("type").and_then(|t| t.as_str()) == Some(TURN_END_EVENT) {
            break;
        }
    }

    // Phase 2: fetch the final assistant text.
    write_command(&mut write, &get_last_text_command()).await?;
    loop {
        let Some(text) = lines.next_line().await.map_err(|e| format!("engine read failed: {e}"))?
        else {
            return Ok(String::new());
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_shapes_and_response_matching() {
        assert_eq!(prompt_command("hi")["type"], "prompt");
        assert_eq!(prompt_command("hi")["message"], "hi");
        assert_eq!(get_last_text_command()["type"], "get_last_assistant_text");
        let ok = serde_json::json!({
            "type": "response", "command": "get_last_assistant_text",
            "id": GET_TEXT_ID, "data": { "text": "an explanation" }
        });
        assert!(is_get_text_response(&ok));
        assert_eq!(assistant_text_of(&ok), "an explanation");
        assert_eq!(assistant_text_of(&serde_json::json!({ "data": { "text": null } })), "");
        assert!(!is_get_text_response(&serde_json::json!({ "type": "agent_end" })));
    }

    #[tokio::test]
    async fn drives_a_scripted_pi_to_its_answer() {
        // A scripted pi: after the prompt it emits a couple of events + agent_end,
        // then answers the get_last_assistant_text request.
        let pi_out = concat!(
            "{\"type\":\"message_start\"}\n",
            "{\"type\":\"agent_end\"}\n",
            "{\"type\":\"response\",\"command\":\"get_last_assistant_text\",\"id\":\"arlen-engine-get-last-text\",\"data\":{\"text\":\"you are editing a file\"}}\n",
        );
        let mut written = Vec::new();
        let answer = drive_for_answer(pi_out.as_bytes(), &mut written, "explain the current state")
            .await
            .unwrap();
        assert_eq!(answer, "you are editing a file");
        // Both the prompt and the get-text command were sent.
        let sent = String::from_utf8(written).unwrap();
        assert!(sent.contains("\"type\":\"prompt\""));
        assert!(sent.contains("get_last_assistant_text"));
    }

    #[tokio::test]
    async fn an_engine_that_closes_early_errors() {
        let r = drive_for_answer(&b"{\"type\":\"message_start\"}\n"[..], &mut Vec::new(), "x").await;
        assert!(r.is_err());
    }
}

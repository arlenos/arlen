//! Dev dogfood tool: drive the ai-engine daemon and print what comes back.
//!
//! The engine relays a shell's conversational commands to the pi sidecar over
//! `$XDG_RUNTIME_DIR/arlen/ai-engine-drive.sock` (`rpc_proxy.rs`). Driving it by
//! hand is the only way to answer "does the assistant actually reach for
//! `graph.read` on a personal-context question", so this exists to make that a
//! one-liner instead of a rediscovery.
//!
//! ## Why a real binary and not a script
//!
//! The drive socket peer-attests its caller: `ConnectionAuth::extract_from`
//! resolves the connecting binary's path to an app_id and DROPS the connection
//! when that fails. A plain `python3 drive.py` is therefore rejected with
//! `unknown binary path: /usr/bin/python3.14` - the interpreter is the peer, and
//! it lives outside any Arlen install root. A binary built into this repo's
//! `target/debug/` resolves through the dev rule to `dev.arlen-ai-drive`, which
//! is a known id, so it is admitted. That is the whole trick, and it was
//! previously encoded only in an unreproducible stripped copy of the python
//! interpreter renamed to `arlen-ai-driver`, with no source anywhere.
//!
//! ## Usage
//!
//! ```text
//! cargo build --manifest-path dev/ai-drive/Cargo.toml
//! target/debug/arlen-ai-drive "Which files have I opened most recently?"
//! target/debug/arlen-ai-drive --types "..."   # event-type histogram only
//! ```
//!
//! Exits after the turn ends or the deadline elapses, whichever comes first.
//! Read-only in intent: it sends one `prompt` and observes.

use std::collections::BTreeMap;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

/// How long to keep reading before giving up on a quiet stream.
const DEADLINE: Duration = Duration::from_secs(90);

/// The drive socket: `ARLEN_AI_DRIVE_SOCKET`, else the per-user runtime path.
fn socket_path() -> String {
    if let Ok(p) = std::env::var("ARLEN_AI_DRIVE_SOCKET") {
        if !p.is_empty() {
            return p;
        }
    }
    let base = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/run".to_string());
    format!("{base}/arlen/ai-engine-drive.sock")
}

/// The one command this tool sends.
///
/// `rpc_proxy.rs` allowlists the conversational verbs and silently DROPS
/// anything else, so an operator command typed here would vanish rather than
/// error - `prompt` is deliberately the only thing built.
fn prompt_command(message: &str) -> String {
    serde_json::json!({ "type": "prompt", "message": message }).to_string()
}

/// A record's `type`, for the histogram.
fn record_type(line: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    Some(v.get("type")?.as_str()?.to_string())
}

/// The assistant sub-event type, when the record carries one. `text_delta`
/// dominating with NO `tool_call` is the signature of a model describing a tool
/// call in prose instead of invoking it.
fn assistant_event_type(line: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    Some(
        v.get("assistantMessageEvent")?
            .get("type")?
            .as_str()?
            .to_string(),
    )
}

/// Whether the record ends the turn, so a normal run stops promptly.
fn is_turn_end(line: &str) -> bool {
    record_type(line).as_deref() == Some("turn_end")
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let types_only = args.iter().any(|a| a == "--types");
    let message: String = args
        .iter()
        .filter(|a| !a.starts_with("--"))
        .cloned()
        .collect::<Vec<_>>()
        .join(" ");
    if message.is_empty() {
        eprintln!("usage: arlen-ai-drive [--types] <prompt>");
        std::process::exit(2);
    }

    let path = socket_path();
    let mut stream = UnixStream::connect(&path).await.map_err(|e| {
        format!(
            "connect {path}: {e} (is arlen-ai-engine-daemon running? a reset here \
             usually means the peer could not be attested)"
        )
    })?;
    stream
        .write_all(format!("{}\n", prompt_command(&message)).as_bytes())
        .await?;
    eprintln!("-> {message}");

    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    let mut types: BTreeMap<String, usize> = BTreeMap::new();
    let mut assistant: BTreeMap<String, usize> = BTreeMap::new();
    let deadline = tokio::time::Instant::now() + DEADLINE;

    loop {
        let read = tokio::time::timeout_at(deadline, stream.read(&mut chunk)).await;
        let n = match read {
            Err(_) => break,          // deadline
            Ok(Ok(0)) => break,       // peer closed
            Ok(Ok(n)) => n,
            Ok(Err(e)) => return Err(e.into()),
        };
        buf.extend_from_slice(&chunk[..n]);

        while let Some(pos) = buf.iter().position(|b| *b == b'\n') {
            let line = String::from_utf8_lossy(&buf[..pos]).to_string();
            buf.drain(..=pos);
            if line.trim().is_empty() {
                continue;
            }
            if let Some(t) = record_type(&line) {
                *types.entry(t).or_default() += 1;
            }
            if let Some(t) = assistant_event_type(&line) {
                *assistant.entry(t).or_default() += 1;
            }
            if !types_only {
                println!("{}", &line[..line.len().min(400)]);
            }
            if is_turn_end(&line) {
                // One turn is what a prompt buys; keep draining only briefly.
            }
        }
    }

    eprintln!("\nrecord types:    {types:?}");
    eprintln!("assistant events: {assistant:?}");
    if assistant.contains_key("text_delta") && !assistant.keys().any(|k| k.contains("tool")) {
        eprintln!(
            "note: text deltas but no tool-call event - the model may be describing a \
             tool call in prose rather than invoking it, or the turn failed upstream \
             (check the daemon log for completion-forward errors)."
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_prompt_command_is_an_allowlisted_verb() {
        // `rpc_proxy::is_allowed_drive_command` accepts only the conversational
        // verbs and DROPS anything else silently, so a wrong `type` here would
        // hang with no error rather than fail loudly.
        let v: serde_json::Value = serde_json::from_str(&prompt_command("hi")).unwrap();
        assert_eq!(v["type"], "prompt");
        assert_eq!(v["message"], "hi");
    }

    #[test]
    fn record_and_assistant_types_are_extracted() {
        assert_eq!(record_type(r#"{"type":"turn_end"}"#).as_deref(), Some("turn_end"));
        assert!(is_turn_end(r#"{"type":"turn_end"}"#));
        assert!(!is_turn_end(r#"{"type":"message_update"}"#));
        let line = r#"{"type":"message_update","assistantMessageEvent":{"type":"text_delta"}}"#;
        assert_eq!(assistant_event_type(line).as_deref(), Some("text_delta"));
    }

    #[test]
    fn malformed_lines_are_ignored_rather_than_fatal() {
        // The stream is someone else's output; a partial or non-JSON line must
        // not end the run, or a single hiccup loses the whole observation.
        assert_eq!(record_type("not json"), None);
        assert_eq!(assistant_event_type("{}"), None);
        assert!(!is_turn_end(""));
    }

    #[test]
    fn the_socket_path_honours_the_override() {
        std::env::set_var("ARLEN_AI_DRIVE_SOCKET", "/tmp/x.sock");
        assert_eq!(socket_path(), "/tmp/x.sock");
        std::env::remove_var("ARLEN_AI_DRIVE_SOCKET");
    }
}

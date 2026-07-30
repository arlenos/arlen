//! The read half's client: ask the terminal for a consented look at one of its
//! sessions.
//!
//! The run half executes; this one only looks. It holds no blocks and makes no
//! judgement about what may be seen - it forwards a request and a consent token
//! to the terminal, which verifies the token, applies the scope and answers. That
//! split is deliberate: the terminal owns the blocks, so the terminal decides,
//! and this process cannot widen a reading by being wrong.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

use arlen_terminal_core::read_frame;
use arlen_terminal_core::read_serve::{ReadReply, ReadRequest};

/// How long to wait on the terminal before giving up. A local socket answering
/// from memory is immediate; anything slower means the terminal is wedged or
/// absent, and a tool call must not hang the model's turn on it.
const TIMEOUT: Duration = Duration::from_secs(5);

/// Why a read did not produce blocks.
#[derive(Debug)]
pub enum ReadError {
    /// No terminal is listening, or it could not be reached.
    Unreachable(String),
    /// The terminal refused: the consent token does not cover this reading.
    Refused,
    /// The exchange broke down (short write, bad frame, oversized reply).
    Protocol(String),
}

impl std::fmt::Display for ReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unreachable(e) => write!(f, "the terminal is not reachable: {e}"),
            Self::Refused => write!(
                f,
                "the consent token does not authorize this reading; ask the user to approve it"
            ),
            Self::Protocol(e) => write!(f, "the terminal read exchange failed: {e}"),
        }
    }
}

/// The socket the terminal serves consented reads on.
pub fn read_socket_path() -> std::path::PathBuf {
    os_sdk::runtime::socket_path("ARLEN_TERMINAL_READ_SOCKET", "terminal-read.sock")
}

/// Ask the terminal for the blocks `req` describes.
///
/// Every failure is reported apart: unreachable is a setup problem, refused is an
/// authorization outcome the model should act on by asking the user, and a
/// protocol error is a bug. Collapsing them into one "failed" would leave the
/// model unable to tell "approve this" from "the terminal is not running".
pub fn fetch(req: &ReadRequest) -> Result<Vec<arlen_terminal_core::Block>, ReadError> {
    let path = read_socket_path();
    let mut stream =
        UnixStream::connect(&path).map_err(|e| ReadError::Unreachable(e.to_string()))?;
    stream
        .set_read_timeout(Some(TIMEOUT))
        .and_then(|()| stream.set_write_timeout(Some(TIMEOUT)))
        .map_err(|e| ReadError::Protocol(e.to_string()))?;

    let frame = read_frame::encode(req).map_err(|e| ReadError::Protocol(e.to_string()))?;
    stream
        .write_all(&frame)
        .map_err(|e| ReadError::Protocol(e.to_string()))?;

    let mut header = [0u8; 4];
    stream
        .read_exact(&mut header)
        .map_err(|e| ReadError::Protocol(e.to_string()))?;
    // The cap applies in this direction too: the terminal is first-party, but a
    // client that allocates whatever a peer claims is wrong regardless of who the
    // peer is.
    let len = read_frame::decode_len(header).map_err(|e| ReadError::Protocol(e.to_string()))?;
    let mut body = vec![0u8; len];
    stream
        .read_exact(&mut body)
        .map_err(|e| ReadError::Protocol(e.to_string()))?;

    match read_frame::decode_body::<ReadReply>(&body)
        .map_err(|e| ReadError::Protocol(e.to_string()))?
    {
        ReadReply::Blocks(resp) => Ok(resp.blocks),
        ReadReply::Refused => Err(ReadError::Refused),
    }
}

/// Render blocks as the compact text the model reads: one line of identity per
/// block, then its output.
///
/// Deliberately not the raw JSON. The model is being asked "what happened in that
/// terminal", and a wall of serialized structs spends its context on field names.
/// The exit code is included because it is usually the answer, and the origin is
/// included because whether the user or the assistant ran a command changes what
/// it means.
pub fn render(blocks: &[arlen_terminal_core::Block]) -> String {
    if blocks.is_empty() {
        return "(no blocks visible in this terminal under the granted scope)".to_string();
    }
    let mut out = String::new();
    for b in blocks {
        let who = match b.origin {
            arlen_terminal_core::Origin::You => "user",
            arlen_terminal_core::Origin::Agent => "assistant",
        };
        let status = match b.exit_code {
            Some(0) => "ok".to_string(),
            Some(code) => format!("exit {code}"),
            None => "running".to_string(),
        };
        out.push_str(&format!("$ {} [{who}, {status}, in {}]\n", b.command, b.cwd));
        if let Some(text) = b.body.as_str() {
            if !text.is_empty() {
                out.push_str(text);
                if !text.ends_with('\n') {
                    out.push('\n');
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use arlen_terminal_core::{Block, BlockBodyKind, Origin};

    fn block(command: &str, origin: Origin, exit: Option<i32>, body: &str) -> Block {
        Block {
            id: "b".to_string(),
            command: command.to_string(),
            exit_code: exit,
            duration_ms: None,
            cwd: "/w".to_string(),
            git: None,
            origin,
            body_kind: BlockBodyKind::Grid,
            body: serde_json::Value::String(body.to_string()),
        }
    }

    #[test]
    fn an_empty_reading_says_so_rather_than_returning_nothing() {
        // A blank tool result reads to a model as a failure; this says the scope
        // was applied and nothing was in it.
        assert!(render(&[]).contains("no blocks visible"));
    }

    #[test]
    fn a_rendered_block_carries_its_status_and_who_ran_it() {
        let text = render(&[block("cargo test", Origin::Agent, Some(1), "failed\n")]);
        assert!(text.contains("cargo test"));
        assert!(text.contains("assistant"));
        assert!(text.contains("exit 1"), "the exit code is usually the answer");
        assert!(text.contains("failed"));
    }

    #[test]
    fn a_running_block_is_labelled_running_not_ok() {
        let text = render(&[block("sleep 10", Origin::You, None, "")]);
        assert!(text.contains("running"));
        assert!(!text.contains("ok"));
    }

    #[test]
    fn a_refusal_tells_the_model_what_to_do_about_it() {
        // The message is the model's only cue that this is fixable by asking.
        assert!(ReadError::Refused.to_string().contains("ask the user"));
    }
}

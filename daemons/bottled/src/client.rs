//! Asking the bottle daemon something, from a process that is not it.
//!
//! BLOCKING and one connection per call, which is the shape the Settings backend
//! wants: it runs each of these on a blocking thread and has no runtime of its own
//! to share. The daemon's own side is async because it serves many callers; a
//! caller asking one question does not need that.
//!
//! A daemon that is not running is a failed connect, and that is the answer -
//! never an empty list. The two are different things and the panel says so.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;

use crate::protocol::{Request, Response};
use crate::server::MAX_FRAME;

/// Why an ask did not come back.
#[derive(Debug)]
pub enum ClientError {
    /// The daemon could not be reached at all.
    NotRunning(std::io::Error),
    /// It was reached and the exchange failed.
    Transport(std::io::Error),
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClientError::NotRunning(e) => write!(f, "the Windows runtime is not running: {e}"),
            ClientError::Transport(e) => write!(f, "the Windows runtime stopped answering: {e}"),
        }
    }
}

impl std::error::Error for ClientError {}

/// Ask the daemon at `socket` one question.
pub fn ask(socket: &Path, request: &Request) -> Result<Response, ClientError> {
    let mut stream = UnixStream::connect(socket).map_err(ClientError::NotRunning)?;
    let body = serde_json::to_vec(request).map_err(|e| {
        ClientError::Transport(std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    })?;
    stream
        .write_all(&(body.len() as u32).to_be_bytes())
        .and_then(|()| stream.write_all(&body))
        .and_then(|()| stream.flush())
        .map_err(ClientError::Transport)?;

    let mut len_buf = [0u8; 4];
    stream
        .read_exact(&mut len_buf)
        .map_err(ClientError::Transport)?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > MAX_FRAME {
        return Err(ClientError::Transport(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "frame exceeds MAX_FRAME",
        )));
    }
    let mut answer = vec![0u8; len];
    stream
        .read_exact(&mut answer)
        .map_err(ClientError::Transport)?;
    serde_json::from_slice(&answer).map_err(|e| {
        ClientError::Transport(std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_daemon_that_is_not_there_is_not_an_empty_machine() {
        let dir = tempfile::tempdir().unwrap();
        let err = ask(&dir.path().join("absent.sock"), &Request::ListBottles).unwrap_err();
        assert!(
            matches!(err, ClientError::NotRunning(_)),
            "a failed connect must not read as a machine with no bottles"
        );
    }
}

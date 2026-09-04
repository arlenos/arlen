//! Asking the sentinel something, from a process that is not it.
//!
//! Blocking and one connection per call, the shape the Settings backend wants: it
//! runs each of these on a blocking thread and has no runtime of its own to
//! share.
//!
//! A daemon that is not running is a failed connect, and that is the answer. It
//! is never an all-clear: the page has a separate state for "nothing is
//! reporting" precisely because an unreachable sentinel and a machine with
//! nothing wrong look identical to anything that flattens them.

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
            ClientError::NotRunning(e) => write!(f, "the privacy sentinel is not running: {e}"),
            ClientError::Transport(e) => write!(f, "the privacy sentinel stopped answering: {e}"),
        }
    }
}

impl std::error::Error for ClientError {}

fn invalid(e: impl std::fmt::Display) -> ClientError {
    ClientError::Transport(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        e.to_string(),
    ))
}

/// Ask the daemon at `socket` one question.
pub fn ask(socket: &Path, request: &Request) -> Result<Response, ClientError> {
    let mut stream = UnixStream::connect(socket).map_err(ClientError::NotRunning)?;
    let body = serde_json::to_vec(request).map_err(invalid)?;
    stream
        .write_all(&(body.len() as u32).to_be_bytes())
        .and_then(|()| stream.write_all(&body))
        .and_then(|()| stream.flush())
        .map_err(ClientError::Transport)?;

    let mut len = [0u8; 4];
    stream.read_exact(&mut len).map_err(ClientError::Transport)?;
    let len = u32::from_be_bytes(len) as usize;
    if len > MAX_FRAME {
        return Err(invalid("the answer is larger than this protocol allows"));
    }
    let mut out = vec![0u8; len];
    stream.read_exact(&mut out).map_err(ClientError::Transport)?;
    serde_json::from_slice(&out).map_err(invalid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_daemon_that_is_not_there_is_not_an_all_clear() {
        let dir = tempfile::tempdir().unwrap();
        let e = ask(&dir.path().join("absent.sock"), &Request::GetState).unwrap_err();
        assert!(matches!(e, ClientError::NotRunning(_)));
        assert!(e.to_string().contains("not running"));
    }
}

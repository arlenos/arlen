//! Running a language server: the process half, kept behind a seam.
//!
//! [`crate::session`] decides what to say; this carries it. The split is not
//! ceremony - it means the protocol rules are tested without a server, and the
//! process rules are tested with a fake one, so neither test needs the other's
//! machinery.
//!
//! CONFINEMENT IS NOT SETTLED HERE, and this file must not be read as settling
//! it. A language server reads the whole project and, for Rust, runs build
//! scripts and proc macros to do it - that is arbitrary code from the tree you
//! opened. `arlen-run` exists for exactly this shape of problem, and wiring it in
//! is a decision about which paths a server may read and whether it may reach the
//! network for dependencies. Until that is decided, [`Server::spawn`] runs the
//! binary as the editor's own child, which is what every other editor does and
//! is honestly weaker than what Arlen intends to ship.

use std::io::{BufReader, Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use crate::wire::{self, FrameError, Incoming, Outgoing};

/// What can go wrong carrying messages.
#[derive(Debug)]
pub enum HostError {
    /// The server binary could not be started.
    Spawn(std::io::Error),
    /// The pipe broke - almost always the server exiting.
    Io(std::io::Error),
    /// A frame the reader refused.
    Frame(FrameError),
    /// A frame that was not the JSON we expect.
    Malformed(String),
    /// The server closed its output. Terminal: no further message is coming.
    Closed,
}

impl std::fmt::Display for HostError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Spawn(e) => write!(f, "could not start the language server: {e}"),
            Self::Io(e) => write!(f, "language server pipe: {e}"),
            Self::Frame(e) => write!(f, "{e}"),
            Self::Malformed(e) => write!(f, "the server sent something that is not a message: {e}"),
            Self::Closed => write!(f, "the language server closed its output"),
        }
    }
}

impl std::error::Error for HostError {}

/// A running language server and the pipes to it.
pub struct Server {
    child: Child,
    stdin: ChildStdin,
    out: BufReader<ChildStdout>,
    /// Bytes read but not yet a whole frame. A stream reader always has some.
    pending: Vec<u8>,
}

impl Server {
    /// Start `program` with `args`, speaking LSP over its stdio.
    ///
    /// stderr is inherited rather than piped: a server's stderr is where its
    /// panics and its "I cannot find a manifest" go, and a pipe nobody drains
    /// fills and blocks the process. Inheriting puts it in the editor's own log
    /// where a person can see it.
    pub fn spawn(program: &str, args: &[&str], cwd: &std::path::Path) -> Result<Self, HostError> {
        let mut child = Command::new(program)
            .args(args)
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(HostError::Spawn)?;
        let stdin = child.stdin.take().expect("stdin was piped");
        let stdout = child.stdout.take().expect("stdout was piped");
        Ok(Self {
            child,
            stdin,
            out: BufReader::new(stdout),
            pending: Vec::new(),
        })
    }

    /// Frame and write one message.
    pub fn send(&mut self, msg: &Outgoing) -> Result<(), HostError> {
        self.stdin.write_all(&msg.to_frame()).map_err(HostError::Io)?;
        self.stdin.flush().map_err(HostError::Io)
    }

    /// Block until one whole message arrives.
    ///
    /// Reads into a buffer and takes frames off the front, because a single read
    /// can return half a frame or three of them; treating a read as a message is
    /// the classic way to lose every message after the first burst.
    pub fn receive(&mut self) -> Result<Incoming, HostError> {
        loop {
            if let Some((body, used)) = wire::decode(&self.pending).map_err(HostError::Frame)? {
                self.pending.drain(..used);
                return serde_json::from_slice(&body)
                    .map_err(|e| HostError::Malformed(e.to_string()));
            }
            let mut chunk = [0u8; 8192];
            match self.out.read(&mut chunk) {
                Ok(0) => return Err(HostError::Closed),
                Ok(n) => self.pending.extend_from_slice(&chunk[..n]),
                Err(e) => return Err(HostError::Io(e)),
            }
        }
    }

    /// Stop the server, without waiting for a graceful shutdown to complete.
    ///
    /// The polite sequence is `shutdown` then `exit`, and the caller should try
    /// it. This is the backstop for a server that ignores both: an editor that
    /// leaves a rust-analyzer behind on every close is a memory leak the user
    /// experiences as their machine getting slower through the day.
    pub fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.kill();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stand-in server: `cat` speaks no LSP, but the framing is ours on both
    /// sides, so writing a frame and reading it back proves the transport
    /// without needing a language server installed.
    #[test]
    fn a_message_written_comes_back_off_the_pipe() {
        let mut s = Server::spawn("cat", &[], std::path::Path::new("/")).expect("cat exists");
        s.send(&Outgoing::request(1, "initialize", serde_json::json!({ "rootUri": "file:///x" })))
            .expect("write");
        let back = s.receive().expect("read");
        assert_eq!(back.method.as_deref(), Some("initialize"));
        assert_eq!(back.id.and_then(|v| v.as_i64()), Some(1));
    }

    /// Three messages in one burst must come back as three, not as one read.
    #[test]
    fn a_burst_is_taken_apart_into_its_messages() {
        let mut s = Server::spawn("cat", &[], std::path::Path::new("/")).expect("cat exists");
        for n in 1..=3 {
            s.send(&Outgoing::request(n, "ping", serde_json::json!({}))).expect("write");
        }
        for n in 1..=3 {
            let got = s.receive().expect("read");
            assert_eq!(got.id.and_then(|v| v.as_i64()), Some(n));
        }
    }

    /// A server that exits reports closure rather than blocking. An editor that
    /// hangs here looks frozen to its user.
    #[test]
    fn a_server_that_exits_is_reported_as_closed() {
        let mut s = Server::spawn("true", &[], std::path::Path::new("/")).expect("true exists");
        assert!(matches!(s.receive(), Err(HostError::Closed) | Err(HostError::Io(_))));
    }

    #[test]
    fn a_missing_binary_is_a_spawn_error_not_a_panic() {
        let e = Server::spawn("arlen-no-such-language-server", &[], std::path::Path::new("/"));
        assert!(matches!(e, Err(HostError::Spawn(_))));
    }
}

//! The consent control-socket CLIENT: the trusted-shell / dialog side of the
//! control transport (`system-dialog-plan.md`, the functional-base keystone).
//!
//! The broker serves one request per connection, so each call opens a fresh
//! connection, writes a length-prefixed [`ControlRequest`] and reads the framed
//! [`ControlReply`]. This is the transport the functional consent dialog drives:
//! poll [`ControlClient::fetch`] for the front pending request, render it, and
//! POST the user's decision with [`ControlClient::resolve`]; the "what you
//! allowed" surface uses [`ControlClient::list_grants`] / [`revoke_grant`].

use std::io::{self, ErrorKind};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use arlen_consent_contract::ConsentOutcome;

use crate::control::PendingView;
use crate::daemon::{ControlReply, ControlRequest};
use crate::grant::ConsentGrant;
use crate::socket::{read_frame, write_frame, MAX_FRAME};

/// The default control-socket path: `$XDG_RUNTIME_DIR/arlen/consent-control.sock`,
/// or `None` when `XDG_RUNTIME_DIR` is unset.
///
/// `None` rather than a `/run/arlen` fallback, because the broker's own
/// `runtime_dir` REFUSES to start without `XDG_RUNTIME_DIR` - it never binds
/// under `/run`. Dialling there could only ever fail, and the previous doc
/// claimed this mirrored the bind while doing the opposite. A client that cannot
/// name the socket should say the environment is missing, not produce a path
/// nothing is listening on and let the caller read the resulting "no such file"
/// as a broker that is down.
pub fn control_socket_path() -> Option<PathBuf> {
    control_socket_path_from(std::env::var_os("XDG_RUNTIME_DIR").as_deref())
}

/// [`control_socket_path`] over an explicit `XDG_RUNTIME_DIR`, so the precedence
/// is unit-tested without mutating the process environment - which is shared, and
/// would race whatever else is running.
fn control_socket_path_from(xdg: Option<&std::ffi::OsStr>) -> Option<PathBuf> {
    Some(PathBuf::from(xdg?).join("arlen").join("consent-control.sock"))
}

/// A client for the consent broker's control socket.
pub struct ControlClient {
    path: PathBuf,
}

impl ControlClient {
    /// A client targeting an explicit control-socket path.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// A client targeting the default control-socket path, or an error naming
    /// the missing environment. Same shape as `capsuled`'s control client, which
    /// faces the same question.
    pub fn at_default_path() -> io::Result<Self> {
        control_socket_path().map(Self::new).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "XDG_RUNTIME_DIR is unset; the consent broker's control socket has no path",
            )
        })
    }

    /// The control-socket path this client dials.
    pub fn path(&self) -> &Path {
        &self.path
    }

    fn call(&self, request: &ControlRequest) -> io::Result<ControlReply> {
        let mut stream = UnixStream::connect(&self.path)?;
        let bytes =
            serde_json::to_vec(request).map_err(|e| io::Error::new(ErrorKind::InvalidData, e))?;
        write_frame(&mut stream, &bytes)?;
        let frame = read_frame(&mut stream, MAX_FRAME)?.ok_or_else(|| {
            io::Error::new(ErrorKind::UnexpectedEof, "broker closed without a reply")
        })?;
        serde_json::from_slice(&frame).map_err(|e| io::Error::new(ErrorKind::InvalidData, e))
    }

    /// Fetch the front pending consent request to render, or `None` when the queue
    /// is empty.
    pub fn fetch(&self) -> io::Result<Option<PendingView>> {
        match self.call(&ControlRequest::Fetch)? {
            ControlReply::Pending { view } => Ok(view),
            other => Err(unexpected(&other)),
        }
    }

    /// Submit the user's decision for a pending request; `Ok(false)` if the id was
    /// unknown or already resolved.
    pub fn resolve(&self, id: u64, outcome: ConsentOutcome) -> io::Result<bool> {
        match self.call(&ControlRequest::Resolve { id, outcome })? {
            ControlReply::Resolved { ok } => Ok(ok),
            other => Err(unexpected(&other)),
        }
    }

    /// List the remembered grants (the "what you allowed" surface).
    pub fn list_grants(&self) -> io::Result<Vec<ConsentGrant>> {
        match self.call(&ControlRequest::ListGrants)? {
            ControlReply::Grants { grants } => Ok(grants),
            other => Err(unexpected(&other)),
        }
    }

    /// Revoke a remembered grant by its revocation handle; `Ok(false)` if the
    /// handle was unknown or already revoked.
    pub fn revoke_grant(&self, handle: &str) -> io::Result<bool> {
        match self.call(&ControlRequest::RevokeGrant {
            handle: handle.to_string(),
        })? {
            ControlReply::Revoked { ok } => Ok(ok),
            other => Err(unexpected(&other)),
        }
    }
}

fn unexpected(reply: &ControlReply) -> io::Error {
    io::Error::new(
        ErrorKind::InvalidData,
        format!("unexpected reply for the request: {reply:?}"),
    )
}

#[cfg(test)]
mod tests {

    #[test]
    fn the_default_path_is_absent_without_a_runtime_dir() {
        // The broker refuses to start without XDG_RUNTIME_DIR, so there is no
        // path to dial - and inventing `/run/arlen` would turn "your session has
        // no runtime dir" into "the broker is not running", which sends whoever
        // reads the error to the wrong place entirely.
        assert_eq!(
            control_socket_path_from(Some(std::ffi::OsStr::new("/run/user/1000"))),
            Some(std::path::PathBuf::from("/run/user/1000/arlen/consent-control.sock"))
        );
        assert_eq!(control_socket_path_from(None), None);
    }
    use super::*;
    use std::os::unix::net::UnixListener;
    use std::thread;

    /// A one-shot mock broker: accept one connection, read the request frame, and
    /// write `reply` back. Returns the request the client sent.
    fn mock_once(path: PathBuf, reply: ControlReply) -> thread::JoinHandle<ControlRequest> {
        let listener = UnixListener::bind(&path).unwrap();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let frame = read_frame(&mut stream, MAX_FRAME).unwrap().unwrap();
            let request: ControlRequest = serde_json::from_slice(&frame).unwrap();
            let bytes = serde_json::to_vec(&reply).unwrap();
            write_frame(&mut stream, &bytes).unwrap();
            request
        })
    }

    #[test]
    fn resolve_round_trips_the_request_and_reply() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("control.sock");
        let server = mock_once(path.clone(), ControlReply::Resolved { ok: true });
        let client = ControlClient::new(&path);
        let ok = client.resolve(42, ConsentOutcome::AllowedOnce).unwrap();
        assert!(ok);
        // The server received exactly the Resolve the client sent.
        match server.join().unwrap() {
            ControlRequest::Resolve { id, outcome } => {
                assert_eq!(id, 42);
                assert_eq!(outcome, ConsentOutcome::AllowedOnce);
            }
            other => panic!("expected Resolve, got {other:?}"),
        }
    }

    #[test]
    fn fetch_parses_an_empty_queue_as_none() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("control.sock");
        let server = mock_once(path.clone(), ControlReply::Pending { view: None });
        let client = ControlClient::new(&path);
        assert!(client.fetch().unwrap().is_none());
        assert!(matches!(server.join().unwrap(), ControlRequest::Fetch));
    }

    #[test]
    fn a_mismatched_reply_is_an_error_not_a_wrong_value() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("control.sock");
        // The broker answers a Fetch with a Resolved reply (a protocol violation).
        let _server = mock_once(path.clone(), ControlReply::Resolved { ok: true });
        let client = ControlClient::new(&path);
        assert!(client.fetch().is_err());
    }
}

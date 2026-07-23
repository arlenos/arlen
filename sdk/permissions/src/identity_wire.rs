//! The identity broker's wire contract + the sync client.
//!
//! The register/lookup request and reply types are a SHARED contract:
//! the config-broker daemon dispatches them, and callers (the launcher
//! `arlen-run` registering a child, a daemon's resolver looking up its
//! peer) speak them. They live here in the low-level permissions crate,
//! not the daemon, so a daemon can `lookup` via
//! [`crate::identity_wire::lookup_identity`] without depending on the
//! broker daemon crate (the wrong dependency direction). The daemon
//! re-exports these types for its dispatch.
//!
//! The client is SYNCHRONOUS: the resolver that consumes it
//! ([`crate::peer_pidfd`]-based `app_id_from_connection`, a later slice)
//! is sync, and a register/lookup is one short local round trip. The
//! pidfd travels over `SCM_RIGHTS` ([`crate::fd_passing`]); the framed
//! reply mirrors the broker's 4-byte-big-endian length prefix.

use std::io::{Read, Write};
use std::os::fd::{AsRawFd, BorrowedFd};
use std::os::unix::net::UnixStream;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::fd_passing::send_fd_msg;

/// The largest identity reply frame accepted before allocating. Replies
/// are a discriminant plus a short app_id, so this is generous.
pub const MAX_IDENTITY_FRAME: usize = 4096;

/// A request to the identity broker. The pidfd travels out of band (over
/// `SCM_RIGHTS`), so it is not a field here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum IdentityRequest {
    /// Stamp `app_id` onto the process the accompanying pidfd pins.
    /// Launcher-only (the daemon's registrar allowlist).
    Register {
        /// The launcher-attested app id for the child.
        app_id: String,
    },
    /// Resolve the app_id of the process the accompanying pidfd pins.
    Lookup,
}

/// The identity broker's reply.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum IdentityResponse {
    /// A `Register` was accepted (the pidfd is now held + stamped).
    Registered,
    /// A `Lookup` matched a live record.
    Resolved {
        /// The launcher-stamped app id of the looked-up process.
        app_id: String,
    },
    /// A `Lookup` found no live record for the presented pidfd.
    NotFound,
    /// A `Register` from a non-launcher caller; nothing was stamped.
    Refused(String),
    /// The op could not be honoured (a dead/absent pidfd, a corrupt
    /// request). The caller must NOT proceed on a guessed identity.
    Error(String),
}

/// A failure talking to the identity broker. Every variant means the
/// caller must NOT proceed on a fabricated identity.
#[derive(Debug, thiserror::Error)]
pub enum IdentityClientError {
    /// Connect/read/write/framing failed (broker down, socket gone).
    #[error("identity-broker transport: {0}")]
    Transport(String),
    /// The broker refused the op (a non-launcher tried to register).
    #[error("identity-broker refused: {0}")]
    Refused(String),
    /// The broker reported an op error (a dead pidfd, a corrupt request).
    #[error("identity-broker error: {0}")]
    Broker(String),
    /// The broker returned a reply of the wrong shape for the request.
    #[error("identity-broker unexpected reply")]
    Unexpected,
}

/// Send one request + its pidfd over a connected stream, then read the
/// framed reply. Shared by [`register_over`] and [`lookup_over`].
fn exchange<S: AsRawFd + Read>(
    stream: &mut S,
    request: &IdentityRequest,
    pidfd: BorrowedFd<'_>,
) -> Result<IdentityResponse, IdentityClientError> {
    let bytes = serde_json::to_vec(request)
        .map_err(|e| IdentityClientError::Transport(e.to_string()))?;
    send_fd_msg(stream, &bytes, pidfd).map_err(|e| IdentityClientError::Transport(e.to_string()))?;
    read_response(stream)
}

/// Read one 4-byte-big-endian length-prefixed JSON reply.
fn read_response<R: Read>(reader: &mut R) -> Result<IdentityResponse, IdentityClientError> {
    let mut len = [0u8; 4];
    reader
        .read_exact(&mut len)
        .map_err(|e| IdentityClientError::Transport(e.to_string()))?;
    let n = u32::from_be_bytes(len) as usize;
    if n > MAX_IDENTITY_FRAME {
        return Err(IdentityClientError::Transport("reply exceeds MAX_IDENTITY_FRAME".into()));
    }
    let mut body = vec![0u8; n];
    reader
        .read_exact(&mut body)
        .map_err(|e| IdentityClientError::Transport(e.to_string()))?;
    serde_json::from_slice(&body).map_err(|e| IdentityClientError::Transport(e.to_string()))
}

/// Register `app_id` for the process `child_pidfd` pins, over a connected
/// stream. Launcher-only at the broker; a non-launcher caller gets
/// [`IdentityClientError::Refused`].
pub fn register_over<S: AsRawFd + Read>(
    stream: &mut S,
    child_pidfd: BorrowedFd<'_>,
    app_id: &str,
) -> Result<(), IdentityClientError> {
    let request = IdentityRequest::Register {
        app_id: app_id.to_string(),
    };
    match exchange(stream, &request, child_pidfd)? {
        IdentityResponse::Registered => Ok(()),
        IdentityResponse::Refused(r) => Err(IdentityClientError::Refused(r)),
        IdentityResponse::Error(e) => Err(IdentityClientError::Broker(e)),
        _ => Err(IdentityClientError::Unexpected),
    }
}

/// Look up the stamped app_id of the process `peer_pidfd` pins, over a
/// connected stream. `Ok(None)` when no live record matches (the caller
/// falls to a weaker tier or denies).
pub fn lookup_over<S: AsRawFd + Read>(
    stream: &mut S,
    peer_pidfd: BorrowedFd<'_>,
) -> Result<Option<String>, IdentityClientError> {
    match exchange(stream, &IdentityRequest::Lookup, peer_pidfd)? {
        IdentityResponse::Resolved { app_id } => Ok(Some(app_id)),
        IdentityResponse::NotFound => Ok(None),
        IdentityResponse::Error(e) => Err(IdentityClientError::Broker(e)),
        IdentityResponse::Refused(r) => Err(IdentityClientError::Refused(r)),
        _ => Err(IdentityClientError::Unexpected),
    }
}

/// Connect to the broker at `socket` and register `app_id` for the child
/// `child_pidfd` pins. Used by the launcher at spawn.
pub fn register_identity(
    socket: &Path,
    child_pidfd: BorrowedFd<'_>,
    app_id: &str,
) -> Result<(), IdentityClientError> {
    let mut stream = connect(socket)?;
    register_over(&mut stream, child_pidfd, app_id)
}

/// Connect to the broker at `socket` and look up the process `peer_pidfd`
/// pins. Used by a daemon's resolver at accept.
pub fn lookup_identity(
    socket: &Path,
    peer_pidfd: BorrowedFd<'_>,
) -> Result<Option<String>, IdentityClientError> {
    let mut stream = connect(socket)?;
    lookup_over(&mut stream, peer_pidfd)
}

fn connect(socket: &Path) -> Result<UnixStream, IdentityClientError> {
    UnixStream::connect(socket)
        .map_err(|e| IdentityClientError::Transport(format!("connect {}: {e}", socket.display())))
}

/// Write a framed [`IdentityResponse`] the sync way (the test-server
/// side; the daemon writes its replies via its own async framer). Kept
/// here so a sync consumer/test can round-trip the exact wire the client
/// reads.
pub fn write_response<W: Write>(writer: &mut W, response: &IdentityResponse) -> std::io::Result<()> {
    let body = serde_json::to_vec(response)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    writer.write_all(&(body.len() as u32).to_be_bytes())?;
    writer.write_all(&body)?;
    writer.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fd_passing::recv_fd_msg;
    use std::os::fd::{AsFd, FromRawFd, OwnedFd};
    use std::thread;

    fn self_pidfd() -> OwnedFd {
        // SAFETY: pidfd_open(getpid()) returns a fresh owned fd (self alive).
        let raw = unsafe { libc::syscall(libc::SYS_pidfd_open, libc::getpid() as libc::pid_t, 0) };
        assert!(raw >= 0);
        // SAFETY: fresh owned fd from the kernel.
        unsafe { OwnedFd::from_raw_fd(raw as libc::c_int) }
    }

    /// lookup_over sends the request + pidfd and parses a Resolved reply.
    /// A canned test server receives the fd (proving the SCM_RIGHTS wire)
    /// and replies with the framed response the real daemon would.
    #[test]
    fn lookup_over_round_trips_a_resolved_reply() {
        let (mut client, server) = UnixStream::pair().unwrap();
        let srv = thread::spawn(move || {
            let (bytes, fd) = recv_fd_msg(&server, MAX_IDENTITY_FRAME).unwrap();
            // The request is a Lookup and a real pidfd arrived.
            assert_eq!(
                serde_json::from_slice::<IdentityRequest>(&bytes).unwrap(),
                IdentityRequest::Lookup
            );
            assert!(fd.is_some(), "the peer pidfd must arrive over SCM_RIGHTS");
            let mut server = server;
            write_response(
                &mut server,
                &IdentityResponse::Resolved {
                    app_id: "com.example.app".into(),
                },
            )
            .unwrap();
        });

        let p = self_pidfd();
        let got = lookup_over(&mut client, p.as_fd()).unwrap();
        assert_eq!(got, Some("com.example.app".to_string()));
        srv.join().unwrap();
    }

    /// A NotFound reply maps to `Ok(None)`, not an error or a fake id.
    #[test]
    fn lookup_over_maps_not_found_to_none() {
        let (mut client, server) = UnixStream::pair().unwrap();
        let srv = thread::spawn(move || {
            let (_bytes, _fd) = recv_fd_msg(&server, MAX_IDENTITY_FRAME).unwrap();
            let mut server = server;
            write_response(&mut server, &IdentityResponse::NotFound).unwrap();
        });
        let p = self_pidfd();
        let got = lookup_over(&mut client, p.as_fd()).unwrap();
        assert_eq!(got, None);
        srv.join().unwrap();
    }

    /// register_over maps a Refused reply (a non-launcher caller) to the
    /// Refused error variant.
    #[test]
    fn register_over_maps_refused() {
        let (mut client, server) = UnixStream::pair().unwrap();
        let srv = thread::spawn(move || {
            let (_bytes, _fd) = recv_fd_msg(&server, MAX_IDENTITY_FRAME).unwrap();
            let mut server = server;
            write_response(&mut server, &IdentityResponse::Refused("nope".into())).unwrap();
        });
        let p = self_pidfd();
        let err = register_over(&mut client, p.as_fd(), "com.x").unwrap_err();
        assert!(matches!(err, IdentityClientError::Refused(_)));
        srv.join().unwrap();
    }
}

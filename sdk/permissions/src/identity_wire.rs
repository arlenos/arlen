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
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::fd_passing::send_fd_msg;

/// The environment override for the identity-broker socket. Shared with the
/// config-broker daemon's bind path, so setting it moves BOTH the daemon's bind
/// and every client's connect to the same place (the dev/test seam).
pub const IDENTITY_SOCKET_ENV: &str = "ARLEN_CONFIG_BROKER_IDENTITY_SOCKET";

/// The identity-broker socket file name under the arlen runtime dir. The
/// config-broker daemon binds it; the launcher (`arlen-run` registering a child)
/// and a daemon's resolver (looking up its peer) connect to it. One shared name
/// so producer, consumer and daemon can never drift onto different sockets.
pub const IDENTITY_SOCKET_NAME: &str = "config-broker-identity.sock";

/// Where a CLIENT connects to the identity broker.
///
/// The env override ([`IDENTITY_SOCKET_ENV`]) wins. Otherwise prefer whichever
/// socket actually EXISTS - the per-user path (`$XDG_RUNTIME_DIR/arlen/<name>`)
/// first, then the system path (`/run/arlen/<name>`) the separate-uid broker
/// binds (it cannot write the session user's 0700 runtime dir) - falling back to
/// the per-user path so an error names the expected location. Mirrors the
/// master-switch `connect_path`: existence is the right test for connecting,
/// since the dev (per-user) and separate-uid (system) deployments put the socket
/// in different places and a client should not have to know which is running.
pub fn identity_broker_connect_path() -> PathBuf {
    if let Some(p) = std::env::var_os(IDENTITY_SOCKET_ENV) {
        return PathBuf::from(p);
    }
    let per_user = per_user_identity_socket();
    let system = PathBuf::from("/run/arlen").join(IDENTITY_SOCKET_NAME);
    resolve_connect_path(per_user, system, |p| p.exists())
}

/// The per-user identity-broker socket (`$XDG_RUNTIME_DIR/arlen/<name>`, else
/// `/run/arlen/<name>` when the runtime dir is unset).
fn per_user_identity_socket() -> PathBuf {
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/run"));
    base.join("arlen").join(IDENTITY_SOCKET_NAME)
}

/// Pure prefer-existing resolution: `per_user` if it exists, else `system` if it
/// exists, else `per_user` (so an error names the expected per-user location).
/// `exists` is injected so the branch logic is unit-testable without the
/// filesystem.
fn resolve_connect_path(
    per_user: PathBuf,
    system: PathBuf,
    exists: impl Fn(&Path) -> bool,
) -> PathBuf {
    if exists(&per_user) {
        return per_user;
    }
    if exists(&system) {
        return system;
    }
    per_user
}

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
    /// The process listening at the broker socket is NOT the trusted broker
    /// (its `SO_PEERCRED` uid did not match the expected service uid). A
    /// same-uid squatter at the session-owned per-user socket path is the
    /// threat this closes: the reply is not trusted and the caller must fall
    /// through to a weaker tier, never accept a stamp from it.
    #[error("identity-broker not authenticated: {0}")]
    Unauthenticated(String),
}

/// The environment variable naming the identity broker's expected service uid.
/// The separate-uid deployment sets it (systemd unit / dev stack) to the uid the
/// broker runs as, so a daemon can reject a squatter at the session-owned socket.
pub const IDENTITY_BROKER_UID_ENV: &str = "ARLEN_CONFIG_BROKER_IDENTITY_UID";

/// The uid a client should require the identity broker to run as.
///
/// [`IDENTITY_BROKER_UID_ENV`] wins when set to a valid uid (the separate-uid
/// production deployment: the broker runs as its own service uid). Otherwise the
/// caller's own uid (the dev single-uid deployment, where the broker is same-uid).
/// FAIL-SAFE: in a separate-uid deployment that FORGETS to set the env, the
/// expected uid falls back to the caller's own, which will NOT match the broker's
/// service uid, so authentication fails and the resolver falls through to `/proc`
/// (stamping is simply OFF) rather than trusting an unauthenticated broker.
pub fn broker_expected_uid(caller_uid: u32) -> u32 {
    std::env::var(IDENTITY_BROKER_UID_ENV)
        .ok()
        .and_then(|v| v.trim().parse::<u32>().ok())
        .unwrap_or(caller_uid)
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

/// Like [`lookup_identity`], but AUTHENTICATE the broker first: after connecting,
/// read the listener's `SO_PEERCRED` uid and require it to equal
/// `expected_broker_uid` (see [`broker_expected_uid`]). A mismatch is an
/// [`IdentityClientError::Unauthenticated`] and NO request is sent (the peer
/// pidfd is never handed to an untrusted listener), so a same-uid squatter at the
/// session-owned socket path cannot mint a stamp - the resolver treats the error
/// as fall-through. This is the client-side half of the trust boundary the broker
/// already enforces the other way (it authenticates its callers).
pub fn lookup_identity_authenticated(
    socket: &Path,
    peer_pidfd: BorrowedFd<'_>,
    expected_broker_uid: u32,
) -> Result<Option<String>, IdentityClientError> {
    let mut stream = connect(socket)?;
    let broker_uid = crate::peer_pidfd::peer_uid(stream.as_raw_fd())
        .map_err(|e| IdentityClientError::Unauthenticated(format!("peer uid unreadable: {e}")))?;
    if broker_uid != expected_broker_uid {
        return Err(IdentityClientError::Unauthenticated(format!(
            "broker uid {broker_uid} != expected {expected_broker_uid}"
        )));
    }
    lookup_over(&mut stream, peer_pidfd)
}

/// Bound on a single identity round trip. A register/lookup is one short local
/// exchange (sub-millisecond), so this only fires if the broker accepted the
/// connection but stalled without replying. It caps BOTH ops: the resolver on
/// the hot admission path must not hang on a wedged broker (a timed-out lookup
/// becomes a transport error the resolver treats as fall-through), and the
/// launcher must not hang registering a child.
const IDENTITY_IO_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

fn connect(socket: &Path) -> Result<UnixStream, IdentityClientError> {
    let stream = UnixStream::connect(socket)
        .map_err(|e| IdentityClientError::Transport(format!("connect {}: {e}", socket.display())))?;
    // Best-effort: a failed setsockopt (effectively never on a live Unix socket)
    // must not abort an otherwise-valid connection; the round trip is still
    // bounded by the broker's own read timeout on its side.
    let _ = stream.set_read_timeout(Some(IDENTITY_IO_TIMEOUT));
    let _ = stream.set_write_timeout(Some(IDENTITY_IO_TIMEOUT));
    Ok(stream)
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

    /// The connect resolver prefers the per-user socket when it exists, falls to
    /// the system socket when only that exists, and defaults to the per-user path
    /// when neither exists (so an error names the expected per-user location).
    #[test]
    fn connect_path_prefers_the_existing_socket() {
        let per_user = PathBuf::from("/run/user/1000/arlen/config-broker-identity.sock");
        let system = PathBuf::from("/run/arlen/config-broker-identity.sock");

        // Per-user present -> per-user, even if the system one also exists.
        let got = resolve_connect_path(per_user.clone(), system.clone(), |_| true);
        assert_eq!(got, per_user);

        // Only the system socket exists -> the system path.
        let got = resolve_connect_path(per_user.clone(), system.clone(), |p| p == system);
        assert_eq!(got, system);

        // Neither exists -> the per-user path (names the expected location).
        let got = resolve_connect_path(per_user.clone(), system.clone(), |_| false);
        assert_eq!(got, per_user);
    }

    /// The shared name + env consts are the exact strings the config-broker binds
    /// against; a rename here without updating the daemon would silently split
    /// producer and consumer onto different sockets, so pin them.
    #[test]
    fn identity_socket_contract_strings_are_pinned() {
        assert_eq!(IDENTITY_SOCKET_NAME, "config-broker-identity.sock");
        assert_eq!(IDENTITY_SOCKET_ENV, "ARLEN_CONFIG_BROKER_IDENTITY_SOCKET");
    }

    /// An authenticated lookup ACCEPTS a broker whose SO_PEERCRED uid matches the
    /// expected uid, and returns its reply. The in-process listener is our own uid,
    /// so expecting our uid authenticates it.
    #[test]
    fn authenticated_lookup_accepts_a_matching_broker_uid() {
        use crate::identity_wire::write_response;
        use std::os::unix::net::UnixListener;
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("id.sock");
        let listener = UnixListener::bind(&sock).unwrap();
        let srv = thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            let (_bytes, fd) = recv_fd_msg(&conn, MAX_IDENTITY_FRAME).unwrap();
            assert!(fd.is_some());
            write_response(
                &mut conn,
                &IdentityResponse::Resolved {
                    app_id: "com.example.app".into(),
                },
            )
            .unwrap();
        });
        // SAFETY: getuid never fails.
        let me = unsafe { libc::getuid() };
        let p = self_pidfd();
        let got = lookup_identity_authenticated(&sock, p.as_fd(), me).unwrap();
        assert_eq!(got, Some("com.example.app".to_string()));
        srv.join().unwrap();
    }

    /// An authenticated lookup REJECTS a broker whose uid does not match the
    /// expected uid, with `Unauthenticated`, and sends NO request (the listener
    /// only accepts; it never receives a Lookup). This is the same-uid-squatter
    /// defense: in the separate-uid deployment the expected uid is the broker's
    /// service uid, so a session-uid squatter is rejected.
    #[test]
    fn authenticated_lookup_rejects_a_mismatched_broker_uid() {
        use std::os::unix::net::UnixListener;
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("id.sock");
        let listener = UnixListener::bind(&sock).unwrap();
        let srv = thread::spawn(move || {
            // Accept so the client's connect succeeds, then drop: a mismatched uid
            // means the client bails BEFORE sending, so we must not block on recv.
            let _ = listener.accept();
        });
        // SAFETY: getuid never fails.
        let wrong = unsafe { libc::getuid() }.wrapping_add(1);
        let p = self_pidfd();
        match lookup_identity_authenticated(&sock, p.as_fd(), wrong) {
            Err(IdentityClientError::Unauthenticated(_)) => {}
            other => panic!("expected Unauthenticated, got {other:?}"),
        }
        srv.join().unwrap();
    }

    /// The expected broker uid defaults to the caller's own uid when the env is
    /// unset (the dev single-uid deployment). Guarded so a set env does not fail it.
    #[test]
    fn broker_expected_uid_defaults_to_caller() {
        if std::env::var_os(IDENTITY_BROKER_UID_ENV).is_none() {
            assert_eq!(broker_expected_uid(4321), 4321);
        }
    }
}

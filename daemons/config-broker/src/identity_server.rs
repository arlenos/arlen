//! The identity broker's socket handler (the SCM_RIGHTS wire).
//!
//! Bridges the pure [`handle_identity`](crate::identity_op::handle_identity)
//! dispatch onto the daemon's async runtime. Each connection is a single
//! one-shot exchange: the caller sends one [`IdentityRequest`] JSON with a
//! pidfd attached over `SCM_RIGHTS`, the broker dispatches it against the
//! shared [`IdentityStore`], and writes back one framed
//! [`IdentityResponse`]. Fail-closed: a peer-auth failure, an unresolved
//! caller app id, a malformed request, or a missing pidfd all end the
//! connection without stamping or resolving anything.

use std::path::Path;
use std::sync::{Arc, Mutex};

use tokio::io::Interest;
use tokio::net::{UnixListener, UnixStream};

use arlen_permissions::fd_passing::{recv_fd_msg, MAX_FD_MSG};
use arlen_permissions::identity::app_id_from_pid;
use arlen_permissions::identity_store::IdentityStore;
use arlen_permissions::peer_pidfd::PeerPidfd;

use crate::identity_op::{handle_identity, IdentityRequest, IdentityResponse};
use crate::protocol::write_frame_async;

/// Receive one payload + optional pidfd over a non-blocking tokio
/// `UnixStream`, driving the synchronous `recvmsg`-based
/// [`recv_fd_msg`] through tokio's readiness so the accept loop is not
/// blocked. Retries on `WouldBlock` after re-awaiting readability.
async fn recv_fd_msg_async(
    stream: &UnixStream,
    max: usize,
) -> std::io::Result<(Vec<u8>, Option<std::os::fd::OwnedFd>)> {
    loop {
        stream.readable().await?;
        match stream.try_io(Interest::READABLE, || recv_fd_msg(stream, max)) {
            Ok(v) => return Ok(v),
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
            Err(e) => return Err(e),
        }
    }
}

/// Serve one identity-broker connection: authenticate the peer
/// (`SO_PEERPIDFD` + uid), resolve the CALLER's app id (the registrar
/// gate keys on it), receive the request + its attached pidfd, dispatch,
/// and reply. One request per connection; returns after the reply.
///
/// Every failure path drops the connection without acting: a mid-check
/// disconnect never stamps or resolves an identity.
pub async fn serve_identity_connection(
    stream: UnixStream,
    store: Arc<Mutex<IdentityStore>>,
    caller_uid: u32,
) {
    let peer = match PeerPidfd::from_socket(&stream, caller_uid) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("identity: peer auth refused: {e}");
            return;
        }
    };

    let (bytes, fd) = match recv_fd_msg_async(&stream, MAX_FD_MSG).await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("identity: request read failed: {e}");
            return;
        }
    };
    let request: IdentityRequest = match serde_json::from_slice(&bytes) {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("identity: malformed request: {e}");
            let _ = reply(stream, IdentityResponse::Error("malformed request".into())).await;
            return;
        }
    };

    // Register is registrar-gated, so it resolves the CALLER's app id (the
    // one place the broker still reads /proc/exe, Tier-2, for the launcher
    // only); a resolution failure fail-closed refuses the register. Lookup
    // deliberately SKIPS caller resolution - it ignores the caller entirely
    // (handle_identity's Lookup arm never reads it), so a hardened or
    // cross-uid peer the broker cannot readlink can still resolve its own
    // peer. This is the fix for the broker re-importing the /proc/exe
    // fragility onto the open lookup path.
    let response = if matches!(request, IdentityRequest::Register { .. }) {
        match app_id_from_pid(peer.pid()) {
            Ok(caller) => handle_identity(&store, &caller, request, fd),
            Err(e) => {
                tracing::warn!("identity: registrar app-id unresolved, refusing register: {e}");
                IdentityResponse::Refused("registrar identity unresolved".into())
            }
        }
    } else {
        // Lookup ignores the caller identity; the empty id is inert here.
        handle_identity(&store, "", request, fd)
    };
    let _ = reply(stream, response).await;
}

/// Write one framed response, then let the connection close.
async fn reply(mut stream: UnixStream, response: IdentityResponse) -> std::io::Result<()> {
    write_frame_async(&mut stream, &response).await
}

/// Accept identity-broker connections, serving each as a one-shot
/// register/lookup exchange against the shared [`IdentityStore`]. The
/// store is `Arc<Mutex<..>>` because it holds OS resources (the
/// registered pidfds) mutated across connections. `caller_uid` is the uid
/// the broker accepts as legitimate (the master-switch socket's
/// [`crate::server::owner_uid`]); a peer of a different uid is rejected.
pub async fn run(
    store: Arc<Mutex<IdentityStore>>,
    socket: &Path,
    caller_uid: u32,
) -> std::io::Result<()> {
    let listener = bind_identity_socket(socket)?;
    tracing::info!(
        socket = %socket.display(),
        owner_uid = caller_uid,
        "config-broker identity listening"
    );
    loop {
        let (stream, _) = listener.accept().await?;
        let store = Arc::clone(&store);
        tokio::spawn(async move {
            serve_identity_connection(stream, store, caller_uid).await;
        });
    }
}

/// Bind the identity socket, reusing the master-switch socket's
/// stale-probe + 0666 policy (a live server is not clobbered; the peer
/// credential, not the socket mode, is the access boundary).
fn bind_identity_socket(socket: &Path) -> std::io::Result<UnixListener> {
    crate::server::bind_socket(socket)
}

#[cfg(test)]
mod tests {
    use super::*;
    use arlen_permissions::fd_passing::send_fd_msg;
    use std::os::fd::{AsFd, FromRawFd, OwnedFd};

    /// A pidfd to this very process.
    fn self_pidfd() -> OwnedFd {
        // SAFETY: pidfd_open(getpid()) returns a fresh owned fd (self is alive).
        let raw = unsafe { libc::syscall(libc::SYS_pidfd_open, libc::getpid() as libc::pid_t, 0) };
        assert!(raw >= 0);
        // SAFETY: fresh owned fd from the kernel.
        unsafe { OwnedFd::from_raw_fd(raw as libc::c_int) }
    }

    /// End-to-end over a real socketpair with real SCM_RIGHTS: a process
    /// pre-stamped in the store resolves when a client presents a pidfd to
    /// it over the wire. Proves recv_fd_msg_async + dispatch + framed reply.
    #[tokio::test]
    async fn lookup_over_the_wire_resolves_a_stamped_process() {
        // Pre-stamp self.
        let store = Arc::new(Mutex::new(IdentityStore::new()));
        store
            .lock()
            .unwrap()
            .register(self_pidfd(), "com.example.self".into())
            .unwrap();

        let (client, server) = UnixStream::pair().unwrap();
        // SAFETY: getuid never fails.
        let uid = unsafe { libc::getuid() };
        let store2 = store.clone();
        let handler = tokio::spawn(async move {
            serve_identity_connection(server, store2, uid).await;
        });

        // Client sends a Lookup + a pidfd to self over SCM_RIGHTS.
        let req = serde_json::to_vec(&IdentityRequest::Lookup).unwrap();
        let presented = self_pidfd();
        send_fd_msg(&client, &req, presented.as_fd()).unwrap();

        // Read the framed reply.
        let resp: IdentityResponse = crate::protocol::read_frame_async(&mut { client })
            .await
            .unwrap();
        assert_eq!(
            resp,
            IdentityResponse::Resolved {
                app_id: "com.example.self".into()
            }
        );
        handler.await.unwrap();
    }

    /// A malformed request gets an Error reply, not a fabricated identity.
    #[tokio::test]
    async fn a_malformed_request_gets_an_error() {
        let store = Arc::new(Mutex::new(IdentityStore::new()));
        let (client, server) = UnixStream::pair().unwrap();
        // SAFETY: getuid never fails.
        let uid = unsafe { libc::getuid() };
        let handler = tokio::spawn(async move {
            serve_identity_connection(server, store, uid).await;
        });

        // Garbage payload + a pidfd (so the read succeeds, the parse fails).
        let presented = self_pidfd();
        send_fd_msg(&client, b"not json", presented.as_fd()).unwrap();

        let resp: IdentityResponse = crate::protocol::read_frame_async(&mut { client })
            .await
            .unwrap();
        assert!(matches!(resp, IdentityResponse::Error(_)));
        handler.await.unwrap();
    }
}

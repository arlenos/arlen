//! The broker's Unix-socket server.
//!
//! Per connection: authenticate the peer via `SO_PEERPIDFD` + uid
//! ([`arlen_permissions::peer_pidfd`]), resolve its app id from the
//! pinned pid, then field [`Request`]s through the pure
//! [`handle_request`] dispatch until the peer closes or dies. Auth
//! failure drops the connection silently - a credential lookup that
//! did not cleanly succeed never serves.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};

use arlen_permissions::identity::app_id_from_pid;
use arlen_permissions::peer_pidfd::PeerPidfd;

use crate::protocol::{handle_request, Request, Response, MAX_FRAME};
use crate::state::StateStore;

/// The broker socket path: the `ARLEN_CONFIG_BROKER_SOCKET` override,
/// else `$XDG_RUNTIME_DIR/arlen/config-broker.sock`, else
/// `/run/arlen/config-broker.sock`.
pub fn socket_path() -> PathBuf {
    if let Some(p) = std::env::var_os("ARLEN_CONFIG_BROKER_SOCKET") {
        return PathBuf::from(p);
    }
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/run"));
    base.join("arlen").join("config-broker.sock")
}

/// The uid the broker process runs as (the deployment's dedicated
/// config uid; in dev, the developer's uid).
pub fn current_uid() -> u32 {
    // SAFETY: getuid never fails.
    unsafe { libc::getuid() }
}

/// The uid the broker ACCEPTS as the legitimate caller. In the
/// separate-uid deployment the broker runs as a distinct service uid
/// while the legitimate callers (Settings, the AI daemon/agent) run
/// as the session user, so the expected peer uid is the session
/// user's, NOT the broker's - `ARLEN_CONFIG_BROKER_OWNER_UID` carries
/// it (set by the systemd unit). With no override it falls back to
/// the broker's own uid, the correct dev/single-uid behaviour. A peer
/// whose uid differs is rejected by [`PeerPidfd::from_socket`].
pub fn owner_uid() -> u32 {
    std::env::var("ARLEN_CONFIG_BROKER_OWNER_UID")
        .ok()
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or_else(current_uid)
}

/// Bind the broker socket 0600 after a stale-socket probe: a path a
/// live process still serves is not clobbered (a singleton guard); a
/// dead leftover is cleared first. Mirrors the audit daemon.
pub fn bind_socket(path: &Path) -> std::io::Result<UnixListener> {
    use std::os::unix::fs::PermissionsExt;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if path.exists() {
        match std::os::unix::net::UnixStream::connect(path) {
            Ok(_) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::AddrInUse,
                    format!("{} is already served by a live process", path.display()),
                ));
            }
            Err(_) => {
                let _ = std::fs::remove_file(path);
            }
        }
    }
    let listener = UnixListener::bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}

/// Read one length-prefixed JSON request; refuse an oversized
/// declared length before allocating.
async fn read_request(stream: &mut UnixStream) -> std::io::Result<Request> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > MAX_FRAME {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "frame exceeds MAX_FRAME",
        ));
    }
    let mut body = vec![0u8; len];
    stream.read_exact(&mut body).await?;
    serde_json::from_slice(&body)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

/// Write one length-prefixed JSON response.
async fn write_response(stream: &mut UnixStream, resp: &Response) -> std::io::Result<()> {
    let body = serde_json::to_vec(resp)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    stream
        .write_all(&(body.len() as u32).to_be_bytes())
        .await?;
    stream.write_all(&body).await?;
    stream.flush().await
}

/// Serve one connection. Authenticates (SO_PEERPIDFD + uid), resolves
/// the caller app id from the pinned pid, then fields requests until
/// the peer closes or stops being alive. Drops silently on any auth
/// failure (deny).
pub async fn serve_connection(mut stream: UnixStream, store: Arc<StateStore>, caller_uid: u32) {
    let peer = match PeerPidfd::from_socket(&stream, caller_uid) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("peer auth refused: {e}");
            return;
        }
    };
    let app_id = match app_id_from_pid(peer.pid()) {
        Ok(id) => id,
        Err(e) => {
            tracing::warn!("caller app-id unresolved, refusing: {e}");
            return;
        }
    };
    loop {
        // Re-verify the original process still pins this connection
        // before honoring each request: the pidfd liveness, so a
        // recycled pid cannot masquerade mid-session.
        if !peer.is_alive() {
            tracing::warn!(app_id = %app_id, "peer no longer alive; dropping");
            return;
        }
        let request = match read_request(&mut stream).await {
            Ok(r) => r,
            // A closed connection or framing error ends the session.
            Err(_) => return,
        };
        let response = handle_request(&store, &app_id, request);
        if write_response(&mut stream, &response).await.is_err() {
            return;
        }
    }
}

/// Bind + serve the broker socket until the accept loop errors.
pub async fn run(store: Arc<StateStore>, socket: &Path) -> std::io::Result<()> {
    let listener = bind_socket(socket)?;
    let uid = owner_uid();
    tracing::info!(socket = %socket.display(), owner_uid = uid, "config-broker listening");
    loop {
        let (stream, _) = listener.accept().await?;
        let store = Arc::clone(&store);
        tokio::spawn(async move {
            serve_connection(stream, store, uid).await;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::Request;
    use crate::state::AiMasterSwitches;

    /// Drive a real socket end-to-end: bind, connect, `Get`, and
    /// confirm the framed `State` reply. Exercises the genuine
    /// SO_PEERPIDFD auth + app-id resolution + framing path (the
    /// dispatch gate itself is unit-tested in `protocol`). Gated to
    /// debug: only there does the test binary's `target/debug` path
    /// resolve to a `dev.*` app id rather than UnknownBinary (which
    /// would correctly drop the connection).
    #[cfg(debug_assertions)]
    #[tokio::test]
    async fn get_over_the_socket_returns_the_state() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(StateStore::open(dir.path()).unwrap());
        let want = AiMasterSwitches {
            enabled: true,
            access_level: 3,
            ..Default::default()
        };
        store.store(&want).unwrap();

        let sock = dir.path().join("broker.sock");
        let listener = bind_socket(&sock).unwrap();
        let uid = current_uid();
        let srv_store = Arc::clone(&store);
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            serve_connection(stream, srv_store, uid).await;
        });

        let mut client = UnixStream::connect(&sock).await.unwrap();
        let req = serde_json::to_vec(&Request::Get).unwrap();
        client
            .write_all(&(req.len() as u32).to_be_bytes())
            .await
            .unwrap();
        client.write_all(&req).await.unwrap();
        client.flush().await.unwrap();

        let mut len = [0u8; 4];
        client.read_exact(&mut len).await.unwrap();
        let n = u32::from_be_bytes(len) as usize;
        let mut body = vec![0u8; n];
        client.read_exact(&mut body).await.unwrap();
        let resp: Response = serde_json::from_slice(&body).unwrap();
        match resp {
            Response::State(got) => assert_eq!(got, want),
            other => panic!("expected State, got {other:?}"),
        }

        drop(client);
        let _ = server.await;
    }

    /// A `Set` from the (non-admitted) test caller is refused over the
    /// real socket - the auth + gate wiring rejects an unprivileged
    /// writer end-to-end without touching the store.
    #[cfg(debug_assertions)]
    #[tokio::test]
    async fn set_from_a_non_admitted_caller_is_refused_over_the_socket() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(StateStore::open(dir.path()).unwrap());
        let sock = dir.path().join("broker.sock");
        let listener = bind_socket(&sock).unwrap();
        let uid = current_uid();
        let srv_store = Arc::clone(&store);
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            serve_connection(stream, srv_store, uid).await;
        });

        let mut client = UnixStream::connect(&sock).await.unwrap();
        let hostile = AiMasterSwitches {
            executor_live: true,
            ..Default::default()
        };
        let req = serde_json::to_vec(&Request::Set(hostile)).unwrap();
        client
            .write_all(&(req.len() as u32).to_be_bytes())
            .await
            .unwrap();
        client.write_all(&req).await.unwrap();
        client.flush().await.unwrap();

        let mut len = [0u8; 4];
        client.read_exact(&mut len).await.unwrap();
        let n = u32::from_be_bytes(len) as usize;
        let mut body = vec![0u8; n];
        client.read_exact(&mut body).await.unwrap();
        let resp: Response = serde_json::from_slice(&body).unwrap();
        assert!(
            matches!(resp, Response::Refused(_)),
            "test caller is not an admitted writer, got {resp:?}"
        );
        // the store stayed at the floor
        assert_eq!(store.load().unwrap(), AiMasterSwitches::default());

        drop(client);
        let _ = server.await;
    }
}

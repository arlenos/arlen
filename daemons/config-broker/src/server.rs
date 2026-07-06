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

use tokio::net::{UnixListener, UnixStream};

use arlen_permissions::identity::app_id_from_pid;
use arlen_permissions::peer_pidfd::PeerPidfd;
use audit_proto::sink::AuditSink;

use crate::protocol::{handle_request, read_frame_async, write_frame_async, Request};
use crate::state::{changed_security_keys, switch_change_event, StateStore};

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

/// Bind the broker socket 0666 after a stale-socket probe: a path a
/// live process still serves is not clobbered (a singleton guard); a
/// dead leftover is cleared first.
///
/// 0666 (not 0600) because in the separate-uid deployment the broker runs
/// as a distinct service uid (root, or a dedicated config uid) while the
/// legitimate callers (Settings, the AI daemon + agent) run as the session
/// user, so a 0600 owner-only socket would refuse them. File permission is
/// NOT the boundary here: every connection is authenticated by SO_PEERPIDFD
/// ([`serve_connection`] rejects a uid mismatch via [`owner_uid`]) and Set is
/// gated to admitted writers by the kernel-attested app id. A 0666 socket
/// only lets a process of the expected uid connect; reading the switches
/// (Get) is open by design, and Set is still gated. This mirrors the
/// event-bus + knowledge sockets, whose access boundary is likewise the
/// peer credential, not the socket mode.
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
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o666))?;
    Ok(listener)
}

/// Serve one connection. Authenticates (SO_PEERPIDFD + uid), resolves
/// the caller app id from the pinned pid, then fields requests until
/// the peer closes or stops being alive. Drops silently on any auth
/// failure (deny).
pub async fn serve_connection(
    mut stream: UnixStream,
    store: Arc<StateStore>,
    caller_uid: u32,
    sink: Arc<dyn AuditSink>,
) {
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
        let request: Request = match read_frame_async(&mut stream).await {
            Ok(r) => r,
            // A closed connection or framing error ends the session.
            Err(_) => return,
        };
        // Audit-on-change: snapshot the pre-state for a Set so a flip of a
        // security-relevant switch is recorded even though the change itself is
        // gated by `is_admitted_writer`. The audit is accountability, not the
        // primary defence, so it is fail-open-after: a change still applies (and is
        // written) even if the ledger is down - a down audit daemon must never block
        // a caller turning `executor_live` back off.
        let pre = match &request {
            Request::Set(_) => store.load().ok(),
            _ => None,
        };
        let new_switches = match &request {
            Request::Set(s) => Some(s.clone()),
            _ => None,
        };
        let response = handle_request(&store, &app_id, request);
        if let (Some(old), Some(new)) = (pre, new_switches) {
            if matches!(response, crate::protocol::Response::Committed) {
                let changed = changed_security_keys(&old, &new.sanitised());
                if !changed.is_empty() {
                    if let Err(e) = sink.submit(switch_change_event(&app_id, &changed)).await {
                        tracing::warn!(
                            app_id = %app_id,
                            error = %e,
                            "config-broker: failed to audit an AI master-switch change"
                        );
                    }
                }
            }
        }
        if write_frame_async(&mut stream, &response).await.is_err() {
            return;
        }
    }
}

/// Bind + serve the broker socket until the accept loop errors.
pub async fn run(
    store: Arc<StateStore>,
    socket: &Path,
    sink: Arc<dyn AuditSink>,
) -> std::io::Result<()> {
    let listener = bind_socket(socket)?;
    let uid = owner_uid();
    tracing::info!(socket = %socket.display(), owner_uid = uid, "config-broker listening");
    loop {
        let (stream, _) = listener.accept().await?;
        let store = Arc::clone(&store);
        let sink = Arc::clone(&sink);
        tokio::spawn(async move {
            serve_connection(stream, store, uid, sink).await;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{Request, Response};
    use crate::state::AiMasterSwitches;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// A throwaway audit sink for the auth/framing tests (they never commit a
    /// change, so nothing is recorded; the change-audit path is covered by the
    /// `switch_change_event` + `changed_security_keys` unit tests in `state`).
    fn mock_sink() -> Arc<audit_proto::sink::MockAuditSink> {
        Arc::new(audit_proto::sink::MockAuditSink::accepting())
    }

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
            serve_connection(stream, srv_store, uid, mock_sink()).await;
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
            serve_connection(stream, srv_store, uid, mock_sink()).await;
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

    /// The separate-uid boundary at the serve path: a peer whose uid does NOT
    /// match the broker's configured expected caller uid is refused before any
    /// request is honored. Models the deployment where the broker (a distinct
    /// service uid) expects the session user's uid and a caller of a different uid
    /// connects - `PeerPidfd::from_socket` rejects it, `serve_connection` drops the
    /// connection with no reply, and the store is never touched.
    #[cfg(debug_assertions)]
    #[tokio::test]
    async fn a_peer_whose_uid_mismatches_the_expected_caller_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(StateStore::open(dir.path()).unwrap());
        let sock = dir.path().join("broker.sock");
        let listener = bind_socket(&sock).unwrap();
        // Expect a uid the test process does NOT have, so the auth rejects it.
        let wrong_uid = current_uid().wrapping_add(1);
        let srv_store = Arc::clone(&store);
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            serve_connection(stream, srv_store, wrong_uid, mock_sink()).await;
        });

        let mut client = UnixStream::connect(&sock).await.unwrap();
        let req = serde_json::to_vec(&Request::Get).unwrap();
        client
            .write_all(&(req.len() as u32).to_be_bytes())
            .await
            .unwrap();
        client.write_all(&req).await.unwrap();
        client.flush().await.unwrap();

        // The auth refusal drops the connection with no framed reply, so the read
        // of the length prefix hits EOF rather than returning a `State`.
        let mut len = [0u8; 4];
        let read = client.read_exact(&mut len).await;
        assert!(
            read.is_err(),
            "a uid-mismatched peer must be refused (connection dropped), got a reply"
        );

        let _ = server.await;
        assert_eq!(store.load().unwrap(), AiMasterSwitches::default());
    }
}

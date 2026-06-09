//! The signer's submit/lookup serve loop
//! (reversible-receipts-and-the-effect-model.md §6).
//!
//! A tokio `UnixListener` accepts connections, [`crate::auth::authenticate`]s the
//! peer (admit only the agent), then serves submit/lookup requests against the
//! shared [`SignerStore`] until the connection closes. The peer is re-verified
//! alive before each request (PID-reuse guard), the store is locked per request
//! so the chained log is appended serially, and every framing or codec error
//! ends the session fail-closed.
//!
//! [`dispatch`] is the pure request→response core (unit-tested without a socket);
//! [`serve_requests`] is the post-auth wire loop (tested over a socket pair);
//! [`run`] binds the socket and is the daemon's long-lived task.

use std::path::Path;
use std::sync::Arc;

use arlen_ai_undo_proto::{read_request, write_response, Request, Response, StateReply};
// The rendezvous path is part of the protocol contract; re-exported so existing
// `server::socket_path()` callers keep working.
pub use arlen_ai_undo_proto::socket_path;
use arlen_permissions::ConnectionAuth;
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{Mutex, Semaphore};

use crate::error::{Result, SignerError};
use crate::store::SignerStore;

/// Cap on concurrently-served connections. The sole admitted caller is the agent,
/// which uses one or few connections; this bounds task/fd growth so a peer that
/// opens many connections (and stalls each mid-frame) cannot exhaust the helper.
const MAX_CONNECTIONS: usize = 16;

/// Map one request to its response against the store. The pure dispatch core: no
/// I/O, no auth, just the store operations. A store error becomes a coarse
/// `Error` response (the messages carry no captured prior state, only operation
/// labels).
pub fn dispatch(store: &mut SignerStore, request: Request) -> Response {
    match request {
        Request::SubmitCreated(entry) => match store.submit_created(entry) {
            Ok(()) => Response::Sealed,
            Err(e) => Response::Error(e.to_string()),
        },
        Request::Transition { op_id, state } => match store.transition(&op_id, state) {
            Ok(()) => Response::Sealed,
            Err(e) => Response::Error(e.to_string()),
        },
        Request::LookupState { op_id } => {
            let reply = match store.state(&op_id) {
                None => StateReply::Absent,
                Some(Ok(state)) => StateReply::Present(state),
                // A folded-illegal chain is reported as corrupt, never a guessed
                // state (fail-closed; matches the in-memory fold contract).
                Some(Err(_)) => StateReply::Corrupt,
            };
            Response::State(reply)
        }
        Request::LookupEntry { op_id } => Response::Entry(store.entry(&op_id).cloned()),
    }
}

/// Serve requests on an authenticated connection until it closes. Before each
/// request the peer is re-verified alive (PID-reuse guard); a framing/codec error
/// or a dead peer ends the session. The store is locked only for the dispatch,
/// so a slow reader never holds the chained log.
pub async fn serve_requests(
    mut stream: UnixStream,
    auth: ConnectionAuth,
    store: Arc<Mutex<SignerStore>>,
) -> Result<()> {
    loop {
        // The peer must still be the same live process; a recycled pid or an
        // exited peer drops the connection rather than serving a stale identity.
        if auth.verify_alive().is_err() {
            return Ok(());
        }
        let request = match read_request(&mut stream).await {
            Ok(r) => r,
            // EOF or a framing/codec error ends the session.
            Err(_) => return Ok(()),
        };
        let response = match request.validate() {
            Ok(()) => {
                let mut guard = store.lock().await;
                dispatch(&mut guard, request)
            }
            Err(e) => Response::Error(e.to_string()),
        };
        if write_response(&mut stream, &response).await.is_err() {
            return Ok(());
        }
    }
}

/// Authenticate then serve one accepted connection. Rejected peers are logged and
/// dropped.
async fn handle(stream: UnixStream, caller_uid: u32, store: Arc<Mutex<SignerStore>>) {
    let auth = match crate::auth::authenticate(&stream, caller_uid) {
        Ok(auth) => auth,
        Err(e) => {
            tracing::warn!("undo-signer connection rejected: {e}");
            return;
        }
    };
    if let Err(e) = serve_requests(stream, auth, store).await {
        tracing::warn!("undo-signer connection error: {e}");
    }
}

/// Bind the signer socket and serve it until the accept loop errors. The daemon
/// spawns this as its long-lived task. The socket is created mode 0600 (only the
/// owner reaches it); a stale socket from a dead process is replaced, a live one
/// refused.
pub async fn run(socket_path: &Path, store: Arc<Mutex<SignerStore>>) -> Result<()> {
    let listener = bind_unix_socket(socket_path)?;
    let caller_uid = crate::auth::current_uid();
    let conns = Arc::new(Semaphore::new(MAX_CONNECTIONS));
    tracing::info!(socket = %socket_path.display(), "undo-signer listening");
    loop {
        // Acquire a connection slot before accepting the next peer: at the cap the
        // loop pauses (backpressure) until a served connection ends, so the helper
        // never spawns unbounded handlers. The permit is held by the task and
        // released when the connection closes.
        let permit = Arc::clone(&conns)
            .acquire_owned()
            .await
            .map_err(|e| SignerError::Storage(format!("connection semaphore closed: {e}")))?;
        let (stream, _) = listener
            .accept()
            .await
            .map_err(|e| SignerError::Storage(format!("undo-signer accept: {e}")))?;
        let store = Arc::clone(&store);
        tokio::spawn(async move {
            handle(stream, caller_uid, store).await;
            drop(permit);
        });
    }
}

/// Bind a Unix socket at `path` at mode 0600, creating the parent directory,
/// refusing a path already served by a live process and replacing a stale one.
fn bind_unix_socket(path: &Path) -> Result<UnixListener> {
    use std::os::unix::fs::PermissionsExt;
    if let Some(parent) = path.parent() {
        // Clamp the socket's parent directory to 0700 (as the state dir is), so
        // the socket node lives in an owner-only directory.
        crate::paths::ensure_private_dir(parent)?;
    }
    if path.exists() {
        match std::os::unix::net::UnixStream::connect(path) {
            Ok(_) => {
                return Err(SignerError::Storage(format!(
                    "{} is already served by a live process",
                    path.display()
                )));
            }
            Err(_) => {
                let _ = std::fs::remove_file(path);
            }
        }
    }
    let listener = UnixListener::bind(path)
        .map_err(|e| SignerError::Storage(format!("bind {}: {e}", path.display())))?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}

#[cfg(test)]
mod tests {
    use super::*;
    use arlen_ai_undo_core::effect_model::{CanonicalPath, InverseReceipt};
    use arlen_ai_undo_core::undo_log::{UndoEntry, UndoState};
    use arlen_ai_undo_proto::{read_response, write_request};

    fn entry(op: &str) -> UndoEntry {
        UndoEntry {
            op_id: op.to_string(),
            correlation_id: "run".to_string(),
            inverse: InverseReceipt::RestorePath {
                now: CanonicalPath::new("/b/x").unwrap(),
                prior: CanonicalPath::new("/a/x").unwrap(),
            },
        }
    }

    fn store_in(dir: &Path) -> Arc<Mutex<SignerStore>> {
        Arc::new(Mutex::new(SignerStore::open_in(dir).unwrap()))
    }

    #[tokio::test]
    async fn dispatch_maps_submit_transition_and_lookups() {
        let tmp = tempfile::tempdir().unwrap();
        let store = SignerStore::open_in(&tmp.path().join("undo-log")).unwrap();
        let store = Arc::new(Mutex::new(store));
        let mut s = store.lock().await;

        assert_eq!(dispatch(&mut s, Request::SubmitCreated(entry("op-1"))), Response::Sealed);
        assert_eq!(
            dispatch(&mut s, Request::LookupState { op_id: "op-1".into() }),
            Response::State(StateReply::Present(UndoState::InFlight))
        );
        assert_eq!(
            dispatch(&mut s, Request::Transition { op_id: "op-1".into(), state: UndoState::Committed }),
            Response::Sealed
        );
        assert_eq!(
            dispatch(&mut s, Request::LookupState { op_id: "op-1".into() }),
            Response::State(StateReply::Present(UndoState::Committed))
        );
        assert_eq!(
            dispatch(&mut s, Request::LookupState { op_id: "absent".into() }),
            Response::State(StateReply::Absent)
        );
        match dispatch(&mut s, Request::LookupEntry { op_id: "op-1".into() }) {
            Response::Entry(Some(e)) => assert_eq!(e.op_id, "op-1"),
            other => panic!("expected the sealed entry, got {other:?}"),
        }
    }

    #[test]
    fn socket_path_lands_under_the_runtime_dir() {
        // Whatever the environment, the path ends in the signer socket name.
        assert!(socket_path().ends_with("arlen/undo-signer.sock"));
    }

    // Build a ConnectionAuth bound to the test process so verify_alive passes,
    // bypassing SO_PEERCRED resolution (which would resolve the test binary, not
    // the agent). The profile content is irrelevant: signer admission is by
    // app_id, checked in `auth`, not by profile scopes.
    fn test_auth() -> ConnectionAuth {
        use arlen_permissions::{
            AppTier, PermissionProfile, ProfileInfo,
        };
        let profile = PermissionProfile {
            info: ProfileInfo { app_id: "ai-agent".into(), tier: AppTier::FirstParty },
            graph: Default::default(),
            event_bus: Default::default(),
            filesystem: Default::default(),
            network: Default::default(),
            notifications: Default::default(),
            clipboard: Default::default(),
            system: Default::default(),
            input: Default::default(),
            search: Default::default(),
            intents: Default::default(),
            mcp: Default::default(),
        };
        ConnectionAuth::for_test("ai-agent", profile)
    }

    #[tokio::test]
    async fn serve_requests_round_trips_over_a_socket_pair() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store_in(&tmp.path().join("undo-log"));
        let (mut client, server) = UnixStream::pair().unwrap();
        let task = tokio::spawn(serve_requests(server, test_auth(), store));

        // Submit, confirm sealed, then look the state back.
        write_request(&mut client, &Request::SubmitCreated(entry("op-1"))).await.unwrap();
        assert_eq!(read_response(&mut client).await.unwrap(), Response::Sealed);

        write_request(&mut client, &Request::LookupState { op_id: "op-1".into() }).await.unwrap();
        assert_eq!(
            read_response(&mut client).await.unwrap(),
            Response::State(StateReply::Present(UndoState::InFlight))
        );

        // An over-long id is rejected with Error, not sealed, and the session
        // continues (the connection is still the authed agent).
        write_request(&mut client, &Request::LookupState { op_id: "x".repeat(1000) }).await.unwrap();
        match read_response(&mut client).await.unwrap() {
            Response::Error(_) => {}
            other => panic!("expected Error for an over-long id, got {other:?}"),
        }

        // Closing the client ends the serve loop.
        drop(client);
        task.await.unwrap().unwrap();
    }
}

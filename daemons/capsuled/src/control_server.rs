//! The owner-facing control socket: `capsuled`'s management surface for the capsule
//! owner (the harness / settings), distinct from the recipient grant-read serve loop
//! (`server.rs`). A same-uid owner connects here to list their active capsules and
//! revoke one by handle (the "share a slice" surface, CC-R6).
//!
//! Same shell as the read serve loop: a `0600` Unix socket, SO_PEERCRED same-uid
//! admission with a PID-reuse re-check (there is no app-id allowlist - listing and
//! revoking one's OWN capsules is a same-user operation), the shared length-prefixed
//! framing. `mint` is not served here (it composes the knowledge daemon's slice
//! materialization with the local mint and is a checked human action; a later slice).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use arlen_forage_store::Store;
use arlen_permissions::ConnectionAuth;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

use crate::control::{ControlRequest, ControlResponse};
use crate::mint::revoke_capsule;
use crate::revocation::RevocationFile;
use crate::server::{bind_socket, current_uid, read_frame, write_frame};

/// The largest control request accepted (a `List` or `Revoke` is tiny); a hostile
/// length cannot force a large allocation.
const MAX_CONTROL_REQUEST_FRAME: usize = 64 * 1024;

/// The control socket path: `$XDG_RUNTIME_DIR/arlen/capsule-control.sock`, beside the
/// read socket. `None` when the runtime dir is unset (fail closed rather than bind
/// elsewhere).
pub fn control_socket_path() -> Option<PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR")
        .filter(|v| !v.is_empty())
        .map(|rt| PathBuf::from(rt).join("arlen/capsule-control.sock"))
}

/// The durable resources the control ops act on.
#[derive(Clone)]
pub struct ControlContext {
    /// The revoke/op-count ledger (list + revoke).
    pub ledger: Arc<RevocationFile>,
    /// The frozen-slice blob store (revoke releases the owner's blob).
    pub store: Arc<Store>,
}

/// Apply one control request, producing its reply. Pure over the ledger + store
/// (no socket), so the dispatch is unit-tested directly.
pub fn handle_control(
    req: ControlRequest,
    ledger: &RevocationFile,
    store: &Store,
) -> ControlResponse {
    if let Err(e) = req.validate() {
        return ControlResponse::Error(e);
    }
    match req {
        ControlRequest::List => match ledger.list() {
            Ok(list) => ControlResponse::Capsules(list),
            Err(e) => ControlResponse::Error(format!("list failed: {e}")),
        },
        ControlRequest::Revoke { handle } => match revoke_capsule(&handle, store, ledger) {
            Ok(()) => ControlResponse::Revoked,
            Err(e) => ControlResponse::Error(format!("revoke failed: {e}")),
        },
    }
}

/// Serve one control connection: read the framed [`ControlRequest`], apply it, write
/// the framed [`ControlResponse`]. A malformed request is answered with an `Error`,
/// not a dropped connection.
pub async fn serve_control_connection<S>(
    mut stream: S,
    ledger: &RevocationFile,
    store: &Store,
) -> std::io::Result<()>
where
    S: AsyncReadExt + AsyncWriteExt + Unpin,
{
    let request = read_frame(&mut stream, MAX_CONTROL_REQUEST_FRAME).await?;
    let response = match serde_json::from_slice::<ControlRequest>(&request) {
        Ok(req) => handle_control(req, ledger, store),
        Err(e) => ControlResponse::Error(format!("malformed control request: {e}")),
    };
    let bytes = serde_json::to_vec(&response).unwrap_or_else(|_| b"null".to_vec());
    write_frame(&mut stream, &bytes).await
}

/// Serve the control socket at `path` until the accept loop errors. Each connection
/// is admitted by SO_PEERCRED (same-uid only, PID-reuse re-checked) then served.
pub async fn run_control(path: &Path, ctx: ControlContext) -> std::io::Result<()> {
    let listener = bind_socket(path)?;
    let caller_uid = current_uid();
    loop {
        let (stream, _) = listener.accept().await?;
        let ctx = ctx.clone();
        tokio::spawn(async move {
            handle_control_conn(stream, caller_uid, ctx).await;
        });
    }
}

/// Admit and serve one accepted control connection. A cross-uid peer or a recycled
/// pid is rejected before any request is read.
async fn handle_control_conn(stream: UnixStream, caller_uid: u32, ctx: ControlContext) {
    let auth = match ConnectionAuth::extract_from(&stream, caller_uid) {
        Ok(a) => a,
        Err(e) => {
            tracing::debug!(error = %e, "capsule control peer rejected at admission");
            return;
        }
    };
    if auth.verify_alive().is_err() {
        return;
    }
    if let Err(e) = serve_control_connection(stream, &ctx.ledger, &ctx.store).await {
        tracing::debug!(error = %e, "capsule control connection closed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ledger_and_store() -> (RevocationFile, Store) {
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir()
            .join(format!("capsule-control-test-{}-{n}", std::process::id()));
        let ledger = RevocationFile::open(dir.join("ledger")).unwrap();
        let store = Store::open(dir.join("store")).unwrap();
        (ledger, store)
    }

    #[test]
    fn list_returns_the_registered_capsules() {
        let (ledger, store) = ledger_and_store();
        ledger.register("h-1").unwrap();
        ledger.register("h-2").unwrap();
        match handle_control(ControlRequest::List, &ledger, &store) {
            ControlResponse::Capsules(list) => {
                let handles: Vec<_> = list.iter().map(|e| e.handle.as_str()).collect();
                assert_eq!(handles, vec!["h-1", "h-2"]);
            }
            other => panic!("expected Capsules, got {other:?}"),
        }
    }

    #[test]
    fn revoke_marks_the_capsule_revoked() {
        let (ledger, store) = ledger_and_store();
        ledger.register("h-1").unwrap();
        assert_eq!(
            handle_control(ControlRequest::Revoke { handle: "h-1".into() }, &ledger, &store),
            ControlResponse::Revoked
        );
        assert!(ledger.state("h-1").unwrap().unwrap().revoked, "the capsule is now revoked");
    }

    #[test]
    fn a_blank_revoke_handle_is_an_error() {
        let (ledger, store) = ledger_and_store();
        assert!(matches!(
            handle_control(ControlRequest::Revoke { handle: "  ".into() }, &ledger, &store),
            ControlResponse::Error(_)
        ));
    }

    #[tokio::test]
    async fn a_framed_list_request_round_trips_over_a_connection() {
        let (ledger, store) = ledger_and_store();
        ledger.register("h-1").unwrap();
        let (mut client, server) = UnixStream::pair().unwrap();

        let srv = tokio::spawn(async move {
            serve_control_connection(server, &ledger, &store).await.unwrap();
        });

        let req = serde_json::to_vec(&ControlRequest::List).unwrap();
        write_frame(&mut client, &req).await.unwrap();
        let resp = read_frame(&mut client, MAX_CONTROL_REQUEST_FRAME).await.unwrap();
        srv.await.unwrap();

        match serde_json::from_slice::<ControlResponse>(&resp).unwrap() {
            ControlResponse::Capsules(list) => assert_eq!(list[0].handle, "h-1"),
            other => panic!("expected Capsules, got {other:?}"),
        }
    }
}

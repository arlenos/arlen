//! The owner-facing control socket: `capsuled`'s management surface for the capsule
//! owner (the harness / settings), distinct from the recipient grant-read serve loop
//! (`server.rs`). A same-uid owner connects here to list their active capsules and
//! revoke one by handle (the "share a slice" surface, CC-R6).
//!
//! Same shell as the read serve loop: a `0600` Unix socket, SO_PEERCRED same-uid
//! admission with a PID-reuse re-check (there is no app-id allowlist - listing and
//! revoking one's OWN capsules is a same-user operation), the shared length-prefixed
//! framing. `Mint` (the deliberate human "share a slice" action) is served here too
//! but is gated by MINT-REQUIRES-HUMAN: the accept loop hands the signing key only to
//! a mint-admitted app_id (the human UIs harness/settings), so a Mint from the agent
//! or any other same-uid peer fails closed. The caller materializes the slice via the
//! knowledge daemon's `0x07` op; this daemon signs and registers it.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use arlen_forage_store::Store;
use arlen_permissions::ConnectionAuth;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

use ed25519_dalek::SigningKey;

use crate::control::{ControlRequest, ControlResponse};
use crate::mint::{mint_capsule, revoke_capsule};
use crate::revocation::RevocationFile;
use crate::server::{bind_socket, current_uid, read_frame, write_frame};

/// The largest control request accepted. `List`/`Revoke` are tiny, but a `Mint`
/// carries a frozen slice (bounded by the knowledge daemon's manifest cap), so the
/// bound is sized for a slice; a hostile length past this cannot force a larger
/// allocation, and the same-uid admission bounds who can send one.
const MAX_CONTROL_REQUEST_FRAME: usize = 16 * 1024 * 1024;

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
    /// The capsule signing key, used only for a `Mint` from a mint-admitted caller.
    pub key: SigningKey,
}

/// App ids permitted to MINT a capsule: the human-driven UIs only (the "share a
/// slice" surface is a deliberate human action). The agent is explicitly refused -
/// a mint is never agent-reachable (context-capsule.md: mint-requires-human). List
/// and revoke are open to any same-uid caller (managing one's own capsules), but
/// mint alone carries this gate. In debug a `dev.`-prefixed id is admitted (the
/// dev/test convention), never in release.
fn mint_caller_admitted(app_id: &str) -> bool {
    if app_id == "harness" || app_id == "settings" {
        return true;
    }
    #[cfg(debug_assertions)]
    if app_id.starts_with("dev.") {
        return true;
    }
    false
}

/// Apply one control request, producing its reply. Pure over the ledger + store +
/// the optional signing key (no socket), so the dispatch is unit-tested directly.
/// `key` is present only when a mint is permitted (the signing key is wired and the
/// caller passed the mint gate); `Mint` fails closed without it.
pub fn handle_control(
    req: ControlRequest,
    ledger: &RevocationFile,
    store: &Store,
    key: Option<&SigningKey>,
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
        ControlRequest::Mint { slice, params } => match key {
            Some(key) => match mint_capsule(&slice, params, store, ledger, key) {
                Ok((hash, signed)) => ControlResponse::Minted {
                    handle: signed.grant.revocation_handle,
                    slice_hash: hash.as_str().to_string(),
                },
                Err(e) => ControlResponse::Error(format!("mint failed: {e}")),
            },
            None => ControlResponse::Error(
                "mint is not available (no signing key wired)".to_string(),
            ),
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
    key: Option<&SigningKey>,
) -> std::io::Result<()>
where
    S: AsyncReadExt + AsyncWriteExt + Unpin,
{
    let request = read_frame(&mut stream, MAX_CONTROL_REQUEST_FRAME).await?;
    let response = match serde_json::from_slice::<ControlRequest>(&request) {
        // `key` is `Some` only for a mint-admitted caller (the accept loop gates it);
        // a `Mint` from any other caller fails closed, List/Revoke are unaffected.
        Ok(req) => handle_control(req, ledger, store, key),
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
    // Mint is gated to human-UI callers (mint-requires-human): only a mint-admitted
    // app_id gets the signing key, so a Mint from the agent (or any other same-uid
    // peer) fails closed. List/revoke need no such gate (managing one's own capsules
    // is a same-user operation).
    let mint_key = mint_caller_admitted(auth.app_id()).then_some(&ctx.key);
    if let Err(e) = serve_control_connection(stream, &ctx.ledger, &ctx.store, mint_key).await {
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
        match handle_control(ControlRequest::List, &ledger, &store, None) {
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
            handle_control(ControlRequest::Revoke { handle: "h-1".into() }, &ledger, &store, None),
            ControlResponse::Revoked
        );
        assert!(ledger.state("h-1").unwrap().unwrap().revoked, "the capsule is now revoked");
    }

    #[test]
    fn a_blank_revoke_handle_is_an_error() {
        let (ledger, store) = ledger_and_store();
        assert!(matches!(
            handle_control(ControlRequest::Revoke { handle: "  ".into() }, &ledger, &store, None),
            ControlResponse::Error(_)
        ));
    }

    #[test]
    fn mint_is_admitted_only_for_the_human_uis_not_the_agent() {
        assert!(mint_caller_admitted("harness"));
        assert!(mint_caller_admitted("settings"));
        for other in ["ai-agent", "ai-daemon", "com.x", "knowledge", "unknown", ""] {
            assert!(!mint_caller_admitted(other), "{other} must not mint (mint-requires-human)");
        }
        // A `dev.`-prefixed id is admitted in debug only, never in release.
        assert_eq!(mint_caller_admitted("dev.arlen-harness"), cfg!(debug_assertions));
    }

    #[test]
    fn mint_is_refused_without_a_key_and_records_metadata_with_one() {
        use crate::scope::CapsuleScope;
        use crate::slice::{FrozenSlice, SliceNode, SliceValue};
        use crate::mint::MintParams;
        use std::collections::BTreeMap;

        let (ledger, store) = ledger_and_store();
        let mut fields = BTreeMap::new();
        fields.insert("path".to_string(), SliceValue::Text("/a".to_string()));
        let slice = FrozenSlice {
            nodes: vec![SliceNode { id: "f1".into(), label: "File".into(), fields }],
            relations: vec![],
        };
        let params = MintParams {
            scope: CapsuleScope { roots: vec!["p1".into()], expand_hops: 1 },
            audience_hex: "00".repeat(32),
            expires_at_micros: i64::MAX,
            max_ops: 3,
            originating_user: "tim".into(),
            label: "Reading list".into(),
            scope_summary: "files in p1".into(),
        };

        // No key: mint fails closed (the mint-gate/key wiring is a follow-up).
        let refused = handle_control(
            ControlRequest::Mint { slice: slice.clone(), params: params.clone() },
            &ledger,
            &store,
            None,
        );
        assert!(matches!(refused, ControlResponse::Error(_)));

        // With a key: mints, and the handle lists with its recorded metadata.
        let key = SigningKey::from_bytes(&[2u8; 32]);
        match handle_control(ControlRequest::Mint { slice, params }, &ledger, &store, Some(&key)) {
            ControlResponse::Minted { handle, slice_hash } => {
                assert!(!handle.is_empty() && !slice_hash.is_empty());
                let listed = ledger.list().unwrap();
                let e = listed.iter().find(|e| e.handle == handle).unwrap();
                assert_eq!(e.meta.as_ref().unwrap().label, "Reading list");
            }
            other => panic!("expected Minted, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_framed_list_request_round_trips_over_a_connection() {
        let (ledger, store) = ledger_and_store();
        ledger.register("h-1").unwrap();
        let (mut client, server) = UnixStream::pair().unwrap();

        let srv = tokio::spawn(async move {
            serve_control_connection(server, &ledger, &store, None).await.unwrap();
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

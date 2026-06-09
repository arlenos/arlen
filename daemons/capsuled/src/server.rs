//! The capsule serve loop's per-connection core (context-capsule.md §6).
//!
//! A reader connects, presents a framed [`SignedGrant`], and `capsuled` serves the
//! frozen slice or a refusal. This module holds the length-prefixed framing, the
//! content-free `CapsuleRead` audit (recorded fail-closed BEFORE serving, S13
//! audit-before-acting), and the per-request handler that ties them to
//! [`verify_and_serve`]. The accept loop, the 0600 socket bind and the SO_PEERCRED
//! same-uid admission are the daemon shell that wraps this (verified on a running
//! system); the handler here is exercised over a socket pair.

use arlen_forage_store::Store;
use audit_proto::{AuditKind, AuditSink, IngestRequest, StructuralRecord};
use ed25519_dalek::VerifyingKey;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::proto::{verify_and_serve, SignedGrant};
use crate::revocation::RevocationFile;
use crate::serve::Refusal;

/// The largest request frame accepted (a presented grant is small).
const MAX_REQUEST_FRAME: usize = 64 * 1024;
/// The largest slice a single read serves (a coarse bound; the scope's hop clamp
/// and the projection keep a real slice well under it).
const MAX_RESPONSE_FRAME: usize = 16 * 1024 * 1024;

/// The content-free `CapsuleRead` audit entry. Recorded for every read attempt
/// before serving, so the durable ledger shows a capsule was read (or refused)
/// without leaking the slice hash, the scope or the recipient. Classified as
/// [`AuditKind::GraphAccess`] (a capsule read is graph data served outward) with a
/// fixed `capsule.read` subject; the `outcome` is `served` or `refused:<reason>`.
fn capsule_read_event(outcome: &str, correlation_id: &str) -> IngestRequest {
    IngestRequest {
        kind: AuditKind::GraphAccess,
        structural: StructuralRecord {
            subject: "capsule.read".to_string(),
            node_types: Vec::new(),
            relations: Vec::new(),
            result_count: None,
            duration_ms: None,
            outcome: outcome.to_string(),
            depth: None,
        },
        forensic: None,
        call_chain_id: Some(correlation_id.to_string()),
        project_id: None,
    }
}

/// The content-free outcome label for a refusal (a class, never operands).
fn refusal_label(r: Refusal) -> &'static str {
    match r {
        Refusal::BadSignature => "refused:bad-signature",
        Refusal::Expired => "refused:expired",
        Refusal::Revoked => "refused:revoked",
        Refusal::Exhausted => "refused:exhausted",
        Refusal::Unknown => "refused:unknown",
        Refusal::Unavailable => "refused:unavailable",
    }
}

/// Handle one already-parsed request: audit the attempt fail-closed, then serve.
/// Returns the response bytes (the slice, or `ERROR: <reason>` for a refusal or an
/// audit outage). The audit is recorded BEFORE the serve decision is acted on: if
/// the audit sink is down, the read is refused without serving (no un-audited
/// read). `correlation_id` links the audit entry to the request.
pub async fn handle_request(
    signed: &SignedGrant,
    originator: &VerifyingKey,
    now_micros: i64,
    ledger: &RevocationFile,
    store: &Store,
    audit: &dyn AuditSink,
    correlation_id: &str,
) -> Vec<u8> {
    match verify_and_serve(signed, originator, now_micros, ledger, store) {
        Ok(bytes) => {
            // Audit the served read before returning it; a down sink fails closed.
            if audit
                .submit(capsule_read_event("served", correlation_id))
                .await
                .is_err()
            {
                return b"ERROR: audit unavailable".to_vec();
            }
            bytes
        }
        Err(refusal) => {
            // Record the refusal too (a refused read is activity worth the ledger),
            // best-effort: the response is the refusal regardless.
            let _ = audit
                .submit(capsule_read_event(refusal_label(refusal), correlation_id))
                .await;
            format!("ERROR: {}", refusal_label(refusal)).into_bytes()
        }
    }
}

/// Serve one connection: read the framed `SignedGrant`, handle it, write the
/// framed response. Fail-closed framing (a bad/oversized frame closes the
/// connection). `correlation_id` is supplied by the accept loop (e.g. a per-
/// connection id).
pub async fn serve_connection<S>(
    mut stream: S,
    originator: &VerifyingKey,
    now_micros: i64,
    ledger: &RevocationFile,
    store: &Store,
    audit: &dyn AuditSink,
    correlation_id: &str,
) -> std::io::Result<()>
where
    S: AsyncReadExt + AsyncWriteExt + Unpin,
{
    let request = read_frame(&mut stream, MAX_REQUEST_FRAME).await?;
    let signed: SignedGrant = serde_json::from_slice(&request)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let response =
        handle_request(&signed, originator, now_micros, ledger, store, audit, correlation_id).await;
    write_frame(&mut stream, &response).await
}

/// Read a length-prefixed frame (4-byte big-endian length + body), bounded by
/// `max` so a hostile length cannot force a large allocation.
async fn read_frame<S: AsyncReadExt + Unpin>(stream: &mut S, max: usize) -> std::io::Result<Vec<u8>> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > max {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "frame exceeds the maximum",
        ));
    }
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf).await?;
    Ok(buf)
}

/// Write a length-prefixed frame, bounded by [`MAX_RESPONSE_FRAME`].
async fn write_frame<S: AsyncWriteExt + Unpin>(stream: &mut S, bytes: &[u8]) -> std::io::Result<()> {
    if bytes.len() > MAX_RESPONSE_FRAME {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "response exceeds the maximum",
        ));
    }
    stream.write_all(&(bytes.len() as u32).to_be_bytes()).await?;
    stream.write_all(bytes).await?;
    stream.flush().await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grant::CapsuleGrant;
    use crate::scope::CapsuleScope;
    use crate::slice::{FrozenSlice, SliceNode, SliceValue};
    use crate::store::store_frozen_slice;
    use audit_proto::MockAuditSink;
    use ed25519_dalek::SigningKey;
    use std::collections::BTreeMap;
    use tokio::net::UnixStream;

    fn setup() -> (Store, RevocationFile, SigningKey, SignedGrant, Vec<u8>) {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let tmp = std::env::temp_dir().join(format!("capsule-server-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let store = Store::open(tmp.join("store")).unwrap();
        let ledger = RevocationFile::open(tmp.join("ledger")).unwrap();

        let mut fields = BTreeMap::new();
        fields.insert("path".to_string(), SliceValue::Text("/a".to_string()));
        let slice = FrozenSlice {
            nodes: vec![SliceNode { id: "f1".into(), label: "File".into(), fields }],
            relations: vec![],
        };
        let slice_bytes = slice.canonical_bytes();
        let hash = store_frozen_slice(&store, &slice, "capsule:test").unwrap();
        ledger.register("rev-1").unwrap();
        let key = SigningKey::from_bytes(&[5u8; 32]);
        let grant = CapsuleGrant {
            scope: CapsuleScope { roots: vec!["p1".into()], expand_hops: 1 },
            slice_hash: hash.as_str().to_string(),
            audience_hex: "00".repeat(32),
            expires_at_micros: i64::MAX,
            max_ops: 5,
            originating_user: "tim".into(),
            revocation_handle: "rev-1".into(),
        };
        let signed = SignedGrant::sign(grant, &key);
        (store, ledger, key, signed, slice_bytes)
    }

    #[tokio::test]
    async fn a_valid_request_serves_the_slice_over_a_socket_pair() {
        let (store, ledger, key, signed, slice_bytes) = setup();
        let audit = MockAuditSink::accepting();
        let (client, server) = UnixStream::pair().unwrap();

        let vk = key.verifying_key();
        let server_task = tokio::spawn(async move {
            serve_connection(server, &vk, 1, &ledger, &store, &audit, "conn-1")
                .await
                .unwrap();
            // The served read was audited.
            assert_eq!(audit.recorded().await.len(), 1);
        });

        let mut client = client;
        let req = serde_json::to_vec(&signed).unwrap();
        client.write_all(&(req.len() as u32).to_be_bytes()).await.unwrap();
        client.write_all(&req).await.unwrap();
        client.flush().await.unwrap();
        let resp = read_frame(&mut client, MAX_RESPONSE_FRAME).await.unwrap();
        server_task.await.unwrap();

        assert_eq!(resp, slice_bytes, "the slice is served back");
    }

    #[tokio::test]
    async fn a_down_audit_sink_refuses_without_serving() {
        let (store, ledger, key, signed, _) = setup();
        let audit = MockAuditSink::failing();
        let resp = handle_request(&signed, &key.verifying_key(), 1, &ledger, &store, &audit, "c").await;
        assert_eq!(resp, b"ERROR: audit unavailable");
    }

    #[tokio::test]
    async fn a_revoked_grant_is_refused_with_an_error() {
        let (store, ledger, key, signed, _) = setup();
        ledger.revoke("rev-1").unwrap();
        let audit = MockAuditSink::accepting();
        let resp = handle_request(&signed, &key.verifying_key(), 1, &ledger, &store, &audit, "c").await;
        assert_eq!(resp, b"ERROR: refused:revoked");
    }
}

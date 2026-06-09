//! The capsule serve decision (context-capsule.md §6).
//!
//! The pure heart of `capsuled`'s audited read: given a presented signed grant,
//! the originator's verifying key, the current time, the durable revoke/op-count
//! ledger and the slice store, decide whether to serve the frozen slice and which
//! bytes to return, or which [`Refusal`] applies. The daemon shell (the
//! SO_PEERCRED Unix socket, the request framing, the fail-closed `CapsuleRead`
//! audit before serving) wraps this; keeping the decision a pure function makes
//! every refusal path testable without a socket.
//!
//! Order matters and is security-relevant: verify the signature first, then check
//! expiry, **then** consume an op (so a forged or expired grant never burns the
//! op-count), and only then read the blob. The op is consumed atomically by the
//! ledger before the read, so the op-count bound holds even under concurrent reads
//! (a read failure may conservatively consume one op, never serve an extra one).

use arlen_forage_store::{ContentHash, Store};
use ed25519_dalek::{Signature, VerifyingKey};

use crate::grant::{verify_grant, CapsuleGrant};
use crate::revocation::{ConsumeVerdict, RevocationFile};

/// Why a capsule read was refused. Every variant means "do not serve".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// The grant signature does not verify against the originator key.
    BadSignature,
    /// The grant's mandatory expiry has passed.
    Expired,
    /// The capsule has been revoked.
    Revoked,
    /// The op-count bound is reached.
    Exhausted,
    /// No such capsule is registered in the ledger.
    Unknown,
    /// The grant was valid but the slice could not be served (missing or corrupt
    /// blob, a malformed hash, or a ledger/store I/O error) — fail closed.
    Unavailable,
}

/// Decide and produce a capsule read. Returns the frozen-slice bytes to serve, or
/// the [`Refusal`] that applies. See the module note for the (security-relevant)
/// ordering.
pub fn serve_capsule_read(
    grant: &CapsuleGrant,
    signature: &Signature,
    originator: &VerifyingKey,
    now_micros: i64,
    ledger: &RevocationFile,
    store: &Store,
) -> Result<Vec<u8>, Refusal> {
    if !verify_grant(grant, signature, originator) {
        return Err(Refusal::BadSignature);
    }
    if now_micros >= grant.expires_at_micros {
        return Err(Refusal::Expired);
    }
    match ledger.consume(&grant.revocation_handle, grant.max_ops) {
        Ok(ConsumeVerdict::Allowed) => {}
        Ok(ConsumeVerdict::Revoked) => return Err(Refusal::Revoked),
        Ok(ConsumeVerdict::Exhausted) => return Err(Refusal::Exhausted),
        Ok(ConsumeVerdict::Unknown) => return Err(Refusal::Unknown),
        // A ledger I/O error means the op-count cannot be enforced: fail closed.
        Err(_) => return Err(Refusal::Unavailable),
    }
    let hash = ContentHash::parse(&grant.slice_hash).map_err(|_| Refusal::Unavailable)?;
    store.read(&hash).map_err(|_| Refusal::Unavailable)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grant::sign_grant;
    use crate::scope::CapsuleScope;
    use crate::slice::{FrozenSlice, SliceNode, SliceRelation, SliceValue};
    use crate::store::store_frozen_slice;
    use ed25519_dalek::SigningKey;
    use std::collections::BTreeMap;

    struct Fixture {
        _tmp: std::path::PathBuf,
        store: Store,
        ledger: RevocationFile,
        key: SigningKey,
        grant: CapsuleGrant,
        slice_bytes: Vec<u8>,
    }

    fn fixture(max_ops: u64, expires_at_micros: i64) -> Fixture {
        // A process-unique dir per fixture, so concurrently-run tests never share a
        // store or ledger (keying on expires_at_micros collided for i64::MAX).
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let tmp = std::env::temp_dir().join(format!("capsule-serve-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let store = Store::open(tmp.join("store")).unwrap();
        let ledger = RevocationFile::open(tmp.join("ledger")).unwrap();

        let mut fields = BTreeMap::new();
        fields.insert("path".to_string(), SliceValue::Text("/a".to_string()));
        let slice = FrozenSlice {
            nodes: vec![SliceNode { id: "f1".into(), label: "File".into(), fields }],
            relations: vec![SliceRelation { from: "f1".into(), rel_type: "FILE_PART_OF".into(), to: "p1".into() }],
        };
        let slice_bytes = slice.canonical_bytes();
        let hash = store_frozen_slice(&store, &slice, "capsule:test").unwrap();

        let key = SigningKey::from_bytes(&[3u8; 32]);
        ledger.register("rev-1").unwrap();
        let grant = CapsuleGrant {
            scope: CapsuleScope { roots: vec!["p1".into()], expand_hops: 1 },
            slice_hash: hash.as_str().to_string(),
            audience_hex: "00".repeat(32),
            expires_at_micros,
            max_ops,
            originating_user: "tim".into(),
            revocation_handle: "rev-1".into(),
        };
        Fixture { _tmp: tmp, store, ledger, key, grant, slice_bytes }
    }

    #[test]
    fn a_valid_grant_serves_the_slice() {
        let f = fixture(2, i64::MAX);
        let sig = sign_grant(&f.grant, &f.key);
        let bytes = serve_capsule_read(&f.grant, &sig, &f.key.verifying_key(), 1000, &f.ledger, &f.store).unwrap();
        assert_eq!(bytes, f.slice_bytes);
    }

    #[test]
    fn a_bad_signature_is_refused_without_consuming() {
        let f = fixture(2, i64::MAX);
        let wrong = SigningKey::from_bytes(&[9u8; 32]);
        let sig = sign_grant(&f.grant, &wrong);
        assert_eq!(
            serve_capsule_read(&f.grant, &sig, &f.key.verifying_key(), 1000, &f.ledger, &f.store),
            Err(Refusal::BadSignature)
        );
        // The op-count was not touched (still 0 used).
        assert_eq!(f.ledger.state("rev-1").unwrap().unwrap().ops_used, 0);
    }

    #[test]
    fn an_expired_grant_is_refused() {
        let f = fixture(2, 500);
        let sig = sign_grant(&f.grant, &f.key);
        assert_eq!(
            serve_capsule_read(&f.grant, &sig, &f.key.verifying_key(), 1000, &f.ledger, &f.store),
            Err(Refusal::Expired)
        );
        assert_eq!(f.ledger.state("rev-1").unwrap().unwrap().ops_used, 0);
    }

    #[test]
    fn the_op_count_bound_is_enforced_then_exhausted() {
        let f = fixture(1, i64::MAX);
        let sig = sign_grant(&f.grant, &f.key);
        let vk = f.key.verifying_key();
        assert!(serve_capsule_read(&f.grant, &sig, &vk, 1000, &f.ledger, &f.store).is_ok());
        assert_eq!(
            serve_capsule_read(&f.grant, &sig, &vk, 1000, &f.ledger, &f.store),
            Err(Refusal::Exhausted)
        );
    }

    #[test]
    fn a_revoked_capsule_is_refused() {
        let f = fixture(5, i64::MAX);
        f.ledger.revoke("rev-1").unwrap();
        let sig = sign_grant(&f.grant, &f.key);
        assert_eq!(
            serve_capsule_read(&f.grant, &sig, &f.key.verifying_key(), 1000, &f.ledger, &f.store),
            Err(Refusal::Revoked)
        );
    }
}

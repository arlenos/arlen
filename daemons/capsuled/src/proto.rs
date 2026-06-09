//! The capsule serve protocol (context-capsule.md §6).
//!
//! A reader presents a [`SignedGrant`] — the grant manifest plus its detached
//! Ed25519 signature, the two parts of the bundle that travel together (§3). On
//! the same-machine path `capsuled` is itself the originator, so it verifies the
//! signature with its own capsule key's verifying key (the reader supplies no
//! key); the peer is kernel-attested by SO_PEERCRED at the socket. This module is
//! the wire request type and the pure dispatch over it (decode the signature, then
//! [`serve_capsule_read`]); the socket framing, the SO_PEERCRED admission and the
//! fail-closed `CapsuleRead` audit wrap it in the serve loop.

use arlen_forage_store::Store;
use ed25519_dalek::{Signature, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};

use crate::grant::{sign_grant, CapsuleGrant};
use crate::revocation::RevocationFile;
use crate::serve::{serve_capsule_read, Refusal};

/// A grant plus its detached signature — what a reader presents to be served.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedGrant {
    /// The grant manifest.
    pub grant: CapsuleGrant,
    /// The detached Ed25519 signature over the grant's canonical bytes (64 bytes).
    pub signature: Vec<u8>,
}

impl SignedGrant {
    /// Sign `grant` with the originator's key, producing the presentable bundle
    /// half. Used by the mint side and by tests.
    pub fn sign(grant: CapsuleGrant, key: &SigningKey) -> SignedGrant {
        let signature = sign_grant(&grant, key).to_bytes().to_vec();
        SignedGrant { grant, signature }
    }
}

/// Decode the presented signature and decide the read. A malformed signature (not
/// 64 valid bytes) is [`Refusal::BadSignature`], like a wrong one, so a junk
/// presentation is refused exactly as a forged one — the serve order (sig, expiry,
/// consume, read) is then [`serve_capsule_read`]'s.
pub fn verify_and_serve(
    signed: &SignedGrant,
    originator: &VerifyingKey,
    now_micros: i64,
    ledger: &RevocationFile,
    store: &Store,
) -> Result<Vec<u8>, Refusal> {
    let signature = Signature::from_slice(&signed.signature).map_err(|_| Refusal::BadSignature)?;
    serve_capsule_read(&signed.grant, &signature, originator, now_micros, ledger, store)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scope::CapsuleScope;
    use crate::slice::{FrozenSlice, SliceNode, SliceValue};
    use crate::store::store_frozen_slice;
    use std::collections::BTreeMap;

    fn setup() -> (Store, RevocationFile, SigningKey, CapsuleGrant) {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let tmp = std::env::temp_dir().join(format!("capsule-proto-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let store = Store::open(tmp.join("store")).unwrap();
        let ledger = RevocationFile::open(tmp.join("ledger")).unwrap();

        let mut fields = BTreeMap::new();
        fields.insert("path".to_string(), SliceValue::Text("/a".to_string()));
        let slice = FrozenSlice {
            nodes: vec![SliceNode { id: "f1".into(), label: "File".into(), fields }],
            relations: vec![],
        };
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
        (store, ledger, key, grant)
    }

    #[test]
    fn a_signed_grant_round_trips_and_serves() {
        let (store, ledger, key, grant) = setup();
        let signed = SignedGrant::sign(grant, &key);
        // The presentable bundle serializes (it travels beside the slice).
        let wire = serde_json::to_vec(&signed).unwrap();
        let back: SignedGrant = serde_json::from_slice(&wire).unwrap();
        assert_eq!(back, signed);

        let bytes = verify_and_serve(&back, &key.verifying_key(), 1, &ledger, &store).unwrap();
        assert!(!bytes.is_empty());
    }

    #[test]
    fn a_malformed_signature_is_refused() {
        let (store, ledger, key, grant) = setup();
        let mut signed = SignedGrant::sign(grant, &key);
        signed.signature = vec![0u8; 10]; // not 64 valid bytes
        assert_eq!(
            verify_and_serve(&signed, &key.verifying_key(), 1, &ledger, &store),
            Err(Refusal::BadSignature)
        );
    }
}

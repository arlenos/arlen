//! Minting a capsule (context-capsule.md §3, §6).
//!
//! Minting turns an already-materialized frozen slice into a lendable capsule: it
//! stores the slice content-addressed (rooted so it survives the store's gc),
//! registers a fresh revocation handle in the durable ledger, and signs the grant.
//! The result is the bundle's two parts — the slice content hash (the capsule's
//! identity) and the [`SignedGrant`] — which the caller packages into the `.capsule`
//! bundle (§3).
//!
//! The slice is materialized by the caller (the daemon's `0x07` read), so this
//! function does not touch the graph and stays unit-testable. Minting is
//! **human-gated**: the caller invokes it only on an explicit user action, never
//! from an agent path (§6). It is the inverse-direction-safe counterpart of
//! revocation: a fresh handle per mint means each capsule is revoked independently.

use arlen_forage_store::{ContentHash, Store, StoreError};
use ed25519_dalek::SigningKey;

use crate::grant::CapsuleGrant;
use crate::proto::SignedGrant;
use crate::revocation::RevocationFile;
use crate::scope::CapsuleScope;
use crate::slice::FrozenSlice;
use crate::store::store_frozen_slice;

/// The inputs to a mint that are not the slice or the local resources: who the
/// capsule is for, until when, how many reads, and under whose name.
pub struct MintParams {
    /// The shared scope (the same selection that produced the slice).
    pub scope: CapsuleScope,
    /// The audience: hex of the Ed25519 verifying key permitted to read.
    pub audience_hex: String,
    /// Mandatory expiry, epoch microseconds.
    pub expires_at_micros: i64,
    /// The op-count bound.
    pub max_ops: u64,
    /// The minting user.
    pub originating_user: String,
}

/// Why a mint failed.
#[derive(Debug, thiserror::Error)]
pub enum MintError {
    /// The slice could not be stored.
    #[error("capsule store error: {0}")]
    Store(#[from] StoreError),
    /// The revocation handle could not be registered.
    #[error("revocation ledger error: {0}")]
    Ledger(#[from] std::io::Error),
    /// The CSPRNG failed to produce a revocation handle.
    #[error("csprng error: {0}")]
    Csprng(String),
}

/// Mint a capsule from an already-materialized [`FrozenSlice`]: store it, register
/// a fresh revocation handle, and sign the grant. Returns the slice content hash
/// (the capsule identity) and the signed grant. The blob is rooted to a
/// per-capsule owner (`capsule:<handle>`) so revoking the handle can release it.
pub fn mint_capsule(
    slice: &FrozenSlice,
    params: MintParams,
    store: &Store,
    ledger: &RevocationFile,
    key: &SigningKey,
) -> Result<(ContentHash, SignedGrant), MintError> {
    let handle = fresh_handle()?;
    let owner = format!("capsule:{handle}");
    let hash = store_frozen_slice(store, slice, &owner)?;
    ledger.register(&handle)?;
    let grant = CapsuleGrant {
        scope: params.scope,
        slice_hash: hash.as_str().to_string(),
        audience_hex: params.audience_hex,
        expires_at_micros: params.expires_at_micros,
        max_ops: params.max_ops,
        originating_user: params.originating_user,
        revocation_handle: handle,
    };
    let signed = SignedGrant::sign(grant, key);
    Ok((hash, signed))
}

/// Revoke a capsule: stop every future read AND release the originator's own
/// frozen-slice blob (context-capsule.md §6, CC-R5). First the ledger revoke (the
/// security property: a revoked capsule is refused on every read, terminal), then
/// release the per-capsule blob ref so the store's gc can collect the local copy
/// (the "forget my copy" half — not erasure of a recipient's copy, which §2 drops,
/// but reclaiming the originator's disk). Revoke is durable the moment the ledger
/// write returns; the release is idempotent (an already-released owner is a no-op),
/// so a retry after a release failure is safe.
pub fn revoke_capsule(
    handle: &str,
    store: &Store,
    ledger: &RevocationFile,
) -> Result<(), MintError> {
    ledger.revoke(handle)?;
    store.release(&format!("capsule:{handle}"))?;
    Ok(())
}

/// A fresh, unguessable revocation handle: 16 CSPRNG bytes as hex. Random (not
/// derived from the slice) so two capsules of the same slice revoke independently.
fn fresh_handle() -> Result<String, MintError> {
    let mut bytes = [0u8; 16];
    getrandom::getrandom(&mut bytes).map_err(|e| MintError::Csprng(e.to_string()))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grant::verify_grant;
    use crate::serve::{serve_capsule_read, Refusal};
    use crate::slice::{SliceNode, SliceValue};
    use ed25519_dalek::Signature;
    use std::collections::BTreeMap;

    fn paths() -> (Store, RevocationFile) {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let tmp = std::env::temp_dir().join(format!("capsule-mint-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        (
            Store::open(tmp.join("store")).unwrap(),
            RevocationFile::open(tmp.join("ledger")).unwrap(),
        )
    }

    fn slice() -> FrozenSlice {
        let mut fields = BTreeMap::new();
        fields.insert("path".to_string(), SliceValue::Text("/a".to_string()));
        FrozenSlice {
            nodes: vec![SliceNode { id: "f1".into(), label: "File".into(), fields }],
            relations: vec![],
        }
    }

    fn params() -> MintParams {
        MintParams {
            scope: CapsuleScope { roots: vec!["p1".into()], expand_hops: 1 },
            audience_hex: "00".repeat(32),
            expires_at_micros: i64::MAX,
            max_ops: 3,
            originating_user: "tim".into(),
        }
    }

    #[test]
    fn a_minted_capsule_is_signed_registered_and_servable() {
        let (store, ledger) = paths();
        let key = SigningKey::from_bytes(&[2u8; 32]);
        let s = slice();

        let (hash, signed) = mint_capsule(&s, params(), &store, &ledger, &key).unwrap();

        // The hash is the slice's content hash, and the grant points at it.
        assert_eq!(hash, ContentHash::of(&s.canonical_bytes()));
        assert_eq!(signed.grant.slice_hash, hash.as_str());
        // The grant verifies under the minting key.
        let sig = Signature::from_slice(&signed.signature).unwrap();
        assert!(verify_grant(&signed.grant, &sig, &key.verifying_key()));
        // The handle was registered, so a serve can find it.
        assert!(ledger.state(&signed.grant.revocation_handle).unwrap().is_some());
        // End to end: the minted capsule serves its slice.
        let bytes = serve_capsule_read(&signed.grant, &sig, &key.verifying_key(), 1, &ledger, &store).unwrap();
        assert_eq!(bytes, s.canonical_bytes());
    }

    #[test]
    fn each_mint_gets_a_distinct_handle() {
        let (store, ledger) = paths();
        let key = SigningKey::from_bytes(&[2u8; 32]);
        let a = mint_capsule(&slice(), params(), &store, &ledger, &key).unwrap().1;
        let b = mint_capsule(&slice(), params(), &store, &ledger, &key).unwrap().1;
        assert_ne!(
            a.grant.revocation_handle, b.grant.revocation_handle,
            "two capsules of the same slice revoke independently"
        );
    }

    #[test]
    fn revoking_a_minted_capsule_refuses_its_reads() {
        let (store, ledger) = paths();
        let key = SigningKey::from_bytes(&[2u8; 32]);
        let (_, signed) = mint_capsule(&slice(), params(), &store, &ledger, &key).unwrap();
        ledger.revoke(&signed.grant.revocation_handle).unwrap();
        let sig = Signature::from_slice(&signed.signature).unwrap();
        assert_eq!(
            serve_capsule_read(&signed.grant, &sig, &key.verifying_key(), 1, &ledger, &store),
            Err(Refusal::Revoked)
        );
    }

    #[test]
    fn revoke_capsule_stops_reads_and_releases_the_blob() {
        let (store, ledger) = paths();
        let key = SigningKey::from_bytes(&[2u8; 32]);
        let (hash, signed) = mint_capsule(&slice(), params(), &store, &ledger, &key).unwrap();
        let handle = signed.grant.revocation_handle.clone();
        // Before revoke: the blob is rooted (one ref) and reads serve.
        assert_eq!(store.refcount(&hash).unwrap(), 1);

        revoke_capsule(&handle, &store, &ledger).unwrap();

        // No future read.
        let sig = Signature::from_slice(&signed.signature).unwrap();
        assert_eq!(
            serve_capsule_read(&signed.grant, &sig, &key.verifying_key(), 1, &ledger, &store),
            Err(Refusal::Revoked)
        );
        // The originator's blob ref is released, so gc can collect it.
        assert_eq!(store.refcount(&hash).unwrap(), 0, "the slice blob ref is released");
        // Idempotent: a retry is safe.
        revoke_capsule(&handle, &store, &ledger).unwrap();
    }
}

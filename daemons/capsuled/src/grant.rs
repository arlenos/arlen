//! The signed capsule grant (context-capsule.md §5).
//!
//! A capsule carries a grant manifest beside its frozen slice: the scope, the
//! slice's content hash, the audience, a **mandatory** expiry, an op-count bound,
//! the originating user and a revocation handle, Ed25519-signed over a canonical
//! byte form. This is the macaroon **root** block; the attenuation chain (CC-R3,
//! delegation blocks, new crypto enforcing only on the gated external path) builds
//! on it later.
//!
//! Honest about enforcement (§5): the *value* of `audience` (an Ed25519 verifying
//! key) is unforgeable, but binding "this read came from the audience" needs an
//! attested peer, which on the same-machine path is `capsuled`'s SO_PEERCRED, not
//! this field. Expiry and op-count need no external identity, so they hold
//! regardless. This module is the grant type, its canonical serialization and the
//! sign/verify over it; the persisted signing key custody is a sibling piece, and
//! the serve-time expiry/op-count/audience checks are CC-R4/R5.

use arlen_capsule::scope::CapsuleScope;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};

/// A capsule grant: what is shared, with whom, until when, and how it is revoked.
/// The content the originator signs; the signature travels beside it in the bundle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapsuleGrant {
    /// The shared scope (the same selection that materialized the slice).
    pub scope: CapsuleScope,
    /// The frozen slice's content hash — the capsule's identity (CC-R1).
    pub slice_hash: String,
    /// The audience: the hex of the Ed25519 verifying key permitted to read.
    pub audience_hex: String,
    /// Mandatory expiry, epoch microseconds (the capsule promotes the token's
    /// always-optional expiry to a required field).
    pub expires_at_micros: i64,
    /// The op-count bound: the maximum number of reads the serving component will
    /// honour before refusing (a durable counter under flock, CC-R5).
    pub max_ops: u64,
    /// The user who minted the capsule.
    pub originating_user: String,
    /// The durable revocation handle: revoking it refuses all future reads (CC-R5).
    pub revocation_handle: String,
}

impl CapsuleGrant {
    /// The canonical byte form that is signed and verified. The scope roots are
    /// sorted and a fixed-field view is serialized, so the bytes depend only on
    /// the logical grant, not on the order the roots were supplied — the signer
    /// and any verifier derive identical bytes for the same grant.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut roots = self.scope.roots.clone();
        roots.sort();
        let canonical = Canonical {
            roots: &roots,
            expand_hops: self.scope.expand_hops,
            slice_hash: &self.slice_hash,
            audience_hex: &self.audience_hex,
            expires_at_micros: self.expires_at_micros,
            max_ops: self.max_ops,
            originating_user: &self.originating_user,
            revocation_handle: &self.revocation_handle,
        };
        serde_json::to_vec(&canonical)
            .expect("canonical grant serialization is infallible for plain owned data")
    }
}

/// The fixed-field canonical view (sorted roots, flat fields) so the signed bytes
/// are stable and order-independent.
#[derive(Serialize)]
struct Canonical<'a> {
    roots: &'a [String],
    expand_hops: u32,
    slice_hash: &'a str,
    audience_hex: &'a str,
    expires_at_micros: i64,
    max_ops: u64,
    originating_user: &'a str,
    revocation_handle: &'a str,
}

/// Sign a grant with the originator's signing key (Ed25519 over the canonical
/// bytes), returning the detached signature that travels beside the grant.
pub fn sign_grant(grant: &CapsuleGrant, key: &SigningKey) -> Signature {
    key.sign(&grant.canonical_bytes())
}

/// Verify a grant against its signature and the originator's verifying key. Returns
/// false on any mismatch (a tampered field, a wrong key, a forged signature), so a
/// caller treats false as "do not honour this grant".
pub fn verify_grant(grant: &CapsuleGrant, signature: &Signature, key: &VerifyingKey) -> bool {
    key.verify(&grant.canonical_bytes(), signature).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grant() -> CapsuleGrant {
        CapsuleGrant {
            scope: CapsuleScope { roots: vec!["p1".into()], expand_hops: 1 },
            slice_hash: "sha256:abc".into(),
            audience_hex: "00".repeat(32),
            expires_at_micros: 1_000_000,
            max_ops: 10,
            originating_user: "tim".into(),
            revocation_handle: "rev-1".into(),
        }
    }

    fn key() -> SigningKey {
        SigningKey::from_bytes(&[7u8; 32])
    }

    #[test]
    fn a_signed_grant_verifies() {
        let k = key();
        let g = grant();
        let sig = sign_grant(&g, &k);
        assert!(verify_grant(&g, &sig, &k.verifying_key()));
    }

    #[test]
    fn a_tampered_grant_fails_verification() {
        let k = key();
        let g = grant();
        let sig = sign_grant(&g, &k);
        // Extending the expiry (the obvious attack) breaks the signature.
        let mut tampered = g.clone();
        tampered.expires_at_micros = i64::MAX;
        assert!(!verify_grant(&tampered, &sig, &k.verifying_key()));
        // So does widening the op-count or swapping the audience.
        let mut more_ops = g.clone();
        more_ops.max_ops = u64::MAX;
        assert!(!verify_grant(&more_ops, &sig, &k.verifying_key()));
    }

    #[test]
    fn a_wrong_key_fails_verification() {
        let g = grant();
        let sig = sign_grant(&g, &key());
        let other = SigningKey::from_bytes(&[9u8; 32]);
        assert!(!verify_grant(&g, &sig, &other.verifying_key()));
    }

    #[test]
    fn canonical_bytes_are_pinned() {
        // Pin the exact signed form. The signature covers these bytes, so a silent
        // change to the field order or shape (a serde reorder, a renamed field)
        // would invalidate every existing grant; this test fails first instead.
        let g = grant();
        let expected = format!(
            "{{\"roots\":[\"p1\"],\"expand_hops\":1,\"slice_hash\":\"sha256:abc\",\"audience_hex\":\"{}\",\"expires_at_micros\":1000000,\"max_ops\":10,\"originating_user\":\"tim\",\"revocation_handle\":\"rev-1\"}}",
            "00".repeat(32)
        );
        assert_eq!(g.canonical_bytes(), expected.into_bytes());
    }

    #[test]
    fn canonical_bytes_are_root_order_independent() {
        let mut a = grant();
        a.scope.roots = vec!["a".into(), "b".into(), "c".into()];
        let mut b = grant();
        b.scope.roots = vec!["c".into(), "a".into(), "b".into()];
        assert_eq!(a.canonical_bytes(), b.canonical_bytes());
        // A signature made over one verifies the other (same logical grant).
        let k = key();
        let sig = sign_grant(&a, &k);
        assert!(verify_grant(&b, &sig, &k.verifying_key()));
    }
}

//! Ledger entry types and the HMAC hash-chain.
//!
//! The wire types `AuditKind`, `StructuralRecord`, and
//! `ForensicRecord` are defined once, in `audit-proto`, and
//! re-exported here so the rest of the daemon refers to them through
//! `crate::ledger`. The daemon-internal types — [`AuditEntry`] (the
//! committed row, with its raw hashes) and [`StructuralView`] (the
//! read-API projection) — live here.
//!
//! The two-tier split from foundation §8.4.7 is enforced by the type
//! system: `StructuralRecord` has no field that can hold a query
//! string, result content, or a node ID, and `StructuralView`
//! likewise; the opt-in `ForensicRecord` is a separate struct
//! attached only as an `Option`.

use hmac::{Hmac, Mac};
use sha2::Sha256;

pub use audit_proto::{AuditKind, ForensicRecord, StructuralRecord, StructuralView};

type HmacSha256 = Hmac<Sha256>;

/// The `prev_hash` of the genesis entry: 32 zero bytes. The verifier
/// expects index 0 to chain from this fixed value.
pub const GENESIS_PREV_HASH: [u8; 32] = [0u8; 32];

/// One committed ledger entry. Produced by the store on append and
/// reconstructed from the store on read or verify.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditEntry {
    /// Sequential index, 0-based — the chain order.
    pub index: u64,
    /// Append time, microseconds since the Unix epoch. Informational
    /// only: the chain order is the `index`, so a backward clock
    /// correction does not break verification.
    pub timestamp_micros: i64,
    /// What kind of action this entry records.
    pub kind: AuditKind,
    /// `app_id` of the component that performed the action.
    pub actor: String,
    /// The always-on, content-free metadata tier.
    pub structural: StructuralRecord,
    /// The opt-in content tier, present only if Forensic Mode was on.
    pub forensic: Option<ForensicRecord>,
    /// MCP call-chain id, when the entry belongs to one.
    pub call_chain_id: Option<String>,
    /// Project context, when one was active.
    pub project_id: Option<String>,
    /// `entry_hash` of the previous entry; `GENESIS_PREV_HASH` at
    /// index 0.
    pub prev_hash: [u8; 32],
    /// This entry's HMAC chain hash.
    pub entry_hash: [u8; 32],
}

// `StructuralView` (the read-API projection) is defined once in
// `audit-proto` and re-exported above, so the daemon and its read
// clients share one type.

/// Compute the HMAC chain hash for an entry.
///
/// The hashed input is a fixed-order, length-prefixed byte
/// concatenation, so it is fully deterministic without relying on
/// JSON-canonicalisation rules. Any change to any field, or to
/// `prev_hash`, changes the result — which is exactly what lets the
/// verifier detect a retroactive edit, deletion, or insertion.
#[allow(clippy::too_many_arguments)]
pub fn compute_entry_hash(
    key: &[u8],
    index: u64,
    timestamp_micros: i64,
    kind: AuditKind,
    actor: &str,
    structural: &StructuralRecord,
    forensic: Option<&ForensicRecord>,
    call_chain_id: Option<&str>,
    project_id: Option<&str>,
    prev_hash: &[u8; 32],
) -> [u8; 32] {
    // HMAC accepts a key of any length; `new_from_slice` only errors
    // on an allocator failure, which is not a recoverable condition.
    let mut mac =
        HmacSha256::new_from_slice(key).expect("HMAC accepts any key length");

    mac.update(&index.to_be_bytes());
    mac.update(&timestamp_micros.to_be_bytes());
    feed(&mut mac, kind.as_str().as_bytes());
    feed(&mut mac, actor.as_bytes());
    // Structural and forensic go in as their serde_json encoding.
    // Both structs have a fixed field order and only string / int /
    // string-vector / option fields — no maps, no floats — so the
    // encoding is deterministic. The length prefix from `feed` keeps
    // the field boundary unambiguous.
    feed(
        &mut mac,
        &serde_json::to_vec(structural).expect("structural record serialises"),
    );
    match forensic {
        Some(f) => {
            mac.update(&[1u8]);
            feed(
                &mut mac,
                &serde_json::to_vec(f).expect("forensic record serialises"),
            );
        }
        None => mac.update(&[0u8]),
    }
    feed(&mut mac, call_chain_id.unwrap_or("").as_bytes());
    feed(&mut mac, project_id.unwrap_or("").as_bytes());
    mac.update(prev_hash);

    mac.finalize().into_bytes().into()
}

/// Feed one length-prefixed field into the MAC.
fn feed(mac: &mut HmacSha256, bytes: &[u8]) {
    mac.update(&(bytes.len() as u64).to_be_bytes());
    mac.update(bytes);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn structural() -> StructuralRecord {
        StructuralRecord {
            subject: "com.arlen.files".into(),
            node_types: vec!["File".into()],
            relations: vec![],
            result_count: Some(7),
            duration_ms: Some(12),
            outcome: "ok".into(),
            depth: None,
            capability_change: None,
        }
    }

    #[test]
    fn hash_is_deterministic() {
        let key = b"test-key";
        let a = compute_entry_hash(
            key, 0, 1000, AuditKind::Query, "ai-daemon", &structural(),
            None, None, None, &GENESIS_PREV_HASH,
        );
        let b = compute_entry_hash(
            key, 0, 1000, AuditKind::Query, "ai-daemon", &structural(),
            None, None, None, &GENESIS_PREV_HASH,
        );
        assert_eq!(a, b, "same inputs must hash the same");
    }

    #[test]
    fn hash_changes_when_any_field_changes() {
        let key = b"test-key";
        let base = compute_entry_hash(
            key, 0, 1000, AuditKind::Query, "ai-daemon", &structural(),
            None, None, None, &GENESIS_PREV_HASH,
        );
        let other_actor = compute_entry_hash(
            key, 0, 1000, AuditKind::Query, "evil", &structural(),
            None, None, None, &GENESIS_PREV_HASH,
        );
        assert_ne!(base, other_actor);
        let other_prev = compute_entry_hash(
            key, 0, 1000, AuditKind::Query, "ai-daemon", &structural(),
            None, None, None, &[9u8; 32],
        );
        assert_ne!(base, other_prev);
        let other_index = compute_entry_hash(
            key, 1, 1000, AuditKind::Query, "ai-daemon", &structural(),
            None, None, None, &GENESIS_PREV_HASH,
        );
        assert_ne!(base, other_index);
    }

    #[test]
    fn hash_depends_on_the_key() {
        let s = structural();
        let with_a = compute_entry_hash(
            b"key-a", 0, 1000, AuditKind::Query, "ai-daemon", &s,
            None, None, None, &GENESIS_PREV_HASH,
        );
        let with_b = compute_entry_hash(
            b"key-b", 0, 1000, AuditKind::Query, "ai-daemon", &s,
            None, None, None, &GENESIS_PREV_HASH,
        );
        assert_ne!(with_a, with_b, "the HMAC key must affect the hash");
    }
}

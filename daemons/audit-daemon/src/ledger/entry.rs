//! Audit entry types and the HMAC hash-chain.
//!
//! The two-tier split from foundation §8.4.7 is enforced by the type
//! system: [`StructuralRecord`] has no field that can hold a query
//! string, result content, or a node ID, while [`ForensicRecord`] —
//! the opt-in tier that *does* carry that content — is a separate
//! struct attached only as an `Option`. A content leak into the
//! always-on Structural tier is therefore a compile error.

use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// The `prev_hash` of the genesis entry: 32 zero bytes. The verifier
/// expects index 0 to chain from this fixed value.
pub const GENESIS_PREV_HASH: [u8; 32] = [0u8; 32];

/// What kind of audited action an entry records.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditKind {
    /// An AI natural-language query.
    Query,
    /// An MCP tool invocation.
    ToolCall,
    /// A user confirmation of a high-impact action.
    Confirm,
    /// A rejected call: depth limit, permission, or always-confirm.
    PolicyViolation,
    /// A Knowledge-Graph access.
    GraphAccess,
    /// A permission grant or denial.
    Permission,
}

impl AuditKind {
    /// Stable wire string. Also the suffix of the Event Bus event
    /// type re-emitted after a successful append (`audit.ai.<kind>`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Query => "query",
            Self::ToolCall => "tool_call",
            Self::Confirm => "confirm",
            Self::PolicyViolation => "policy_violation",
            Self::GraphAccess => "graph_access",
            Self::Permission => "permission",
        }
    }

    /// Parse a kind back from its [`as_str`](Self::as_str) wire form.
    /// Returns `None` for an unrecognised string (a corrupt ledger
    /// row), which the store surfaces as a storage error.
    pub fn from_wire(s: &str) -> Option<Self> {
        match s {
            "query" => Some(Self::Query),
            "tool_call" => Some(Self::ToolCall),
            "confirm" => Some(Self::Confirm),
            "policy_violation" => Some(Self::PolicyViolation),
            "graph_access" => Some(Self::GraphAccess),
            "permission" => Some(Self::Permission),
            _ => None,
        }
    }
}

/// Content-free interaction metadata — the Structural tier of
/// foundation §8.4.7, always recorded. Every field here is a coarse
/// identifier or a count; none can hold a query string, a result
/// value, or a concrete node ID.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StructuralRecord {
    /// Coarse subject identifier: an MCP server id, a tool name, or
    /// a graph target label. Never free-form content.
    pub subject: String,
    /// Graph node types touched, if any.
    #[serde(default)]
    pub node_types: Vec<String>,
    /// Graph relations traversed, if any.
    #[serde(default)]
    pub relations: Vec<String>,
    /// Number of results, when meaningful for this kind.
    #[serde(default)]
    pub result_count: Option<u64>,
    /// Wall-clock duration of the action, when measured.
    #[serde(default)]
    pub duration_ms: Option<u64>,
    /// Coarse outcome label: `ok`, `denied`, `error`, ...
    pub outcome: String,
    /// MCP call-chain depth, when the entry is part of one.
    #[serde(default)]
    pub depth: Option<u8>,
}

/// The opt-in Forensic tier (foundation §8.4.7): the content the
/// Structural tier deliberately omits. An entry carries this only
/// when Forensic Mode was active at the time it was written; on
/// every other entry it is `None`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ForensicRecord {
    /// The full query string.
    pub query_string: String,
    /// Query parameters (not result contents).
    pub parameters: String,
    /// The calling process stack trace.
    pub stack_trace: String,
}

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
    /// Project context, when one was active — drives the
    /// project-scoped export.
    pub project_id: Option<String>,
    /// `entry_hash` of the previous entry; `GENESIS_PREV_HASH` at
    /// index 0.
    pub prev_hash: [u8; 32],
    /// This entry's HMAC chain hash.
    pub entry_hash: [u8; 32],
}

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
            subject: "com.lunaris.files".into(),
            node_types: vec!["File".into()],
            relations: vec![],
            result_count: Some(7),
            duration_ms: Some(12),
            outcome: "ok".into(),
            depth: None,
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
        // A different actor.
        let other_actor = compute_entry_hash(
            key, 0, 1000, AuditKind::Query, "evil", &structural(),
            None, None, None, &GENESIS_PREV_HASH,
        );
        assert_ne!(base, other_actor);
        // A different prev_hash (the chain linkage).
        let other_prev = compute_entry_hash(
            key, 0, 1000, AuditKind::Query, "ai-daemon", &structural(),
            None, None, None, &[9u8; 32],
        );
        assert_ne!(base, other_prev);
        // A different index.
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

    #[test]
    fn kind_strings_are_stable_and_distinct() {
        let kinds = [
            AuditKind::Query,
            AuditKind::ToolCall,
            AuditKind::Confirm,
            AuditKind::PolicyViolation,
            AuditKind::GraphAccess,
            AuditKind::Permission,
        ];
        let mut seen = std::collections::HashSet::new();
        for k in kinds {
            assert!(seen.insert(k.as_str()), "kind strings must be distinct");
        }
    }
}

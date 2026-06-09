//! The durable revoke-set and op-count ledger (context-capsule.md §5-§6).
//!
//! Every capsule read is checked against this ledger by the serving component: a
//! revoked capsule is refused (revocation = no future read), and a capsule that
//! has been read its op-count bound is refused (a replay past the bound is stopped
//! by the holder of the counter, not by the grant alone). The ledger is keyed by a
//! capsule's revocation handle; `register` records it at mint, `revoke` makes every
//! future read refuse, and `consume` is the per-read gate that increments the
//! count.
//!
//! This module is the pure state + verdict core. The persistence (a flock'd file
//! that loads, applies one operation, and atomically writes back, so concurrent
//! reads serialize) wraps it as a sibling piece.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The durable per-capsule state.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapsuleState {
    /// Whether the capsule has been revoked (no future read is honoured).
    pub revoked: bool,
    /// How many reads have been served (checked against the grant's `max_ops`).
    pub ops_used: u64,
}

/// The outcome of a per-read check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsumeVerdict {
    /// The read is permitted; the op count was incremented.
    Allowed,
    /// The capsule is revoked; refuse.
    Revoked,
    /// The op-count bound is reached; refuse.
    Exhausted,
    /// No such capsule is registered; refuse (a read against an unknown handle).
    Unknown,
}

/// The ledger: capsule revocation handle → its durable state.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevocationLedger {
    entries: BTreeMap<String, CapsuleState>,
}

impl RevocationLedger {
    /// Record a freshly minted capsule (op count zero, not revoked). Idempotent:
    /// re-registering an existing handle leaves its state untouched, so a re-mint
    /// never resets an accrued count or un-revokes a revoked capsule.
    pub fn register(&mut self, handle: &str) {
        self.entries.entry(handle.to_string()).or_default();
    }

    /// Revoke a capsule: every future read refuses. Inserts a revoked entry if the
    /// handle is unknown, so a pre-emptive revoke is honoured. Terminal: a revoked
    /// capsule cannot be un-revoked here.
    pub fn revoke(&mut self, handle: &str) {
        self.entries.entry(handle.to_string()).or_default().revoked = true;
    }

    /// The per-read gate: returns the verdict and, on [`ConsumeVerdict::Allowed`],
    /// increments the op count. An unknown or revoked handle, or one already at its
    /// `max_ops`, is refused without incrementing.
    pub fn consume(&mut self, handle: &str, max_ops: u64) -> ConsumeVerdict {
        let Some(state) = self.entries.get_mut(handle) else {
            return ConsumeVerdict::Unknown;
        };
        if state.revoked {
            return ConsumeVerdict::Revoked;
        }
        if state.ops_used >= max_ops {
            return ConsumeVerdict::Exhausted;
        }
        state.ops_used += 1;
        ConsumeVerdict::Allowed
    }

    /// The state of a handle, for inspection / a mint-or-revoke surface.
    pub fn state(&self, handle: &str) -> Option<&CapsuleState> {
        self.entries.get(handle)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unknown_handle_is_refused() {
        let mut l = RevocationLedger::default();
        assert_eq!(l.consume("nope", 10), ConsumeVerdict::Unknown);
    }

    #[test]
    fn reads_are_allowed_up_to_the_bound_then_exhausted() {
        let mut l = RevocationLedger::default();
        l.register("h");
        assert_eq!(l.consume("h", 2), ConsumeVerdict::Allowed);
        assert_eq!(l.consume("h", 2), ConsumeVerdict::Allowed);
        assert_eq!(l.consume("h", 2), ConsumeVerdict::Exhausted);
        // A refused read does not increment further.
        assert_eq!(l.state("h").unwrap().ops_used, 2);
    }

    #[test]
    fn a_revoked_capsule_is_refused_even_with_ops_left() {
        let mut l = RevocationLedger::default();
        l.register("h");
        assert_eq!(l.consume("h", 10), ConsumeVerdict::Allowed);
        l.revoke("h");
        assert_eq!(l.consume("h", 10), ConsumeVerdict::Revoked);
        // Revocation is terminal: it stays refused.
        assert_eq!(l.consume("h", 10), ConsumeVerdict::Revoked);
    }

    #[test]
    fn register_is_idempotent_and_preserves_state() {
        let mut l = RevocationLedger::default();
        l.register("h");
        let _ = l.consume("h", 10);
        l.revoke("h");
        // A re-register (e.g. a re-mint with the same handle) must not reset the
        // count or un-revoke.
        l.register("h");
        assert_eq!(l.state("h").unwrap().ops_used, 1);
        assert!(l.state("h").unwrap().revoked);
    }

    #[test]
    fn a_preemptive_revoke_inserts_a_revoked_entry() {
        let mut l = RevocationLedger::default();
        l.revoke("h");
        assert_eq!(l.consume("h", 10), ConsumeVerdict::Revoked);
    }
}

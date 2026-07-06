//! The TPM anchor for the audit ledger (`physical-boot-security-plan.md` §b): the
//! chain head is anchored to a TPM NV MONOTONIC counter so that truncating or
//! rolling back the log and rewriting the software `head.checkpoint` (both of them
//! same-uid-rewritable, the documented residual) leaves a DETECTABLE gap against
//! the hardware counter, which a same-uid attacker cannot decrement. The CMU
//! tamper-proof-logging pattern: advance and record the counter when sealing a
//! fresh checkpoint (at shutdown / each checkpoint write), read and compare it at
//! restart.
//!
//! The TPM operation sits behind the [`TpmAnchor`] seam so the daemon logic is
//! headless-testable with [`MockTpmAnchor`]; the real `tss-esapi` NV-counter impl
//! and the on-metal verify are the follow-up. This module is the buildable-now
//! software structure the plan calls "the one physical-boot piece that goes early".

use std::sync::atomic::{AtomicU64, Ordering};

/// A monotonic hardware counter (a TPM NV counter). It only ever advances; a
/// same-uid attacker cannot roll it back, which is the whole point of anchoring
/// the ledger head to it.
pub trait TpmAnchor: Send + Sync {
    /// The current counter value.
    fn read_counter(&self) -> std::io::Result<u64>;
    /// Advance the counter by one and return the new value. Called when sealing a
    /// fresh checkpoint so the recorded value tracks the latest seal.
    fn increment_counter(&self) -> std::io::Result<u64>;
}

/// The verdict of comparing the checkpoint's recorded counter against the live
/// hardware counter at restart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnchorVerdict {
    /// The recorded counter equals the hardware counter: this checkpoint is the
    /// most-recently-sealed one, so the log was not rolled back or truncated.
    Consistent,
    /// The hardware counter is AHEAD of the recorded one: an older log and
    /// checkpoint were restored (a rollback / truncation). The monotonic counter
    /// exposes it because it advanced past the restored value and cannot go back.
    RolledBack { recorded: u64, hardware: u64 },
    /// The recorded counter is AHEAD of the hardware counter: impossible for a
    /// monotonic counter that only this daemon advances, so the checkpoint was
    /// forged ahead (or points at a different TPM).
    Forged { recorded: u64, hardware: u64 },
}

/// Compare the checkpoint's recorded counter against the live hardware counter.
/// Pure, so the rollback / forgery logic is unit-tested without a TPM.
pub fn assess_anchor(recorded: u64, hardware: u64) -> AnchorVerdict {
    match recorded.cmp(&hardware) {
        std::cmp::Ordering::Equal => AnchorVerdict::Consistent,
        std::cmp::Ordering::Less => AnchorVerdict::RolledBack { recorded, hardware },
        std::cmp::Ordering::Greater => AnchorVerdict::Forged { recorded, hardware },
    }
}

/// An in-memory monotonic counter for tests. NOT a production anchor: the value
/// is lost on restart, so it provides no real rollback resistance. The daemon runs
/// with no anchor (the current behaviour) until the real `tss-esapi` NV-counter
/// impl lands and is verified on metal.
pub struct MockTpmAnchor {
    counter: AtomicU64,
}

impl MockTpmAnchor {
    /// A mock counter starting at `start`.
    pub fn new(start: u64) -> Self {
        Self {
            counter: AtomicU64::new(start),
        }
    }
}

impl TpmAnchor for MockTpmAnchor {
    fn read_counter(&self) -> std::io::Result<u64> {
        Ok(self.counter.load(Ordering::SeqCst))
    }

    fn increment_counter(&self) -> std::io::Result<u64> {
        // fetch_add returns the PREVIOUS value; the new value is +1.
        Ok(self.counter.fetch_add(1, Ordering::SeqCst) + 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assess_is_consistent_when_the_counters_match() {
        assert_eq!(assess_anchor(7, 7), AnchorVerdict::Consistent);
    }

    #[test]
    fn assess_flags_a_rollback_when_hardware_is_ahead() {
        // An old checkpoint (counter 3) is restored, but the hardware counter has
        // advanced to 5 - the truncation is detectable.
        assert_eq!(
            assess_anchor(3, 5),
            AnchorVerdict::RolledBack {
                recorded: 3,
                hardware: 5
            }
        );
    }

    #[test]
    fn assess_flags_a_forgery_when_the_recorded_value_is_impossible() {
        // A checkpoint claiming a counter the monotonic hardware never reached.
        assert_eq!(
            assess_anchor(9, 4),
            AnchorVerdict::Forged {
                recorded: 9,
                hardware: 4
            }
        );
    }

    #[test]
    fn mock_counter_advances_monotonically() {
        let a = MockTpmAnchor::new(0);
        assert_eq!(a.read_counter().unwrap(), 0);
        assert_eq!(a.increment_counter().unwrap(), 1);
        assert_eq!(a.increment_counter().unwrap(), 2);
        assert_eq!(a.read_counter().unwrap(), 2);
    }
}

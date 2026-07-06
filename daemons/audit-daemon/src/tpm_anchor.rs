//! TPM-anchor scaffolding for the audit ledger (`physical-boot-security-plan.md`
//! §b). **INCOMPLETE - inert scaffolding only; this counter check does NOT close
//! the same-uid truncate-and-rewrite residual on its own (adversarial review,
//! 7 July).** The idea is to anchor the chain head to a TPM NV monotonic counter
//! so a rollback of the log + the same-uid-rewritable `head.checkpoint` is
//! detectable at restart.
//!
//! Why the bare-counter compare here is insufficient: the checkpoint file is
//! same-uid-writable and the hardware counter is same-uid-READABLE, so an attacker
//! erasing the tail (a) reads the current hardware value `H`, (b) truncates the
//! log to a genuine earlier head, (c) writes a forged checkpoint recording `H` for
//! that head. At restart the chain still verifies and `recorded == hardware`, so
//! [`assess_with_anchor`] reports `Consistent` - a MISS. A monotonic counter only
//! catches a *decrement* (a lazy attacker who left an old, lower counter, which
//! the plain checkpoint already alarms on via the head hash); the attacker COPIES
//! the current value instead. The counter is not bound to the log content.
//!
//! The sound mechanism binds the head to the hardware: a **TPM-signed attestation
//! over `(head_index, head_hash)`** (a `TPM2_Quote` / NV-signed tuple), which a
//! same-uid attacker cannot forge for a truncated head because the signing key is
//! TPM-held. That is a redesign of this seam (from a bare counter to an
//! `attest(head)`/`verify` pair) and is the real follow-up; the counter type here
//! is retained only as inert scaffolding (the daemon default is no anchor) until
//! that redesign lands. Do NOT wire a real anchor against this check and trust it.
//!
//! Also open (adversarial review): a benign crash between the append-time counter
//! increment and the checkpoint write false-flags `Tampered` (breaks crash-ahead
//! recovery); first-enabling a real anchor on an existing ledger false-flags a
//! rollback (`stored == 0` via serde-default vs a non-zero fresh NV counter); and
//! a TPM read error fails open (a same-uid attacker who induces read errors strips
//! the check). All three must be resolved in the redesign.

use std::sync::atomic::{AtomicU64, Ordering};

use crate::checkpoint::StartupCheck;

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

/// Layer the counter verdict onto the checkpoint's startup check: a `Consistent`
/// check becomes `Tampered` when the recorded counter is BELOW the hardware
/// counter (`RolledBack`) or above it (`Forged`). **This catches only a lazy
/// rollback that left an old, lower counter; it MISSES a same-uid attacker who
/// copies the current hardware value into a forged checkpoint - see the module
/// doc.** It also currently false-flags a benign crash-ahead and a first-anchor-
/// enable (module doc). `Genesis` and an already-`Tampered` check pass through.
/// Retained as inert scaffolding pending the signed-attestation redesign.
pub fn assess_with_anchor(
    check: StartupCheck,
    recorded_counter: u64,
    hardware_counter: u64,
) -> StartupCheck {
    match check {
        StartupCheck::Consistent => match assess_anchor(recorded_counter, hardware_counter) {
            AnchorVerdict::Consistent => StartupCheck::Consistent,
            AnchorVerdict::RolledBack { recorded, hardware } => StartupCheck::Tampered {
                detail: format!(
                    "ledger head rolled back: checkpoint counter {recorded} is behind \
                     the TPM counter {hardware}"
                ),
            },
            AnchorVerdict::Forged { recorded, hardware } => StartupCheck::Tampered {
                detail: format!(
                    "ledger head counter forged: checkpoint counter {recorded} exceeds \
                     the TPM counter {hardware}"
                ),
            },
        },
        other => other,
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
    fn assess_with_anchor_passes_a_matched_consistent_check() {
        assert_eq!(
            assess_with_anchor(StartupCheck::Consistent, 5, 5),
            StartupCheck::Consistent
        );
    }

    #[test]
    fn assess_with_anchor_escalates_a_consistent_check_on_rollback_or_forgery() {
        // Software says Consistent, but the hardware counter is ahead -> rollback.
        assert!(matches!(
            assess_with_anchor(StartupCheck::Consistent, 3, 5),
            StartupCheck::Tampered { .. }
        ));
        // Recorded counter exceeds the monotonic hardware -> forgery.
        assert!(matches!(
            assess_with_anchor(StartupCheck::Consistent, 9, 4),
            StartupCheck::Tampered { .. }
        ));
    }

    #[test]
    fn assess_with_anchor_passes_genesis_and_tampered_through() {
        assert_eq!(
            assess_with_anchor(StartupCheck::Genesis, 0, 7),
            StartupCheck::Genesis
        );
        let t = StartupCheck::Tampered {
            detail: "already tampered".into(),
        };
        assert_eq!(assess_with_anchor(t.clone(), 5, 5), t);
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

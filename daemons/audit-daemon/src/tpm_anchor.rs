//! TPM anchor for the audit ledger (`physical-boot-security-plan.md` §b): bind
//! the chain head to hardware so a same-uid truncate-and-rewrite is detectable
//! at restart.
//!
//! # Why this is an attestation and not a counter
//!
//! The first design compared a TPM NV monotonic counter recorded in the
//! checkpoint against the live hardware counter. Adversarial review (7 July)
//! showed that does not hold: the checkpoint file is same-uid-writable and the
//! hardware counter is same-uid-READABLE, so an attacker erasing the tail can
//! (a) read the current hardware value `H`, (b) truncate the log to a genuine
//! earlier head, (c) write a forged checkpoint recording `H` for that head. The
//! chain still verifies and `recorded == hardware`, so the check reports
//! consistent. The counter only ever caught a LAZY attacker who left an old,
//! lower value behind. Its fundamental flaw is that the value is not bound to
//! the log content: any head can claim any counter.
//!
//! An attestation is bound to the content. The TPM signs the tuple
//! `(head_index, head_hash)` with a key it holds, so a forged checkpoint for a
//! truncated head carries a signature the TPM never produced, and a same-uid
//! attacker cannot produce one. Replaying the OLD attestation does not help
//! either: it names the old head, which no longer matches the truncated log.
//!
//! # What the shape change also fixed
//!
//! Three defects the review left open fall out of dropping the counter:
//!
//! - **Crash-ahead false alarm.** The counter had to be advanced at append time,
//!   separately from the checkpoint write, so a crash between the two left a
//!   recorded value that no longer matched and false-flagged tampering. There is
//!   no separate step now: the attestation is produced over the head being
//!   sealed and written with it. A crash between append and checkpoint leaves an
//!   attestation over an earlier head, which still verifies, and the existing
//!   crash-ahead reseed handles the rest.
//! - **First-enable false alarm.** A pre-anchor ledger has no attestation.
//!   [`AttestationCheck::Absent`] is distinct from `Invalid`, so enabling an
//!   anchor on an existing ledger is admitted (and sealed on the next write)
//!   rather than read as a rollback.
//! - **Read errors failing open.** A TPM that cannot be consulted was previously
//!   left to the software verdict, so an attacker who could induce read errors
//!   stripped the check. [`AttestationCheck::Unavailable`] now fails CLOSED: a
//!   configured anchor that cannot answer freezes ingest.
//!
//! The daemon still runs with no anchor by default; a real `tss-esapi`
//! implementation of [`TpmAnchor`] is the remaining piece and needs a machine
//! with a TPM to verify.

use std::sync::atomic::{AtomicU64, Ordering};

use crate::checkpoint::StartupCheck;

/// A hardware root of trust that can vouch for a specific ledger head.
///
/// Both halves take the head explicitly: an attestation is only ever meaningful
/// for the `(index, entry_hash_hex)` it was produced over, and accepting one for
/// a different head is exactly the forgery this exists to stop.
pub trait TpmAnchor: Send + Sync {
    /// Produce an attestation binding this TPM to `(index, entry_hash_hex)`.
    /// Called when sealing a checkpoint, and written with it.
    fn attest(&self, index: u64, entry_hash_hex: &str) -> std::io::Result<Vec<u8>>;

    /// Whether `attestation` is one this TPM produced for exactly this head.
    /// `Ok(false)` is a definite no; an `Err` means the TPM could not be
    /// consulted, which the caller must treat as failure, not as absence.
    fn verify(
        &self,
        index: u64,
        entry_hash_hex: &str,
        attestation: &[u8],
    ) -> std::io::Result<bool>;
}

/// What the TPM said about the attestation stored in the checkpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttestationCheck {
    /// The TPM confirms it produced this attestation for this exact head.
    Valid,
    /// The TPM does not recognise this attestation for this head: either the
    /// head was changed under a replayed attestation, or the attestation was
    /// forged. Both are tampering.
    Invalid,
    /// The checkpoint carries no attestation at all - a ledger written before
    /// the anchor was enabled.
    Absent,
    /// The TPM could not be consulted.
    Unavailable {
        /// Why, for the log and the `audit.tampered` payload.
        detail: String,
    },
}

/// Layer the attestation verdict onto the checkpoint's software startup check.
///
/// `anchor_required` is the policy knob for [`AttestationCheck::Absent`]. It is
/// false while migrating a pre-anchor ledger (the missing attestation is
/// expected and the next seal writes one) and true once every checkpoint is
/// expected to carry one, at which point a stripped attestation is tampering
/// rather than history - otherwise removing it would be a downgrade attack.
///
/// `Genesis` and an already-`Tampered` check pass through: there is nothing for
/// hardware to add to an empty ledger, and a software verdict of tampering is
/// not something hardware can clear.
pub fn assess_with_attestation(
    check: StartupCheck,
    attestation: AttestationCheck,
    anchor_required: bool,
) -> StartupCheck {
    let StartupCheck::Consistent = check else {
        return check;
    };
    match attestation {
        AttestationCheck::Valid => StartupCheck::Consistent,
        AttestationCheck::Invalid => StartupCheck::Tampered {
            detail: "ledger head attestation does not match: the TPM did not sign \
                     this head, so the log or the checkpoint was rewritten"
                .to_string(),
        },
        AttestationCheck::Absent if !anchor_required => StartupCheck::Consistent,
        AttestationCheck::Absent => StartupCheck::Tampered {
            detail: "ledger head attestation is missing though the anchor is \
                     required: the checkpoint was written without hardware \
                     backing or the attestation was stripped"
                .to_string(),
        },
        // Fail CLOSED. Leaving the software verdict alone would let anyone who
        // can make the TPM unreadable turn the anchor off.
        AttestationCheck::Unavailable { detail } => StartupCheck::Tampered {
            detail: format!("ledger head attestation could not be verified: {detail}"),
        },
    }
}

/// Ask `anchor` about the attestation a checkpoint carried, mapping both
/// emptiness and TPM errors onto [`AttestationCheck`] so the caller never has to
/// decide what an error means.
pub fn check_attestation(
    anchor: &dyn TpmAnchor,
    index: u64,
    entry_hash_hex: &str,
    attestation: &[u8],
) -> AttestationCheck {
    if attestation.is_empty() {
        return AttestationCheck::Absent;
    }
    match anchor.verify(index, entry_hash_hex, attestation) {
        Ok(true) => AttestationCheck::Valid,
        Ok(false) => AttestationCheck::Invalid,
        Err(e) => AttestationCheck::Unavailable {
            detail: e.to_string(),
        },
    }
}

/// An in-process stand-in for a TPM. NOT a production anchor: the "signature" is
/// a plain tag over the head with a per-instance secret held in this process's
/// memory, so it is forgeable by anything that can read it and is lost on
/// restart. It exists so the verdict logic can be exercised without hardware,
/// and it is faithful in the one way that matters for those tests: an
/// attestation only verifies against the head it was produced over.
pub struct MockTpmAnchor {
    secret: u64,
    /// Counts attestations produced, so a test can assert sealing happened.
    sealed: AtomicU64,
}

impl MockTpmAnchor {
    /// A mock anchor holding `secret` as its stand-in signing key.
    pub fn new(secret: u64) -> Self {
        Self {
            secret,
            sealed: AtomicU64::new(0),
        }
    }

    /// How many attestations this anchor has produced.
    pub fn sealed_count(&self) -> u64 {
        self.sealed.load(Ordering::SeqCst)
    }

    fn tag(&self, index: u64, entry_hash_hex: &str) -> Vec<u8> {
        format!("mock:{}:{index}:{entry_hash_hex}", self.secret).into_bytes()
    }
}

impl TpmAnchor for MockTpmAnchor {
    fn attest(&self, index: u64, entry_hash_hex: &str) -> std::io::Result<Vec<u8>> {
        self.sealed.fetch_add(1, Ordering::SeqCst);
        Ok(self.tag(index, entry_hash_hex))
    }

    fn verify(
        &self,
        index: u64,
        entry_hash_hex: &str,
        attestation: &[u8],
    ) -> std::io::Result<bool> {
        Ok(attestation == self.tag(index, entry_hash_hex).as_slice())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_attestation_only_verifies_against_the_head_it_was_made_for() {
        // The property the whole redesign rests on. The counter it replaced had
        // no equivalent: any head could claim any counter value, which is why an
        // attacker could copy the live value into a forged checkpoint.
        let a = MockTpmAnchor::new(42);
        let sig = a.attest(7, "abc123").unwrap();
        assert!(a.verify(7, "abc123", &sig).unwrap());
        // Truncated to an earlier index, replaying the same attestation.
        assert!(!a.verify(5, "abc123", &sig).unwrap());
        // Same index, different entry - the log was rewritten under it.
        assert!(!a.verify(7, "def456", &sig).unwrap());
        // A different TPM cannot vouch for this head.
        assert!(!MockTpmAnchor::new(43).verify(7, "abc123", &sig).unwrap());
    }

    #[test]
    fn a_truncate_and_rewrite_is_caught_where_the_counter_missed_it() {
        // The exact attack the review documented as a MISS: the attacker
        // truncates to a genuine earlier head and writes a checkpoint for it.
        // Against a counter they simply copied the live value and passed. They
        // cannot produce an attestation for the head they truncated to.
        let tpm = MockTpmAnchor::new(1);
        let genuine = tpm.attest(9, "head-at-9").unwrap();
        // Attacker rolls the log back to index 4 and reuses what they have.
        let verdict = check_attestation(&tpm, 4, "head-at-4", &genuine);
        assert_eq!(verdict, AttestationCheck::Invalid);
        assert!(matches!(
            assess_with_attestation(StartupCheck::Consistent, verdict, true),
            StartupCheck::Tampered { .. }
        ));
    }

    #[test]
    fn a_valid_attestation_leaves_a_consistent_check_alone() {
        let tpm = MockTpmAnchor::new(1);
        let sig = tpm.attest(3, "h").unwrap();
        assert_eq!(check_attestation(&tpm, 3, "h", &sig), AttestationCheck::Valid);
        assert_eq!(
            assess_with_attestation(StartupCheck::Consistent, AttestationCheck::Valid, true),
            StartupCheck::Consistent
        );
    }

    #[test]
    fn enabling_the_anchor_on_an_existing_ledger_is_not_a_rollback() {
        // A pre-anchor checkpoint carries no attestation. Under the counter this
        // read as `stored == 0` against a non-zero fresh NV counter and
        // false-flagged a rollback on the very first run with the anchor on.
        let tpm = MockTpmAnchor::new(1);
        assert_eq!(check_attestation(&tpm, 3, "h", &[]), AttestationCheck::Absent);
        assert_eq!(
            assess_with_attestation(StartupCheck::Consistent, AttestationCheck::Absent, false),
            StartupCheck::Consistent
        );
    }

    #[test]
    fn a_stripped_attestation_is_tampering_once_the_anchor_is_required() {
        // The other side of the migration knob: once every checkpoint is
        // expected to be attested, deleting the attestation must not be a way to
        // turn the check off.
        assert!(matches!(
            assess_with_attestation(StartupCheck::Consistent, AttestationCheck::Absent, true),
            StartupCheck::Tampered { .. }
        ));
    }

    #[test]
    fn an_unreachable_tpm_fails_closed() {
        // Previously a read error left the software verdict untouched, so
        // inducing TPM errors stripped the anchor. A configured anchor that
        // cannot answer must freeze ingest instead.
        let unavailable = AttestationCheck::Unavailable {
            detail: "device busy".to_string(),
        };
        assert!(matches!(
            assess_with_attestation(StartupCheck::Consistent, unavailable.clone(), true),
            StartupCheck::Tampered { .. }
        ));
        // Not even during migration, where absence is tolerated but silence is
        // still not an answer.
        assert!(matches!(
            assess_with_attestation(StartupCheck::Consistent, unavailable, false),
            StartupCheck::Tampered { .. }
        ));
    }

    #[test]
    fn genesis_and_an_existing_tampered_verdict_pass_through() {
        assert_eq!(
            assess_with_attestation(StartupCheck::Genesis, AttestationCheck::Absent, true),
            StartupCheck::Genesis
        );
        let tampered = StartupCheck::Tampered {
            detail: "already tampered".into(),
        };
        assert_eq!(
            assess_with_attestation(tampered.clone(), AttestationCheck::Valid, true),
            tampered
        );
    }

    #[test]
    fn the_mock_records_that_it_sealed() {
        let a = MockTpmAnchor::new(0);
        assert_eq!(a.sealed_count(), 0);
        a.attest(1, "h").unwrap();
        a.attest(2, "h").unwrap();
        assert_eq!(a.sealed_count(), 2);
    }
}

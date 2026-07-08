//! The system monitor's sovereign-overlay backend (`system-monitor-plan.md`): the
//! Arlen-original reads that make the monitor a SOVEREIGNTY window rather than a
//! resource gauge - the audit-ledger integrity verdict, the daemon-health verdict,
//! and per-app access aggregation over Arlen's own event bus and audit ledger. This
//! is the half with no GPL question (the resource-metrics half - CPU/GPU/SMART,
//! adjacent to GPLv3 tools - is a separate, later track).
//!
//! Every read here is itself a principal in the same audit ledger it inspects, and
//! reads Arlen's own capability-scoped contracts - nothing here is an ambient
//! `/proc` grab.

/// The daemon-health verdict: a liveness probe over the core Arlen daemons.
pub mod health;

/// Per-app access aggregation over the audit ledger's activity (the sovereign lens).
pub mod access;

use audit_proto::{ReadClient, ReadClientError, ReadPage};

/// The audit ledger's integrity state - the monitor's "nothing was altered"
/// verdict (`system-monitor-plan.md` rank 4). Never inferred: a tamper is only ever
/// what the audit daemon itself reports over its hash-chained ledger.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntegrityVerdict {
    /// The ledger verifies - its append-only hash chain is intact. `entry_count`
    /// is the total number of audited events (the ledger head).
    Verified {
        /// The total audited-event count (the ledger head).
        entry_count: u64,
    },
    /// The audit daemon reports its ledger as TAMPERED - the append-only chain
    /// broke. The alarm state; surfaced, never guessed.
    Tampered,
    /// The verdict could not be obtained (the audit daemon was unreachable, timed
    /// out, or rejected the read). Fail-closed: the absence of a verdict is NOT
    /// "verified" - the monitor shows "cannot verify", never a false "verified".
    Unavailable {
        /// A stable, content-free reason for the missing verdict.
        reason: String,
    },
}

/// Map an audit [`ReadPage`] to the integrity verdict (pure). A daemon-reported
/// tamper wins outright; otherwise the ledger verifies and its head is the count.
pub fn integrity_verdict(page: &ReadPage) -> IntegrityVerdict {
    if page.tampered {
        IntegrityVerdict::Tampered
    } else {
        IntegrityVerdict::Verified { entry_count: page.head }
    }
}

/// Read the audit ledger's integrity verdict over `client`. One small read carries
/// the daemon's tamper status and head regardless of how many entries it returns,
/// so this fetches at most one. A read failure fails CLOSED to
/// [`IntegrityVerdict::Unavailable`].
pub async fn audit_integrity(client: &ReadClient) -> IntegrityVerdict {
    match client.read(0, u64::MAX, 1, None).await {
        Ok(page) => integrity_verdict(&page),
        Err(e) => IntegrityVerdict::Unavailable { reason: unavailable_reason(&e) },
    }
}

/// A stable, content-free reason string for an unavailable verdict (no ledger
/// content, no daemon internals - just why the read did not resolve).
fn unavailable_reason(e: &ReadClientError) -> String {
    match e {
        ReadClientError::Transport(_) => "audit daemon unreachable".to_string(),
        ReadClientError::Server(_) => "audit daemon rejected the read".to_string(),
        ReadClientError::Timeout => "audit daemon timed out".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(tampered: bool, head: u64) -> ReadPage {
        ReadPage { entries: Vec::new(), tampered, head }
    }

    #[test]
    fn clean_ledger_verifies_with_the_head_count() {
        assert_eq!(
            integrity_verdict(&page(false, 42)),
            IntegrityVerdict::Verified { entry_count: 42 }
        );
    }

    #[test]
    fn an_empty_clean_ledger_is_verified_at_zero() {
        assert_eq!(
            integrity_verdict(&page(false, 0)),
            IntegrityVerdict::Verified { entry_count: 0 }
        );
    }

    #[test]
    fn a_reported_tamper_wins_outright() {
        assert_eq!(integrity_verdict(&page(true, 42)), IntegrityVerdict::Tampered);
        // Tamper wins even when the head is zero (a truncated-to-empty ledger).
        assert_eq!(integrity_verdict(&page(true, 0)), IntegrityVerdict::Tampered);
    }

    #[test]
    fn unavailable_reasons_are_content_free_and_stable() {
        assert_eq!(unavailable_reason(&ReadClientError::Timeout), "audit daemon timed out");
        assert_eq!(
            unavailable_reason(&ReadClientError::Transport("boom".into())),
            "audit daemon unreachable"
        );
        assert_eq!(
            unavailable_reason(&ReadClientError::Server("nope".into())),
            "audit daemon rejected the read"
        );
    }
}

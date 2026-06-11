//! The transfer gate: the single decision surface (profile-system-plan.md, Decided 4-5).
//!
//! [`decide_transfer`] composes the three independent checks a transfer must
//! pass, each fail-closed:
//! 1. CALLER ADMISSION - is the requesting process allowed to ask for a transfer
//!    at all ([`crate::auth::caller_is_admitted`])? An unadmitted caller is
//!    refused before anything else.
//! 2. REQUEST VALIDITY - is the request well-formed
//!    ([`crate::request::TransferRequest::validate`])? A malformed request is
//!    refused.
//! 3. POLICY - does the directional, default-deny, Locked-aware policy permit
//!    `(source, dest, type)` ([`crate::policy::decide`])?
//!
//! On an `allow`, the gate writes the `allowed` decision to BOTH profiles'
//! ledgers BEFORE returning an approval (audit-before-act); if either ledger
//! fails, the transfer is refused. A denied attempt (by policy) is also audited
//! as `denied`. The only way to obtain an [`ApprovedTransfer`] - the token the
//! broker's `deliver` requires - is a successful, audited pass through this
//! gate, so an ungated transfer can never reach a delivery.
//!
//! Caller admission and request validity are audited only as a `denied`
//! best-effort when they fail; the policy decision is the always-recorded
//! dual-ledger event. The daemon (not this fn) re-checks the peer is still alive
//! per request (the PID-reuse close) before calling the gate.

use crate::audit::{outcome, DualLedger};
use crate::broker::ApprovedTransfer;
use crate::policy::{decide, ProfileRef, TransferPolicy, Verdict};
use crate::request::TransferRequest;

/// The outcome of a gate decision.
#[derive(Debug)]
pub enum GateDecision {
    /// The transfer is approved and recorded in both ledgers; the broker may
    /// deliver this exact approval.
    Approved(ApprovedTransfer),
    /// The transfer is refused. The `reason` is a coarse, content-free label for
    /// the daemon's log.
    Refused {
        /// Why the gate refused (caller, request, policy, or audit).
        reason: RefusalReason,
    },
}

/// Why the gate refused a transfer. Coarse and content-free.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefusalReason {
    /// The requesting process is not on the requester allowlist.
    CallerNotAdmitted,
    /// The request was malformed (empty/NUL/oversized payload).
    InvalidRequest,
    /// The Locked-resolved profile refs did not name the same profiles the
    /// request body does, so deciding on the refs would approve/audit a
    /// different direction than the request describes. A wiring fault, refused.
    InconsistentRequest,
    /// The directional policy denied the flow (or a Locked profile is involved).
    PolicyDenied,
    /// The dual-ledger audit could not record the decision, so the transfer is
    /// refused fail-closed (foundation §8.4.6: no un-audited flow).
    AuditUnavailable,
}

impl RefusalReason {
    /// A coarse log label.
    pub fn as_str(self) -> &'static str {
        match self {
            RefusalReason::CallerNotAdmitted => "caller-not-admitted",
            RefusalReason::InvalidRequest => "invalid-request",
            RefusalReason::InconsistentRequest => "inconsistent-request",
            RefusalReason::PolicyDenied => "policy-denied",
            RefusalReason::AuditUnavailable => "audit-unavailable",
        }
    }
}

/// Decide one transfer end-to-end, audit-before-act.
///
/// `caller_app_id` is the resolved identity of the requesting process (from
/// `SO_PEERCRED + path_to_app_id`); the daemon also binds `request.source` to
/// the socket the request arrived on, so `source` is not caller-forgeable. The
/// `source` and `dest` profile refs carry the Locked flag resolved from the
/// profile registry.
///
/// Order is fail-closed:
/// - An unadmitted caller is refused immediately (no policy read, no audit -
///   the caller has no standing to record an attempt).
/// - A malformed request is refused.
/// - The policy decision is then taken; both `allow` and `deny` are audited to
///   BOTH ledgers. On `deny`, refuse with `PolicyDenied`. On `allow`, return an
///   approval ONLY after the dual-ledger `allowed` write succeeds; if it fails,
///   refuse with `AuditUnavailable`.
pub async fn decide_transfer(
    caller_app_id: &str,
    request: &TransferRequest,
    source: &ProfileRef<'_>,
    dest: &ProfileRef<'_>,
    policy: &TransferPolicy,
    ledger: &DualLedger,
) -> GateDecision {
    // 1. Caller admission.
    if !crate::auth::caller_is_admitted(caller_app_id) {
        return GateDecision::Refused {
            reason: RefusalReason::CallerNotAdmitted,
        };
    }

    // 2. Request validity.
    if request.validate().is_err() {
        return GateDecision::Refused {
            reason: RefusalReason::InvalidRequest,
        };
    }

    // 2b. Internal consistency: the Locked-resolved profile refs the policy is
    // decided on MUST name the same profiles the request body carries (the
    // daemon resolves `source`/`dest` from `request.source`/`request.dest`). A
    // mismatch would let the policy be decided on one direction while the
    // minted approval and the audit record describe another, the direction
    // bypass; refuse it (a wiring fault, no standing to audit a real attempt).
    if source.id != &request.source || dest.id != &request.dest {
        return GateDecision::Refused {
            reason: RefusalReason::InconsistentRequest,
        };
    }

    // 3. Policy decision (directional, default-deny, Locked-aware).
    let verdict = decide(policy, source, dest, request.ty);
    let recorded = match verdict {
        Verdict::Allow => outcome::ALLOWED,
        Verdict::Deny => outcome::DENIED,
    };

    // Audit-before-act, both-must-succeed. The decision (allow or deny) is the
    // always-recorded dual-ledger event.
    if ledger
        .record(&request.source, &request.dest, request.ty, recorded)
        .await
        .is_err()
    {
        return GateDecision::Refused {
            reason: RefusalReason::AuditUnavailable,
        };
    }

    match verdict {
        Verdict::Deny => GateDecision::Refused {
            reason: RefusalReason::PolicyDenied,
        },
        // The approval is minted ONLY here, after the allow decision was
        // recorded in both ledgers.
        Verdict::Allow => GateDecision::Approved(ApprovedTransfer::new(request.clone())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request::{PayloadRef, ProfileId, TransferType};
    use audit_proto::MockAuditSink;
    use std::sync::Arc;

    // A debug-build admitted caller (the dev requester id), so the auth gate
    // passes in the test build.
    const ADMITTED_CALLER: &str = "desktop-shell";

    fn pid(name: &str) -> ProfileId {
        ProfileId::new(name).expect("valid test profile id")
    }

    fn request(source: &str, dest: &str, ty: TransferType) -> TransferRequest {
        TransferRequest {
            source: pid(source),
            dest: pid(dest),
            ty,
            payload: match ty {
                TransferType::Clipboard => PayloadRef::Clipboard {
                    handle: "sel-1".into(),
                },
                TransferType::File => PayloadRef::File {
                    source_path: "/home/work/report.pdf".into(),
                },
            },
        }
    }

    fn allow_policy(source: &str, dest: &str, ty: TransferType) -> TransferPolicy {
        TransferPolicy {
            rules: vec![crate::policy::TransferRule {
                source: pid(source),
                dest: pid(dest),
                ty,
                allow: true,
            }],
        }
    }

    fn ledger() -> (DualLedger, Arc<MockAuditSink>, Arc<MockAuditSink>) {
        let source = Arc::new(MockAuditSink::accepting());
        let dest = Arc::new(MockAuditSink::accepting());
        (DualLedger::new(source.clone(), dest.clone()), source, dest)
    }

    #[tokio::test]
    async fn an_admitted_valid_allowed_transfer_is_approved_and_audited() {
        let req = request("work", "personal", TransferType::File);
        let policy = allow_policy("work", "personal", TransferType::File);
        let (ledger, src_sink, dst_sink) = ledger();
        let s = pid("work");
        let d = pid("personal");
        let decision = decide_transfer(
            ADMITTED_CALLER,
            &req,
            &ProfileRef::unlocked(&s),
            &ProfileRef::unlocked(&d),
            &policy,
            &ledger,
        )
        .await;
        assert!(matches!(decision, GateDecision::Approved(_)));
        // The allow decision was recorded in BOTH ledgers before the approval.
        assert_eq!(src_sink.count().await, 1);
        assert_eq!(dst_sink.count().await, 1);
        assert_eq!(src_sink.recorded().await[0].structural.outcome, "allowed");
    }

    #[tokio::test]
    async fn an_unadmitted_caller_is_refused_before_audit() {
        let req = request("work", "personal", TransferType::File);
        let policy = allow_policy("work", "personal", TransferType::File);
        let (ledger, src_sink, _dst) = ledger();
        let s = pid("work");
        let d = pid("personal");
        let decision = decide_transfer(
            "com.example.evil",
            &req,
            &ProfileRef::unlocked(&s),
            &ProfileRef::unlocked(&d),
            &policy,
            &ledger,
        )
        .await;
        assert!(matches!(
            decision,
            GateDecision::Refused {
                reason: RefusalReason::CallerNotAdmitted
            }
        ));
        // An unadmitted caller has no standing to record an attempt.
        assert_eq!(src_sink.count().await, 0);
    }

    #[tokio::test]
    async fn a_policy_denied_transfer_is_refused_but_audited() {
        let req = request("personal", "work", TransferType::File);
        // The allow rule is for work->personal; personal->work is default-deny.
        let policy = allow_policy("work", "personal", TransferType::File);
        let (ledger, src_sink, dst_sink) = ledger();
        let s = pid("personal");
        let d = pid("work");
        let decision = decide_transfer(
            ADMITTED_CALLER,
            &req,
            &ProfileRef::unlocked(&s),
            &ProfileRef::unlocked(&d),
            &policy,
            &ledger,
        )
        .await;
        assert!(matches!(
            decision,
            GateDecision::Refused {
                reason: RefusalReason::PolicyDenied
            }
        ));
        // The denied attempt is recorded in both ledgers.
        assert_eq!(src_sink.recorded().await[0].structural.outcome, "denied");
        assert_eq!(dst_sink.count().await, 1);
    }

    #[tokio::test]
    async fn a_locked_profile_is_refused_even_with_an_allow_rule() {
        let req = request("exam", "personal", TransferType::File);
        let policy = allow_policy("exam", "personal", TransferType::File);
        let (ledger, _src, _dst) = ledger();
        let s = pid("exam");
        let d = pid("personal");
        let decision = decide_transfer(
            ADMITTED_CALLER,
            &req,
            &ProfileRef::locked(&s),
            &ProfileRef::unlocked(&d),
            &policy,
            &ledger,
        )
        .await;
        assert!(matches!(
            decision,
            GateDecision::Refused {
                reason: RefusalReason::PolicyDenied
            }
        ));
    }

    #[tokio::test]
    async fn a_down_ledger_refuses_the_transfer_fail_closed() {
        let req = request("work", "personal", TransferType::File);
        let policy = allow_policy("work", "personal", TransferType::File);
        // The source ledger is down: an otherwise-allowed transfer is refused.
        let source_down = Arc::new(MockAuditSink::failing());
        let dest = Arc::new(MockAuditSink::accepting());
        let ledger = DualLedger::new(source_down, dest);
        let s = pid("work");
        let d = pid("personal");
        let decision = decide_transfer(
            ADMITTED_CALLER,
            &req,
            &ProfileRef::unlocked(&s),
            &ProfileRef::unlocked(&d),
            &policy,
            &ledger,
        )
        .await;
        assert!(matches!(
            decision,
            GateDecision::Refused {
                reason: RefusalReason::AuditUnavailable
            }
        ));
    }

    #[tokio::test]
    async fn a_request_disagreeing_with_the_profile_refs_is_refused() {
        // The request body says personal->work, but the resolved refs say
        // work->personal (which has an allow rule). The gate must NOT approve the
        // ref direction while the request describes the opposite: it refuses the
        // inconsistency before the policy is even read, so no divergent approval
        // or audit is minted.
        let req = request("personal", "work", TransferType::File);
        let policy = allow_policy("work", "personal", TransferType::File);
        let (ledger, src_sink, dst_sink) = ledger();
        let s = pid("work");
        let d = pid("personal");
        let decision = decide_transfer(
            ADMITTED_CALLER,
            &req,
            &ProfileRef::unlocked(&s),
            &ProfileRef::unlocked(&d),
            &policy,
            &ledger,
        )
        .await;
        assert!(matches!(
            decision,
            GateDecision::Refused {
                reason: RefusalReason::InconsistentRequest
            }
        ));
        // No audit was written for a wiring-fault mismatch.
        assert_eq!(src_sink.count().await, 0);
        assert_eq!(dst_sink.count().await, 0);
    }

    #[tokio::test]
    async fn an_invalid_request_is_refused() {
        let mut req = request("work", "personal", TransferType::File);
        req.payload = PayloadRef::File {
            source_path: "".into(),
        };
        let policy = allow_policy("work", "personal", TransferType::File);
        let (ledger, _src, _dst) = ledger();
        let s = pid("work");
        let d = pid("personal");
        let decision = decide_transfer(
            ADMITTED_CALLER,
            &req,
            &ProfileRef::unlocked(&s),
            &ProfileRef::unlocked(&d),
            &policy,
            &ledger,
        )
        .await;
        assert!(matches!(
            decision,
            GateDecision::Refused {
                reason: RefusalReason::InvalidRequest
            }
        ));
    }
}

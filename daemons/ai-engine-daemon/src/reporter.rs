//! Phase-1 reporter seam: record a tool result to the audit ledger and screen
//! its content (S17/S18) before it re-enters the engine's context
//! (`pi-agent-adoption.md` Phase 1, "the audit/compensation/S17-S18 path
//! (Report)").
//!
//! The daemon audits every reported result content-free (S13 audit-before-act:
//! a result cannot re-enter the model's context without a durable record) and
//! screens it through the shared [`Screener`]. Audit is fail-closed: a result
//! that cannot be recorded is blocked, never passed through unaudited - and
//! since [`ReportAck`] carries no error variant, a [`ScreenVerdict::Block`] IS
//! the refusal that keeps the content out of the engine's context. Screening
//! maps the shared [`Verdict`] onto the contract verdict; a configured-but-broken
//! classifier already fails closed inside the screener, and an unconfigured one
//! flows under [`ScreenVerdict::Clean`] (the gate's confirm-on-external-trigger
//! is the action-level containment, the same posture the agent loop takes).
//!
//! Compensation registration (an undo receipt for a mutating tool result) lands
//! with the write executor: a read or an ordinary result has nothing to undo,
//! and the write proxy tools are not wired yet, so this seam records + screens.

use crate::dispatch::Reporter;
use crate::session::SessionGrant;
use ai_engine_contract::{Report, ReportAck, ScreenVerdict};
use crate::capability_map::{gate_class_for_tool, GateClass};
use arlen_ai_core::audit::{behaviour_action_event, behaviour_graph_access_event};
use arlen_ai_core::screen::{Screener, Verdict};
use async_trait::async_trait;
use audit_proto::sink::AuditSink;
use std::sync::Arc;

/// Map the shared screening [`Verdict`] onto the contract [`ScreenVerdict`] the
/// engine receives. `Allow` lets the content re-enter context (`Clean`), `Warn`
/// passes it through logged, and `Block` keeps it out.
fn verdict_to_screen(v: Verdict) -> ScreenVerdict {
    match v {
        Verdict::Allow => ScreenVerdict::Clean,
        Verdict::Warn => ScreenVerdict::Warn,
        Verdict::Block => ScreenVerdict::Block,
    }
}

/// The reporter seam: audits a tool result content-free, then screens its
/// content. Holds the audit sink (the `arlen-auditd` ledger in the daemon binary,
/// a mock in tests) and the shared [`Screener`].
///
/// The write's undo receipt is NOT registered here: the write executor registers
/// the op-id-keyed compensation authoritatively at apply time (from the daemon's
/// own op id), so a non-cooperative engine that skips Report cannot leave a
/// committed write un-undoable. This seam only records + screens the reported
/// result content.
pub struct ScreeningReporter {
    audit: Arc<dyn AuditSink>,
    screener: Screener,
}

impl ScreeningReporter {
    /// Build the reporter over an audit sink and a screener.
    pub fn new(audit: Arc<dyn AuditSink>, screener: Screener) -> Self {
        Self { audit, screener }
    }
}

#[async_trait]
impl Reporter for ScreeningReporter {
    async fn report(&self, req: &Report, _grant: &SessionGrant) -> ReportAck {
        // Audit BEFORE the content can re-enter the model (S13). The entry is
        // content-free: it names the tool and whether it errored, correlated by
        // the engine's tool_call_id; `behaviour_action_event` validates the
        // subject so no free text reaches the Structural tier. If the ledger
        // cannot record it, the content must not pass - fail closed to Block.
        let outcome = if req.is_error { "tool-result:error" } else { "tool-result" };
        // A read tool's result is a GRAPH ACCESS, audited under that kind so the
        // transparency drawer's anti-Recall "what the AI read" view (which filters
        // the ledger to `GraphAccess`) actually sees it. Every other tool is a
        // routine action (`Permission`). Without this split a `graph.read` was
        // audited as `Permission` and the reads feed stayed empty despite the AI
        // reading - the read was in the ledger under a kind that view ignores.
        let event = if gate_class_for_tool(&req.tool_name) == GateClass::Read {
            behaviour_graph_access_event(&req.tool_name, outcome, &req.tool_call_id)
        } else {
            behaviour_action_event(&req.tool_name, outcome, &req.tool_call_id)
        };
        if self.audit.submit(event).await.is_err() {
            return ReportAck { screen: ScreenVerdict::Block };
        }

        // The write's op-id-keyed compensation is registered by the write executor
        // at apply time (from the daemon's own op id), NOT here from the engine's
        // reported result - so a skipped or forged Report cannot leave a committed
        // write un-undoable or overwrite the authoritative receipt.

        // Screen the result content (S17/S18) before it re-enters the engine's
        // context. A string result is screened verbatim; any other shape is
        // screened by its JSON form, since an injection can ride a structured
        // field as easily as a top-level string.
        let text = match &req.result {
            serde_json::Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        let verdict = self.screener.screen(&text).await;
        ReportAck { screen: verdict_to_screen(verdict) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ai_engine_contract::{CapabilityContext, ReadTier};
    use arlen_ai_core::screen::ScreeningMode;
    use audit_proto::sink::MockAuditSink;

    fn grant() -> SessionGrant {
        SessionGrant {
            capability_context: CapabilityContext { generic_tools: vec![], proxy_tools: vec![] },
            project_anchor: None,
            read_tier: ReadTier::None,
            externally_triggered: false,
            pid: 1,
        }
    }

    fn report(result: serde_json::Value, is_error: bool) -> Report {
        Report {
            tool_name: "graph.read".into(),
            tool_call_id: "call-1".into(),
            result,
            is_error,
        }
    }

    #[test]
    fn the_verdict_mapping_is_total() {
        assert_eq!(verdict_to_screen(Verdict::Allow), ScreenVerdict::Clean);
        assert_eq!(verdict_to_screen(Verdict::Warn), ScreenVerdict::Warn);
        assert_eq!(verdict_to_screen(Verdict::Block), ScreenVerdict::Block);
    }

    #[tokio::test]
    async fn an_unconfigured_screener_passes_a_recorded_result() {
        // Screener::off -> Verdict::Allow -> Clean, and the result is audited.
        let audit = Arc::new(MockAuditSink::accepting());
        let reporter = ScreeningReporter::new(audit.clone(), Screener::off());
        let ack = reporter.report(&report(serde_json::json!("hello"), false), &grant()).await;
        assert_eq!(ack.screen, ScreenVerdict::Clean);
        assert_eq!(audit.recorded().await.len(), 1, "the result was audited before passing");
    }

    #[tokio::test]
    async fn a_read_is_audited_as_graph_access_for_the_anti_recall_feed() {
        // The reads feed filters the ledger to GraphAccess; a graph.read must be
        // recorded under that kind or "what the AI read" stays empty despite the
        // AI reading.
        use audit_proto::AuditKind;
        let audit = Arc::new(MockAuditSink::accepting());
        let reporter = ScreeningReporter::new(audit.clone(), Screener::off());
        // `report()` uses tool_name "graph.read" (GateClass::Read).
        reporter.report(&report(serde_json::json!({"rows": []}), false), &grant()).await;
        let recorded = audit.recorded().await;
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].kind, AuditKind::GraphAccess, "a read must audit as GraphAccess");
    }

    #[tokio::test]
    async fn a_non_read_tool_stays_a_permission_action() {
        // Only reads become GraphAccess; a write/action stays the generic action
        // kind so the reads feed does not mistake it for a read.
        use audit_proto::AuditKind;
        let audit = Arc::new(MockAuditSink::accepting());
        let reporter = ScreeningReporter::new(audit.clone(), Screener::off());
        let write = Report {
            tool_name: "graph.write".into(),
            tool_call_id: "c".into(),
            result: serde_json::json!("ok"),
            is_error: false,
        };
        reporter.report(&write, &grant()).await;
        let recorded = audit.recorded().await;
        assert_eq!(recorded[0].kind, AuditKind::Permission, "a write must not audit as a read");
    }

    #[tokio::test]
    async fn a_failed_audit_blocks_the_result() {
        // Audit-before-act: a result that cannot be recorded must not re-enter
        // the engine's context, so it is blocked even though the screener is off.
        let audit = Arc::new(MockAuditSink::failing());
        let reporter = ScreeningReporter::new(audit, Screener::off());
        let ack = reporter.report(&report(serde_json::json!("hello"), false), &grant()).await;
        assert_eq!(ack.screen, ScreenVerdict::Block, "unaudited content must not pass");
    }

    #[tokio::test]
    async fn a_fail_closed_screener_blocks_a_recorded_result() {
        // A configured-but-unloadable classifier fails closed: the result is
        // audited but blocked from re-entering context.
        let audit = Arc::new(MockAuditSink::accepting());
        let reporter =
            ScreeningReporter::new(audit.clone(), Screener::new(ScreeningMode::FailClosed));
        let ack = reporter.report(&report(serde_json::json!("hello"), false), &grant()).await;
        assert_eq!(ack.screen, ScreenVerdict::Block);
        assert_eq!(audit.recorded().await.len(), 1, "blocked, but still audited");
    }

    #[tokio::test]
    async fn a_structured_result_is_screened_by_its_json_form() {
        // A non-string result is still audited and screened (off -> Clean here).
        let audit = Arc::new(MockAuditSink::accepting());
        let reporter = ScreeningReporter::new(audit.clone(), Screener::off());
        let ack = reporter
            .report(&report(serde_json::json!({ "rows": [1, 2, 3] }), false), &grant())
            .await;
        assert_eq!(ack.screen, ScreenVerdict::Clean);
        assert_eq!(audit.recorded().await.len(), 1);
    }

    #[tokio::test]
    async fn an_error_result_is_recorded_and_screened() {
        let audit = Arc::new(MockAuditSink::accepting());
        let reporter = ScreeningReporter::new(audit.clone(), Screener::off());
        let ack = reporter.report(&report(serde_json::json!("boom"), true), &grant()).await;
        assert_eq!(ack.screen, ScreenVerdict::Clean);
        assert_eq!(audit.recorded().await.len(), 1);
    }
}

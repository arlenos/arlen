//! Phase-0 placeholder seam implementations.
//!
//! The daemon is runnable now (binds the contract socket, authenticates the
//! engine, routes verbs) but the gate/executor/reporter are not yet wired to
//! the real Rust. These placeholders are fail-closed safe defaults so a session
//! cannot DO anything before Phase 1 re-points the seams: the gate denies every
//! call, the executor runs nothing, and the reporter blocks every result from
//! re-entering the engine's context. Phase 1 replaces them with
//! `Capability::decide`, the trusted privileged-tool runner, and the
//! audit/compensation/screening path.

use crate::dispatch::{Executor, Gate, Reporter};
use crate::session::SessionGrant;
use ai_engine_contract::{
    Authorize, AuthorizeDecision, ContractError, Execute, ExecuteOutcome, Report, ReportAck,
    ScreenVerdict,
};
use async_trait::async_trait;

/// The reason placeholder seams give, so logs show the daemon is pre-Phase-1.
const NOT_WIRED: &str = "ai-engine-daemon Phase 0: the real gate is not wired yet";

/// A gate that denies every call (safe default until `Capability::decide` is wired).
pub struct DenyGate;

#[async_trait]
impl Gate for DenyGate {
    async fn authorize(&self, _req: &Authorize, _grant: &SessionGrant) -> AuthorizeDecision {
        AuthorizeDecision::Deny { reason: NOT_WIRED.to_string() }
    }
}

/// An executor that runs nothing (returns Unavailable until the trusted
/// privileged-tool runner is wired).
pub struct UnavailableExecutor;

#[async_trait]
impl Executor for UnavailableExecutor {
    async fn execute(&self, _req: &Execute, _grant: &SessionGrant) -> ExecuteOutcome {
        ExecuteOutcome::Error { code: ContractError::Unavailable, message: NOT_WIRED.to_string() }
    }
}

/// A reporter that blocks every result (nothing re-enters the engine's context
/// until S17/S18 screening is wired).
pub struct BlockReporter;

#[async_trait]
impl Reporter for BlockReporter {
    async fn report(&self, _req: &Report, _grant: &SessionGrant) -> ReportAck {
        ReportAck { screen: ScreenVerdict::Block }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ai_engine_contract::{CapabilityContext, ReadTier};

    fn grant() -> SessionGrant {
        SessionGrant {
            capability_context: CapabilityContext { generic_tools: vec![], proxy_tools: vec![] },
            project_anchor: None,
            read_tier: ReadTier::None,
            externally_triggered: false,
            pid: 1,
        }
    }

    #[tokio::test]
    async fn placeholders_are_fail_closed() {
        let g = DenyGate
            .authorize(
                &Authorize { tool_name: "bash".into(), tool_input: serde_json::json!({}), external_triggered: false },
                &grant(),
            )
            .await;
        assert!(matches!(g, AuthorizeDecision::Deny { .. }));

        let e = UnavailableExecutor
            .execute(&Execute { tool_name: "graph.read".into(), tool_input: serde_json::json!({}), proof: None }, &grant())
            .await;
        assert!(matches!(e, ExecuteOutcome::Error { code: ContractError::Unavailable, .. }));

        let r = BlockReporter
            .report(
                &Report { tool_name: "x".into(), tool_call_id: "c".into(), result: serde_json::json!({}), is_error: false },
                &grant(),
            )
            .await;
        assert_eq!(r.screen, ScreenVerdict::Block);
    }
}

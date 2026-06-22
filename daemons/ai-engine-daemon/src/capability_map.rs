//! Phase-1 glue: resolve a session's [`SessionGrant`] into the existing Arlen
//! [`Capability`] the gate decides against (`pi-agent-adoption.md` Phase 1,
//! "Authorize calls existing `Capability::decide`").
//!
//! This is the bridge from the engine-neutral contract's coarse [`ReadTier`] to
//! the real read-scoping [`AccessTier`] the graph layer enforces, plus the
//! resolved [`Capability`] the next gate slice calls `.decide()` on. The mapping
//! lives in the daemon, never the contract crate, so the contract stays
//! engine-neutral.
//!
//! The action side resolves to [`ActionPermissions::suggest_only`]: a session
//! carries no per-application autonomy grant, Suggest is the safe baseline, and
//! the executor-live flip that would lift it stays human-gated. So an ordinary
//! action resolves to a proposal and a high-impact or externally-triggered one
//! to a confirmation, never to silent autonomous execution.

use crate::session::SessionGrant;
use ai_engine_contract::{AuthorizeDecision, ReadTier};
use arlen_ai_core::capability::{ActionDecision, ActionPermissions, Capability};
use arlen_ai_core::graph_query::AccessTier;

/// Map the contract's coarse [`ReadTier`] to the graph layer's [`AccessTier`].
///
/// Both are five-level and ordinally aligned, so the mapping is the order-
/// preserving correspondence of their documented scopes: no-read to no-read,
/// session to session, working/standard to project, project-plus-recent to
/// time-windowed, full to full. The graph compiler enforces the resulting tier
/// (and its active-project anchor) per query; this only resolves which tier.
pub fn read_tier_to_access_tier(tier: ReadTier) -> AccessTier {
    match tier {
        ReadTier::None => AccessTier::Minimal,
        ReadTier::Minimal => AccessTier::SessionScoped,
        ReadTier::Standard => AccessTier::ProjectScoped,
        ReadTier::Extended => AccessTier::TimeScoped,
        ReadTier::Full => AccessTier::Full,
    }
}

/// Resolve a session's grant into the [`Capability`] the gate decides against.
///
/// The read tier comes from the grant (mapped through
/// [`read_tier_to_access_tier`]); the action side is the conservative
/// [`ActionPermissions::suggest_only`] baseline (a session is never granted
/// per-application autonomy here, and the executor-live lift is human-gated), so
/// every action is at most a proposal until that flip.
pub fn grant_to_capability(grant: &SessionGrant) -> Capability {
    Capability::new(
        read_tier_to_access_tier(grant.read_tier),
        ActionPermissions::suggest_only(),
    )
}

/// Map the gate's [`ActionDecision`] onto the contract's [`AuthorizeDecision`]
/// the engine receives (`pi-agent-adoption.md`: the suggest-mode + UI-split
/// rules). `tool_name` only names the tool in the user-facing prompt / the
/// model-facing reason.
///
/// - [`ActionDecision::Proceed`] (an individually-enabled autonomous app, only
///   reachable once the human-gated executor-live flip is on) lets the engine
///   run the tool: [`AuthorizeDecision::Allow`].
/// - [`ActionDecision::RequireConfirmation`] (a high-impact or externally-
///   triggered action) holds for the trusted-path consent surface:
///   [`AuthorizeDecision::Confirm`].
/// - [`ActionDecision::PreviewThenExecute`] (Supervised) is a preview-with-
///   cancel hold, which Phase 1 reduces to the same explicit confirmation.
/// - [`ActionDecision::Propose`] (the Suggest baseline) does NOT auto-execute:
///   the action is recorded as a proposal for the pull activity view, so the
///   engine is refused with [`AuthorizeDecision::Deny`].
///
/// Never [`AuthorizeDecision::Modify`]: argument substitution is the daemon's
/// re-validation concern at Execute time, not an action-mode outcome.
pub fn decision_to_authorize(decision: ActionDecision, tool_name: &str) -> AuthorizeDecision {
    match decision {
        ActionDecision::Proceed => AuthorizeDecision::Allow,
        ActionDecision::RequireConfirmation => AuthorizeDecision::Confirm {
            prompt: format!("Confirm {tool_name}? This action needs your explicit approval."),
        },
        ActionDecision::PreviewThenExecute => AuthorizeDecision::Confirm {
            prompt: format!("Allow {tool_name}? Review it before it runs."),
        },
        ActionDecision::Propose => AuthorizeDecision::Deny {
            reason: format!(
                "{tool_name} was recorded as a proposal for the user to review; \
                 suggest mode does not auto-execute mutating actions"
            ),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ai_engine_contract::CapabilityContext;
    use arlen_ai_core::capability::ActionKind;

    fn grant(read_tier: ReadTier) -> SessionGrant {
        SessionGrant {
            capability_context: CapabilityContext { generic_tools: vec![], proxy_tools: vec![] },
            project_anchor: None,
            read_tier,
            pid: 1,
        }
    }

    #[test]
    fn the_read_tier_mapping_is_total_and_ordinal() {
        // Every contract tier maps to its documented graph-layer counterpart.
        assert_eq!(read_tier_to_access_tier(ReadTier::None), AccessTier::Minimal);
        assert_eq!(read_tier_to_access_tier(ReadTier::Minimal), AccessTier::SessionScoped);
        assert_eq!(read_tier_to_access_tier(ReadTier::Standard), AccessTier::ProjectScoped);
        assert_eq!(read_tier_to_access_tier(ReadTier::Extended), AccessTier::TimeScoped);
        assert_eq!(read_tier_to_access_tier(ReadTier::Full), AccessTier::Full);
    }

    #[test]
    fn a_grant_resolves_its_read_tier_and_a_suggest_only_action_baseline() {
        let cap = grant_to_capability(&grant(ReadTier::Standard));
        assert_eq!(cap.read_tier, AccessTier::ProjectScoped);

        // Suggest-only baseline: an ordinary action is a proposal, never an
        // autonomous proceed, regardless of the app id.
        assert_eq!(
            cap.decide("any.app", ActionKind::Ordinary, false),
            ActionDecision::Propose,
        );
        // A high-impact kind always confirms, even under the suggest baseline.
        assert_eq!(
            cap.decide("any.app", ActionKind::PermanentDelete, false),
            ActionDecision::RequireConfirmation,
        );
        // An externally-triggered ordinary action also always confirms.
        assert_eq!(
            cap.decide("any.app", ActionKind::Ordinary, true),
            ActionDecision::RequireConfirmation,
        );
    }

    #[test]
    fn the_no_read_tier_resolves_to_no_graph_access() {
        let cap = grant_to_capability(&grant(ReadTier::None));
        assert_eq!(cap.read_tier, AccessTier::Minimal);
    }

    #[test]
    fn proceed_lets_the_engine_run_the_tool() {
        assert_eq!(
            decision_to_authorize(ActionDecision::Proceed, "graph.write"),
            AuthorizeDecision::Allow,
        );
    }

    #[test]
    fn confirmation_and_preview_hold_for_the_consent_surface() {
        assert!(matches!(
            decision_to_authorize(ActionDecision::RequireConfirmation, "send.email"),
            AuthorizeDecision::Confirm { .. },
        ));
        assert!(matches!(
            decision_to_authorize(ActionDecision::PreviewThenExecute, "graph.write"),
            AuthorizeDecision::Confirm { .. },
        ));
    }

    #[test]
    fn propose_refuses_the_engine_so_it_becomes_a_proposal() {
        // Suggest mode: the engine does not auto-execute; the action is a
        // proposal for the pull activity view, so the tool call is denied.
        assert!(matches!(
            decision_to_authorize(ActionDecision::Propose, "graph.write"),
            AuthorizeDecision::Deny { .. },
        ));
    }

    #[test]
    fn the_mapping_never_substitutes_arguments() {
        // Modify is the daemon's Execute-time re-validation concern, never an
        // action-mode outcome - no decision maps to it.
        for d in [
            ActionDecision::Proceed,
            ActionDecision::RequireConfirmation,
            ActionDecision::PreviewThenExecute,
            ActionDecision::Propose,
        ] {
            assert!(!matches!(
                decision_to_authorize(d, "t"),
                AuthorizeDecision::Modify { .. },
            ));
        }
    }

    /// The full Suggest pipeline a Phase-1 RealGate composes: a session's grant
    /// resolves to a Capability, an ordinary action under the suggest baseline
    /// decides to Propose, and that maps to a Deny the engine cannot execute.
    #[test]
    fn the_suggest_pipeline_denies_an_ordinary_action_end_to_end() {
        let cap = grant_to_capability(&grant(ReadTier::Standard));
        let decision = cap.decide("any.app", ActionKind::Ordinary, false);
        assert_eq!(decision, ActionDecision::Propose);
        assert!(matches!(
            decision_to_authorize(decision, "graph.write"),
            AuthorizeDecision::Deny { .. },
        ));
    }
}

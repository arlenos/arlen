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
use ai_engine_contract::ReadTier;
use arlen_ai_core::capability::{ActionPermissions, Capability};
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

#[cfg(test)]
mod tests {
    use super::*;
    use ai_engine_contract::CapabilityContext;
    use arlen_ai_core::capability::{ActionDecision, ActionKind};

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
}

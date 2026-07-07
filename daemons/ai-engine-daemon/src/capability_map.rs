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

use crate::dispatch::Gate;
use crate::session::SessionGrant;
use ai_engine_contract::{Authorize, AuthorizeDecision, ReadTier};
use arlen_ai_core::capability::{ActionDecision, ActionKind, ActionPermissions, Capability};
use arlen_ai_core::graph_query::{AccessTier, QueryScope};
use arlen_ai_core::graph_schema::GraphSchema;
use arlen_ai_core::mcp::{name_segments, AlwaysConfirm, AlwaysConfirmReason};
use arlen_consent_contract::ConsentClass;
use async_trait::async_trait;

/// The app identity actions are attributed to under an engine session. The
/// action side is the `suggest_only` baseline (a session is granted no
/// per-application autonomy), so the per-app mode is Suggest regardless of this
/// id; it marks the seam where a real per-application grant would flow once the
/// human-gated executor-live lift exists.
const ENGINE_APP_ID: &str = "ai-engine";

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

/// Resolve a session's grant into the [`QueryScope`] a graph read is bounded
/// by (`pi-agent-adoption.md` Phase 1, "graph_query read-scope incl. GAP-21 is
/// re-pointed"). This is the read-side companion to [`grant_to_capability`]: the
/// daemon runs a `graph.read` proxy tool through the scope this returns, never
/// trusting the engine to self-restrict.
///
/// The grant's read tier maps through [`read_tier_to_access_tier`] to the tier's
/// fixed label allowlist. The `ProjectScoped` tier is the GAP-21 case: a bare
/// `ProjectScoped` scope permits its labels across EVERY project, so it is only
/// safe with a mandatory active-project anchor. When the grant carries a
/// `project_anchor` the scope is anchored to it (the compile-time `WHERE EXISTS`
/// the model cannot remove); when it does not, the scope is EMPTY (no read),
/// never the anchorless tier-wide one. Every other tier carries no anchor.
pub fn grant_to_query_scope(grant: &SessionGrant, schema: &GraphSchema) -> QueryScope {
    let tier = read_tier_to_access_tier(grant.read_tier);
    match tier {
        AccessTier::ProjectScoped => match grant.project_anchor.as_deref() {
            Some(project_id) => QueryScope::for_project(project_id, schema),
            // GAP-21: a project-scoped read with no active project resolves to
            // no read at all, never the tier's labels across all projects.
            None => QueryScope::new(Vec::<&str>::new()),
        },
        other => QueryScope::for_tier(other, schema),
    }
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
        ActionDecision::Proceed => AuthorizeDecision::Allow { proof: None },
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

/// Resolve a tool name to the [`ActionKind`] the gate decides on, reusing the
/// existing tool-name classifier ([`AlwaysConfirm::classify`], the same one the
/// MCP gate uses) so the impact classes do not drift. A classified tool maps to
/// its corresponding high-impact kind; an unclassified one is [`ActionKind::
/// Ordinary`]. `GenericExecution` (a `run`/`exec`/`shell`-shaped tool whose
/// effect the name cannot reveal) maps to a high-impact kind so it always
/// confirms; the specific variant is immaterial since every non-`Ordinary` kind
/// already requires confirmation. The effect-derived kinds (`Irreversible`,
/// `ReversibleWithCost`) are not name-derivable and land with the predict step.
pub(crate) fn action_kind_for_tool(tool: &str) -> ActionKind {
    match AlwaysConfirm::classify(tool) {
        Some(AlwaysConfirmReason::FileDeletion) => ActionKind::PermanentDelete,
        Some(AlwaysConfirmReason::ExternalMessage) => ActionKind::SendExternalMessage,
        Some(AlwaysConfirmReason::PackageChange) => ActionKind::PackageChange,
        Some(AlwaysConfirmReason::SystemConfigWrite) => ActionKind::SystemConfigChange,
        Some(AlwaysConfirmReason::ElevatedCommand) => ActionKind::ElevatedPrivilege,
        Some(AlwaysConfirmReason::GenericExecution) => ActionKind::ElevatedPrivilege,
        None => ActionKind::Ordinary,
    }
}

/// Map a tool name to the consent dialog CLASS, reusing the same
/// [`AlwaysConfirm::classify`] the gate decides severity on, so the consent
/// surface renders the right polymorphic dialog (Destructive / ExternalSend /
/// Install / ...) instead of the generic agent-action copy. This is PRESENTATION
/// only - severity comes from [`action_kind_for_tool`] via the broker's
/// `classify`, never from the class - so a reason with no clear class counterpart
/// (a system-config write, or an unrecognised tool) falls back to `AgentAction`
/// rather than risk a misleading specific dialog.
pub(crate) fn consent_class_for_tool(tool: &str) -> ConsentClass {
    match AlwaysConfirm::classify(tool) {
        Some(AlwaysConfirmReason::FileDeletion) => ConsentClass::Destructive,
        Some(AlwaysConfirmReason::ExternalMessage) => ConsentClass::ExternalSend,
        Some(AlwaysConfirmReason::PackageChange) => ConsentClass::Install,
        Some(AlwaysConfirmReason::ElevatedCommand) => ConsentClass::ElevatedPrivilege,
        Some(AlwaysConfirmReason::GenericExecution) => ConsentClass::ExecConfined,
        Some(AlwaysConfirmReason::SystemConfigWrite) | None => ConsentClass::AgentAction,
    }
}

/// The Phase-1 gate seam: an [`Authorize`] is decided against the real Arlen
/// [`Capability`] (`pi-agent-adoption.md` Phase 1, "Authorize calls existing
/// `Capability::decide`"), replacing the Phase-0 deny-all placeholder.
///
/// It composes the built glue: [`grant_to_capability`] resolves the session's
/// capability, [`action_kind_for_tool`] resolves the tool's impact class, and
/// `Capability::decide` applies the two non-configurable overrides (a high-impact
/// kind or an externally-triggered action always confirms) then the action mode,
/// whose verdict [`decision_to_authorize`] maps to the contract decision. Under
/// the `suggest_only` baseline the mode is always Suggest, so this can never
/// return [`AuthorizeDecision::Allow`] (that needs Autonomous mode, which only
/// the human-gated executor-live lift grants): every call is a proposal (`Deny`)
/// or a confirmation (`Confirm`), never silent autonomous execution.
pub struct CapabilityGate;

#[async_trait]
impl Gate for CapabilityGate {
    async fn authorize(&self, req: &Authorize, grant: &SessionGrant) -> AuthorizeDecision {
        // D3: a read Allows, bounded by the grant's read SCOPE (applied when the
        // read executes, via `grant_to_scope` incl. the GAP-21 active-project
        // anchor). The anti-Recall guarantee is the scope, not a per-read confirm.
        if gate_class_for_tool(&req.tool_name) == GateClass::Read {
            return AuthorizeDecision::Allow { proof: None };
        }
        // D2 hard case 2: an externally-triggered egress call is the prompt-
        // injection exfiltration vector - hard-Deny it (stronger than the confirm
        // the `decide` model would give), so injected content cannot leak data.
        if req.external_triggered && is_egress_tool(&req.tool_name) {
            return AuthorizeDecision::Deny {
                reason: format!(
                    "{} is egress and this action was triggered by external content; \
                     egress is denied to prevent exfiltration",
                    req.tool_name
                ),
            };
        }
        // Every other tool: the existing capability path decides (the always-confirm
        // + external-trigger overrides, then the action mode). The fine-grained
        // reversible -> autonomous `Allow` lift and the unknown -> fail-closed `Deny`
        // land WITH the D2 typed proxy tools (`graph.assert_edge` etc.); until those
        // replace today's coarse names (`graph.write`), a hard Deny-for-unknown here
        // would wrongly refuse the live coarse tools, so the coarse path stands.
        let kind = action_kind_for_tool(&req.tool_name);
        let capability = grant_to_capability(grant);
        let decision = capability.decide(ENGINE_APP_ID, kind, req.external_triggered);
        decision_to_authorize(decision, &req.tool_name)
    }
}

/// The gate class of a pi tool, resolved from its NAME alone (`pi-gate-class-
/// registry.md` D1: no arg-inspection in the security path - the class is a fixed
/// property of the tool, so an attacker cannot shape `tool_input` to change the
/// decision). Possession of a table entry is the trust proof; an unknown tool is
/// fail-closed [`GateClass::Deny`].
///
/// The daemon-side executor still re-validates every effect; this table is the
/// FRONT gate that decides Allow / Confirm / Deny per tool name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateClass {
    /// A read tool: `Allow`, bounded by the grant's read scope (D3 - the anti-Recall
    /// guarantee is the read_tier + project-anchor scope, not a confirm).
    Read,
    /// A reversible action (a compensation exists): `Allow` autonomous within the
    /// granted tier. Undo + audit + revoke is the net (reversibility gates autonomy,
    /// not impact - a reversible-destructive `fs.trash` is still autonomous).
    ReversibleAction,
    /// An irreversible, opaque-exec, external-send/reach, or elevated action:
    /// `Confirm` per instance. External-triggered content confirms regardless.
    Confirm,
    /// Not in the registry: fail-closed `Deny`.
    Deny,
}

/// Whether a tool reaches the network (an egress / external-send tool). An
/// externally-triggered egress call is the classic prompt-injection EXFILTRATION
/// vector (the reversibility model does not capture "a read that leaks"), so the
/// gate hard-denies it - stronger than the `decide` model's confirm
/// (`pi-gate-class-registry.md` D2, hard case 2).
pub fn is_egress_tool(tool: &str) -> bool {
    // External-send tools: reuse the segment-based `AlwaysConfirm` classifier so the
    // egress surface matches the one the rest of the gate already recognises
    // (`send`/`email`/`mail`/`message`/`post`/`publish` as name segments), catching
    // `slack.send`, `webhook.publish`, `mailer` etc., not just the bare names.
    if matches!(
        AlwaysConfirm::classify(tool),
        Some(AlwaysConfirmReason::ExternalMessage)
    ) {
        return true;
    }
    // Network-reach tools the classifier treats as ordinary, but which still leak
    // data when triggered by external content (a GET that exfiltrates). Segment-based
    // so `http.get`, `net_fetch`, `curl` all match. Over-matching only ever hard-denies
    // an externally-triggered call, which is the fail-safe direction.
    name_segments(tool).iter().any(|s| {
        matches!(
            s.as_str(),
            "fetch"
                | "http"
                | "https"
                | "curl"
                | "wget"
                | "upload"
                | "webhook"
                | "request"
                | "download"
                | "dns"
                | "socket"
        )
    })
}

/// Map a pi tool NAME to its gate class - the D1 static table
/// (`pi-gate-class-registry.md`). The fine-grained graph/fs tool names (D2) each
/// resolve to one fixed class so no `tool_input` parsing is needed in the gate.
pub fn gate_class_for_tool(tool: &str) -> GateClass {
    match tool {
        // Reads: Allow, scope-bounded (D3).
        "graph.read" => GateClass::Read,
        // Reversible graph + fs actions: Allow autonomous.
        "graph.assert_edge" | "graph.retract_edge" | "fs.move" | "fs.trash" => {
            GateClass::ReversibleAction
        }
        // Irreversible graph + fs actions: Confirm.
        "graph.set_field" | "graph.retract_node" | "fs.delete" => GateClass::Confirm,
        // Package / external-send / egress-reach / opaque-exec / elevated: Confirm
        // (standing autonomy for these comes only via the heavy consent surface).
        "install" | "uninstall" | "send" | "email" | "post" | "fetch" | "http"
        | "run_command" | "exec" | "eval" | "sudo" | "pkexec" => GateClass::Confirm,
        // Anything not in the registry: fail-closed Deny (possession of an entry is
        // the trust proof).
        _ => GateClass::Deny,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gate_class_table_classifies_by_name() {
        // Reads short-circuit to a scope-bounded Allow.
        assert_eq!(gate_class_for_tool("graph.read"), GateClass::Read);
        // Reversible actions (graph + fs) are autonomous.
        assert_eq!(gate_class_for_tool("graph.assert_edge"), GateClass::ReversibleAction);
        assert_eq!(gate_class_for_tool("graph.retract_edge"), GateClass::ReversibleAction);
        assert_eq!(gate_class_for_tool("fs.move"), GateClass::ReversibleAction);
        assert_eq!(gate_class_for_tool("fs.trash"), GateClass::ReversibleAction);
        // Irreversible / exec / egress / elevated confirm.
        assert_eq!(gate_class_for_tool("graph.set_field"), GateClass::Confirm);
        assert_eq!(gate_class_for_tool("graph.retract_node"), GateClass::Confirm);
        assert_eq!(gate_class_for_tool("fs.delete"), GateClass::Confirm);
        assert_eq!(gate_class_for_tool("run_command"), GateClass::Confirm);
        assert_eq!(gate_class_for_tool("fetch"), GateClass::Confirm);
        assert_eq!(gate_class_for_tool("sudo"), GateClass::Confirm);
        // Unknown tool: fail-closed Deny.
        assert_eq!(gate_class_for_tool("graph.write"), GateClass::Deny); // coarse name is not in the fine-grained table
        assert_eq!(gate_class_for_tool("totally.unknown"), GateClass::Deny);
    }

    #[test]
    fn egress_tools_are_identified_for_the_exfiltration_guard() {
        // Bare names.
        for t in ["fetch", "http", "send", "email", "post"] {
            assert!(is_egress_tool(t), "{t} should be egress");
        }
        // The segment-based surface the old exact-match list missed (the review
        // finding): external-send and network-reach under a namespace / camelCase.
        for t in [
            "slack.send",
            "webhook.publish",
            "http.get",
            "net_fetch",
            "curl",
            "httpRequest",
            "cloud.upload",
        ] {
            assert!(is_egress_tool(t), "{t} should be egress");
        }
        // Residual (classifier-level, not this guard's): a sub-word segment like
        // `mailer` != the `mail` segment, so it is not recognised - closing that
        // needs unifying the three egress lists at the classifier, a wider change.
        assert!(!is_egress_tool("graph.read"));
        assert!(!is_egress_tool("fs.move"));
        assert!(!is_egress_tool("graph.assert_edge"));
    }
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
            AuthorizeDecision::Allow { proof: None },
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

    fn schema() -> GraphSchema {
        GraphSchema::knowledge_graph()
    }

    fn grant_anchored(read_tier: ReadTier, anchor: Option<&str>) -> SessionGrant {
        SessionGrant {
            capability_context: CapabilityContext { generic_tools: vec![], proxy_tools: vec![] },
            project_anchor: anchor.map(str::to_string),
            read_tier,
            pid: 1,
        }
    }

    #[test]
    fn a_session_scoped_grant_permits_session_labels_not_files() {
        let scope = grant_to_query_scope(&grant_anchored(ReadTier::Minimal, None), &schema());
        assert!(scope.permits("Session"));
        assert!(scope.permits("Event"));
        assert!(!scope.permits("File"), "session tier cannot name a File");
        assert!(scope.project_anchor().is_none());
    }

    #[test]
    fn a_project_scoped_grant_with_an_anchor_is_anchored() {
        // ReadTier::Standard -> AccessTier::ProjectScoped.
        let scope =
            grant_to_query_scope(&grant_anchored(ReadTier::Standard, Some("proj-7")), &schema());
        assert!(scope.permits("File"));
        assert!(scope.permits("Project"));
        assert!(!scope.permits("Session"), "project tier cannot name a Session");
        assert_eq!(
            scope.project_anchor().map(|a| a.project_id()),
            Some("proj-7"),
            "GAP-21: the read is anchored to the active project",
        );
    }

    #[test]
    fn a_project_scoped_grant_without_an_anchor_reads_nothing() {
        // GAP-21: an anchorless project-scoped read must NOT see the tier's
        // labels across every project; it resolves to an empty scope instead.
        let scope = grant_to_query_scope(&grant_anchored(ReadTier::Standard, None), &schema());
        assert!(scope.is_empty(), "no anchor -> no project-scoped read");
        assert!(!scope.permits("File"));
    }

    #[test]
    fn the_no_read_tier_yields_an_empty_scope() {
        let scope = grant_to_query_scope(&grant_anchored(ReadTier::None, None), &schema());
        assert!(scope.is_empty());
    }

    #[test]
    fn the_full_tier_permits_files_and_sessions() {
        let scope = grant_to_query_scope(&grant_anchored(ReadTier::Full, None), &schema());
        assert!(scope.permits("File"));
        assert!(scope.permits("Session"));
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

    fn authorize(tool: &str, external: bool) -> Authorize {
        Authorize {
            tool_name: tool.into(),
            tool_input: serde_json::json!({}),
            external_triggered: external,
        }
    }

    #[test]
    fn the_tool_kind_classifier_reuses_the_always_confirm_set() {
        assert_eq!(action_kind_for_tool("delete_file"), ActionKind::PermanentDelete);
        assert_eq!(action_kind_for_tool("send_email"), ActionKind::SendExternalMessage);
        assert_eq!(action_kind_for_tool("install_pkg"), ActionKind::PackageChange);
        // A generic execution tool the name cannot judge always confirms.
        assert_eq!(action_kind_for_tool("run_shell"), ActionKind::ElevatedPrivilege);
        // An unclassified tool is ordinary.
        assert_eq!(action_kind_for_tool("note.append"), ActionKind::Ordinary);
    }

    #[test]
    fn the_consent_class_matches_the_tool_family() {
        assert_eq!(consent_class_for_tool("delete_file"), ConsentClass::Destructive);
        assert_eq!(consent_class_for_tool("send_email"), ConsentClass::ExternalSend);
        assert_eq!(consent_class_for_tool("install_pkg"), ConsentClass::Install);
        assert_eq!(consent_class_for_tool("run_shell"), ConsentClass::ExecConfined);
        // An unrecognised tool falls back to the generic agent-action dialog.
        assert_eq!(consent_class_for_tool("note.append"), ConsentClass::AgentAction);
    }

    #[tokio::test]
    async fn the_gate_confirms_a_high_impact_tool() {
        let d = CapabilityGate
            .authorize(&authorize("delete_file", false), &grant(ReadTier::Standard))
            .await;
        assert!(matches!(d, AuthorizeDecision::Confirm { .. }));
    }

    #[tokio::test]
    async fn the_gate_denies_an_ordinary_tool_as_a_proposal_under_suggest() {
        // Suggest baseline: an ordinary action is a proposal, refused to the
        // engine so it never auto-executes.
        let d = CapabilityGate
            .authorize(&authorize("note.append", false), &grant(ReadTier::Standard))
            .await;
        assert!(matches!(d, AuthorizeDecision::Deny { .. }));
    }

    #[tokio::test]
    async fn the_gate_confirms_an_externally_triggered_ordinary_tool() {
        // The external-content override: even an ordinary tool confirms when the
        // run was triggered by external content.
        let d = CapabilityGate
            .authorize(&authorize("note.append", true), &grant(ReadTier::Standard))
            .await;
        assert!(matches!(d, AuthorizeDecision::Confirm { .. }));
    }

    #[tokio::test]
    async fn the_gate_never_allows_under_the_suggest_baseline() {
        // No tool, ordinary or high-impact, ever reaches Allow under the
        // suggest_only baseline (Allow needs Autonomous mode, which only the
        // human-gated executor-live lift grants).
        for tool in ["note.append", "delete_file", "send_email", "run_shell"] {
            let d = CapabilityGate
                .authorize(&authorize(tool, false), &grant(ReadTier::Standard))
                .await;
            assert!(!matches!(d, AuthorizeDecision::Allow { .. }), "{tool} must not be Allowed");
        }
    }
}

//! The action gate: the single point every action a behaviour proposes
//! must pass before it is surfaced or executed.
//!
//! It composes things that already exist:
//! * **tool-scope enforcement** — the proposed tool must be in the
//!   behaviour's declared `tools` scope (the manifest map); an
//!   out-of-scope proposal is refused fail-closed, so a behaviour can only
//!   ever act through the tools it declared;
//! * the **capability decision** (S16) — combining the behaviour's
//!   requested mode *ceiling* with the trusted per-app grant
//!   ([`Capability::decide_for_behaviour`]), so an untrusted behaviour can
//!   only ever narrow authority;
//! * a **fail-closed audit-before-acting** write — the decision is
//!   recorded in the ledger *before* the action is surfaced/executed; if
//!   the ledger cannot record it the gate refuses (no un-audited AI
//!   activity, foundation §8.4.6/.7);
//! * the [`GateObserver`] seam — the read-only tap the audit/anomaly/
//!   inspection layers attach to.
//!
//! ## Trust boundary
//!
//! The action proposal is **untrusted** (it originates from a behaviour /
//! the model). It must never be able to classify its own risk or claim
//! its own provenance, so this gate accepts **neither** from the proposal:
//!
//! * **All authorization inputs come from the trusted [`ActionContext`]**,
//!   resolved by the dispatcher — never from the proposal. That includes
//!   the **target `app_id`** (which per-app grant applies: a proposal
//!   cannot name an autonomous app to get a laxer decision), the
//!   **`external_trigger`** flag (any externally-triggered action always
//!   confirms — prompt-injection containment), and the correlation id. The
//!   proposal carries only the tool name + a human summary.
//! * **High-impact classification** is done here, using the *same* shared
//!   classifier the MCP layer uses ([`AlwaysConfirm`], keyed on the tool
//!   name) — so a destructive tool (delete / send / install / exec / …)
//!   always resolves to `RequireConfirmation` regardless of the configured
//!   mode. The proposer supplies only the tool *name* (drawn from its
//!   declared `tools` scope), never a risk class, so it cannot label a
//!   delete as `Ordinary`. The MCP dispatch boundary re-classifies the
//!   *real* tool at execution time as defense-in-depth.
//!
//! ## Boundary with the world model (B2)
//!
//! Name-based classification catches the *clearly* destructive set
//! (delete / send / install / sudo / exec / config-write). It cannot judge
//! *argument-dependent* destructiveness — e.g. an `fs.move` is reversible
//! unless its destination is occupied, in which case it is an irreversible
//! overwrite (design-doc gap F4). That judgment belongs to the **world-model
//! action schema** (preconditions + effects), not a name heuristic.
//!
//! So before an executing decision (PreviewThenExecute / Proceed) is allowed
//! through, the gate runs a **predict-before-act** step: it resolves the
//! action's trusted, registry-resolved schema, builds a bounded graph slice
//! (through the behaviour-scoped graph handle, so the proof never reads more
//! than the behaviour may) for the invocation's operands, and asks the
//! world-model interpreter whether the preconditions hold and the effects
//! apply cleanly. A `Valid` prediction lifts the conservative cap; otherwise
//! (no registered rule, an unprovable invocation, or any failure or timeout)
//! the executing decision is downgraded to explicit confirmation, so nothing
//! auto-executes whose argument-level safety is unproven. Suggest/Propose is
//! unaffected (the user executes manually). The prediction comes only from the
//! trusted world model, never the proposal, and the lifted-or-capped decision
//! is what the audit records.
//!
//! The lift is deliberately conservative: a proven action is lifted only to a
//! **previewed execution (PreviewThenExecute), never silent autonomous
//! Proceed**. Two boundaries that safe auto-execution needs are not yet in
//! place: the proof is a point-in-time slice (the graph exposes no
//! snapshot/version), so an executor must atomically re-check the
//! preconditions at write time (gap A2), and the per-app grant consulted is
//! the agent's own (the acting app), a coarse model a finer per-target grant
//! will refine. The human-visible preview is the bridge until those land;
//! nothing executes today (there is no executor), so the lifted decision is
//! the authorization the executor will later honour, not an execution.
//!
//! ## Executor obligations (the contract a lifted decision carries)
//!
//! The design (ground truth) deliberately separates this gate's *lift* (the
//! authorization) from the executor's *enforcement*. Before acting on a lifted
//! `PreviewThenExecute`, the executor (a later increment) must:
//! 1. **Execute exactly the proven effect**, the schema effect for the proven
//!    operands (e.g. the `FILE_PART_OF` `AssertEdge`), never a free-form
//!    re-interpretation of the tool name (else a different mutation rides on
//!    the proof).
//! 2. **Atomically re-check the preconditions at write time** (gap A2): the
//!    proof is a point-in-time slice and the graph exposes no snapshot, so a
//!    just-read absence can go stale; the write must be conditional on the
//!    preconditions still holding, and idempotent.
//! 3. **Enforce the manifest's per-tool scope *values*** (e.g. `graph.write`
//!    restricted to certain projects) and **resolve the real per-target/app
//!    binding**, refining today's coarse agent-grant model.
//!
//! Until all three exist, the cap holds beyond preview and nothing
//! auto-executes; these are not this increment's to build.

use std::collections::BTreeMap;
use std::time::Duration;

use arlen_ai_core::audit::{behaviour_action_event, behaviour_policy_violation_event, AuditSink};
use arlen_ai_core::capability::{ActionDecision, ActionKind, BaselineMode, Capability};
use arlen_ai_core::mcp::{AlwaysConfirm, AlwaysConfirmReason};

use crate::canary;
use crate::effect_model::InverseClass;
use crate::registry;
use crate::seams::{GateObserver, GraphHandle};
use crate::slice::{build_slice_trusted, MountPolicy, PathResolver};
use crate::world::{self, EvalContext};

/// How long the predict-before-act proof may run before it is treated as
/// unproven (so the conservative cap stands). It reads the graph and the
/// filesystem; a stalled dependency must not park the gate.
///
/// This bounds the async work (the graph round trips). It cannot interrupt a
/// *blocking* filesystem call mid-syscall (the production path/mount resolvers
/// use `std::fs`), so a hung FUSE/NFS mount could still park the worker past
/// the deadline. Making the path/mount seams async (resolving on a blocking
/// pool) so the deadline bounds them too is a follow-up; the common stall (a
/// slow knowledge socket) is async and is bounded here today.
const PROOF_TIMEOUT: Duration = Duration::from_secs(5);

/// An action a behaviour proposes. Carries only what a proposer may
/// legitimately state — the tool/operation it wants to invoke, a
/// human-facing summary, and the operands (arguments) the invocation will
/// use. It deliberately carries **no authorization inputs**: not the target
/// app id, not a risk class, not an external-content flag. Every input that
/// steers the gate decision is trusted and arrives via [`ActionContext`],
/// never the proposal — an untrusted proposal must not be able to pick which
/// per-app grant applies, label its own risk, or claim non-external
/// provenance. The `summary` is for the proposal/preview UI and is never
/// audited (the audit subject is content-free).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProposedAction {
    /// The MCP tool / operation the behaviour wants to invoke. Classified
    /// by the shared always-confirm classifier and checked against the
    /// behaviour's declared `tools` scope; the *real* tool is re-classified
    /// at MCP dispatch.
    pub tool: String,
    /// Human-facing description for the proposal/preview surface.
    pub summary: String,
    /// The action's operands, as parameter-name to value (a node id or a
    /// path literal). These are **untrusted** — the proposer states them —
    /// so they prove nothing on their own; the predict-before-act step checks
    /// them against the action's trusted, registry-resolved schema and the
    /// real graph before any execution cap is lifted. Empty when the proposer
    /// states no operands (the action can then only be suggested, not proven).
    pub arguments: BTreeMap<String, String>,
}

/// The **trusted** context for a gate decision, resolved by the dispatcher
/// — never taken from the (untrusted) proposal.
#[derive(Debug, Clone, Copy)]
pub struct ActionContext<'a> {
    /// The application whose per-app grant applies. The dispatcher resolves
    /// it from the behaviour identity / the tool's binding; a proposal can
    /// never name an arbitrary app to pick a laxer grant.
    pub app_id: &'a str,
    /// Whether this run was triggered by external content (forces
    /// confirmation — prompt-injection containment). A run-context fact.
    pub external_trigger: bool,
    /// Whether the acting behaviour is a registered DETERMINISTIC workflow
    /// (`kind: workflow`, zero model calls), set by the dispatcher from the
    /// behaviour's manifest kind — never the proposal. The deterministic-workflow
    /// external-trigger carve-out (Tim-approved): a workflow makes no model call,
    /// so external content has no model to inject into, so the `external_trigger`
    /// override (a prompt-injection defense) is structurally vacuous for it and
    /// does not apply. `kind: agent` (any model call) sets this false and keeps
    /// always-confirm on an external trigger — the containment is untouched on the
    /// path that actually has an injection surface. The write the carve-out
    /// admits is still bounded by `executor_live` + the predict-proof + the
    /// behaviour's narrow grant + reversibility.
    pub deterministic_workflow: bool,
    /// Per-action correlation id, carried into the audit ledger so this
    /// decision links to the subsequent execution/outcome entry.
    pub correlation_id: &'a str,
}

/// The faithful reason the gate reached its decision, derived from the gate's
/// own logic (the action's class, the trigger, the proof), never the model's
/// free-text rationale (Foundation D2: the explanation must be the real causal
/// chain, not a narrative the model could fabricate). Surfaced so a confirmation
/// prompt or the activity view can say *why* an action needs the user.
///
/// Several causes can apply at once: a high-impact action triggered by external
/// content confirms for both reasons, and losing the external-content
/// provenance of a high-impact action would hide the prompt-injection path in
/// incident reconstruction. So this records the full cause set (both overrides
/// plus the base disposition), not a single winner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecisionReason {
    /// The action's high-impact class, if its kind always requires confirmation
    /// (a hardcoded class or a schema-derived [`ActionKind::Irreversible`]).
    pub high_impact: Option<ActionKind>,
    /// The trigger carried external content, which always confirms regardless of
    /// mode (prompt-injection containment) , the key incident-reconstruction fact.
    pub external_trigger: bool,
    /// The base disposition absent the two overrides above.
    pub basis: DecisionBasis,
}

/// The base disposition of a decision, before the always-confirm overrides
/// (external trigger, high-impact class) are applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecisionBasis {
    /// The per-application action mode decided it (the suggest flow, where the
    /// user executes the proposal manually).
    Mode,
    /// A proven, reversible action was lifted to previewed execution.
    ProvenReversible,
    /// An executing decision could not be proven safe (no rule, an unprovable
    /// invocation, an irreversible effect, or a timed-out proof), so the
    /// conservative cap (confirmation) stands. Only the cause when neither
    /// override applied.
    Unproven,
    /// Confirmation was forced purely by an override (external/high-impact); the
    /// base disposition is not itself the cause.
    Overridden,
}

impl DecisionReason {
    /// The plain suggest-mode flow (propose), no overrides.
    pub fn mode() -> Self {
        Self { high_impact: None, external_trigger: false, basis: DecisionBasis::Mode }
    }
    /// A proven, reversible lift to preview, no overrides.
    pub fn proven_reversible() -> Self {
        Self { high_impact: None, external_trigger: false, basis: DecisionBasis::ProvenReversible }
    }
    /// An external-trigger-only confirmation.
    pub fn external() -> Self {
        Self { high_impact: None, external_trigger: true, basis: DecisionBasis::Overridden }
    }
    /// A high-impact-class-only confirmation.
    pub fn high_impact(kind: ActionKind) -> Self {
        Self { high_impact: Some(kind), external_trigger: false, basis: DecisionBasis::Overridden }
    }
}

/// The gate's verdict for one proposed action, plus the ledger index of
/// the audit entry that recorded it. The executor attaches `audit_index`
/// to the subsequent execution/outcome record so the two link in the
/// ledger.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GateReceipt {
    /// What the gate decided.
    pub decision: ActionDecision,
    /// The faithful reason for that decision.
    pub reason: DecisionReason,
    /// The audit ledger index of the recorded decision.
    pub audit_index: u64,
}

/// Why the gate refused to let an action proceed.
#[derive(Debug, thiserror::Error)]
pub enum GateError {
    /// The audit ledger could not record the decision. Fail-closed: the
    /// action must not be surfaced or executed.
    #[error("audit log unavailable, action refused: {0}")]
    AuditUnavailable(String),
    /// The proposed tool is not in the behaviour's declared `tools` scope.
    /// A behaviour may only ever act through the tools it declared, so an
    /// out-of-scope proposal (a compromised or buggy behaviour) is refused.
    #[error("tool '{tool}' is not in the behaviour's declared scope")]
    ToolOutOfScope {
        /// The out-of-scope tool the proposal named.
        tool: String,
    },
    /// A proposal's operand named a reserved canary id (canary-honeytools.md §3).
    /// Honest operands are real ingestion ids that never bear the reserved
    /// prefix, so a canary touch is deterministic proof the operand came from an
    /// external injection. The action is refused and, in the agent loop, the loop
    /// is stopped closed: a tripped run is treated as hijacked, never fed back for
    /// the model to retry. The message carries no operand content.
    #[error("structural canary touched: a proposal operand named a reserved canary id (probable prompt-injection)")]
    CanaryTripped,
    /// A proposal named a honeytool (canary-honeytools.md §2): an attractive bait
    /// tool that honest behaviour never proposes, so a selection is deterministic
    /// proof of hijack. Refused, and in the agent loop the loop is stopped closed
    /// rather than fed back. The message carries no operand content.
    #[error("honeytool selected: a proposal named a bait tool no honest behaviour uses (probable prompt-injection)")]
    HoneytoolTripped,
}

/// The action gate, holding the long-lived collaborators: the capability, the
/// audit sink, the observer seam, and the system path/mount resolvers the
/// predict-before-act step reads through. The graph is *not* held here: it is
/// passed per call to [`Gate::decide_action`] as the behaviour-scoped handle
/// the dispatcher chose (a denying handle for a `reads: minimal` behaviour),
/// so the proof can never read more of the graph than the behaviour may.
pub struct Gate<'a> {
    capability: &'a Capability,
    audit: &'a dyn AuditSink,
    observer: &'a dyn GateObserver,
    paths: &'a dyn PathResolver,
    mounts: &'a dyn MountPolicy,
}

impl<'a> Gate<'a> {
    /// Build a gate over its collaborators.
    pub fn new(
        capability: &'a Capability,
        audit: &'a dyn AuditSink,
        observer: &'a dyn GateObserver,
        paths: &'a dyn PathResolver,
        mounts: &'a dyn MountPolicy,
    ) -> Self {
        Self {
            capability,
            audit,
            observer,
            paths,
            mounts,
        }
    }

    /// The pre-gate tripwires (canary-honeytools.md §2-§3): two deterministic
    /// hijack proofs checked before scope and before any proof, so they fire even
    /// in suggest-mode. First the **honeytool** (a proposed bait tool name that
    /// honest behaviour never names), then the **structural canary** (an operand
    /// that mentions a reserved canary id, which an honest ingestion id never
    /// does). Either audits a content-free trip outcome fail-closed and returns
    /// its error; the audit records the class and the external-trigger flag only,
    /// never the tool, operand value or specific canary id.
    async fn pre_gate_tripwires(
        &self,
        behaviour_name: &str,
        action: &ProposedAction,
        ctx: &ActionContext<'_>,
    ) -> Result<(), GateError> {
        if registry::is_honeytool(&action.tool) {
            self.audit
                .submit(behaviour_policy_violation_event(
                    behaviour_name,
                    honeytool_trip_outcome(ctx.external_trigger),
                    ctx.correlation_id,
                ))
                .await
                .map_err(|e| GateError::AuditUnavailable(e.to_string()))?;
            return Err(GateError::HoneytoolTripped);
        }
        if canary::touched_by(&action.arguments).is_some() {
            self.audit
                .submit(behaviour_policy_violation_event(
                    behaviour_name,
                    canary_trip_outcome(ctx.external_trigger),
                    ctx.correlation_id,
                ))
                .await
                .map_err(|e| GateError::AuditUnavailable(e.to_string()))?;
            return Err(GateError::CanaryTripped);
        }
        Ok(())
    }

    /// Run the pre-gate tripwires without the full action gate. The agent loop
    /// calls this for EVERY proposal before it splits into the read branch: a
    /// declared read tool is gate-exempt (D9) and never reaches
    /// [`Gate::decide_action`], so without this the tripwires would not cover
    /// reads (the channel that renders results verbatim into the model transcript,
    /// the dangerous one §2-§3 identify) and a honeytool named like a read tool
    /// would execute as one. A trip is audited and returns its error; the caller
    /// stops the loop closed.
    pub async fn screen_pre_gate(
        &self,
        behaviour_name: &str,
        action: &ProposedAction,
        ctx: &ActionContext<'_>,
    ) -> Result<(), GateError> {
        self.pre_gate_tripwires(behaviour_name, action, ctx).await
    }

    /// Decide the gate for one proposed action: resolve the capability
    /// decision, record it in the audit ledger fail-closed, notify the
    /// observer, and return a [`GateReceipt`] for the caller to act on.
    ///
    /// `behaviour_name` must be a validated kebab-case behaviour name (it
    /// becomes the content-free audit subject). `external_trigger` and
    /// `correlation_id` are supplied by the trusted dispatcher, never the
    /// proposal (see the trust-boundary note on this module).
    pub async fn decide_action(
        &self,
        behaviour_name: &str,
        ceiling: BaselineMode,
        tools: &BTreeMap<String, Vec<String>>,
        action: &ProposedAction,
        ctx: &ActionContext<'_>,
        graph: &dyn GraphHandle,
    ) -> Result<GateReceipt, GateError> {
        // Pre-gate tripwires (canary-honeytools.md §2-§3): honeytool + structural
        // canary, checked FIRST, before tool-scope and before the predict-before-
        // act proof. Either is deterministic proof of hijack, so it is the
        // strongest refusal and must fire regardless of tool scope or mode. (The
        // agent loop also runs these through `screen_pre_gate` before the read
        // branch, since a read tool is gate-exempt and never reaches this gate.)
        self.pre_gate_tripwires(behaviour_name, action, ctx).await?;

        // Tool-scope enforcement: a behaviour may only act through a tool
        // it declared. An out-of-scope proposal is refused fail-closed and
        // still audited (a scope violation is AI activity worth recording).
        // NB: only the tool *name* is enforced here; the scope-list *values*
        // (e.g. `fs.move: [~/Downloads]`) need structured action arguments
        // to verify and are enforced by the B2 world-model/executor layer.
        if !tools.contains_key(&action.tool) {
            // Record the content-free causes (external trigger, high-impact class
            // by tool name) on the refusal too, so a prompt-injection attempt to
            // call an undeclared high-impact tool (delete_file, send_email) is
            // reconstructable from the durable ledger, not flattened to a bare
            // "out of scope". The tool is unregistered here, so reversibility is
            // not derivable; name-based classification is what applies.
            let outcome =
                refusal_outcome(action_kind_for_tool(&action.tool), ctx.external_trigger);
            self.audit
                .submit(behaviour_action_event(
                    behaviour_name,
                    outcome,
                    ctx.correlation_id,
                ))
                .await
                .map_err(|e| GateError::AuditUnavailable(e.to_string()))?;
            return Err(GateError::ToolOutOfScope {
                tool: action.tool.clone(),
            });
        }

        // Resolve the tool's trusted schema ONCE: the reversibility lift-bit, the
        // action-kind classification, and the predict-before-act proof below all
        // bind THIS resolution, so the gate cannot classify, lift, and prove
        // against three different schemas (a single source for every
        // schema-derived part of the decision; today `lookup` is pure so the
        // three were already identical, but threading one keeps that structural
        // if a future dynamic-rule path makes resolution context-dependent).
        let trusted = registry::lookup(&action.tool);

        // Reversibility (Foundation B1) grounds the gate's high-impact logic:
        // "reversible" was assumed but never defined, leaving it circular. An
        // action is reversible iff its registry-resolved schema has a derivable
        // compensation. `None` = unregistered (unmodelled, not a *declared*
        // irreversibility), `Some(false)` = registered but irreversible,
        // `Some(true)` = reversible. A static property of the schema's effects.
        let schema_reversible = trusted.as_ref().map(|t| t.is_reversible());

        // Classify the proposed tool with the shared always-confirm classifier
        // (the same one MCP dispatch and the live executor use) — never a risk
        // class taken from the proposal. A tool already high-impact by name keeps
        // that specific class; otherwise a registered-but-irreversible schema
        // escalates to `Irreversible` (also high-impact), so an irreversible
        // action always requires confirmation in EVERY mode, not only the
        // executing one. An unregistered tool stays Ordinary and is held back
        // instead by the lift below, which needs a proof it cannot get. Combine
        // with the mode (ceiling ∧ grant) and the external-trigger override.
        let kind = resolved_action_kind_from(&action.tool, trusted.as_ref());
        // The external-trigger override applies UNLESS the acting behaviour is a
        // deterministic workflow (the Tim-approved carve-out): a workflow makes no
        // model call, so external content cannot inject an action into it, so the
        // prompt-injection defense is vacuous there. `kind: agent` keeps the
        // override (it has a model, hence an injection surface). A high-impact /
        // irreversible `kind` still always-confirms via `always_requires_confirmation`
        // below regardless, so the carve-out never lifts an irreversible action.
        let external_override = ctx.external_trigger && !ctx.deterministic_workflow;
        let decision =
            self.capability
                .decide_for_behaviour(ctx.app_id, kind, external_override, ceiling);

        // Predict-before-act. An executing decision (PreviewThenExecute /
        // Proceed) is only authorised autonomously if the world model proves
        // *this* invocation safe: its trusted, registry-resolved schema holds
        // against the action's operands and a bounded graph slice. A `Valid`
        // prediction lifts the conservative cap to the capability's real
        // decision; no rule, an unprovable invocation, or any failure keeps
        // the cap (downgrade to explicit confirmation). Suggest/Propose is
        // unaffected (there the user executes manually). The proof runs only
        // for an executing decision (the cap would not change the others), and
        // the lifted decision is what the audit below records.
        // The proof reads the graph and the filesystem, so bound it: a stalled
        // knowledge socket or a slow path lookup must fail closed (unproven,
        // so the cap stands) rather than park the gate and stall later
        // dispatch. A timeout is treated exactly like an unprovable action.
        let proven = if matches!(
            decision,
            ActionDecision::PreviewThenExecute | ActionDecision::Proceed
        ) {
            tokio::time::timeout(
                PROOF_TIMEOUT,
                self.prove_action(action, trusted.as_ref(), kind, ctx, ceiling, graph),
            )
            .await
            .unwrap_or(false)
        } else {
            false
        };

        let decision = match decision {
            // A proven, reversible executing action is lifted to a silent,
            // immediate execution ONLY for a deterministic `kind: workflow` (the
            // promotion-spine curation: no model call, zero tokens, reviewed after
            // the fact via the pull activity view). A `kind: agent` (judgment)
            // action is NEVER silently lifted, even when proven reversible: the
            // agent has a model (hence a judgment/injection surface), so per the
            // go-live decision "the agent never silently writes - everything it
            // does is seen/confirmed" it stays `RequireConfirmation`, surfaced as a
            // gate card whose `[Approve]` runs it. So the silent auto-execute lift
            // is gated on `ctx.deterministic_workflow`.
            //
            // (`PreviewThenExecute` is the silent-execute decision the workflow
            // dispatch path acts on immediately; `RequireConfirmation` is the
            // human-approved path `proposal_view` surfaces. The two boundaries that
            // still cap even the workflow lift to a *previewed* rather than fully
            // autonomous `Proceed`: (1) the proof is a point-in-time slice with no
            // graph snapshot, gap A2, so the executor re-checks atomically at write
            // time; (2) the per-app grant is the agent's own, the coarse model.)
            // Only a `Some(true)` reversible schema is lifted: an irreversible one
            // was already escalated to `Irreversible` above (so it never reaches
            // this arm), and an unregistered tool (`None`) cannot be proven, so
            // both stay confirmation.
            ActionDecision::PreviewThenExecute | ActionDecision::Proceed => {
                if proven && schema_reversible == Some(true) && ctx.deterministic_workflow {
                    ActionDecision::PreviewThenExecute
                } else {
                    ActionDecision::RequireConfirmation
                }
            }
            keep @ (ActionDecision::Propose | ActionDecision::RequireConfirmation) => keep,
        };

        // The faithful reason for the final decision, read off the gate's own
        // logic (never the model's rationale), recording ALL causes that apply.
        // The base disposition: a previewed execution is the proven-reversible
        // lift; a propose is the plain suggest flow; a confirmation is either an
        // override (high-impact and/or external) or an executing decision that
        // could not be proven/lifted. The two overrides are recorded
        // independently so a high-impact action's external-content provenance is
        // never lost.
        let basis = match decision {
            ActionDecision::PreviewThenExecute => DecisionBasis::ProvenReversible,
            ActionDecision::Propose => DecisionBasis::Mode,
            // The lift caps `Proceed` to a preview, so it is unreachable here;
            // report it faithfully as the mode flow rather than assert.
            ActionDecision::Proceed => DecisionBasis::Mode,
            ActionDecision::RequireConfirmation => {
                if kind.always_requires_confirmation() || external_override {
                    DecisionBasis::Overridden
                } else {
                    DecisionBasis::Unproven
                }
            }
        };
        let reason = DecisionReason {
            high_impact: kind.always_requires_confirmation().then_some(kind),
            external_trigger: ctx.external_trigger,
            basis,
        };

        // Audit-before-acting, fail-closed. The decision AND its faithful reason
        // are committed to the ledger before the caller is told what it is, so
        // there is no path on which the action is surfaced/executed without a
        // durable record of what was decided and why, even if the dispatch is
        // aborted (reload/shutdown) before the in-memory outcome is logged.
        let audit_index = self
            .audit
            .submit(behaviour_action_event(
                behaviour_name,
                format!("{}:{}", decision_label(decision), reason_label(reason)),
                ctx.correlation_id,
            ))
            .await
            .map_err(|e| GateError::AuditUnavailable(e.to_string()))?;

        self.observer.observed(&decision);
        Ok(GateReceipt {
            decision,
            reason,
            audit_index,
        })
    }

    /// Whether the world model proves this invocation safe: its trusted,
    /// registry-resolved schema's preconditions hold and its effects apply
    /// cleanly over a bounded graph slice for the action's operands. Fails
    /// closed (returns `false`) on no registered rule, any slice-build failure
    /// (an unreachable graph, a malformed result, an unresolved path, an
    /// operand the schema does not name), or a prediction that is not `Valid`.
    /// A `false` here is not a refusal, it just means the conservative cap
    /// stands.
    ///
    /// The proof binds the trusted schema (resolved by the tool id) and the
    /// invocation's exact operands to a specific effect (e.g. the schema's
    /// `AssertEdge`). The executor that eventually acts on a lifted decision
    /// must execute *that proven effect* with *those operands*, not a free-form
    /// re-interpretation of the tool name, or a different mutation could ride
    /// on the proof. That obligation, with the atomic precondition re-check, is
    /// the executor's (it does not exist yet).
    async fn prove_action(
        &self,
        action: &ProposedAction,
        trusted: Option<&registry::TrustedActionSchema>,
        kind: ActionKind,
        ctx: &ActionContext<'_>,
        ceiling: BaselineMode,
        graph: &dyn GraphHandle,
    ) -> bool {
        // The schema must come from the trusted registry, never the proposal;
        // with no registered rule the action cannot be proven. It is the same
        // resolution the caller classified and lifted against (threaded in), so
        // the proof cannot bind a different schema than the decision used.
        let Some(trusted) = trusted else {
            return false;
        };
        // Build the bounded slice for this invocation's operands, reading
        // through the behaviour-scoped `graph` the caller passed (a denying
        // handle for a `reads: minimal` behaviour, so the proof cannot read
        // more than the behaviour may). `arguments` is the (untrusted) operand
        // set; the schema and the slice are what make a `Valid` prediction
        // trustworthy. Any build failure fails closed.
        let (state, bindings) = match build_slice_trusted(
            trusted,
            &action.tool,
            &action.arguments,
            graph,
            self.paths,
            self.mounts,
        )
        .await
        {
            Ok(slice) => slice,
            Err(_) => return false,
        };
        let eval = EvalContext {
            capability: self.capability,
            action_id: &action.tool,
            app_id: ctx.app_id,
            action_kind: kind,
            external_trigger: ctx.external_trigger,
            ceiling,
        };
        world::predict(trusted.schema(), &bindings, &state, &eval).is_valid()
    }
}

/// Classify the proposed tool into a capability [`ActionKind`] using the
/// shared [`AlwaysConfirm`] classifier (the same patterns the MCP dispatch
/// layer uses). A tool not in the always-confirm set is [`ActionKind::Ordinary`];
/// every confirm-set reason maps to a high-impact kind whose
/// `always_requires_confirmation()` is true. Generic command execution has
/// no narrower kind — it maps to [`ActionKind::ElevatedPrivilege`], which
/// (correctly) forces confirmation.
/// Resolve an action's risk class for a tool: the name-based class, escalated to
/// `Irreversible` when the registry-resolved schema has no derivable
/// compensation (so an irreversible action always confirms). Shared by the gate
/// decision and the live executor's re-validation, so both classify a tool
/// identically and the executor cannot under-classify what the gate proved.
pub(crate) fn resolved_action_kind(tool: &str) -> ActionKind {
    resolved_action_kind_from(tool, registry::lookup(tool).as_ref())
}

/// [`resolved_action_kind`] over an already-resolved schema, so the gate
/// decision can resolve a tool's schema ONCE and then classify, lift, and prove
/// against the same resolution (the three uses cannot bind three different
/// schemas). `trusted` is the registry's resolution for `tool`, or `None` when
/// unregistered.
fn resolved_action_kind_from(
    tool: &str,
    trusted: Option<&registry::TrustedActionSchema>,
) -> ActionKind {
    let base_kind = action_kind_for_tool(tool);
    // A non-graph schema declares a three-way `InverseClass` on its
    // `Effect::External`; map it (the single reversibility source, §3.2). A
    // pure-graph schema has none, so fall back to the op-id self-inverse bool: a
    // non-confirm tool whose graph effects are not reversible is `Irreversible`.
    if let Some(class) = trusted.and_then(|t| t.declared_inverse_class()) {
        return action_kind_from_class(class, base_kind);
    }
    let schema_reversible = trusted.map(|t| t.is_reversible());
    if !base_kind.always_requires_confirmation() && schema_reversible == Some(false) {
        ActionKind::Irreversible
    } else {
        base_kind
    }
}

/// Map a declared [`InverseClass`] to the audit-honest [`ActionKind`] (§3.2): a
/// `Reversible` effect keeps the tool's base kind (it may still be lifted), a
/// `ReversibleWithCost` always confirms with the cost surfaced, and an
/// `Irreversible` always confirms. The single reversibility source read for the
/// kind, the twin of `is_reversible` reading it for the lift bit.
fn action_kind_from_class(class: InverseClass, base: ActionKind) -> ActionKind {
    match class {
        InverseClass::Reversible { .. } => base,
        InverseClass::ReversibleWithCost { .. } => ActionKind::ReversibleWithCost,
        InverseClass::Irreversible { .. } => ActionKind::Irreversible,
    }
}

fn action_kind_for_tool(tool: &str) -> ActionKind {
    match AlwaysConfirm::classify(tool) {
        None => ActionKind::Ordinary,
        Some(AlwaysConfirmReason::FileDeletion) => ActionKind::PermanentDelete,
        Some(AlwaysConfirmReason::ExternalMessage) => ActionKind::SendExternalMessage,
        Some(AlwaysConfirmReason::PackageChange) => ActionKind::PackageChange,
        Some(AlwaysConfirmReason::SystemConfigWrite) => ActionKind::SystemConfigChange,
        Some(AlwaysConfirmReason::ElevatedCommand | AlwaysConfirmReason::GenericExecution) => {
            ActionKind::ElevatedPrivilege
        }
    }
}

/// The coarse, content-free decision label recorded in the audit ledger.
fn decision_label(decision: ActionDecision) -> &'static str {
    match decision {
        ActionDecision::Propose => "propose",
        ActionDecision::PreviewThenExecute => "preview-then-execute",
        ActionDecision::Proceed => "proceed",
        ActionDecision::RequireConfirmation => "require-confirmation",
    }
}

/// A content-free label for the faithful decision reason (D2), folded into the
/// durable audit `outcome` so the ledger records *why* a decision was made, not
/// only what it was. This survives a dispatch aborted after the audit write
/// (a reload/shutdown winning before the outcome is logged), which is exactly
/// the post-incident path where the reason matters most. ALL applicable causes
/// are joined with `+` (e.g. `external-trigger+high-impact-permanent-delete`),
/// so a high-impact action's external-content provenance is never lost. The
/// high-impact label preserves the specific action class (still content-free: a
/// class, never the operands).
pub(crate) fn reason_label(reason: DecisionReason) -> String {
    let mut parts: Vec<&str> = Vec::new();
    if reason.external_trigger {
        parts.push("external-trigger");
    }
    if let Some(kind) = reason.high_impact {
        parts.push(high_impact_class_label(kind));
    }
    match reason.basis {
        DecisionBasis::Mode => parts.push("mode"),
        DecisionBasis::ProvenReversible => parts.push("proven-reversible"),
        DecisionBasis::Unproven => parts.push("unproven"),
        // The overrides already pushed above are the cause; nothing to add.
        DecisionBasis::Overridden => {}
    }
    if parts.is_empty() {
        parts.push("unspecified");
    }
    parts.join("+")
}

/// The durable audit outcome for an out-of-scope refusal, with the same
/// content-free cause set as a decision: the external-trigger fact and the
/// high-impact class (by tool name, since an undeclared tool has no schema), so
/// an injection attempt to call an undeclared destructive tool is reconstructable
/// rather than flattened to a bare "out of scope".
fn refusal_outcome(kind: ActionKind, external_trigger: bool) -> String {
    let mut causes: Vec<&str> = Vec::new();
    if external_trigger {
        causes.push("external-trigger");
    }
    if kind.always_requires_confirmation() {
        causes.push(high_impact_class_label(kind));
    }
    if causes.is_empty() {
        "refused-out-of-scope".to_string()
    } else {
        format!("refused-out-of-scope:{}", causes.join("+"))
    }
}

/// The content-free outcome for a structural-canary trip. Records the class
/// (a canary was touched) and the external-trigger flag, never the operand value
/// or the specific canary id, so the durable ledger shows a hijack tripwire fired
/// without leaking what named it.
fn canary_trip_outcome(external_trigger: bool) -> String {
    if external_trigger {
        "canary-tripped:structural+external-trigger".to_string()
    } else {
        "canary-tripped:structural".to_string()
    }
}

/// The content-free outcome for a honeytool trip. Records the class (a honeytool
/// was selected) and the external-trigger flag, never the tool name, so the
/// durable ledger shows a hijack tripwire fired without leaking which bait.
fn honeytool_trip_outcome(external_trigger: bool) -> String {
    if external_trigger {
        "honeytool-tripped+external-trigger".to_string()
    } else {
        "honeytool-tripped".to_string()
    }
}

/// The content-free label for a high-impact action class (a class, never the
/// operands), so the ledger distinguishes an irreversible link from a permanent
/// delete or an external message.
fn high_impact_class_label(kind: ActionKind) -> &'static str {
    match kind {
        ActionKind::PermanentDelete => "high-impact-permanent-delete",
        ActionKind::SendExternalMessage => "high-impact-external-message",
        ActionKind::PackageChange => "high-impact-package-change",
        ActionKind::SystemConfigChange => "high-impact-system-config",
        ActionKind::UndeclaredNetwork => "high-impact-undeclared-network",
        ActionKind::ElevatedPrivilege => "high-impact-elevated-privilege",
        ActionKind::Irreversible => "high-impact-irreversible",
        ActionKind::ReversibleWithCost => "reversible-with-cost",
        // Ordinary is not high-impact, so it never reaches here; defensive.
        ActionKind::Ordinary => "high-impact",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    use std::collections::HashMap;

    use audit_proto::MockAuditSink;
    use arlen_ai_core::capability::{AccessTier, ActionPermissions};

    #[test]
    fn action_kind_from_class_maps_the_three_way() {
        use crate::effect_model::{CaptureShape, IrreversibilityReason, ResidualCost};
        // Reversible keeps the base kind (still possibly liftable).
        assert!(matches!(
            action_kind_from_class(
                InverseClass::Reversible { capture: CaptureShape::RestorePath },
                ActionKind::Ordinary,
            ),
            ActionKind::Ordinary
        ));
        // Reversible does not downgrade a high-impact base kind.
        assert!(matches!(
            action_kind_from_class(
                InverseClass::Reversible { capture: CaptureShape::RestorePath },
                ActionKind::PermanentDelete,
            ),
            ActionKind::PermanentDelete
        ));
        // With-cost always confirms.
        assert!(matches!(
            action_kind_from_class(
                InverseClass::ReversibleWithCost {
                    capture: CaptureShape::RestoreValue,
                    cost: ResidualCost::Fee,
                },
                ActionKind::Ordinary,
            ),
            ActionKind::ReversibleWithCost
        ));
        // Irreversible always confirms.
        assert!(matches!(
            action_kind_from_class(
                InverseClass::Irreversible { reason: IrreversibilityReason::ExternalSend },
                ActionKind::Ordinary,
            ),
            ActionKind::Irreversible
        ));
    }

    use crate::seams::{DeniedGraph, GraphError};
    use crate::slice::{FsPathResolver, SliceError, StaticMountPolicy};

    // A recording observer doubles as the GateObserver test double.
    #[derive(Default)]
    struct Recorder(Mutex<Vec<ActionDecision>>);
    impl GateObserver for Recorder {
        fn observed(&self, decision: &ActionDecision) {
            self.0.lock().unwrap().push(*decision);
        }
    }

    fn action() -> ProposedAction {
        ProposedAction {
            tool: "graph.write".to_string(),
            summary: "tag foo.rs as part of arlenos".to_string(),
            arguments: BTreeMap::new(),
        }
    }

    fn irreversible_action() -> ProposedAction {
        ProposedAction {
            tool: "test.irreversible".to_string(),
            summary: "an action with no derivable compensation".to_string(),
            arguments: BTreeMap::new(),
        }
    }

    #[tokio::test]
    async fn an_irreversible_action_requires_confirmation_even_in_suggest_mode() {
        // Foundation B1: an action whose registry schema has no compensation is
        // irreversible -> high-impact -> always confirm, in EVERY mode. Under
        // Suggest the preliminary decision would be Propose; the schema-derived
        // Irreversible classification escalates it to RequireConfirmation.
        let cap = suggest_only();
        let audit = MockAuditSink::accepting();
        let obs = Recorder::default();
        let receipt = Gate::new(&cap, &audit, &obs, &FsPathResolver, &StaticMountPolicy::empty())
            .decide_action(
                "some-behaviour",
                BaselineMode::Suggest,
                &scope(&["test.irreversible"]),
                &irreversible_action(),
                &ctx(false, "run-irrev"),
                &DeniedGraph,
            )
            .await
            .expect("accepting sink");
        assert_eq!(receipt.decision, ActionDecision::RequireConfirmation);
        // The faithful reason is the schema-derived irreversibility, not the mode.
        assert_eq!(
            receipt.reason,
            DecisionReason::high_impact(ActionKind::Irreversible)
        );
        // The durable audit preserves the specific high-impact class, not just
        // "high-impact", so a post-incident reader knows it was irreversibility.
        assert_eq!(
            audit.recorded().await[0].structural.outcome,
            "require-confirmation:high-impact-irreversible"
        );
    }

    #[tokio::test]
    async fn an_external_high_impact_action_records_both_causes() {
        // An irreversible action *also* triggered by external content confirms
        // for both reasons. The durable audit must record BOTH, so a
        // prompt-injection incident reconstruction does not lose the external
        // provenance of a high-impact action.
        let cap = suggest_only();
        let audit = MockAuditSink::accepting();
        let obs = Recorder::default();
        let receipt = Gate::new(&cap, &audit, &obs, &FsPathResolver, &StaticMountPolicy::empty())
            .decide_action(
                "some-behaviour",
                BaselineMode::Suggest,
                &scope(&["test.irreversible"]),
                &irreversible_action(),
                &ctx(true, "run-both"), // external trigger AND irreversible
                &DeniedGraph,
            )
            .await
            .expect("accepting sink");
        assert_eq!(receipt.decision, ActionDecision::RequireConfirmation);
        assert!(receipt.reason.external_trigger);
        assert_eq!(receipt.reason.high_impact, Some(ActionKind::Irreversible));
        assert_eq!(
            audit.recorded().await[0].structural.outcome,
            "require-confirmation:external-trigger+high-impact-irreversible"
        );
    }

    /// A trusted action context targeting a fixed app.
    fn ctx<'a>(external: bool, correlation_id: &'a str) -> ActionContext<'a> {
        ActionContext {
            app_id: "org.arlen.files",
            external_trigger: external,
            deterministic_workflow: false,
            correlation_id,
        }
    }

    /// A trusted context for a deterministic `kind: workflow` behaviour (the
    /// external-trigger carve-out applies). Used to verify an externally-triggered
    /// workflow is not force-confirmed while an agent on the same trigger is.
    fn ctx_workflow<'a>(external: bool, correlation_id: &'a str) -> ActionContext<'a> {
        ActionContext {
            app_id: "org.arlen.files",
            external_trigger: external,
            deterministic_workflow: true,
            correlation_id,
        }
    }

    /// A declared tool scope containing exactly the given tool names.
    fn scope(names: &[&str]) -> BTreeMap<String, Vec<String>> {
        names.iter().map(|n| (n.to_string(), Vec::new())).collect()
    }

    fn suggest_only() -> Capability {
        Capability::new(AccessTier::Full, ActionPermissions::suggest_only())
    }

    #[tokio::test]
    async fn proposes_and_audits_a_suggest_behaviour() {
        let cap = suggest_only();
        let audit = MockAuditSink::accepting();
        let obs = Recorder::default();

        let receipt = Gate::new(&cap, &audit, &obs, &FsPathResolver, &StaticMountPolicy::empty())
            .decide_action(
                "auto-tag-by-project",
                BaselineMode::Suggest,
                &scope(&["graph.write"]),
                &action(),
                &ctx(false, "run-1"),
                &DeniedGraph,
            )
            .await
            .expect("accepting sink");

        assert_eq!(receipt.decision, ActionDecision::Propose);
        assert_eq!(receipt.reason, DecisionReason::mode());
        assert_eq!(receipt.audit_index, 0);
        // The decision was recorded, content-free + correlated, before returning.
        let recorded = audit.recorded().await;
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].structural.subject, "agent.auto-tag-by-project");
        // The durable audit records the decision AND its faithful reason.
        assert_eq!(recorded[0].structural.outcome, "propose:mode");
        assert_eq!(recorded[0].call_chain_id.as_deref(), Some("run-1"));
        assert_eq!(obs.0.lock().unwrap().as_slice(), &[ActionDecision::Propose]);
    }

    #[tokio::test]
    async fn a_canary_operand_trips_the_freeze_before_tool_scope() {
        // A reserved canary id in the operands is proof of prompt-injection: the
        // gate refuses with CanaryTripped and records a content-free trip outcome.
        // It fires even though the tool IS in scope, proving the canary check runs
        // pre-scope, and it does not notify the observer (a refusal, not a decision).
        let cap = suggest_only();
        let audit = MockAuditSink::accepting();
        let obs = Recorder::default();
        let act = ProposedAction {
            tool: "graph.write".to_string(),
            summary: "x".to_string(),
            arguments: BTreeMap::from([(
                "project".to_string(),
                crate::canary::CANARY_IDS[0].to_string(),
            )]),
        };

        let err = Gate::new(&cap, &audit, &obs, &FsPathResolver, &StaticMountPolicy::empty())
            .decide_action(
                "auto-tag-by-project",
                BaselineMode::Suggest,
                &scope(&["graph.write"]),
                &act,
                &ctx(true, "run-canary"),
                &DeniedGraph,
            )
            .await
            .expect_err("a canary touch must refuse");

        assert!(matches!(err, GateError::CanaryTripped));
        let recorded = audit.recorded().await;
        assert_eq!(recorded.len(), 1);
        assert_eq!(
            recorded[0].structural.outcome,
            "canary-tripped:structural+external-trigger"
        );
        assert_eq!(recorded[0].structural.subject, "agent.auto-tag-by-project");
        // A trip is a deterministic hijack proof, classified as a policy
        // violation so a ledger reader can surface it by kind.
        assert_eq!(recorded[0].kind, audit_proto::AuditKind::PolicyViolation);
        assert!(obs.0.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_honeytool_proposal_trips_the_freeze_before_tool_scope() {
        // Proposing a honeytool is deterministic proof of hijack: the gate refuses
        // with HoneytoolTripped and records a content-free trip outcome, firing
        // even when the bait IS in the declared scope (so it is the high-signal
        // trip, not a low-signal ToolOutOfScope) and without notifying the observer.
        let cap = suggest_only();
        let audit = MockAuditSink::accepting();
        let obs = Recorder::default();
        let act = ProposedAction {
            tool: "export_all_secrets".to_string(),
            summary: "x".to_string(),
            arguments: BTreeMap::new(),
        };

        let err = Gate::new(&cap, &audit, &obs, &FsPathResolver, &StaticMountPolicy::empty())
            .decide_action(
                "auto-tag-by-project",
                BaselineMode::Suggest,
                &scope(&["export_all_secrets"]),
                &act,
                &ctx(false, "run-ht"),
                &DeniedGraph,
            )
            .await
            .expect_err("a honeytool must refuse");

        assert!(matches!(err, GateError::HoneytoolTripped));
        let recorded = audit.recorded().await;
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].structural.outcome, "honeytool-tripped");
        assert_eq!(recorded[0].structural.subject, "agent.auto-tag-by-project");
        assert_eq!(recorded[0].kind, audit_proto::AuditKind::PolicyViolation);
        assert!(obs.0.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn high_impact_tool_requires_confirmation_even_under_supervised() {
        // A supervised behaviour proposing a destructive tool must confirm,
        // not preview-then-execute: the tool name is classified by the
        // shared always-confirm classifier, never trusted from the proposal.
        let cap = Capability::new(
            AccessTier::Full,
            ActionPermissions::new(BaselineMode::Supervised, Vec::<String>::new()),
        );
        let audit = MockAuditSink::accepting();
        let obs = Recorder::default();
        for tool in ["delete_file", "send_email", "pkg_uninstall", "shell_exec", "sudo_thing"] {
            let act = ProposedAction {
                tool: tool.to_string(),
                summary: "x".to_string(),
                arguments: BTreeMap::new(),
            };
            let receipt = Gate::new(&cap, &audit, &obs, &FsPathResolver, &StaticMountPolicy::empty())
                .decide_action(
                    "tidy-downloads",
                    BaselineMode::Supervised,
                    &scope(&[tool]),
                    &act,
                    &ctx(false, "run-x"),
                    &DeniedGraph,
                )
                .await
                .unwrap();
            assert_eq!(
                receipt.decision,
                ActionDecision::RequireConfirmation,
                "destructive tool {tool} must require confirmation"
            );
        }
    }

    #[tokio::test]
    async fn external_trigger_forces_confirmation_regardless_of_mode() {
        // Even an autonomous-for-this-app behaviour must confirm when the
        // run was triggered by external content (a trusted dispatcher fact,
        // not a proposal claim).
        let cap = Capability::new(
            AccessTier::Full,
            ActionPermissions::new(BaselineMode::Suggest, ["org.arlen.files"]),
        );
        let audit = MockAuditSink::accepting();
        let obs = Recorder::default();
        let receipt = Gate::new(&cap, &audit, &obs, &FsPathResolver, &StaticMountPolicy::empty())
            .decide_action(
                "tidy-downloads",
                BaselineMode::Supervised,
                &scope(&["graph.write"]),
                &action(),
                &ctx(true, "run-2"), // external trigger
                &DeniedGraph,
            )
            .await
            .unwrap();
        assert_eq!(receipt.decision, ActionDecision::RequireConfirmation);
        assert_eq!(receipt.reason, DecisionReason::external());
        // The reason is durable in the ledger, not only in the in-memory outcome.
        assert_eq!(
            audit.recorded().await[0].structural.outcome,
            "require-confirmation:external-trigger"
        );
    }

    #[tokio::test]
    async fn fails_closed_when_audit_is_unavailable() {
        let cap = suggest_only();
        let audit = MockAuditSink::failing();
        let obs = Recorder::default();

        let err = Gate::new(&cap, &audit, &obs, &FsPathResolver, &StaticMountPolicy::empty())
            .decide_action(
                "auto-tag-by-project",
                BaselineMode::Suggest,
                &scope(&["graph.write"]),
                &action(),
                &ctx(false, "run-3"),
                &DeniedGraph,
            )
            .await
            .expect_err("failing audit must refuse the action");
        assert!(matches!(err, GateError::AuditUnavailable(_)));
        // Fail-closed: the decision was never handed to the observer.
        assert!(obs.0.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn b1_caps_autonomous_execution_to_confirmation() {
        // The app is granted autonomy; a Supervised-ceiling ordinary action
        // would resolve to PreviewThenExecute by the capability model, but
        // B1 has no argument/world-model validation, so the gate caps any
        // executing decision to explicit confirmation.
        let cap = Capability::new(
            AccessTier::Full,
            ActionPermissions::new(BaselineMode::Suggest, ["org.arlen.files"]),
        );
        let audit = MockAuditSink::accepting();
        let obs = Recorder::default();
        let receipt = Gate::new(&cap, &audit, &obs, &FsPathResolver, &StaticMountPolicy::empty())
            .decide_action(
                "auto-tag-by-project",
                BaselineMode::Supervised,
                &scope(&["graph.write"]),
                &action(),
                &ctx(false, "run-5"),
                &DeniedGraph,
            )
            .await
            .unwrap();
        assert_eq!(receipt.decision, ActionDecision::RequireConfirmation);
    }

    // --- predict-before-act: a Valid prediction lifts the conservative cap ---

    /// A graph returning canned rows when the query contains a needle.
    struct MockGraph(Vec<(&'static str, Vec<HashMap<String, serde_json::Value>>)>);

    #[async_trait::async_trait]
    impl GraphHandle for MockGraph {
        async fn query(
            &self,
            cypher: &str,
        ) -> Result<Vec<HashMap<String, serde_json::Value>>, GraphError> {
            for (needle, rows) in &self.0 {
                if cypher.contains(needle) {
                    return Ok(rows.clone());
                }
            }
            Ok(Vec::new())
        }
    }

    /// A resolver that accepts an already-canonical absolute path as itself.
    struct IdentityResolver;
    impl PathResolver for IdentityResolver {
        fn resolve(&self, raw: &str) -> Result<String, SliceError> {
            if raw.starts_with('/') {
                Ok(raw.to_string())
            } else {
                Err(SliceError::PathResolve {
                    raw: raw.to_string(),
                    reason: "not absolute".to_string(),
                })
            }
        }
    }

    fn row(pairs: &[(&str, serde_json::Value)]) -> HashMap<String, serde_json::Value> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
    }

    /// The graph for tagging `/proj/a.rs` (under `/proj`) to project `p1`,
    /// with the `FILE_PART_OF` edge present or not.
    fn tag_graph(linked: bool) -> MockGraph {
        MockGraph(vec![
            (
                "n:File {id: '/proj/a.rs'}",
                vec![row(&[("id", "/proj/a.rs".into()), ("path", "/proj/a.rs".into())])],
            ),
            (
                "n:Project {id: 'p1'}",
                vec![row(&[("id", "p1".into()), ("root_path", "/proj".into())])],
            ),
            (
                "count(*) AS cnt",
                vec![row(&[("cnt", serde_json::Value::from(i64::from(linked)))])],
            ),
        ])
    }

    fn graph_write_action() -> ProposedAction {
        ProposedAction {
            tool: "graph.write".to_string(),
            summary: "tag /proj/a.rs as part of p1".to_string(),
            arguments: BTreeMap::from([
                ("file".to_string(), "/proj/a.rs".to_string()),
                ("project".to_string(), "p1".to_string()),
            ]),
        }
    }

    /// The autonomy capability that resolves an ordinary Supervised-ceiling
    /// action to `PreviewThenExecute` (as in `b1_caps_...`).
    fn executing_cap() -> Capability {
        Capability::new(
            AccessTier::Full,
            ActionPermissions::new(BaselineMode::Suggest, ["org.arlen.files"]),
        )
    }

    #[tokio::test]
    async fn a_valid_prediction_lifts_a_deterministic_workflow() {
        let cap = executing_cap();
        let audit = MockAuditSink::accepting();
        let obs = Recorder::default();
        // The file lies under the project root and is not yet linked.
        let graph = tag_graph(false);
        let receipt = Gate::new(&cap, &audit, &obs, &IdentityResolver, &StaticMountPolicy::empty())
            .decide_action(
                "auto-tag-by-project",
                BaselineMode::Supervised,
                &scope(&["graph.write"]),
                &graph_write_action(),
                &ctx_workflow(false, "run-lift"), // a deterministic workflow
                &graph,
            )
            .await
            .unwrap();
        // The world model proved the link safe AND the actor is a deterministic
        // workflow, so the silent auto-execute lift stands instead of being capped
        // to confirmation.
        assert_eq!(receipt.decision, ActionDecision::PreviewThenExecute);
        assert_eq!(receipt.reason, DecisionReason::proven_reversible());
        assert_eq!(obs.0.lock().unwrap().as_slice(), &[ActionDecision::PreviewThenExecute]);
    }

    #[tokio::test]
    async fn a_proven_agent_action_is_confirm_gated_not_silently_lifted() {
        // The SAME proven-reversible action from a `kind: agent` (non-workflow,
        // has a model/judgment surface) is NEVER silently lifted: it stays
        // RequireConfirmation, surfaced as a gate card whose [Approve] runs it.
        // "The agent never silently writes" (go-live decision).
        let cap = executing_cap();
        let audit = MockAuditSink::accepting();
        let obs = Recorder::default();
        let graph = tag_graph(false);
        let receipt = Gate::new(&cap, &audit, &obs, &IdentityResolver, &StaticMountPolicy::empty())
            .decide_action(
                "auto-tag-by-project",
                BaselineMode::Supervised,
                &scope(&["graph.write"]),
                &graph_write_action(),
                &ctx(false, "run-agent-proven"), // a kind: agent, internal trigger
                &graph,
            )
            .await
            .unwrap();
        assert_eq!(receipt.decision, ActionDecision::RequireConfirmation);
    }

    #[tokio::test]
    async fn external_trigger_carve_out_lifts_a_deterministic_workflow() {
        // The Tim-approved carve-out: a file.opened-triggered (external) action
        // from a deterministic `kind: workflow` is NOT force-confirmed - it lifts
        // to PreviewThenExecute when proven-reversible, exactly as it does off an
        // internal trigger. This is the auto-tag go-live path.
        let cap = executing_cap();
        let audit = MockAuditSink::accepting();
        let obs = Recorder::default();
        let graph = tag_graph(false);
        let receipt = Gate::new(&cap, &audit, &obs, &IdentityResolver, &StaticMountPolicy::empty())
            .decide_action(
                "auto-tag-by-project",
                BaselineMode::Supervised,
                &scope(&["graph.write"]),
                &graph_write_action(),
                &ctx_workflow(true, "run-carveout"), // EXTERNAL trigger, but a workflow
                &graph,
            )
            .await
            .unwrap();
        assert_eq!(receipt.decision, ActionDecision::PreviewThenExecute);
        // The external-content fact is still recorded (incident reconstruction),
        // but it did not force confirmation: the basis is the proven lift.
        assert!(receipt.reason.external_trigger);
        assert_eq!(receipt.reason.basis, DecisionBasis::ProvenReversible);
        assert!(receipt.reason.high_impact.is_none());
    }

    #[tokio::test]
    async fn external_trigger_still_confirms_a_kind_agent_action() {
        // Containment untouched on the model path: the SAME external-triggered
        // proven-reversible action from a NON-workflow (an agent has a model,
        // hence an injection surface) is still force-confirmed.
        let cap = executing_cap();
        let audit = MockAuditSink::accepting();
        let obs = Recorder::default();
        let graph = tag_graph(false);
        let receipt = Gate::new(&cap, &audit, &obs, &IdentityResolver, &StaticMountPolicy::empty())
            .decide_action(
                "auto-tag-by-project",
                BaselineMode::Supervised,
                &scope(&["graph.write"]),
                &graph_write_action(),
                &ctx(true, "run-agent-ext"), // external trigger, NOT a workflow
                &graph,
            )
            .await
            .unwrap();
        assert_eq!(receipt.decision, ActionDecision::RequireConfirmation);
        assert!(receipt.reason.external_trigger);
    }

    #[tokio::test]
    async fn an_unprovable_action_keeps_the_cap() {
        let cap = executing_cap();
        let audit = MockAuditSink::accepting();
        let obs = Recorder::default();
        // The same action, but the edge already exists: `Not(EdgeExists)`
        // fails, so the prediction is not Valid and the cap stands.
        let graph = tag_graph(true);
        let receipt = Gate::new(&cap, &audit, &obs, &IdentityResolver, &StaticMountPolicy::empty())
            .decide_action(
                "auto-tag-by-project",
                BaselineMode::Supervised,
                &scope(&["graph.write"]),
                &graph_write_action(),
                &ctx(false, "run-cap"),
                &graph,
            )
            .await
            .unwrap();
        assert_eq!(receipt.decision, ActionDecision::RequireConfirmation);
    }

    #[tokio::test]
    async fn an_unregistered_tool_keeps_the_cap() {
        // A tool with no registry rule cannot be proven, so even an executing
        // capability decision is capped.
        let cap = executing_cap();
        let audit = MockAuditSink::accepting();
        let obs = Recorder::default();
        let action = ProposedAction {
            tool: "graph.query".to_string(),
            summary: "x".to_string(),
            arguments: BTreeMap::new(),
        };
        let receipt = Gate::new(&cap, &audit, &obs, &FsPathResolver, &StaticMountPolicy::empty())
            .decide_action(
                "auto-tag-by-project",
                BaselineMode::Supervised,
                &scope(&["graph.query"]),
                &action,
                &ctx(false, "run-unreg"),
                &DeniedGraph,
            )
            .await
            .unwrap();
        assert_eq!(receipt.decision, ActionDecision::RequireConfirmation);
    }

    #[tokio::test]
    async fn an_extra_operand_cannot_ride_on_the_proof() {
        // The action carries an operand the schema does not name. The proof
        // must not pass (the extra operand was never constrained), so the cap
        // stands rather than authorising an under-specified invocation.
        let cap = executing_cap();
        let audit = MockAuditSink::accepting();
        let obs = Recorder::default();
        let graph = tag_graph(false);
        let mut action = graph_write_action();
        action.arguments.insert("rogue".to_string(), "/etc/shadow".to_string());
        let receipt = Gate::new(&cap, &audit, &obs, &IdentityResolver, &StaticMountPolicy::empty())
            .decide_action(
                "auto-tag-by-project",
                BaselineMode::Supervised,
                &scope(&["graph.write"]),
                &action,
                &ctx(false, "run-extra"),
                &graph,
            )
            .await
            .unwrap();
        assert_eq!(receipt.decision, ActionDecision::RequireConfirmation);
    }

    #[tokio::test]
    async fn a_denied_graph_cannot_prove_so_the_cap_stands() {
        // The dispatcher hands a `reads: minimal` behaviour a denying graph
        // handle. The proof reads through that same handle, so even a
        // graph.write with real operands cannot be proven and the cap stands:
        // the proof path is not a read-scope side channel.
        let cap = executing_cap();
        let audit = MockAuditSink::accepting();
        let obs = Recorder::default();
        let receipt = Gate::new(&cap, &audit, &obs, &IdentityResolver, &StaticMountPolicy::empty())
            .decide_action(
                "auto-tag-by-project",
                BaselineMode::Supervised,
                &scope(&["graph.write"]),
                &graph_write_action(),
                &ctx(false, "run-denied"),
                &DeniedGraph,
            )
            .await
            .unwrap();
        assert_eq!(receipt.decision, ActionDecision::RequireConfirmation);
    }

    #[tokio::test]
    async fn refuses_a_tool_outside_the_declared_scope() {
        let cap = suggest_only();
        let audit = MockAuditSink::accepting();
        let obs = Recorder::default();
        // The behaviour declared only graph.query, but proposes graph.write.
        let err = Gate::new(&cap, &audit, &obs, &FsPathResolver, &StaticMountPolicy::empty())
            .decide_action(
                "auto-tag-by-project",
                BaselineMode::Suggest,
                &scope(&["graph.query"]),
                &action(),
                &ctx(false, "run-4"),
                &DeniedGraph,
            )
            .await
            .expect_err("an out-of-scope tool must be refused");
        assert!(matches!(err, GateError::ToolOutOfScope { .. }));
        // The refusal is still audited, and never handed to the observer.
        assert_eq!(
            audit.recorded().await[0].structural.outcome,
            "refused-out-of-scope"
        );
        assert!(obs.0.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn an_external_high_impact_out_of_scope_refusal_records_both_causes() {
        // A prompt-injection attempt: an externally-triggered proposal for an
        // undeclared destructive tool. It is refused out of scope, but the
        // durable ledger must still record the external trigger and the
        // high-impact class (by name), not flatten it to a bare refusal.
        let cap = suggest_only();
        let audit = MockAuditSink::accepting();
        let obs = Recorder::default();
        let delete = ProposedAction {
            tool: "delete_file".to_string(),
            summary: "remove the file".to_string(),
            arguments: BTreeMap::new(),
        };
        let err = Gate::new(&cap, &audit, &obs, &FsPathResolver, &StaticMountPolicy::empty())
            .decide_action(
                "some-behaviour",
                BaselineMode::Suggest,
                &scope(&["graph.write"]), // delete_file is not declared
                &delete,
                &ctx(true, "run-inject"), // external trigger
                &DeniedGraph,
            )
            .await
            .expect_err("an out-of-scope tool must be refused");
        assert!(matches!(err, GateError::ToolOutOfScope { .. }));
        assert_eq!(
            audit.recorded().await[0].structural.outcome,
            "refused-out-of-scope:external-trigger+high-impact-permanent-delete"
        );
    }
}

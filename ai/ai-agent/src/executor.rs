//! Dry-run action executor: turns a lifted gate decision into the concrete
//! graph write it authorises, and records that write WITHOUT performing any
//! I/O.
//!
//! This is the first half of the executor the gate's lift anticipates (see the
//! "Executor obligations" contract in [`crate::gate`]). It honours obligation 1
//! — **execute exactly the proven effect** — by deriving the write solely from
//! the trusted, registry-resolved schema for the invoked action: the single
//! `AssertEdge` effect gives the edge type and the endpoint binds, the schema's
//! `NodeExists` preconditions give those binds' node types, and only the
//! concrete node ids come from the (untrusted) invocation arguments. A schema
//! whose effect is anything other than one `AssertEdge` is refused, so a
//! different mutation can never ride on the proof.
//!
//! It is deliberately dry-run: it computes the planned write and returns it for
//! logging / the activity surface, but performs no write.
//!
//! ## What going live still needs (the strict-create gap)
//!
//! The `graph.write` rule is **strict-create**: its proof includes
//! `Not(EdgeExists)`, and its derived compensation (`RetractEdge`) is sound only
//! because the action is the one that created the edge. The os-sdk relation
//! client persists with the daemon's idempotent `MERGE`, which re-checks the
//! *endpoints* exist but **not** that the edge is absent. So a plain live wiring
//! would treat an edge created concurrently after the proof as a silent success
//! and leave a later compensation able to retract an edge this action did not
//! create. The dry-run report therefore carries
//! [`DryRunReport::conditional_on_absent_edge`]: the live executor must enforce
//! that absence atomically (a conditional create-or-conflict op), or the effect
//! must be re-modelled as an idempotent ensure-edge whose compensation only
//! undoes a create it actually performed. Resolving that absence check, plus
//! re-running the full trusted precondition proof atomically at write time
//! (obligation 2, which the live write inherently couples to), is the live
//! executor increment; nothing here executes.
//!
//! Per-tool scope-value enforcement (obligation 3) **is** done here: the planned
//! write is checked against the behaviour's declared `graph.write` scope, so it
//! can only ever target a relation/entity the behaviour was granted, not merely
//! one it holds the `graph.write` tool name for.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use arlen_ai_core::audit::{behaviour_action_event, AuditSink};
use arlen_ai_core::capability::{ActionDecision, BaselineMode, Capability};

use crate::effect_model::{EffectDomain, InverseReceipt};
use crate::engine::RetainedProposal;
use crate::fs_move::{self, FileMover};
use crate::gate::{resolved_action_kind, ActionContext, ProposedAction};
use crate::registry::{self, TrustedActionSchema};
use crate::seams::GraphHandle;
use crate::slice::{build_slice_trusted, escape_cypher_literal, MountPolicy, PathResolver};
use crate::world::{self, Effect, EvalContext, Predicate};

/// Wall-clock bound on the live re-validation read (graph slice + path
/// resolution), mirroring the gate's proof timeout. A stalled dependency must
/// fail closed (the write is refused) rather than park the executor.
const REVALIDATION_TIMEOUT: Duration = Duration::from_secs(5);

/// Wall-clock bound on the write itself. The daemon's dispatch loop awaits the
/// executor, so an unbounded write against a stalled knowledge socket would
/// block the daemon from honouring a reload or shutdown. A timed-out write fails
/// closed (pre-audited + idempotent, so reconcilable on a later run).
///
/// Sized for a slow backend, not a fast one: the daemon-side FILE_PART_OF write
/// is a NON-cancellable, bitemporal multi-statement Cypher (close-supersede +
/// conditional append) that commits regardless of this client bound, so the only
/// effect of a too-short value is a spurious Indeterminate + a reconcile next run.
/// On a resource-constrained host (the in-VM verify with the model resident) that
/// write measured well over 5 s while reads stayed fast, so 5 s never confirmed a
/// real commit. 30 s still catches a genuinely stalled socket while giving the
/// committing write room to return on a loaded machine.
const WRITE_TIMEOUT: Duration = Duration::from_secs(30);

/// Wall-clock bound on a single reconciliation read that resolves a
/// commit-unknown write. Short, like the write: each is a single indexed edge
/// query, and a stalled read must not re-introduce the hang the write timeout
/// avoided.
const RECONCILE_TIMEOUT: Duration = Duration::from_secs(5);

/// How many times the reconciler reads the op-id edge before giving up. A
/// commit-unknown write may land slightly *after* the immediate post-write read
/// (the daemon's queued CREATE runs on its serial graph thread, which can be
/// briefly behind), so a single read would too often report indeterminate for a
/// write that commits a moment later. Polling a few times with backoff catches
/// that late commit; the total added latency stays bounded (see
/// [`reconcile_backoff`]) so a never-landing write still resolves promptly to
/// indeterminate with its key preserved for the next organic reconcile.
const RECONCILE_ATTEMPTS: u32 = 4;

/// Backoff before the `attempt`-th reconciliation read (1-based; no wait before
/// the first). Grows so the cheap early reads catch a fast commit while the
/// total bound stays small (250 + 500 + 750 ms = 1.5 s across the retries).
fn reconcile_backoff(attempt: u32) -> Duration {
    Duration::from_millis(250 * u64::from(attempt))
}

/// The concrete relation write a proven action would perform: the namespaced
/// endpoint entity types, their resolved node ids, and the edge type. The
/// shape the daemon's relation-write socket expects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelationWrite {
    /// The source node's namespaced entity type (e.g. `system.File`).
    pub from_type: String,
    /// The source node's concrete id.
    pub from_id: String,
    /// The target node's namespaced entity type (e.g. `system.Project`).
    pub to_type: String,
    /// The target node's concrete id.
    pub to_id: String,
    /// The relation (edge) type to create.
    pub relation_type: String,
}

/// Why the executor could not turn a decision into a concrete write. Every
/// variant is fail-closed: the executor produces no write rather than guess.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ExecError {
    /// No registry rule backs the invoked action, so there is nothing to
    /// execute (the gate would not have lifted it either).
    #[error("no registry rule for action '{0}'")]
    NoRule(String),
    /// The action's schema is not a single `AssertEdge`, the only effect this
    /// executor performs. Refused rather than reinterpreted.
    #[error("action '{0}' has no single AssertEdge effect the executor can perform")]
    UnsupportedEffect(String),
    /// An endpoint bind has no `NodeExists` precondition, so its node type
    /// cannot be resolved from the trusted schema.
    #[error("bind '{0}' has no NodeExists precondition, so its node type is unknown")]
    UnknownBindLabel(String),
    /// An endpoint bind has no value in the invocation arguments, so its node
    /// id is unresolved.
    #[error("argument '{0}' is missing, so its node id is unresolved")]
    MissingArgument(String),
    /// The planned write names a target entity or relation type the behaviour
    /// did not declare in its `graph.write` tool scope. The gate enforces the
    /// tool *name*; the executor enforces the scope *values* (obligation 3), so
    /// a behaviour cannot write a relation/target it was not granted.
    #[error("tool '{tool}' scope does not grant '{token}'")]
    ScopeViolation {
        /// The tool whose declared scope was exceeded.
        tool: String,
        /// The target label or relation type the scope did not name.
        token: String,
    },
    /// The predict-before-act proof no longer holds against the current graph:
    /// a precondition went stale between the gate decision and the write (a node
    /// removed, a path moved, the edge created concurrently). The write is
    /// refused fail-closed rather than acting on a stale proof.
    #[error("the action's proof no longer holds against the current graph")]
    ProofStale,
    /// The write itself failed at the graph boundary.
    #[error("write failed: {0}")]
    Write(String),
    /// The re-validation read (graph slice + path resolution) did not finish in
    /// time, so the proof could not be re-established. Fail-closed: a stalled
    /// knowledge socket or a slow path lookup must not park the executor.
    #[error("re-validation timed out before the write")]
    RevalidationTimeout,
    /// The audit ledger could not record the execution. Fail-closed: the write
    /// is not performed, so no graph mutation happens without a durable record
    /// of it (the S13 audit-before-acting invariant the gate also honours).
    #[error("audit unavailable, execution refused: {0}")]
    AuditUnavailable(String),
    /// An external (non-graph) operation's operands could not be turned into a
    /// safe plan: a non-canonical path, a move onto itself, or no free
    /// destination within the collision-avoidance bound. Fail-closed: the
    /// executor performs nothing.
    #[error("the external operation has no safe plan: {0}")]
    Unplannable(String),
    /// The write's commit is **unknown** (a timeout after the request may have
    /// been sent, a post-send transport failure, or a reconciliation read that
    /// could not confirm the op-id edge). Distinct from `Write` (a definite
    /// no-commit), so it is reported as indeterminate and reconciled on a later
    /// run. Carries the [`PendingWrite`] key — the `(write, op_id, correlation
    /// id)` of the unconfirmed operation — so a late commit is not left
    /// uncompensable: the key reaches the dispatcher (and any recovery path),
    /// which can re-run the op-id-keyed reconcile/retract rather than having only
    /// a reason string. `reason` is for logging.
    #[error("the write outcome is unknown: {reason}")]
    WriteIndeterminate {
        /// The durable key needed to reconcile or compensate the unconfirmed write.
        pending: PendingWrite,
        /// Human-readable detail for logging (the edge was not yet observed, or a
        /// read failed); not load-bearing.
        reason: String,
    },
}

/// A failure to persist a planned relation write, classified by whether the
/// relation could have been committed.
#[derive(Debug, thiserror::Error)]
pub enum WriteError {
    /// The write **definitely did not commit**: the daemon received the request
    /// and rejected it (permission denied, endpoints not found), or the
    /// connection failed before the request was sent.
    #[error("relation write failed: {0}")]
    Failed(String),
    /// The write's commit is **unknown**: a transport failure after the request
    /// may already have been sent (a dropped connection, a lost response). The
    /// relation may or may not have been persisted, so it must not be reported
    /// as a definite failure.
    #[error("relation write outcome unknown: {0}")]
    Indeterminate(String),
}

/// Whether a write created the edge or found it already present. The daemon's
/// conditional create reports this atomically for a single attempt (and never
/// double-creates). It is NOT durable across an at-least-once retry: a create
/// whose response is lost and is retried reports `AlreadyExists` the second
/// time, so this alone does not make a compensator that survives retries safe.
/// Durable operation identity (an idempotency key) is the deferred follow-up;
/// the executor's pre-write re-validation is the interim guard (a retry whose
/// edge now exists fails its proof and writes nothing).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteOutcome {
    /// This write created the edge.
    Created,
    /// The edge already existed; the write was an idempotent no-op.
    AlreadyExists,
}

/// Whether a retract removed the op-id-keyed edge or found nothing to remove.
/// The daemon's retract is keyed by the operation id, so it deletes only the
/// edge a matching create stamped; a non-match (already gone, or never this
/// op's edge) is `Absent`, an idempotent no-op.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetractOutcome {
    /// This call removed the edge carrying the given op id.
    Retracted,
    /// No edge carried the given op id; the call was an idempotent no-op.
    Absent,
}

/// The seam through which the live executor performs an authorised write. The
/// production impl wraps the os-sdk graph write client (the knowledge daemon's
/// write socket); tests inject a mock that records the write without I/O. Kept
/// separate from the read-only [`GraphHandle`] so the proof path can never write
/// and a writer is only ever reached after a re-validated proof.
#[async_trait]
pub trait RelationWriter: Send + Sync {
    /// Persist the relation, reporting whether it created the edge or found it
    /// already present. Idempotent at the daemon (a strict conditional create),
    /// so a transport retry re-confirms (`AlreadyExists`) rather than duplicates.
    /// `op_id` is the durable operation identity persisted on the edge, so a
    /// commit-unknown write can later be reconciled by reading whether this
    /// op's edge exists.
    async fn write_relation(
        &self,
        write: &RelationWrite,
        op_id: &str,
    ) -> Result<WriteOutcome, WriteError>;

    /// Retract (compensate) the relation this op created, deleting only the edge
    /// carrying `op_id`. The inverse of [`write_relation`](Self::write_relation),
    /// it undoes exactly the edge the matching create stamped, so it can never
    /// remove an edge this op did not create. Idempotent (a no-match is
    /// [`RetractOutcome::Absent`]), so a transport retry is safe. The same
    /// commit-certainty classification applies: a definite no-commit is
    /// [`WriteError::Failed`], a commit-unknown transport failure is
    /// [`WriteError::Indeterminate`].
    async fn retract_relation(
        &self,
        write: &RelationWrite,
        op_id: &str,
    ) -> Result<RetractOutcome, WriteError>;
}

/// A write the live executor performed: the **execution receipt**, an opaque
/// authority token for compensation.
///
/// It carries not just the relation and whether it was created, but the exact
/// `op_id` stamped on the edge and the `correlation_id` of the decision that
/// produced it. Compensation is keyed off THIS receipt, never a separately
/// passed context, so the retract can only ever target the very edge this
/// execution created (a mis-threaded or stale context cannot redirect the delete
/// to another op's edge).
///
/// Its fields are **private with no public constructor**, so the only way to
/// obtain one is from [`LiveExecutor::execute`] / its reconciliation: the
/// `(write, op_id)` pairing is therefore always the one the executor actually
/// derived and wrote, never a caller-fabricated or corrupted pairing that
/// `compensate` would otherwise trust into retracting the wrong edge. Read
/// access is via accessors; mutation and construction stay inside this module.
/// (A receipt that must survive a process boundary — e.g. a persisted undo log
/// across a daemon restart — needs a signed or ledger-backed form rather than
/// this in-memory token; that is the undo-log increment's concern, since
/// receipts live only in-process today.)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutedWrite {
    /// The relation that was written.
    write: RelationWrite,
    /// Whether this call created the edge or found it present.
    outcome: WriteOutcome,
    /// The durable operation id stamped on the edge by this execution. The
    /// compensation key: retract deletes only the edge carrying this id.
    op_id: String,
    /// The correlation id of the decision that produced this write, so a later
    /// compensation audits under the same decision identity (it is taken from
    /// the receipt, not re-supplied, so it cannot drift from the op_id).
    correlation_id: String,
}

impl ExecutedWrite {
    /// Build a receipt. Module-private: only [`LiveExecutor::execute`] and its
    /// reconciliation construct one, so the `(write, op_id)` pairing is always
    /// executor-derived. (The test submodule, an in-module descendant, builds
    /// receipts directly to exercise compensation.)
    fn new(
        write: RelationWrite,
        outcome: WriteOutcome,
        op_id: String,
        correlation_id: String,
    ) -> Self {
        Self {
            write,
            outcome,
            op_id,
            correlation_id,
        }
    }

    /// Test-only constructor so a sibling module (the receipt-store projection
    /// tests) can build a receipt without the executor. `#[cfg(test)]`, so the
    /// production opacity invariant (only the executor produces a `(write,
    /// op_id)` pairing) is unchanged.
    #[cfg(test)]
    pub(crate) fn for_test(
        write: RelationWrite,
        outcome: WriteOutcome,
        op_id: String,
        correlation_id: String,
    ) -> Self {
        Self::new(write, outcome, op_id, correlation_id)
    }

    /// The relation that was written.
    pub fn write(&self) -> &RelationWrite {
        &self.write
    }

    /// Whether this execution created the edge or found it already present.
    pub fn outcome(&self) -> WriteOutcome {
        self.outcome
    }

    /// The durable operation id stamped on the edge (the compensation key).
    pub fn op_id(&self) -> &str {
        &self.op_id
    }

    /// The correlation id of the decision that produced this write.
    pub fn correlation_id(&self) -> &str {
        &self.correlation_id
    }
}

/// The durable key for a write whose commit is **unknown**: the relation, the
/// op_id the executor stamped, and the decision's correlation id. It is the
/// receipt's counterpart for the indeterminate path — a late in-process commit
/// would otherwise leave this operation's op-id-stamped edge in the graph with
/// nothing holding the key to reconcile or compensate it. Carrying it out of the
/// executor (through [`ExecError::WriteIndeterminate`] and on to the dispatcher)
/// lets a recovery path re-run the op-id-keyed reconcile/retract.
///
/// Like [`ExecutedWrite`] it is opaque (private fields, module-private
/// constructor, read accessors), so the only `(write, op_id)` pairing is the one
/// the executor derived, never a fabricated one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingWrite {
    write: RelationWrite,
    op_id: String,
    correlation_id: String,
}

impl PendingWrite {
    /// Build a pending-write key. Module-private: only the executor's
    /// reconciliation constructs one.
    fn new(write: RelationWrite, op_id: String, correlation_id: String) -> Self {
        Self {
            write,
            op_id,
            correlation_id,
        }
    }

    /// The relation whose write is unconfirmed.
    pub fn write(&self) -> &RelationWrite {
        &self.write
    }

    /// The op id the unconfirmed write stamped (the reconcile/retract key).
    pub fn op_id(&self) -> &str {
        &self.op_id
    }

    /// The correlation id of the decision behind the unconfirmed write.
    pub fn correlation_id(&self) -> &str {
        &self.correlation_id
    }
}

// The non-graph receipt types (reversible-receipts-and-the-effect-model.md §5).
// Built ahead of their constructor (the executor's non-graph arm, EM-R5) and
// their consumer (the generalised `compensate`, the compensate blast-radius), so
// they read unused in the non-test build until those land; the test submodule
// exercises them via the module-private constructors.
#[allow(dead_code)]
/// The resolved, scope-checked non-graph operation an action performed: the
/// effect domain, the concrete op within it, and the resolved target it touched
/// (the bind's argument already resolved to the real resource). The non-graph
/// analogue of [`RelationWrite`] — the "forward" a reconcile re-derives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedExternalOp {
    domain: EffectDomain,
    op: String,
    target: String,
}

#[allow(dead_code)]
impl ResolvedExternalOp {
    /// Build a resolved external op. Module-private: only the executor's
    /// non-graph arm resolves and constructs one.
    fn new(domain: EffectDomain, op: String, target: String) -> Self {
        Self { domain, op, target }
    }

    /// The effect domain (selects the writer / inverse-capture seam).
    pub fn domain(&self) -> EffectDomain {
        self.domain
    }

    /// The concrete operation within the domain (`move`, `write`, ...).
    pub fn op(&self) -> &str {
        &self.op
    }

    /// The resolved resource the operation touched.
    pub fn target(&self) -> &str {
        &self.target
    }
}

/// The receipt of a committed non-graph action and its captured inverse, the
/// non-graph twin of [`ExecutedWrite`]. Opaque (private fields, module-private
/// constructor, read accessors), so a receipt is only ever obtained from the
/// executor and never fabricated, and compensation is keyed off the receipt, not
/// a re-supplied context. `is_reversible` for a non-graph rule is exactly "the
/// executor captured a valid `InverseReceipt` here".
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionWrite {
    forward: ResolvedExternalOp,
    inverse: InverseReceipt,
    op_id: String,
    correlation_id: String,
}

#[allow(dead_code)]
impl ActionWrite {
    /// Build a non-graph receipt. Module-private: only the executor's non-graph
    /// arm constructs one, so the `(forward, inverse, op_id)` pairing is always
    /// executor-derived.
    fn new(
        forward: ResolvedExternalOp,
        inverse: InverseReceipt,
        op_id: String,
        correlation_id: String,
    ) -> Self {
        Self {
            forward,
            inverse,
            op_id,
            correlation_id,
        }
    }

    /// Test-only constructor: a non-graph receipt with a `RestorePath` inverse
    /// (a file move from `prior` to `now`), so a consumer such as the receipt
    /// store's done-view can be tested without driving the executor.
    /// `#[cfg(test)]`, so the production opacity (only the executor's non-graph
    /// arm builds one) is unchanged.
    #[cfg(test)]
    pub(crate) fn for_test(
        op: &str,
        target: &str,
        now: &str,
        prior: &str,
        op_id: &str,
        correlation_id: &str,
    ) -> Self {
        use crate::effect_model::CanonicalPath;
        Self {
            forward: ResolvedExternalOp::new(
                EffectDomain::Filesystem,
                op.to_string(),
                target.to_string(),
            ),
            inverse: InverseReceipt::RestorePath {
                now: CanonicalPath::new(now).expect("test now path is canonical"),
                prior: CanonicalPath::new(prior).expect("test prior path is canonical"),
            },
            op_id: op_id.to_string(),
            correlation_id: correlation_id.to_string(),
        }
    }

    /// The forward operation that was committed.
    pub fn forward(&self) -> &ResolvedExternalOp {
        &self.forward
    }

    /// The captured inverse (replaying it is the undo).
    pub fn inverse(&self) -> &InverseReceipt {
        &self.inverse
    }

    /// The durable operation id (the compensation/reconcile key).
    pub fn op_id(&self) -> &str {
        &self.op_id
    }

    /// The correlation id of the decision that produced this write.
    pub fn correlation_id(&self) -> &str {
        &self.correlation_id
    }
}

/// One receipt vocabulary (§5): the graph variant unchanged (self-inverse via
/// op_id, no captured prior state), and the non-graph variant carrying a captured
/// inverse. `compensate` dispatches on the variant; the graph arm is the current
/// code verbatim.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionReceipt {
    /// A graph edge write, self-inverse via its op_id.
    Graph(ExecutedWrite),
    /// A non-graph action and its captured inverse.
    NonGraph(ActionWrite),
}

#[allow(dead_code)]
impl ActionReceipt {
    /// The correlation id of the decision that produced this receipt - the key a
    /// retained receipt is stored under (so a later undo finds it), uniform across
    /// both variants.
    pub fn correlation_id(&self) -> &str {
        match self {
            ActionReceipt::Graph(e) => e.correlation_id(),
            ActionReceipt::NonGraph(a) => a.correlation_id(),
        }
    }
}

/// The result of compensating (undoing) a previously-executed write. Closes the
/// predict -> gate -> act -> audit -> **compensate** loop: an action that
/// declared a reversible effect (and so was lifted) can have that exact effect
/// retracted, keyed by the op id the create stamped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompensationOutcome {
    /// The compensation removed the edge this action had created.
    Retracted,
    /// There was nothing to undo: the edge was already gone, or the original
    /// write was an idempotent no-op (`AlreadyExists`), so this action never
    /// created the edge and must not retract one it did not write.
    NothingToUndo,
}

/// What a dry run would do, surfaced for logging / the activity view. Holds the
/// concrete write and never performs it.
///
/// This is a **non-authoritative record**, not an execution authority. It does
/// not carry the full proof: the gate's lift also rested on point-in-time
/// preconditions (for `graph.write`, `PathUnderField` proving the file lies
/// under the project root) that the report deliberately omits, because the proof
/// is a point-in-time slice with no graph snapshot (gap A2). A live executor
/// must therefore re-run the complete trusted precondition validation atomically
/// at write time (the gate's obligation 2) and never write straight from this
/// report; the report is for showing the user / activity log what a proven
/// decision would do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DryRunReport {
    /// The relation the live executor would create.
    pub write: RelationWrite,
    /// Whether the proof required the edge to be **absent** (a strict-create
    /// `Not(EdgeExists)` precondition). When true, the live executor must create
    /// only if the edge is absent and treat a concurrently-created edge as a
    /// conflict, not a silent success — a bare idempotent `MERGE` would not
    /// honour the strict-create semantics or keep the derived compensation safe.
    pub conditional_on_absent_edge: bool,
}

/// Whether the schema proves the asserted edge is **absent** (a strict-create
/// precondition `Not(EdgeExists)` matching the single `AssertEdge` effect). The
/// live executor must enforce this atomically: create only if absent, else
/// conflict. A plain idempotent `MERGE` would silently treat a concurrently
/// created edge as success and make a later compensation (retract) unsafe.
fn create_is_conditional_on_absence(schema: &TrustedActionSchema) -> bool {
    let s = schema.schema();
    let Some(Effect::AssertEdge { from, edge, to }) = s.effects.first() else {
        return false;
    };
    s.preconditions.iter().any(|p| {
        if let Predicate::Not(inner) = p {
            if let Predicate::EdgeExists {
                from: f,
                edge: e,
                to: t,
            } = inner.as_ref()
            {
                return f == from && e == edge && t == to;
            }
        }
        false
    })
}

/// Enforce the behaviour's per-tool scope *values* on a planned write
/// (obligation 3). The gate already checked the tool name; here the concrete
/// target and relation must be ones the behaviour declared, so a behaviour
/// granted `graph.write: [Project, FILE_PART_OF]` cannot write some other
/// relation or target even though it holds `graph.write`.
///
/// An empty scope list means the tool is granted without a finer restriction
/// (the manifest convention), so it passes. Otherwise both the target entity's
/// bare label and the relation type must appear in the scope list.
fn enforce_tool_scope(write: &RelationWrite, tool: &str, scope: &[String]) -> Result<(), ExecError> {
    if scope.is_empty() {
        return Ok(());
    }
    let to_label = write
        .to_type
        .strip_prefix("system.")
        .unwrap_or(&write.to_type);
    for token in [to_label, write.relation_type.as_str()] {
        if !scope.iter().any(|s| s == token) {
            return Err(ExecError::ScopeViolation {
                tool: tool.to_string(),
                token: token.to_string(),
            });
        }
    }
    Ok(())
}

/// Argument keys the executor reads for an `fs.move` action: the source file and
/// the destination directory. The (untrusted) behaviour loop supplies both; both
/// are confined to the behaviour's declared dir scope before any move.
const FS_MOVE_SOURCE: &str = "source";
const FS_MOVE_DEST_DIR: &str = "dest_dir";

/// Expand a behaviour tool-scope dir entry to an absolute path. Scope entries are
/// authored with `~` (e.g. `~/Downloads`); expand it against `$HOME`. An absolute
/// `/...` entry is taken verbatim. A relative entry cannot be a confinement root,
/// so it yields `None` (the caller drops it, fail-closed).
fn expand_scope_dir(entry: &str, home: &str) -> Option<String> {
    if entry == "~" {
        Some(home.trim_end_matches('/').to_string())
    } else if let Some(rest) = entry.strip_prefix("~/") {
        Some(format!("{}/{}", home.trim_end_matches('/'), rest.trim_end_matches('/')))
    } else if entry.starts_with('/') {
        Some(entry.trim_end_matches('/').to_string())
    } else {
        None
    }
}

/// Whether `path` is `dir` itself or lies under it as a `/`-bounded prefix (so
/// `/a/bc` is NOT under `/a/b`). Both are absolute and slash-trimmed.
fn is_under(path: &str, dir: &str) -> bool {
    path == dir || path.strip_prefix(dir).is_some_and(|r| r.starts_with('/'))
}

/// A scope violation for the `fs.move` tool naming the offending token.
fn fs_move_scope_violation(token: &str) -> ExecError {
    ExecError::ScopeViolation {
        tool: "fs.move".to_string(),
        token: token.to_string(),
    }
}

/// Resolve the behaviour's declared `fs.move` dir scope to REAL, symlink-resolved
/// roots through `paths` (the production [`FsPathResolver`] canonicalizes, so a
/// scope dir under a symlinked home resolves correctly and the prefix comparison
/// below is over real paths). `~`-rooted entries are expanded against `$HOME`
/// first.
///
/// Fail-closed: an EMPTY scope is REFUSED (a filesystem-move tool must declare its
/// dirs; an unbounded move is never granted by omission, unlike a graph relation
/// where empty means no finer restriction). `$HOME` must resolve to expand
/// `~`-rooted entries. A root that does not resolve (missing, dangling) is
/// dropped; an empty-or-`/` resolved root is dropped (a universal-match root is
/// never a confinement). If NO root resolves, the move is refused.
fn resolve_scope_roots(
    paths: &dyn PathResolver,
    scope: &[String],
) -> Result<Vec<String>, ExecError> {
    if scope.is_empty() {
        return Err(fs_move_scope_violation(
            "<empty scope: a move tool must declare its dirs>",
        ));
    }
    let home = std::env::var("HOME")
        .map_err(|_| fs_move_scope_violation("<no HOME to resolve scope dirs>"))?;
    let roots: Vec<String> = scope
        .iter()
        .filter_map(|e| expand_scope_dir(e, &home))
        .filter_map(|d| paths.resolve(&d).ok())
        .map(|r| r.trim_end_matches('/').to_string())
        // A root that resolved to the empty string or bare `/` would match every
        // absolute path - never a confinement, so drop it.
        .filter(|r| !r.is_empty())
        .collect();
    if roots.is_empty() {
        return Err(fs_move_scope_violation("<no scope dir resolved>"));
    }
    Ok(roots)
}

/// Confine an already-RESOLVED source and destination directory to the resolved
/// scope roots - the executor's path confinement (obligation 3). Unlike
/// `graph.write` (where the gate names the relation and predict proves
/// containment), NOTHING upstream confines a move's dirs: predict has no
/// precondition for an external effect and the gate's tool-scope check is
/// name-only. So this is the SOLE path confinement and must hold before any move.
///
/// The inputs MUST be symlink-resolved (canonicalized) before this is called, so
/// a symlinked operand cannot pass a string prefix check while the real move
/// touches a target outside the scope (a string check on raw operands is unsafe -
/// the caller resolves first via [`resolve_scope_roots`] / the path resolver).
/// Both the source file and the destination directory must lie under (or equal) a
/// resolved scope root.
fn confine_to_roots(source: &str, dest_dir: &str, roots: &[String]) -> Result<(), ExecError> {
    let src = source.trim_end_matches('/');
    let dst = dest_dir.trim_end_matches('/');
    if !roots.iter().any(|r| is_under(src, r)) {
        return Err(fs_move_scope_violation(source));
    }
    if !roots.iter().any(|r| is_under(dst, r)) {
        return Err(fs_move_scope_violation(dest_dir));
    }
    Ok(())
}

/// Derive a stable op id for an external operation from the decision identity (the
/// correlation id) and the concrete move (source plus the landed destination), the
/// non-graph twin of [`derive_op_id`]. Length-delimited, SHA-256, hex.
fn derive_external_op_id(correlation_id: &str, source: &str, destination: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    for part in [correlation_id, "fs.move", source, destination] {
        h.update((part.len() as u64).to_le_bytes());
        h.update(part.as_bytes());
    }
    let digest = h.finalize();
    let mut hex = String::with_capacity(digest.len() * 2);
    for b in digest {
        use std::fmt::Write;
        let _ = write!(hex, "{b:02x}");
    }
    hex
}

/// Derive a stable, collision-resistant operation id for a write, from the
/// decision identity (the correlation id, `event.id:behaviour`) and the concrete
/// write. A crash-replay of the *same* decision yields the same id (so the daemon
/// can recognise it), while a genuinely new decision with the same operands does
/// not. Length-delimited so distinct field boundaries cannot collide, hashed with
/// SHA-256 and hex-encoded (64 chars, within the daemon's `op_id` bound).
fn derive_op_id(correlation_id: &str, write: &RelationWrite) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    for part in [
        correlation_id,
        &write.from_type,
        &write.from_id,
        &write.to_type,
        &write.to_id,
        &write.relation_type,
    ] {
        h.update((part.len() as u64).to_le_bytes());
        h.update(part.as_bytes());
    }
    let digest = h.finalize();
    let mut hex = String::with_capacity(digest.len() * 2);
    for b in digest {
        use std::fmt::Write;
        let _ = write!(hex, "{b:02x}");
    }
    hex
}

/// Map a bare world-model label (`File`) to the daemon's namespaced entity type
/// (`system.File`). The world model and graph node tables use bare labels; the
/// write socket's relation allowlist is namespaced, so the boundary is crossed
/// here, once.
fn namespaced(label: &str) -> String {
    format!("system.{label}")
}

/// Derive the concrete [`RelationWrite`] for a trusted action schema and the
/// invocation's (untrusted) arguments.
///
/// The edge type and the two endpoint binds come from the schema's single
/// `AssertEdge` effect; each bind's node type comes from a matching
/// `NodeExists` precondition in the same trusted schema (never the arguments);
/// only the node ids come from the arguments. Anything that cannot be resolved
/// fail-closes to an [`ExecError`].
pub(crate) fn plan_relation_write(
    schema: &TrustedActionSchema,
    arguments: &BTreeMap<String, String>,
) -> Result<RelationWrite, ExecError> {
    let s = schema.schema();

    // Obligation 1: execute EXACTLY the proven effect. The only effect this
    // executor performs is a single AssertEdge; anything else (a node mutation,
    // a field set, more than one effect) is refused, so no other mutation can
    // ride on the proof.
    let (from_bind, edge, to_bind) = match s.effects.as_slice() {
        [Effect::AssertEdge { from, edge, to }] => (from, edge, to),
        _ => return Err(ExecError::UnsupportedEffect(s.action.clone())),
    };

    // Each endpoint's node label comes from the trusted schema's NodeExists
    // preconditions, the authoritative source of the bind's type.
    let label_for = |bind: &str| -> Option<&str> {
        s.preconditions.iter().find_map(|p| match p {
            Predicate::NodeExists { bind: b, label } if b == bind => Some(label.as_str()),
            _ => None,
        })
    };
    let from_label =
        label_for(from_bind).ok_or_else(|| ExecError::UnknownBindLabel(from_bind.clone()))?;
    let to_label =
        label_for(to_bind).ok_or_else(|| ExecError::UnknownBindLabel(to_bind.clone()))?;

    // Only the concrete ids come from the (untrusted) operands.
    let from_id = arguments
        .get(from_bind)
        .ok_or_else(|| ExecError::MissingArgument(from_bind.clone()))?;
    let to_id = arguments
        .get(to_bind)
        .ok_or_else(|| ExecError::MissingArgument(to_bind.clone()))?;

    Ok(RelationWrite {
        from_type: namespaced(from_label),
        from_id: from_id.clone(),
        to_type: namespaced(to_label),
        to_id: to_id.clone(),
        relation_type: edge.clone(),
    })
}

/// Dry-run the executor for one gated action.
///
/// Returns a plan **only** for `PreviewThenExecute`, the single decision the
/// gate emits that carries a successful predict-before-act proof: that lift is
/// reached only when the world model proved this invocation safe (its operands
/// hold against the trusted schema and the real graph), so deriving a concrete
/// write from those operands is sound. Every other decision yields `Ok(None)`:
/// `RequireConfirmation` is the gate's unproven cap (an override, or a failed or
/// absent proof), so its operands may never have been validated and an
/// executable write derived from them could let an approval corrupt the graph (a
/// confirmation surface shows the proposal summary, not a write); `Propose` is
/// the manual Suggest flow; and `Proceed` is never emitted by the gate (a proven
/// autonomous decision is capped to `PreviewThenExecute`).
///
/// The schema is resolved independently from the trusted registry, keyed by the
/// same tool the gate proved (defence in depth: the executor binds to the
/// registry, not to a caller-passed schema). The planned write is also checked
/// against `tool_scope`, the behaviour's declared `graph.write` scope values
/// (obligation 3). Performs **no I/O**: it records what the live executor would
/// write.
pub(crate) fn dry_run(
    action: &ProposedAction,
    decision: ActionDecision,
    tool_scope: &[String],
) -> Result<Option<DryRunReport>, ExecError> {
    // Only PreviewThenExecute carries a successful predict-before-act proof
    // (the gate lifts to it solely when the world model validated this
    // invocation's operands against the trusted schema and the real graph).
    // Every other final decision is non-executable here: RequireConfirmation is
    // the unproven cap, Propose is the manual flow, and Proceed is never emitted
    // by the gate.
    if decision != ActionDecision::PreviewThenExecute {
        return Ok(None);
    }
    let schema =
        registry::lookup(&action.tool).ok_or_else(|| ExecError::NoRule(action.tool.clone()))?;
    let write = plan_relation_write(&schema, &action.arguments)?;
    enforce_tool_scope(&write, &action.tool, tool_scope)?;
    let conditional_on_absent_edge = create_is_conditional_on_absence(&schema);
    Ok(Some(DryRunReport {
        write,
        conditional_on_absent_edge,
    }))
}

/// The live action executor: re-validates a proven decision against the current
/// graph (obligation 2) and performs the authorised write through a
/// [`RelationWriter`].
///
/// It holds the long-lived collaborators; [`execute`](LiveExecutor::execute)
/// takes the per-call decision and the behaviour-scoped graph handle, mirroring
/// how the gate is structured. The capability, path, and mount collaborators are
/// the SAME the gate proved with, so the re-validation classifies and resolves
/// identically.
pub struct LiveExecutor<'a> {
    capability: &'a Capability,
    paths: &'a dyn PathResolver,
    mounts: &'a dyn MountPolicy,
    writer: &'a dyn RelationWriter,
    audit: &'a dyn AuditSink,
}

impl<'a> LiveExecutor<'a> {
    /// Build a live executor over its collaborators.
    pub fn new(
        capability: &'a Capability,
        paths: &'a dyn PathResolver,
        mounts: &'a dyn MountPolicy,
        writer: &'a dyn RelationWriter,
        audit: &'a dyn AuditSink,
    ) -> Self {
        Self {
            capability,
            paths,
            mounts,
            writer,
            audit,
        }
    }

    /// Execute a gated decision: derive the planned write (only for a proven
    /// `PreviewThenExecute`, with scope enforced), re-run the trusted proof
    /// against the CURRENT graph, and write only if it still holds. Returns the
    /// write performed, `None` for a non-executable decision, or an
    /// [`ExecError`] (the proof went stale, the re-validation timed out, scope
    /// was exceeded, or the write failed). The graph handle is the
    /// behaviour-scoped one, so the re-validation reads no more than the
    /// behaviour may.
    ///
    /// The edge create itself is atomic and reports whether it created the edge:
    /// the daemon's conditional create is a single statement on its serial graph
    /// thread, so it cannot double-create and tells the writer `Created` vs
    /// `AlreadyExists` (so compensation only ever undoes a real create). What the
    /// re-validation **narrows but does not** make atomic is the rest of the
    /// proof: a `PathUnderField` fact (the file still lies under the project
    /// root) can change between this re-check and the write, since the daemon's
    /// create enforces only endpoint existence and edge absence, not the agent's
    /// path-prefix predicate. Fully closing that needs a graph snapshot/version
    /// the engine does not expose (gap A2); it is why nothing wires this live yet.
    ///
    /// The re-validation re-checks the proof's GRAPH facts (node existence, the
    /// path-under-project containment, edge absence), NOT the two non-configurable
    /// confirm rules (high-impact and external-trigger always confirm). Those are
    /// enforced point-in-time upstream, in `capability::decide_for_behaviour` and
    /// the gate lift: the only registered executable schema (`graph.write`)
    /// carries no `CapabilityAllows` precondition, so the `EvalContext` authority
    /// fields built below are inert for it and the executor is NOT an independent
    /// authority re-check today. It re-validates graph state and fails closed on a
    /// stale proof; an authority second line of defence lands when a higher-impact
    /// executable schema is registered carrying a `CapabilityAllows` precondition
    /// (assert it at registry load so a future schema cannot omit it, review
    /// AE-2). That is deferred because no such schema exists yet.
    ///
    /// `behaviour_name` and `ctx` are the trusted, dispatch-supplied ids (never
    /// the proposal), the same the gate decided with, so the execution audit
    /// links to that decision via the shared correlation id. The execution is
    /// audited fail-closed **before** the write (S13 audit-before-acting): if the
    /// ledger is unavailable, nothing is written.
    pub async fn execute(
        &self,
        action: &ProposedAction,
        decision: ActionDecision,
        tool_scope: &[String],
        graph: &dyn GraphHandle,
        behaviour_name: &str,
        ctx: &ActionContext<'_>,
        ceiling: BaselineMode,
    ) -> Result<Option<ExecutedWrite>, ExecError> {
        // Plan: only a proven PreviewThenExecute yields a write, and the planned
        // write is checked against the behaviour's declared scope (obligation 3).
        let Some(report) = dry_run(action, decision, tool_scope)? else {
            return Ok(None);
        };

        // Re-run the trusted proof against the live graph, right before the
        // write, so a precondition that went stale since the gate decision (a
        // node removed, the file moved out from under the project root, the edge
        // created concurrently) refuses the write. The schema and the slice are
        // rebuilt from the trusted registry, never the proposal. The read is
        // time-bounded (like the gate's proof): a stalled knowledge socket or a
        // slow path lookup fails closed rather than parking the executor.
        let trusted = registry::lookup(&action.tool)
            .ok_or_else(|| ExecError::NoRule(action.tool.clone()))?;
        let slice = tokio::time::timeout(
            REVALIDATION_TIMEOUT,
            build_slice_trusted(
                &trusted,
                &action.tool,
                &action.arguments,
                graph,
                self.paths,
                self.mounts,
            ),
        )
        .await
        .map_err(|_| ExecError::RevalidationTimeout)?;
        let (state, bindings) = slice.map_err(|_| ExecError::ProofStale)?;
        let eval = EvalContext {
            capability: self.capability,
            action_id: &action.tool,
            app_id: ctx.app_id,
            action_kind: resolved_action_kind(&action.tool),
            external_trigger: ctx.external_trigger,
            ceiling,
        };
        if !world::predict(trusted.schema(), &bindings, &state, &eval).is_valid() {
            return Err(ExecError::ProofStale);
        }

        // Audit the execution BEFORE the write, fail-closed (S13): the graph is
        // not mutated without a durable, content-free record of the act, linked
        // to the gate's decision entry by the shared correlation id.
        self.audit
            .submit(behaviour_action_event(behaviour_name, "execute", ctx.correlation_id))
            .await
            .map_err(|e| ExecError::AuditUnavailable(e.to_string()))?;

        // The proof still holds and the act is recorded: perform the authorised
        // write. A durable operation id, derived from the decision identity (the
        // correlation id) and the concrete write, is persisted on the edge, so a
        // commit-unknown write can be reconciled by reading whether THIS op's
        // edge exists. The outcome (created vs already-present) is carried back so
        // only a real create is ever compensated. The write is time-bounded: a
        // stalled knowledge socket must not park the executor (and, since the
        // daemon's dispatch loop awaits this, the whole daemon) indefinitely. A
        // timed-out write fails closed; it is pre-audited, so reconcilable later.
        let op_id = derive_op_id(ctx.correlation_id, &report.write);
        let outcome = match tokio::time::timeout(
            WRITE_TIMEOUT,
            self.writer.write_relation(&report.write, &op_id),
        )
        .await
        {
            Ok(Ok(o)) => o,
            // A definite no-commit (daemon rejected, or pre-send transport).
            Ok(Err(WriteError::Failed(e))) => return Err(ExecError::Write(e)),
            // Commit-unknown (a post-send transport failure, or a timeout that
            // may have fired after the request reached the daemon). Reconcile by
            // the durable op_id rather than report a failure that could discard a
            // real commit.
            Ok(Err(WriteError::Indeterminate(_))) | Err(_) => {
                return self.reconcile(report.write, op_id, ctx.correlation_id, graph).await
            }
        };
        Ok(Some(ExecutedWrite::new(
            report.write,
            outcome,
            op_id,
            ctx.correlation_id.to_string(),
        )))
    }

    /// Execute a gated `fs.move` - the executor's external (non-graph) arm and
    /// the first live non-graph action. Like [`execute`](Self::execute) it acts
    /// only for a proven `PreviewThenExecute`; unlike it there is no graph proof
    /// to re-run, because an external effect has no graph precondition (predict is
    /// vacuously `Valid` for it). Its safety is established HERE, at execute time:
    ///
    /// - the schema must be exactly one `Reversible` [`Effect::External`] resolved
    ///   from the trusted registry (obligation 1), so a non-reversible or graph
    ///   effect never reaches a move;
    /// - the source and destination directory are confined to the behaviour's
    ///   declared `fs.move` dir scope ([`confine_move_to_scope`]) - the SOLE path
    ///   confinement, since predict proves nothing for an external effect and the
    ///   gate's tool-scope check is name-only;
    /// - the move is planned collision-safe ([`fs_move::plan_move`]) so it never
    ///   overwrites and yields an exact `RestorePath` inverse, and the mover itself
    ///   refuses an occupied destination (the plan->move TOCTOU is closed);
    /// - the act is audited fail-closed BEFORE the move (S13 audit-before-acting),
    ///   linked to the gate decision by the shared correlation id.
    ///
    /// Returns the non-graph receipt ([`ActionWrite`]: the resolved forward, the
    /// captured inverse, the op id, the correlation id) so the compensate path can
    /// undo exactly this move, `None` for a non-executable decision, or an
    /// [`ExecError`]. `behaviour_name`/`ctx` are the trusted dispatch-supplied ids,
    /// never the proposal.
    pub async fn execute_external(
        &self,
        action: &ProposedAction,
        decision: ActionDecision,
        tool_scope: &[String],
        mover: &dyn FileMover,
        behaviour_name: &str,
        ctx: &ActionContext<'_>,
    ) -> Result<Option<ActionWrite>, ExecError> {
        if decision != ActionDecision::PreviewThenExecute {
            return Ok(None);
        }
        // Bind to the trusted registry schema (never a caller-passed one) and
        // require exactly one Reversible External effect (obligation 1): a
        // non-reversible or graph effect is refused, so nothing else rides this
        // arm and no model-declared "reversible" claim can reach a move (the class
        // comes from the trusted schema, not the proposal).
        let trusted = registry::lookup(&action.tool)
            .ok_or_else(|| ExecError::NoRule(action.tool.clone()))?;
        let (domain, op) = match trusted.schema().effects.as_slice() {
            [Effect::External { domain, op, class, .. }] if class.is_reversible() => {
                (*domain, op.clone())
            }
            _ => return Err(ExecError::UnsupportedEffect(action.tool.clone())),
        };

        // The (untrusted) operands.
        let source = action
            .arguments
            .get(FS_MOVE_SOURCE)
            .ok_or_else(|| ExecError::MissingArgument(FS_MOVE_SOURCE.to_string()))?;
        let dest_dir = action
            .arguments
            .get(FS_MOVE_DEST_DIR)
            .ok_or_else(|| ExecError::MissingArgument(FS_MOVE_DEST_DIR.to_string()))?;

        // Resolve both operands through the filesystem resolver (the SAME seam the
        // graph proof uses): `FsPathResolver` canonicalizes, following symlinks and
        // collapsing `..`, and rejects a dangling symlink. So a `dest_dir` like
        // `.../Projects/work` where `work` is a symlink to `/etc`, or a `source`
        // symlink to `~/.ssh/id_rsa`, resolves to its REAL target - and the
        // confinement below compares real paths, not attacker-controllable strings
        // (a raw string prefix check would let such a symlink escape the scope and
        // the move/copy follow it). An unresolvable operand fails closed.
        let resolved_source = self
            .paths
            .resolve(source)
            .map_err(|_| ExecError::Unplannable(format!("cannot resolve source '{source}'")))?;
        let resolved_dest_dir = self
            .paths
            .resolve(dest_dir)
            .map_err(|_| ExecError::Unplannable(format!("cannot resolve dest_dir '{dest_dir}'")))?;

        // The sole path confinement: both RESOLVED ends under a RESOLVED scope root.
        let roots = resolve_scope_roots(self.paths, tool_scope)?;
        confine_to_roots(&resolved_source, &resolved_dest_dir, &roots)?;

        // Plan a collision-safe destination (never an occupied path) under the
        // resolved dir, reading the live filesystem. `None` means a non-canonical
        // path, a self-move, or no free name within the bound - fail closed.
        let plan = fs_move::plan_move(&resolved_source, &resolved_dest_dir, |p| {
            std::path::Path::new(p).exists()
        })
        .ok_or_else(|| {
            ExecError::Unplannable(format!(
                "no safe destination for '{resolved_source}' in '{resolved_dest_dir}'"
            ))
        })?;

        // Audit the act BEFORE the move, fail-closed (S13): no file is moved
        // without a durable, content-free record linked to the gate decision.
        self.audit
            .submit(behaviour_action_event(behaviour_name, "execute", ctx.correlation_id))
            .await
            .map_err(|e| ExecError::AuditUnavailable(e.to_string()))?;

        // Perform the move; the mover refuses to overwrite, so a file appearing at
        // the planned destination between plan and move cannot be destroyed.
        let inverse = fs_move::execute_move(&plan, mover)
            .map_err(|e| ExecError::Write(e.to_string()))?
            .clone();

        let op_id = derive_external_op_id(
            ctx.correlation_id,
            plan.source.as_str(),
            plan.destination.as_str(),
        );
        let forward = ResolvedExternalOp::new(domain, op, plan.destination.as_str().to_string());
        Ok(Some(ActionWrite::new(
            forward,
            inverse,
            op_id,
            ctx.correlation_id.to_string(),
        )))
    }

    /// Compensate (undo) a write this executor performed, retracting exactly the
    /// edge it created.
    ///
    /// This is the inverse of [`execute`](Self::execute) and closes the
    /// predict -> gate -> act -> audit -> compensate loop. It needs no fresh
    /// world-model proof: the action was lifted precisely because it declared a
    /// reversible effect, so its compensation (the retract) is safe by
    /// construction. The op id and the audit correlation id are taken **from the
    /// execution receipt** (`executed.op_id` / `executed.correlation_id`), never
    /// re-derived from a separately-supplied context, so the retract is keyed to
    /// the very edge THIS execution stamped — a mis-threaded or stale context
    /// over the same relation cannot redirect the delete to another op's edge,
    /// and the audit always links to the decision that actually wrote it.
    ///
    /// Only a [`WriteOutcome::Created`] write is compensated: an `AlreadyExists`
    /// write never created the edge, so there is nothing this action may undo
    /// ([`CompensationOutcome::NothingToUndo`]). Even were that guard absent, the
    /// op-id keying would make the retract a no-op (this op's id is not on an edge
    /// it did not create), so this is defence in depth, not the sole safeguard.
    ///
    /// The compensation is audited fail-closed **before** the retract (the same
    /// S13 audit-before-acting invariant as `execute`), linked to the original
    /// decision by the receipt's correlation id. The retract is time-bounded and
    /// idempotent; a commit-unknown retract is reconciled by reading whether this
    /// op's edge is now gone.
    /// Compensate (undo) a previously-executed action, dispatching on the receipt
    /// variant (reversible-receipts-and-the-effect-model.md §5). The graph arm is
    /// the original op-id-keyed retract verbatim; the non-graph arm replays the
    /// captured [`InverseReceipt`] and is the EM-R6 increment. No non-graph receipt
    /// is produced until the executor's non-graph arm (EM-R5), so the non-graph arm
    /// fails closed today rather than silently no-op.
    pub async fn compensate(
        &self,
        receipt: &ActionReceipt,
        graph: &dyn GraphHandle,
        behaviour_name: &str,
    ) -> Result<CompensationOutcome, ExecError> {
        // The graph-focused executor undoes only graph receipts; a non-graph undo
        // is the owned `Compensator`'s job (it supplies the file mover), so pass
        // `None` here.
        compensate_receipt(self.writer, self.audit, None, receipt, graph, behaviour_name).await
    }


    /// Resolve a commit-unknown write by its durable operation id. The op_id
    /// query is causally sound: an edge carrying THIS op_id can only have been
    /// created by this operation's write, so its presence proves the write
    /// committed (`Created`). If that edge is absent but the relation exists
    /// (some other op or the promotion pipeline created it), this write was an
    /// idempotent no-op (`AlreadyExists`). If neither holds, or a read fails, the
    /// write's commit is genuinely unknown (`Indeterminate`): it may yet commit,
    /// and a retry with the same op_id resolves it (its op_id edge would then be
    /// found). Both reads are time-bounded so a stalled socket cannot hang here.
    async fn reconcile(
        &self,
        write: RelationWrite,
        op_id: String,
        correlation_id: &str,
        graph: &dyn GraphHandle,
    ) -> Result<Option<ExecutedWrite>, ExecError> {
        // The ONLY sound positive verdict is the op_id edge being present: an
        // edge carrying this op_id can only have been created by this write, so
        // it proves the commit (and stays proof even if the edge is later
        // deleted). Anything else is `Indeterminate`: the bare edge cannot
        // prove a no-op, because `FILE_PART_OF` IS deletable (the project store
        // unlinks files), so a present bare edge could be deleted and this
        // still-in-flight write could then create its own op_id edge; and a
        // missing edge may yet be created by this write. A retry with the same
        // op_id resolves it (its op_id edge would then be observed).
        // Poll for the op-id edge becoming PRESENT (target = present): a commit
        // that lands just after the write timed out is caught by the backed-off
        // retries instead of being reported indeterminate prematurely.
        match poll_for(&write, &op_id, graph, true).await {
            Some(true) => Ok(Some(ExecutedWrite::new(
                write,
                WriteOutcome::Created,
                op_id,
                correlation_id.to_string(),
            ))),
            // Still not confirmed after the poll budget. Carry the key out, so a
            // write that commits even LATER (the daemon's queued CREATE may run
            // beyond the budget) is not left with its op-id edge in the graph and
            // no receipt to undo it; the next organic reconcile resolves it.
            Some(false) => Err(ExecError::WriteIndeterminate {
                pending: PendingWrite::new(write, op_id, correlation_id.to_string()),
                reason: "write unconfirmed; its op_id edge is not present".to_string(),
            }),
            None => Err(ExecError::WriteIndeterminate {
                pending: PendingWrite::new(write, op_id, correlation_id.to_string()),
                reason: "reconciliation read failed".to_string(),
            }),
        }
    }

}

/// An owned compensator: holds just the `writer` + `audit` the compensate path
/// needs (Arc, not borrowed), so a long-lived owner (the D-Bus undo interface)
/// can run a compensation without constructing a [`LiveExecutor`] (whose
/// capability/paths/mounts only the forward execute path uses) or a dummy
/// capability. Built at startup under `executor_live`; delegates to the same
/// [`compensate_receipt`] free function as [`LiveExecutor::compensate`], so the
/// undo semantics are identical on both paths. Clone is a cheap Arc bump (the
/// D-Bus interface that owns it is re-registered on a bus reconnect).
#[derive(Clone)]
pub struct Compensator {
    writer: Arc<dyn RelationWriter>,
    audit: Arc<dyn AuditSink>,
    mover: Arc<dyn FileMover>,
}

impl Compensator {
    /// A compensator over an owned writer, audit sink, and file mover. The mover
    /// is the non-graph undo's seam (it replays an `fs.move`'s `RestorePath`); the
    /// production wiring passes [`fs_move::OsFileMover`].
    pub fn new(
        writer: Arc<dyn RelationWriter>,
        audit: Arc<dyn AuditSink>,
        mover: Arc<dyn FileMover>,
    ) -> Self {
        Self {
            writer,
            audit,
            mover,
        }
    }

    /// Compensate (undo) a previously-executed action by its receipt. The single
    /// undo path for BOTH receipt kinds: a graph edge (op-id-keyed retract) and a
    /// non-graph action (replay the captured inverse, e.g. move a file back).
    /// Audits fail-closed before acting, keys the undo to the receipt's own op id,
    /// only undoes a real write.
    pub async fn compensate(
        &self,
        receipt: &ActionReceipt,
        graph: &dyn GraphHandle,
        behaviour_name: &str,
    ) -> Result<CompensationOutcome, ExecError> {
        compensate_receipt(
            &*self.writer,
            &*self.audit,
            Some(&*self.mover),
            receipt,
            graph,
            behaviour_name,
        )
        .await
    }
}

/// The owned executor for the approve path - the actionable half of the harness
/// gate card's `[Approve]`. Like [`Compensator`] it holds owned deps so it can
/// live on the startup-'static D-Bus interface, but unlike the undo path the
/// approve path RE-RUNS the full trusted proof, so it carries every proof
/// collaborator (paths + mounts + writer + audit). The `Capability` is NOT held:
/// it is rebuilt per config epoch (read tier + action permissions), so a startup
/// snapshot could mis-evaluate a future `CapabilityAllows` precondition. The
/// caller passes the LIVE capability at approve time, mirroring how the undo
/// path re-reads `executor_live` live. Clone is a cheap Arc bump (the interface
/// that owns it is re-registered on a bus reconnect).
#[derive(Clone)]
pub struct Approver {
    paths: Arc<dyn PathResolver>,
    mounts: Arc<dyn MountPolicy>,
    writer: Arc<dyn RelationWriter>,
    audit: Arc<dyn AuditSink>,
    mover: Arc<dyn FileMover>,
}

impl Approver {
    /// An approver over the owned proof + write collaborators, including the file
    /// mover the external (`fs.move`) arm needs (the production wiring passes
    /// [`fs_move::OsFileMover`]).
    pub fn new(
        paths: Arc<dyn PathResolver>,
        mounts: Arc<dyn MountPolicy>,
        writer: Arc<dyn RelationWriter>,
        audit: Arc<dyn AuditSink>,
        mover: Arc<dyn FileMover>,
    ) -> Self {
        Self {
            paths,
            mounts,
            writer,
            audit,
            mover,
        }
    }

    /// Perform a user-approved proposal: build the live executor over the owned
    /// deps plus the caller-supplied live `capability`, and execute the retained
    /// proposal as a `PreviewThenExecute` - the human approval IS the lift from
    /// `RequireConfirmation` (a `kind: agent` action never silently lifts, so the
    /// approve path is how its proven action reaches execution). The executor
    /// re-runs the full trusted proof / re-confines the move against the CURRENT
    /// state and audits fail-closed before acting, so approval authorises the act
    /// but never bypasses revalidation: a proposal whose preconditions went stale
    /// since the gate decision is still refused.
    ///
    /// Branches on the tool's registered effect: a graph `AssertEdge` goes through
    /// [`LiveExecutor::execute`] (a `Graph` receipt); a non-graph
    /// [`Effect::External`] (`fs.move`) goes through
    /// [`LiveExecutor::execute_external`] with the owned mover (a `NonGraph`
    /// receipt). Returns the receipt (so the caller retains it for undo) or `None`
    /// when the proposal does not resolve to an action.
    pub async fn approve(
        &self,
        retained: &RetainedProposal,
        graph: &dyn GraphHandle,
        capability: &Capability,
    ) -> Result<Option<ActionReceipt>, ExecError> {
        // A honeytool must never reach execution. It is frozen pre-gate (never
        // captured as a pending proposal) and its empty effects route to the graph
        // arm which refuses them, so this is unreachable in a legit run; the assert
        // is the belt the dispatch-path `maybe_execute` also carries.
        debug_assert!(
            !registry::is_honeytool(&retained.action.tool),
            "a honeytool reached the approve path"
        );
        let executor = LiveExecutor::new(
            capability,
            &*self.paths,
            &*self.mounts,
            &*self.writer,
            &*self.audit,
        );
        let ctx = ActionContext {
            app_id: &retained.app_id,
            external_trigger: retained.external_trigger,
            // An approved agent action is not a deterministic workflow; the field
            // is a gate-decision input, unused on the execute path, set honestly.
            deterministic_workflow: false,
            correlation_id: &retained.correlation_id,
        };
        if registry::is_external_action(&retained.action.tool) {
            let written = executor
                .execute_external(
                    &retained.action,
                    ActionDecision::PreviewThenExecute,
                    &retained.tool_scope,
                    &*self.mover,
                    &retained.behaviour,
                    &ctx,
                )
                .await?;
            Ok(written.map(ActionReceipt::NonGraph))
        } else {
            let written = executor
                .execute(
                    &retained.action,
                    ActionDecision::PreviewThenExecute,
                    &retained.tool_scope,
                    graph,
                    &retained.behaviour,
                    &ctx,
                    retained.ceiling,
                )
                .await?;
            Ok(written.map(ActionReceipt::Graph))
        }
    }
}

/// Compensate (undo) a previously-executed action, dispatching on the receipt
/// variant. A free function over only the `writer` + `audit` it needs (not the
/// full executor's capability/paths/mounts), so both `LiveExecutor::compensate`
/// and the owned `Compensator` (the D-Bus undo path) call it without
/// constructing an executor or a dummy capability. The graph arm retracts the
/// op-id-keyed edge; the non-graph arm is the EM-R6 increment and fails closed
/// (no non-graph receipt is produced until the executor's EM-R5 arm).
async fn compensate_receipt(
    writer: &dyn RelationWriter,
    audit: &dyn AuditSink,
    mover: Option<&dyn FileMover>,
    receipt: &ActionReceipt,
    graph: &dyn GraphHandle,
    behaviour_name: &str,
) -> Result<CompensationOutcome, ExecError> {
    match receipt {
        ActionReceipt::Graph(executed) => {
            compensate_graph(writer, audit, executed, graph, behaviour_name).await
        }
        // A non-graph undo needs a file mover. Only the owned [`Compensator`] (the
        // D-Bus undo path) supplies one; the graph-focused `LiveExecutor` passes
        // `None`, so a NonGraph receipt reaching it fails closed rather than
        // silently no-op (it would be a wiring bug, not a user undo).
        ActionReceipt::NonGraph(action) => match mover {
            Some(m) => compensate_external(audit, m, action, behaviour_name).await,
            None => Err(ExecError::UnsupportedEffect(
                "non-graph compensation needs a file mover; only the undo path supplies one"
                    .to_string(),
            )),
        },
    }
}

/// Compensate (undo) a non-graph action by replaying its captured
/// [`InverseReceipt`]. For a `RestorePath` inverse (the `fs.move` undo) the undo
/// moves the file from where it landed (`now`) back to its origin (`prior`)
/// through the same [`FileMover`] the forward used.
///
/// No fresh proof is needed: the action was lifted precisely because it declared
/// a reversible effect, and the inverse was CAPTURED at execute time from the
/// real move, so replaying it restores exactly that move (the op id /
/// correlation id come from the receipt, never a re-supplied context). The undo
/// is audited fail-closed BEFORE the move (the same S13 invariant as the forward),
/// linked to the original decision by the receipt's correlation id. The mover
/// refuses to overwrite, so a `prior` path now occupied (the user put something
/// back there) refuses the restore rather than clobbering it: the undo is itself
/// non-destructive.
async fn compensate_external(
    audit: &dyn AuditSink,
    mover: &dyn FileMover,
    action: &ActionWrite,
    behaviour_name: &str,
) -> Result<CompensationOutcome, ExecError> {
    // Audit the undo before acting, linked to the original decision.
    audit
        .submit(behaviour_action_event(behaviour_name, "compensate", action.correlation_id()))
        .await
        .map_err(|e| ExecError::AuditUnavailable(e.to_string()))?;
    match action.inverse() {
        InverseReceipt::RestorePath { now, prior } => {
            mover
                .move_file(now.as_str(), prior.as_str())
                .map_err(|e| ExecError::Write(e.to_string()))?;
            Ok(CompensationOutcome::Retracted)
        }
        // The only non-graph receipt produced today is `fs.move`'s `RestorePath`.
        // The other inverse classes (RestoreValue, DeleteCreated, RestoreSnapshot)
        // land with their executor arms; until then, refuse rather than guess.
        other => Err(ExecError::UnsupportedEffect(format!(
            "non-graph compensation for {other:?} is not yet wired"
        ))),
    }
}

/// The graph-edge compensation: retract only the edge this execution's op id
/// stamped. Audits the compensation before the retract (fail-closed, the S13
/// audit-before-acting invariant), keys the retract to the receipt's own op id
/// (never re-derived), and only undoes a real `Created` write. A commit-unknown
/// retract is reconciled by [`reconcile_retract`].
async fn compensate_graph(
    writer: &dyn RelationWriter,
    audit: &dyn AuditSink,
    executed: &ExecutedWrite,
    graph: &dyn GraphHandle,
    behaviour_name: &str,
) -> Result<CompensationOutcome, ExecError> {
    // Only undo a real create. A no-op write created nothing to retract.
    if executed.outcome != WriteOutcome::Created {
        return Ok(CompensationOutcome::NothingToUndo);
    }

    // Audit the compensation before the retract, fail-closed: the graph is not
    // mutated (here, un-mutated) without a durable, content-free record, linked
    // to the original decision by the receipt's own correlation id.
    audit
        .submit(behaviour_action_event(behaviour_name, "compensate", &executed.correlation_id))
        .await
        .map_err(|e| ExecError::AuditUnavailable(e.to_string()))?;

    // The op id comes from the receipt, so the retract is keyed to this
    // execution's own edge and nothing else.
    match tokio::time::timeout(
        WRITE_TIMEOUT,
        writer.retract_relation(&executed.write, &executed.op_id),
    )
    .await
    {
        Ok(Ok(RetractOutcome::Retracted)) => Ok(CompensationOutcome::Retracted),
        Ok(Ok(RetractOutcome::Absent)) => Ok(CompensationOutcome::NothingToUndo),
        // A definite no-commit: the retract did not run, the edge is unchanged.
        Ok(Err(WriteError::Failed(e))) => Err(ExecError::Write(e)),
        // Commit-unknown (post-send transport failure, or a timeout that may have
        // fired after the retract reached the daemon). Reconcile by the op id
        // rather than report a failure that could hide a real retract.
        Ok(Err(WriteError::Indeterminate(_))) | Err(_) => {
            reconcile_retract(&executed.write, &executed.op_id, &executed.correlation_id, graph).await
        }
    }
}

/// Resolve a commit-unknown retract by its durable operation id.
///
/// `Retracted` is reported iff this op's edge is now **non-live** (closed or
/// gone), sound by op-id monotonicity run in reverse: a retract temporally
/// closes the op-id edge and nothing re-opens that specific id (compensation
/// runs only for a `Created` write whose create already happened), so non-live
/// is a terminal post-state. A still-live edge means the retract did not commit
/// ([`ExecError::WriteIndeterminate`], resolved by an idempotent retry); a failed
/// read is likewise indeterminate, never a false success.
async fn reconcile_retract(
    write: &RelationWrite,
    op_id: &str,
    correlation_id: &str,
    graph: &dyn GraphHandle,
) -> Result<CompensationOutcome, ExecError> {
    let pending =
        || PendingWrite::new(write.clone(), op_id.to_string(), correlation_id.to_string());
    // Poll for the edge becoming ABSENT (target = not present): the retract may
    // land slightly after the immediate read, so a few backed-off reads catch it
    // before reporting indeterminate.
    match poll_for(write, op_id, graph, false).await {
        Some(true) => Ok(CompensationOutcome::Retracted),
        Some(false) => Err(ExecError::WriteIndeterminate {
            pending: pending(),
            reason: "retract unconfirmed; this op's edge is still live".to_string(),
        }),
        None => Err(ExecError::WriteIndeterminate {
            pending: pending(),
            reason: "retract reconciliation read failed".to_string(),
        }),
    }
}

/// Reconciler: poll [`edge_exists`] up to [`RECONCILE_ATTEMPTS`] times with
/// backoff, stopping as soon as the edge reaches `target_present` (present
/// for a create's confirmation, absent for a retract's). This is the bounded
/// "read repeatedly until confirmed" job: a commit (or delete) that lands a
/// moment after the immediate read is caught instead of being reported
/// indeterminate too early.
///
/// Returns `Some(true)` once it observes the target state (a definite,
/// sound verdict — for a create, op-id-present can only be this write; for a
/// retract, op-id-absent is terminal by the same monotonicity); `Some(false)`
/// if every definite read disagreed across the whole budget (still
/// unresolved, the key is preserved for the next organic reconcile); `None`
/// if no read ever succeeded (a persistently failing/stalled socket). It
/// never blocks unbounded: a never-landing write resolves to `Some(false)`
/// after the fixed budget rather than waiting forever.
///
/// A free function (not a method): shared by the create-side and compensate-side
/// reconcilers, using no executor state (only the graph, via [`edge_exists`]).
async fn poll_for(
    write: &RelationWrite,
    op_id: &str,
    graph: &dyn GraphHandle,
    target_present: bool,
) -> Option<bool> {
    let mut had_definite_read = false;
    for attempt in 0..RECONCILE_ATTEMPTS {
        if attempt > 0 {
            tokio::time::sleep(reconcile_backoff(attempt)).await;
        }
        if let Some(present) = edge_exists(write, graph, op_id).await {
            had_definite_read = true;
            if present == target_present {
                return Some(true);
            }
        }
    }
    // Never reached the target. Distinguish "definitely never matched" (we
    // got at least one good read) from "could not read at all" (None), so a
    // read outage is not mistaken for a definite disagreement.
    if had_definite_read {
        Some(false)
    } else {
        None
    }
}

/// Read whether this operation's edge (carrying `op_id`) is present and
/// **live** now, time-bounded. Liveness is load-bearing since the daemon's
/// retract is a temporal *close*, not a delete (bitemporal-knowledge-graph.md
/// §4.7): a retracted edge is retained with its intervals set, so a bare
/// presence check would still see it and the retract-reconcile would never
/// confirm. Filtering to the live edge (`invalid_at`/`expired_at` both NULL)
/// makes "present" mean "still asserted": a created edge is live, a closed
/// edge is not. `Some(true)`/`Some(false)` is a definite verdict; `None` means
/// the read could not determine it (a failed/timed-out query, or a malformed
/// result). Fail-closed on a malformed shape (missing row, missing or
/// non-numeric `n`, negative count): a degraded read becomes `None`, never a
/// false absence that would lose a real commit. The endpoint types are
/// built-in system types, so the labels are the type minus `system.`; the ids
/// and op_id are escaped into the literal.
///
/// A free function (not a method): shared by the create-side reconcile and the
/// compensate-side reconcile, and uses no executor state (only the graph).
async fn edge_exists(write: &RelationWrite, graph: &dyn GraphHandle, op_id: &str) -> Option<bool> {
    let from_label = write.from_type.strip_prefix("system.").unwrap_or(&write.from_type);
    let to_label = write.to_type.strip_prefix("system.").unwrap_or(&write.to_type);
    let from_lit = escape_cypher_literal(&write.from_id).ok()?;
    let to_lit = escape_cypher_literal(&write.to_id).ok()?;
    let op_lit = escape_cypher_literal(op_id).ok()?;
    let cypher = format!(
        "MATCH (a:{from_label} {{id: '{from_lit}'}})-[r:{edge} {{op_id: '{op_lit}'}}]->(b:{to_label} {{id: '{to_lit}'}}) \
         WHERE r.invalid_at IS NULL AND r.expired_at IS NULL \
         RETURN count(*) AS n",
        edge = write.relation_type,
    );
    let rows = tokio::time::timeout(RECONCILE_TIMEOUT, graph.query(&cypher))
        .await
        .ok()?
        .ok()?;
    // Exactly one row with a numeric, non-negative count, else fail closed.
    let n = rows.first()?.get("n")?.as_i64()?;
    if n < 0 {
        return None;
    }
    Some(n > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effect_model::CanonicalPath;

    #[test]
    fn non_graph_receipt_carries_its_forward_and_captured_inverse() {
        // The test submodule, an in-module descendant, builds receipts via the
        // module-private constructors (the opacity discipline that keeps them
        // non-fabricable outside the executor).
        let forward = ResolvedExternalOp::new(
            EffectDomain::Filesystem,
            "move".to_string(),
            "/home/tim/b/x".to_string(),
        );
        let inverse = InverseReceipt::RestorePath {
            now: CanonicalPath::new("/home/tim/b/x").unwrap(),
            prior: CanonicalPath::new("/home/tim/a/x").unwrap(),
        };
        let receipt = ActionReceipt::NonGraph(ActionWrite::new(
            forward,
            inverse.clone(),
            "op-1".to_string(),
            "corr-1".to_string(),
        ));
        let ActionReceipt::NonGraph(w) = &receipt else {
            panic!("expected a non-graph receipt");
        };
        assert_eq!(w.forward().domain(), EffectDomain::Filesystem);
        assert_eq!(w.forward().op(), "move");
        assert_eq!(w.forward().target(), "/home/tim/b/x");
        assert_eq!(w.op_id(), "op-1");
        assert_eq!(w.correlation_id(), "corr-1");
        assert_eq!(w.inverse(), &inverse);
    }

    #[test]
    fn reconcile_backoff_grows_per_attempt() {
        // 250ms * attempt: no wait before the first read, a growing wait after,
        // with a small total bound. A constant or a +/- mutation would change
        // the schedule the reconcile poll depends on.
        assert_eq!(reconcile_backoff(1), Duration::from_millis(250));
        assert_eq!(reconcile_backoff(2), Duration::from_millis(500));
        assert_eq!(reconcile_backoff(3), Duration::from_millis(750));
    }

    #[test]
    fn executed_write_exposes_its_compensation_key() {
        // op_id is the durable edge stamp the compensation retract targets, and
        // correlation_id links it to the deciding action. The accessors must
        // return the stored ids - a constant would misdirect every undo.
        let w = receipt(WriteOutcome::Created);
        assert_eq!(w.op_id(), derive_op_id("run-x", w.write()));
        assert_eq!(w.correlation_id(), "run-x");
    }

    fn graph_write_action(args: &[(&str, &str)]) -> ProposedAction {
        ProposedAction {
            tool: "graph.write".to_string(),
            summary: "link file to project".to_string(),
            arguments: args
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }

    /// The auto-tag behaviour's declared `graph.write` scope.
    fn auto_tag_scope() -> Vec<String> {
        vec!["Project".to_string(), "FILE_PART_OF".to_string()]
    }

    #[test]
    fn plans_the_file_part_of_write_from_the_trusted_schema() {
        let schema = registry::lookup("graph.write").unwrap();
        let args: BTreeMap<String, String> =
            [("file", "f1"), ("project", "p1")]
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect();
        let write = plan_relation_write(&schema, &args).unwrap();
        assert_eq!(
            write,
            RelationWrite {
                from_type: "system.File".to_string(),
                from_id: "f1".to_string(),
                to_type: "system.Project".to_string(),
                to_id: "p1".to_string(),
                relation_type: "FILE_PART_OF".to_string(),
            }
        );
    }

    #[test]
    fn a_missing_argument_fails_closed() {
        let schema = registry::lookup("graph.write").unwrap();
        // Only the `file` id is supplied; `project` is missing.
        let args: BTreeMap<String, String> =
            [("file", "f1")].iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        assert_eq!(
            plan_relation_write(&schema, &args),
            Err(ExecError::MissingArgument("project".to_string()))
        );
    }

    #[test]
    fn dry_run_plans_only_for_the_proven_preview_decision() {
        let action = graph_write_action(&[("file", "f1"), ("project", "p1")]);
        let scope = auto_tag_scope();

        // PreviewThenExecute is the only decision carrying a successful proof, so
        // it is the only one that produces an executable write. The built-in link
        // rule is strict-create, so the plan flags that the live executor must
        // check the edge is absent (not a bare MERGE).
        let report = dry_run(&action, ActionDecision::PreviewThenExecute, &scope)
            .unwrap()
            .unwrap();
        assert_eq!(report.write.relation_type, "FILE_PART_OF");
        assert!(
            report.conditional_on_absent_edge,
            "the FILE_PART_OF rule proves Not(EdgeExists), so the create is conditional"
        );

        // Every non-proven decision plans nothing: RequireConfirmation is the
        // unproven cap (operands unvalidated), Propose is manual, and Proceed is
        // not emitted by the gate. None must derive a write from the operands.
        assert_eq!(
            dry_run(&action, ActionDecision::RequireConfirmation, &scope).unwrap(),
            None,
            "an unproven confirmation must not yield an executable write"
        );
        assert_eq!(dry_run(&action, ActionDecision::Propose, &scope).unwrap(), None);
        assert_eq!(dry_run(&action, ActionDecision::Proceed, &scope).unwrap(), None);
    }

    #[test]
    fn op_id_is_deterministic_and_distinguishes_decisions() {
        let write = RelationWrite {
            from_type: "system.File".into(),
            from_id: "f1".into(),
            to_type: "system.Project".into(),
            to_id: "p1".into(),
            relation_type: "FILE_PART_OF".into(),
        };
        // Same decision + write -> same id (so a replay is recognisable).
        assert_eq!(derive_op_id("e1:auto-tag", &write), derive_op_id("e1:auto-tag", &write));
        // A different decision -> a different id.
        assert_ne!(derive_op_id("e1:auto-tag", &write), derive_op_id("e2:auto-tag", &write));
        // A different operand -> a different id (length-delimited, no boundary
        // collision).
        let mut other = write.clone();
        other.to_id = "p2".into();
        assert_ne!(derive_op_id("e1:auto-tag", &write), derive_op_id("e1:auto-tag", &other));
        // 64 hex chars (SHA-256), within the daemon's op_id bound.
        let id = derive_op_id("e1:auto-tag", &write);
        assert_eq!(id.len(), 64);
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn dry_run_refuses_an_unregistered_tool() {
        let action = ProposedAction {
            tool: "fs.delete".to_string(),
            summary: "delete".to_string(),
            arguments: BTreeMap::new(),
        };
        assert_eq!(
            dry_run(&action, ActionDecision::PreviewThenExecute, &[]),
            Err(ExecError::NoRule("fs.delete".to_string()))
        );
    }

    #[test]
    fn dry_run_enforces_the_declared_tool_scope() {
        let action = graph_write_action(&[("file", "f1"), ("project", "p1")]);

        // A scope that grants the relation but not the target entity is refused:
        // the executor enforces scope values (obligation 3), not just the name.
        let scope = vec!["FILE_PART_OF".to_string()];
        assert_eq!(
            dry_run(&action, ActionDecision::PreviewThenExecute, &scope),
            Err(ExecError::ScopeViolation {
                tool: "graph.write".to_string(),
                token: "Project".to_string(),
            })
        );

        // A scope missing the relation type is likewise refused.
        let scope = vec!["Project".to_string()];
        assert_eq!(
            dry_run(&action, ActionDecision::PreviewThenExecute, &scope),
            Err(ExecError::ScopeViolation {
                tool: "graph.write".to_string(),
                token: "FILE_PART_OF".to_string(),
            })
        );
    }

    #[test]
    fn an_empty_scope_grants_without_restriction() {
        let action = graph_write_action(&[("file", "f1"), ("project", "p1")]);
        // The manifest convention: an empty scope list grants the tool without a
        // finer restriction, so the write plans.
        let report = dry_run(&action, ActionDecision::PreviewThenExecute, &[])
            .unwrap()
            .unwrap();
        assert_eq!(report.write.relation_type, "FILE_PART_OF");
    }

    // ---- LiveExecutor: obligation-2 re-validation + write ----

    use crate::seams::GraphError;
    use crate::slice::{SliceError, StaticMountPolicy};
    use audit_proto::MockAuditSink;
    use arlen_ai_core::capability::{AccessTier, ActionPermissions};
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// A graph returning canned rows when the query contains a needle (the same
    /// shape the gate's proof tests use).
    struct MockGraph(Vec<(&'static str, Vec<HashMap<String, serde_json::Value>>)>);

    #[async_trait]
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

    /// Accepts an already-canonical absolute path as itself.
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

    /// The graph for tagging `/proj/a.rs` (under `/proj`) to project `p1`, with
    /// the `FILE_PART_OF` edge present or not.
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

    fn tag_action() -> ProposedAction {
        graph_write_action(&[("file", "/proj/a.rs"), ("project", "p1")])
    }

    fn executing_cap() -> Capability {
        Capability::new(
            AccessTier::Full,
            ActionPermissions::new(BaselineMode::Suggest, ["org.arlen.files"]),
        )
    }

    /// Records each write and retract without performing I/O.
    #[derive(Default)]
    struct MockWriter {
        writes: Mutex<Vec<RelationWrite>>,
        retracts: Mutex<Vec<(RelationWrite, String)>>,
    }

    #[async_trait]
    impl RelationWriter for MockWriter {
        async fn write_relation(&self, write: &RelationWrite, _op_id: &str) -> Result<WriteOutcome, WriteError> {
            self.writes.lock().unwrap().push(write.clone());
            Ok(WriteOutcome::Created)
        }
        async fn retract_relation(&self, write: &RelationWrite, op_id: &str) -> Result<RetractOutcome, WriteError> {
            self.retracts.lock().unwrap().push((write.clone(), op_id.to_string()));
            Ok(RetractOutcome::Retracted)
        }
    }

    /// The trusted per-call context the dispatcher supplies (never the proposal).
    fn ctx() -> ActionContext<'static> {
        ActionContext {
            app_id: "org.arlen.files",
            external_trigger: false,
            deterministic_workflow: false,
            correlation_id: "run-x",
        }
    }

    #[tokio::test]
    async fn live_executor_writes_and_audits_a_revalidated_proof() {
        let cap = executing_cap();
        let writer = MockWriter::default();
        let audit = MockAuditSink::accepting();
        let graph = tag_graph(false); // file under root, not yet linked
        let (resolver, mounts) = (IdentityResolver, StaticMountPolicy::empty());
        let exec = LiveExecutor::new(&cap, &resolver, &mounts, &writer, &audit);
        let written = exec
            .execute(
                &tag_action(),
                ActionDecision::PreviewThenExecute,
                &auto_tag_scope(),
                &graph,
                "auto-tag-by-project",
                &ctx(),
                BaselineMode::Supervised,
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(written.write.relation_type, "FILE_PART_OF");
        assert_eq!(written.outcome, WriteOutcome::Created);
        let recorded = writer.writes.lock().unwrap();
        assert_eq!(recorded.len(), 1, "exactly one write performed");
        assert_eq!(recorded[0].to_type, "system.Project");
        assert_eq!(recorded[0].from_id, "/proj/a.rs");

        // The execution is recorded content-free, linked to the gate decision
        // by the shared correlation id.
        let entries = audit.recorded().await;
        assert_eq!(entries.len(), 1, "the execution is audited once");
        assert_eq!(entries[0].structural.subject, "agent.auto-tag-by-project");
        assert_eq!(entries[0].structural.outcome, "execute");
        assert_eq!(entries[0].call_chain_id.as_deref(), Some("run-x"));
    }

    #[tokio::test]
    async fn approver_executes_a_user_approved_proposal() {
        // The retained proposal the approve store would hold for a confirmed
        // gate card (the auto-tag link, captured as RequireConfirmation).
        let retained = RetainedProposal::capture(
            0,
            "auto-tag-by-project",
            &tag_action(),
            ActionDecision::RequireConfirmation,
            &auto_tag_scope(),
            "org.arlen.files",
            false,
            "run-x",
            BaselineMode::Supervised,
        )
        .expect("a RequireConfirmation decision is retained");

        // Keep an Arc<MockWriter> handle to inspect the write after the approve.
        let writer = Arc::new(MockWriter::default());
        let approver = Approver::new(
            Arc::new(IdentityResolver),
            Arc::new(StaticMountPolicy::empty()),
            writer.clone(),
            Arc::new(MockAuditSink::accepting()),
            Arc::new(crate::fs_move::OsFileMover),
        );

        // The human approval performs the write: the approver re-runs the proof
        // against the current graph (file under the project root, not yet linked)
        // and, on success, writes exactly the proven FILE_PART_OF edge.
        let receipt = approver
            .approve(&retained, &tag_graph(false), &executing_cap())
            .await
            .expect("approve does not error")
            .expect("a proven proposal resolves to a write");
        let ActionReceipt::Graph(written) = receipt else {
            panic!("a graph.write proposal yields a Graph receipt");
        };
        assert_eq!(written.write().relation_type, "FILE_PART_OF");
        assert_eq!(written.outcome(), WriteOutcome::Created);
        let recorded = writer.writes.lock().unwrap();
        assert_eq!(recorded.len(), 1, "exactly one write performed on approval");
        assert_eq!(recorded[0].from_id, "/proj/a.rs");
        assert_eq!(recorded[0].to_type, "system.Project");
    }

    #[tokio::test]
    async fn approver_refuses_a_proposal_whose_proof_went_stale() {
        // Same proposal, but the edge now already exists: the re-run proof fails
        // its `Not(EdgeExists)` precondition, so approval writes nothing (approval
        // authorises the act but never bypasses revalidation).
        let retained = RetainedProposal::capture(
            0,
            "auto-tag-by-project",
            &tag_action(),
            ActionDecision::RequireConfirmation,
            &auto_tag_scope(),
            "org.arlen.files",
            false,
            "run-x",
            BaselineMode::Supervised,
        )
        .expect("retained");
        let writer = Arc::new(MockWriter::default());
        let approver = Approver::new(
            Arc::new(IdentityResolver),
            Arc::new(StaticMountPolicy::empty()),
            writer.clone(),
            Arc::new(MockAuditSink::accepting()),
            Arc::new(crate::fs_move::OsFileMover),
        );
        let result = approver
            .approve(&retained, &tag_graph(true), &executing_cap())
            .await;
        assert!(
            matches!(result, Err(ExecError::ProofStale)),
            "a stale proof refuses the approved write: {result:?}"
        );
        assert!(
            writer.writes.lock().unwrap().is_empty(),
            "no write performed when the proof is stale"
        );
    }

    #[tokio::test]
    async fn approver_executes_a_user_approved_fs_move() {
        // The confirm-gated agent path end to end: a `kind: agent` fs.move is
        // surfaced as RequireConfirmation, the human approves, and the approver
        // performs the collision-safe move via the external arm, returning a
        // NonGraph receipt (so a later [Undo] can move the file back).
        let dir = tempfile::tempdir().unwrap();
        let downloads = dir.path().join("Downloads");
        let projects = dir.path().join("Projects");
        std::fs::create_dir_all(&downloads).unwrap();
        std::fs::create_dir_all(&projects).unwrap();
        let src = downloads.join("paper.pdf");
        std::fs::write(&src, b"x").unwrap();
        let scope = vec![
            downloads.to_str().unwrap().to_string(),
            projects.to_str().unwrap().to_string(),
        ];

        let retained = RetainedProposal::capture(
            0,
            "tidy-downloads",
            &move_action(src.to_str().unwrap(), projects.to_str().unwrap()),
            ActionDecision::RequireConfirmation,
            &scope,
            "org.arlen.files",
            false,
            "run-mv",
            BaselineMode::Supervised,
        )
        .expect("retained");

        let approver = Approver::new(
            Arc::new(crate::slice::FsPathResolver),
            Arc::new(StaticMountPolicy::empty()),
            Arc::new(MockWriter::default()),
            Arc::new(MockAuditSink::accepting()),
            Arc::new(crate::fs_move::OsFileMover),
        );
        // The graph is unused on the external path.
        let receipt = approver
            .approve(&retained, &tag_graph(true), &executing_cap())
            .await
            .expect("approve does not error")
            .expect("an fs.move proposal resolves to a move");
        let ActionReceipt::NonGraph(written) = receipt else {
            panic!("an fs.move proposal yields a NonGraph receipt");
        };
        assert!(matches!(written.inverse(), InverseReceipt::RestorePath { .. }));
        let dst = projects.join("paper.pdf");
        assert!(dst.exists() && !src.exists(), "the approved move was performed");
    }

    #[tokio::test]
    async fn live_executor_refuses_when_the_audit_is_unavailable() {
        let cap = executing_cap();
        let writer = MockWriter::default();
        let audit = MockAuditSink::failing();
        let graph = tag_graph(false); // a valid proof, so only the audit blocks
        let (resolver, mounts) = (IdentityResolver, StaticMountPolicy::empty());
        let exec = LiveExecutor::new(&cap, &resolver, &mounts, &writer, &audit);
        let result = exec
            .execute(
                &tag_action(),
                ActionDecision::PreviewThenExecute,
                &auto_tag_scope(),
                &graph,
                "auto-tag-by-project",
                &ctx(),
                BaselineMode::Supervised,
            )
            .await;
        assert!(matches!(result, Err(ExecError::AuditUnavailable(_))));
        assert!(
            writer.writes.lock().unwrap().is_empty(),
            "no write may happen when the act cannot be audited"
        );
    }

    /// A writer that never returns, to exercise the write/retract timeout.
    struct HangingWriter;
    #[async_trait]
    impl RelationWriter for HangingWriter {
        async fn write_relation(&self, _write: &RelationWrite, _op_id: &str) -> Result<WriteOutcome, WriteError> {
            std::future::pending().await
        }
        async fn retract_relation(&self, _write: &RelationWrite, _op_id: &str) -> Result<RetractOutcome, WriteError> {
            std::future::pending().await
        }
    }

    #[tokio::test(start_paused = true)]
    async fn live_executor_times_out_then_reconciles_indeterminate_when_absent() {
        let cap = executing_cap();
        let writer = HangingWriter;
        let audit = MockAuditSink::accepting();
        // The edge is absent, so after the timeout neither the op_id edge nor any
        // edge is found: the write may yet commit, so it stays indeterminate (a
        // retry with the same op_id resolves it), never a false definite verdict.
        let graph = tag_graph(false);
        let (resolver, mounts) = (IdentityResolver, StaticMountPolicy::empty());
        let exec = LiveExecutor::new(&cap, &resolver, &mounts, &writer, &audit);
        // Paused time auto-advances past WRITE_TIMEOUT once the write is the only
        // thing pending, so this resolves immediately, not after a real 5s.
        let result = exec
            .execute(
                &tag_action(),
                ActionDecision::PreviewThenExecute,
                &auto_tag_scope(),
                &graph,
                "auto-tag-by-project",
                &ctx(),
                BaselineMode::Supervised,
            )
            .await;
        // The indeterminate error carries the pending key (not just a reason),
        // so a late commit is reconcilable: the op_id is the one this execution
        // derived, the relation is the one written, and the correlation id is
        // the decision's. Without this the late commit would be uncompensable.
        match result {
            Err(ExecError::WriteIndeterminate { pending, .. }) => {
                assert_eq!(pending.op_id(), derive_op_id("run-x", pending.write()));
                assert_eq!(pending.write().relation_type, "FILE_PART_OF");
                assert_eq!(pending.correlation_id(), "run-x");
            }
            other => panic!("expected WriteIndeterminate with a pending key, got {other:?}"),
        }
        // The act was still audited before the write was attempted.
        assert_eq!(audit.recorded().await.len(), 1);
    }

    /// A graph that reports the op_id edge present iff the queried op_id matches
    /// `mine`. `malformed` makes every query return a degraded shape (no `n`),
    /// to exercise fail-closed parsing.
    struct ReconcileGraph {
        mine: Option<String>,
        malformed: bool,
    }
    #[async_trait]
    impl GraphHandle for ReconcileGraph {
        async fn query(
            &self,
            cypher: &str,
        ) -> Result<Vec<HashMap<String, serde_json::Value>>, GraphError> {
            if self.malformed {
                // A row missing the `n` alias: fail-closed must treat it as None.
                return Ok(vec![[("wrong".to_string(), serde_json::json!(0))].into_iter().collect()]);
            }
            let n = i64::from(self.mine.as_deref().is_some_and(|m| cypher.contains(m)));
            Ok(vec![[("n".to_string(), serde_json::json!(n))].into_iter().collect()])
        }
    }

    fn reconcile_executor<'a>(
        cap: &'a Capability,
        resolver: &'a IdentityResolver,
        mounts: &'a StaticMountPolicy,
        writer: &'a MockWriter,
        audit: &'a MockAuditSink,
    ) -> LiveExecutor<'a> {
        LiveExecutor::new(cap, resolver, mounts, writer, audit)
    }

    #[tokio::test(start_paused = true)]
    async fn reconcile_resolves_by_op_id() {
        let (cap, resolver, mounts, writer, audit) = (
            executing_cap(),
            IdentityResolver,
            StaticMountPolicy::empty(),
            MockWriter::default(),
            MockAuditSink::accepting(),
        );
        let exec = reconcile_executor(&cap, &resolver, &mounts, &writer, &audit);
        let write = || RelationWrite {
            from_type: "system.File".into(),
            from_id: "f1".into(),
            to_type: "system.Project".into(),
            to_id: "p1".into(),
            relation_type: "FILE_PART_OF".into(),
        };

        // My op_id's edge exists -> this operation created it (causally proven,
        // the only sound positive verdict, valid even if the edge is later
        // deleted).
        let g = ReconcileGraph { mine: Some("op-mine".into()), malformed: false };
        let r = exec.reconcile(write(), "op-mine".to_string(), "run-x", &g).await;
        assert!(matches!(r, Ok(Some(ExecutedWrite { outcome: WriteOutcome::Created, .. }))));

        // My op_id absent -> indeterminate (not AlreadyExists: FILE_PART_OF is
        // deletable, so a bare edge cannot prove this write was a no-op, and the
        // write may yet commit; a retry with the same op_id resolves it).
        let g = ReconcileGraph { mine: Some("op-other".into()), malformed: false };
        let r = exec.reconcile(write(), "op-mine".to_string(), "run-x", &g).await;
        assert!(matches!(r, Err(ExecError::WriteIndeterminate { .. })));

        // A malformed read fails closed to indeterminate, never a false absence.
        let g = ReconcileGraph { mine: None, malformed: true };
        let r = exec.reconcile(write(), "op-mine".to_string(), "run-x", &g).await;
        assert!(matches!(r, Err(ExecError::WriteIndeterminate { .. })));
    }

    /// A graph whose op-id edge is ABSENT until the `present_from`-th read, then
    /// PRESENT — a write that commits a moment after the immediate read. The
    /// reconciler's polling must catch it instead of reporting indeterminate.
    struct LateCommitGraph {
        calls: std::sync::atomic::AtomicU32,
        present_from: u32,
    }
    #[async_trait]
    impl GraphHandle for LateCommitGraph {
        async fn query(
            &self,
            _cypher: &str,
        ) -> Result<Vec<HashMap<String, serde_json::Value>>, GraphError> {
            let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let present = i64::from(n + 1 >= self.present_from);
            Ok(vec![[("n".to_string(), serde_json::json!(present))].into_iter().collect()])
        }
    }

    #[tokio::test(start_paused = true)]
    async fn reconcile_catches_a_commit_that_lands_after_the_first_read() {
        let (cap, resolver, mounts, writer, audit) = (
            executing_cap(),
            IdentityResolver,
            StaticMountPolicy::empty(),
            MockWriter::default(),
            MockAuditSink::accepting(),
        );
        let exec = reconcile_executor(&cap, &resolver, &mounts, &writer, &audit);
        let write = RelationWrite {
            from_type: "system.File".into(),
            from_id: "f1".into(),
            to_type: "system.Project".into(),
            to_id: "p1".into(),
            relation_type: "FILE_PART_OF".into(),
        };
        // Absent on the first two reads, present on the third: within the poll
        // budget, so the reconciler confirms the late commit as Created rather
        // than giving up after the immediate read.
        let g = LateCommitGraph {
            calls: std::sync::atomic::AtomicU32::new(0),
            present_from: 3,
        };
        let r = exec.reconcile(write, "op-late".to_string(), "run-x", &g).await;
        assert!(
            matches!(r, Ok(Some(ExecutedWrite { outcome: WriteOutcome::Created, .. }))),
            "a commit landing within the poll budget is caught, got {r:?}"
        );
    }

    #[tokio::test]
    async fn live_executor_refuses_a_stale_proof() {
        let cap = executing_cap();
        let writer = MockWriter::default();
        let audit = MockAuditSink::accepting();
        // The edge already exists, so Not(EdgeExists) fails: the proof is stale.
        let graph = tag_graph(true);
        let (resolver, mounts) = (IdentityResolver, StaticMountPolicy::empty());
        let exec = LiveExecutor::new(&cap, &resolver, &mounts, &writer, &audit);
        let result = exec
            .execute(
                &tag_action(),
                ActionDecision::PreviewThenExecute,
                &auto_tag_scope(),
                &graph,
                "auto-tag-by-project",
                &ctx(),
                BaselineMode::Supervised,
            )
            .await;
        assert!(matches!(result, Err(ExecError::ProofStale)));
        assert!(
            writer.writes.lock().unwrap().is_empty(),
            "no write may happen on a stale proof"
        );
        // A refused proof is not audited as an execution (the act never happened).
        assert!(audit.recorded().await.is_empty());
    }

    #[tokio::test]
    async fn live_executor_skips_a_non_executing_decision() {
        let cap = executing_cap();
        let writer = MockWriter::default();
        let audit = MockAuditSink::accepting();
        let graph = tag_graph(false);
        let (resolver, mounts) = (IdentityResolver, StaticMountPolicy::empty());
        let exec = LiveExecutor::new(&cap, &resolver, &mounts, &writer, &audit);
        let written = exec
            .execute(
                &tag_action(),
                ActionDecision::RequireConfirmation,
                &auto_tag_scope(),
                &graph,
                "auto-tag-by-project",
                &ctx(),
                BaselineMode::Supervised,
            )
            .await
            .unwrap();
        assert_eq!(written, None);
        assert!(writer.writes.lock().unwrap().is_empty());
        assert!(audit.recorded().await.is_empty(), "nothing executed, nothing audited");
    }

    // ---- LiveExecutor: compensation (undo) ----

    fn file_part_of_write() -> RelationWrite {
        RelationWrite {
            from_type: "system.File".into(),
            from_id: "/proj/a.rs".into(),
            to_type: "system.Project".into(),
            to_id: "p1".into(),
            relation_type: "FILE_PART_OF".into(),
        }
    }

    /// An execution receipt as `execute` would build it: the write, the outcome,
    /// the op_id `execute` derived (from the same correlation id), and that id.
    fn receipt(outcome: WriteOutcome) -> ExecutedWrite {
        let write = file_part_of_write();
        let op_id = derive_op_id("run-x", &write);
        ExecutedWrite {
            write,
            outcome,
            op_id,
            correlation_id: "run-x".to_string(),
        }
    }

    #[tokio::test]
    async fn compensate_retracts_a_created_write_keyed_by_its_op_id() {
        let cap = executing_cap();
        let writer = MockWriter::default();
        let audit = MockAuditSink::accepting();
        let graph = tag_graph(true); // graph not read on the happy path
        let (resolver, mounts) = (IdentityResolver, StaticMountPolicy::empty());
        let exec = LiveExecutor::new(&cap, &resolver, &mounts, &writer, &audit);

        let executed = receipt(WriteOutcome::Created);
        let outcome = exec
            .compensate(&ActionReceipt::Graph(executed.clone()), &graph, "auto-tag-by-project")
            .await
            .unwrap();
        assert_eq!(outcome, CompensationOutcome::Retracted);

        // Exactly one retract, keyed by the SAME op id the write would carry, so
        // it undoes only this op's own edge.
        let retracts = writer.retracts.lock().unwrap();
        assert_eq!(retracts.len(), 1, "exactly one retract performed");
        assert_eq!(retracts[0].0.relation_type, "FILE_PART_OF");
        assert_eq!(retracts[0].1, derive_op_id("run-x", &file_part_of_write()));

        // The compensation is audited content-free, linked by correlation id.
        let entries = audit.recorded().await;
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].structural.outcome, "compensate");
        assert_eq!(entries[0].call_chain_id.as_deref(), Some("run-x"));
    }

    #[tokio::test]
    async fn compensator_retracts_a_created_write_over_owned_deps() {
        // The owned Compensator (the D-Bus undo path) runs the same compensation
        // as LiveExecutor::compensate, over Arc-owned deps and no capability.
        let writer: Arc<dyn RelationWriter> = Arc::new(MockWriter::default());
        let audit: Arc<dyn AuditSink> = Arc::new(MockAuditSink::accepting());
        let mover: Arc<dyn FileMover> = Arc::new(crate::fs_move::OsFileMover);
        let comp = Compensator::new(writer, audit, mover);
        let graph = tag_graph(true); // not read on the happy path
        let executed = receipt(WriteOutcome::Created);
        let outcome = comp
            .compensate(&ActionReceipt::Graph(executed), &graph, "auto-tag-by-project")
            .await
            .unwrap();
        assert_eq!(outcome, CompensationOutcome::Retracted);
    }

    #[tokio::test]
    async fn compensator_restores_a_moved_file_and_audits_the_undo() {
        // The full external loop's undo: a file moved to `now` is restored to
        // `prior` by replaying the captured RestorePath inverse, audited first.
        let dir = tempfile::tempdir().unwrap();
        let prior = dir.path().join("Downloads").join("p.pdf");
        let now = dir.path().join("Projects").join("p.pdf");
        std::fs::create_dir_all(prior.parent().unwrap()).unwrap();
        std::fs::create_dir_all(now.parent().unwrap()).unwrap();
        std::fs::write(&now, b"moved").unwrap(); // it currently lives at `now`

        let audit = Arc::new(MockAuditSink::accepting());
        let comp = Compensator::new(
            Arc::new(MockWriter::default()),
            audit.clone() as Arc<dyn AuditSink>,
            Arc::new(crate::fs_move::OsFileMover),
        );
        let receipt = ActionReceipt::NonGraph(ActionWrite::new(
            ResolvedExternalOp::new(EffectDomain::Filesystem, "move".into(), now.to_str().unwrap().into()),
            InverseReceipt::RestorePath {
                now: crate::effect_model::CanonicalPath::new(now.to_str().unwrap()).unwrap(),
                prior: crate::effect_model::CanonicalPath::new(prior.to_str().unwrap()).unwrap(),
            },
            "op-ng".into(),
            "run-ng".into(),
        ));
        let graph = tag_graph(true); // not read on the non-graph path
        let outcome = comp
            .compensate(&receipt, &graph, "tidy-downloads")
            .await
            .unwrap();
        assert_eq!(outcome, CompensationOutcome::Retracted);
        assert!(prior.exists() && !now.exists(), "the file was moved back");
        let entries = audit.recorded().await;
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].structural.outcome, "compensate");
        assert_eq!(entries[0].call_chain_id.as_deref(), Some("run-ng"));
    }

    #[tokio::test]
    async fn compensator_refuses_to_overwrite_when_the_origin_is_reoccupied() {
        // If the user put a file back at `prior`, the undo refuses rather than
        // clobbering it (the mover's non-overwrite claim): the undo is itself
        // non-destructive.
        let dir = tempfile::tempdir().unwrap();
        let prior = dir.path().join("p.pdf");
        let now = dir.path().join("moved.pdf");
        std::fs::write(&now, b"moved").unwrap();
        std::fs::write(&prior, b"user-replacement").unwrap(); // origin reoccupied

        let comp = Compensator::new(
            Arc::new(MockWriter::default()),
            Arc::new(MockAuditSink::accepting()),
            Arc::new(crate::fs_move::OsFileMover),
        );
        let receipt = ActionReceipt::NonGraph(ActionWrite::new(
            ResolvedExternalOp::new(EffectDomain::Filesystem, "move".into(), now.to_str().unwrap().into()),
            InverseReceipt::RestorePath {
                now: crate::effect_model::CanonicalPath::new(now.to_str().unwrap()).unwrap(),
                prior: crate::effect_model::CanonicalPath::new(prior.to_str().unwrap()).unwrap(),
            },
            "op-ng".into(),
            "run-ng".into(),
        ));
        let graph = tag_graph(true);
        let err = comp
            .compensate(&receipt, &graph, "tidy-downloads")
            .await
            .unwrap_err();
        assert!(matches!(err, ExecError::Write(_)));
        assert_eq!(std::fs::read_to_string(&prior).unwrap(), "user-replacement", "origin untouched");
        assert_eq!(std::fs::read_to_string(&now).unwrap(), "moved", "moved file still there");
    }

    #[tokio::test]
    async fn live_executor_refuses_a_non_graph_receipt_having_no_mover() {
        use crate::effect_model::{CanonicalPath, InverseReceipt};
        // The graph-focused LiveExecutor holds no file mover, so a NonGraph
        // receipt reaching it fails closed (it is a wiring bug, not a user undo;
        // the owned Compensator is the non-graph undo path).
        let cap = executing_cap();
        let writer = MockWriter::default();
        let audit = MockAuditSink::accepting();
        let graph = tag_graph(true);
        let (resolver, mounts) = (IdentityResolver, StaticMountPolicy::empty());
        let exec = LiveExecutor::new(&cap, &resolver, &mounts, &writer, &audit);

        let receipt = ActionReceipt::NonGraph(ActionWrite::new(
            ResolvedExternalOp::new(EffectDomain::Filesystem, "move".into(), "/a/x".into()),
            InverseReceipt::RestorePath {
                now: CanonicalPath::new("/b/x").unwrap(),
                prior: CanonicalPath::new("/a/x").unwrap(),
            },
            "op-ng".into(),
            "run-ng".into(),
        ));
        let result = exec.compensate(&receipt, &graph, "auto-tag-by-project").await;
        assert!(
            matches!(result, Err(ExecError::UnsupportedEffect(_))),
            "a non-graph receipt has no mover here, so it fails closed"
        );
        // Fail-closed before any I/O: no retract, no audit.
        assert!(writer.retracts.lock().unwrap().is_empty());
        assert!(audit.recorded().await.is_empty());
    }

    #[tokio::test]
    async fn compensate_uses_the_receipt_op_id_not_a_re_derivation() {
        let cap = executing_cap();
        let writer = MockWriter::default();
        let audit = MockAuditSink::accepting();
        let graph = tag_graph(true);
        let (resolver, mounts) = (IdentityResolver, StaticMountPolicy::empty());
        let exec = LiveExecutor::new(&cap, &resolver, &mounts, &writer, &audit);

        // A receipt whose stored op id is NOT what derive_op_id(correlation_id,
        // write) would produce. The retract must use the stored id verbatim, so a
        // mis-threaded or stale correlation id can never redirect the delete to a
        // different op's edge.
        let executed = ExecutedWrite {
            write: file_part_of_write(),
            outcome: WriteOutcome::Created,
            op_id: "receipt-op-id".to_string(),
            correlation_id: "decision-A".to_string(),
        };
        exec.compensate(&ActionReceipt::Graph(executed.clone()), &graph, "auto-tag-by-project").await.unwrap();

        let retracts = writer.retracts.lock().unwrap();
        assert_eq!(retracts[0].1, "receipt-op-id", "the retract is keyed by the receipt's op id");
        assert_ne!(
            "receipt-op-id",
            derive_op_id("decision-A", &file_part_of_write()),
            "and that id is not a re-derivation, so this proves it is receipt-keyed"
        );
        // The audit links to the receipt's own decision id.
        let entries = audit.recorded().await;
        assert_eq!(entries[0].call_chain_id.as_deref(), Some("decision-A"));
    }

    #[tokio::test]
    async fn compensate_skips_a_write_that_only_found_the_edge() {
        let cap = executing_cap();
        let writer = MockWriter::default();
        let audit = MockAuditSink::accepting();
        let graph = tag_graph(true);
        let (resolver, mounts) = (IdentityResolver, StaticMountPolicy::empty());
        let exec = LiveExecutor::new(&cap, &resolver, &mounts, &writer, &audit);

        // An AlreadyExists write never created the edge, so there is nothing this
        // action may undo: no retract, and no audit (nothing happened).
        let executed = receipt(WriteOutcome::AlreadyExists);
        let outcome = exec
            .compensate(&ActionReceipt::Graph(executed.clone()), &graph, "auto-tag-by-project")
            .await
            .unwrap();
        assert_eq!(outcome, CompensationOutcome::NothingToUndo);
        assert!(writer.retracts.lock().unwrap().is_empty(), "no edge it created, no retract");
        assert!(audit.recorded().await.is_empty(), "nothing undone, nothing audited");
    }

    #[tokio::test]
    async fn compensate_refuses_when_audit_unavailable() {
        let cap = executing_cap();
        let writer = MockWriter::default();
        let audit = MockAuditSink::failing();
        let graph = tag_graph(true);
        let (resolver, mounts) = (IdentityResolver, StaticMountPolicy::empty());
        let exec = LiveExecutor::new(&cap, &resolver, &mounts, &writer, &audit);

        let executed = receipt(WriteOutcome::Created);
        let result = exec
            .compensate(&ActionReceipt::Graph(executed.clone()), &graph, "auto-tag-by-project")
            .await;
        assert!(matches!(result, Err(ExecError::AuditUnavailable(_))));
        assert!(
            writer.retracts.lock().unwrap().is_empty(),
            "no retract may happen when the compensation cannot be audited"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn compensate_timeout_reconciles_to_retracted_when_edge_is_gone() {
        let cap = executing_cap();
        let writer = HangingWriter;
        let audit = MockAuditSink::accepting();
        // The op id edge is absent now, so the reconciliation confirms the
        // retract committed (only this op could have stamped that edge, so its
        // disappearance proves the delete).
        let graph = ReconcileGraph { mine: None, malformed: false };
        let (resolver, mounts) = (IdentityResolver, StaticMountPolicy::empty());
        let exec = LiveExecutor::new(&cap, &resolver, &mounts, &writer, &audit);

        let executed = receipt(WriteOutcome::Created);
        let outcome = exec
            .compensate(&ActionReceipt::Graph(executed.clone()), &graph, "auto-tag-by-project")
            .await
            .unwrap();
        assert_eq!(outcome, CompensationOutcome::Retracted);
        // The compensation was audited before the retract was attempted.
        assert_eq!(audit.recorded().await.len(), 1);
    }

    // --- the fs.move external executor arm (executor go-live) ---

    /// A mover that records each move without touching disk, for the fail-closed
    /// assertions where NO move must happen.
    #[derive(Default)]
    struct RecordingMover {
        moves: Mutex<Vec<(String, String)>>,
    }
    impl FileMover for RecordingMover {
        fn move_file(&self, from: &str, to: &str) -> std::io::Result<()> {
            self.moves.lock().unwrap().push((from.to_string(), to.to_string()));
            Ok(())
        }
    }

    fn move_action(source: &str, dest_dir: &str) -> ProposedAction {
        ProposedAction {
            tool: "fs.move".to_string(),
            summary: "tidy a download".to_string(),
            arguments: [("source", source), ("dest_dir", dest_dir)]
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }

    #[tokio::test]
    async fn execute_external_moves_audits_and_returns_the_inverse() {
        let dir = tempfile::tempdir().unwrap();
        let downloads = dir.path().join("Downloads");
        let projects = dir.path().join("Projects");
        std::fs::create_dir_all(&downloads).unwrap();
        std::fs::create_dir_all(&projects).unwrap();
        let src = downloads.join("paper.pdf");
        std::fs::write(&src, b"x").unwrap();

        let scope = vec![
            downloads.to_str().unwrap().to_string(),
            projects.to_str().unwrap().to_string(),
        ];
        let cap = executing_cap();
        let writer = MockWriter::default();
        let audit = MockAuditSink::accepting();
        let (resolver, mounts) = (IdentityResolver, StaticMountPolicy::empty());
        let exec = LiveExecutor::new(&cap, &resolver, &mounts, &writer, &audit);

        let receipt = exec
            .execute_external(
                &move_action(src.to_str().unwrap(), projects.to_str().unwrap()),
                ActionDecision::PreviewThenExecute,
                &scope,
                &crate::fs_move::OsFileMover,
                "tidy-downloads",
                &ctx(),
            )
            .await
            .unwrap()
            .expect("a proven move yields a receipt");

        let dst = projects.join("paper.pdf");
        assert!(dst.exists() && !src.exists(), "the file moved to the destination");
        match receipt.inverse() {
            InverseReceipt::RestorePath { now, prior } => {
                assert_eq!(now.as_str(), dst.to_str().unwrap());
                assert_eq!(prior.as_str(), src.to_str().unwrap());
            }
            other => panic!("expected RestorePath, got {other:?}"),
        }
        assert_eq!(audit.recorded().await.len(), 1, "audited once, before the move");
    }

    #[tokio::test]
    async fn execute_external_does_nothing_for_a_non_preview_decision() {
        let cap = executing_cap();
        let writer = MockWriter::default();
        let audit = MockAuditSink::accepting();
        let mover = RecordingMover::default();
        let (resolver, mounts) = (IdentityResolver, StaticMountPolicy::empty());
        let exec = LiveExecutor::new(&cap, &resolver, &mounts, &writer, &audit);
        let out = exec
            .execute_external(
                &move_action("/dl/x.pdf", "/proj"),
                ActionDecision::RequireConfirmation,
                &["/dl".to_string(), "/proj".to_string()],
                &mover,
                "tidy-downloads",
                &ctx(),
            )
            .await
            .unwrap();
        assert!(out.is_none());
        assert!(mover.moves.lock().unwrap().is_empty());
        assert!(audit.recorded().await.is_empty());
    }

    #[tokio::test]
    async fn execute_external_refuses_a_source_outside_the_declared_scope() {
        let cap = executing_cap();
        let writer = MockWriter::default();
        let audit = MockAuditSink::accepting();
        let mover = RecordingMover::default();
        let (resolver, mounts) = (IdentityResolver, StaticMountPolicy::empty());
        let exec = LiveExecutor::new(&cap, &resolver, &mounts, &writer, &audit);
        // Source under /etc, scope only grants /dl and /proj: refused, no move,
        // not even audited.
        let err = exec
            .execute_external(
                &move_action("/etc/shadow", "/proj"),
                ActionDecision::PreviewThenExecute,
                &["/dl".to_string(), "/proj".to_string()],
                &mover,
                "tidy-downloads",
                &ctx(),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, ExecError::ScopeViolation { .. }));
        assert!(mover.moves.lock().unwrap().is_empty());
        assert!(audit.recorded().await.is_empty());
    }

    #[tokio::test]
    async fn execute_external_refuses_a_destination_outside_the_declared_scope() {
        let cap = executing_cap();
        let writer = MockWriter::default();
        let audit = MockAuditSink::accepting();
        let mover = RecordingMover::default();
        let (resolver, mounts) = (IdentityResolver, StaticMountPolicy::empty());
        let exec = LiveExecutor::new(&cap, &resolver, &mounts, &writer, &audit);
        let err = exec
            .execute_external(
                &move_action("/dl/x.pdf", "/etc"),
                ActionDecision::PreviewThenExecute,
                &["/dl".to_string(), "/proj".to_string()],
                &mover,
                "tidy-downloads",
                &ctx(),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, ExecError::ScopeViolation { .. }));
        assert!(mover.moves.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn execute_external_refuses_an_empty_scope() {
        let cap = executing_cap();
        let writer = MockWriter::default();
        let audit = MockAuditSink::accepting();
        let mover = RecordingMover::default();
        let (resolver, mounts) = (IdentityResolver, StaticMountPolicy::empty());
        let exec = LiveExecutor::new(&cap, &resolver, &mounts, &writer, &audit);
        // A move tool with no declared dirs is refused (an unbounded move is never
        // granted by omission).
        let err = exec
            .execute_external(
                &move_action("/dl/x.pdf", "/dl"),
                ActionDecision::PreviewThenExecute,
                &[],
                &mover,
                "tidy-downloads",
                &ctx(),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, ExecError::ScopeViolation { .. }));
        assert!(mover.moves.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn execute_external_fails_closed_when_audit_is_down() {
        let dir = tempfile::tempdir().unwrap();
        let downloads = dir.path().join("Downloads");
        let projects = dir.path().join("Projects");
        std::fs::create_dir_all(&downloads).unwrap();
        std::fs::create_dir_all(&projects).unwrap();
        let src = downloads.join("p.pdf");
        std::fs::write(&src, b"x").unwrap();
        let scope = vec![
            downloads.to_str().unwrap().to_string(),
            projects.to_str().unwrap().to_string(),
        ];
        let cap = executing_cap();
        let writer = MockWriter::default();
        let audit = MockAuditSink::failing();
        let mover = RecordingMover::default();
        let (resolver, mounts) = (IdentityResolver, StaticMountPolicy::empty());
        let exec = LiveExecutor::new(&cap, &resolver, &mounts, &writer, &audit);
        let err = exec
            .execute_external(
                &move_action(src.to_str().unwrap(), projects.to_str().unwrap()),
                ActionDecision::PreviewThenExecute,
                &scope,
                &mover,
                "tidy-downloads",
                &ctx(),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, ExecError::AuditUnavailable(_)));
        assert!(mover.moves.lock().unwrap().is_empty(), "no move when the audit fails");
        assert!(src.exists(), "source untouched");
    }

    #[tokio::test]
    async fn execute_external_refuses_a_symlink_that_escapes_the_scope() {
        use std::os::unix::fs::symlink;
        // The real resolver canonicalizes, so a symlinked operand resolves to its
        // REAL target and the confinement compares real paths: a `dest_dir`
        // pointing (via a symlink) outside the declared scope is refused, with no
        // move and no audit. This is the regression test for the symlink escape.
        let dir = tempfile::tempdir().unwrap();
        let downloads = dir.path().join("Downloads");
        let projects = dir.path().join("Projects");
        let secret = dir.path().join("secret"); // outside the declared scope
        std::fs::create_dir_all(&downloads).unwrap();
        std::fs::create_dir_all(&projects).unwrap();
        std::fs::create_dir_all(&secret).unwrap();
        let src = downloads.join("paper.pdf");
        std::fs::write(&src, b"x").unwrap();
        // `Projects/work` is a symlink OUT of the scope, to `secret`.
        let escape = projects.join("work");
        symlink(&secret, &escape).unwrap();

        let scope = vec![
            downloads.to_str().unwrap().to_string(),
            projects.to_str().unwrap().to_string(),
        ];
        let cap = executing_cap();
        let writer = MockWriter::default();
        let audit = MockAuditSink::accepting();
        let mover = RecordingMover::default();
        // The REAL resolver (canonicalizes), not the identity one.
        let resolver = crate::slice::FsPathResolver;
        let mounts = StaticMountPolicy::empty();
        let exec = LiveExecutor::new(&cap, &resolver, &mounts, &writer, &audit);

        let err = exec
            .execute_external(
                &move_action(src.to_str().unwrap(), escape.to_str().unwrap()),
                ActionDecision::PreviewThenExecute,
                &scope,
                &mover,
                "tidy-downloads",
                &ctx(),
            )
            .await
            .unwrap_err();
        assert!(
            matches!(err, ExecError::ScopeViolation { .. }),
            "a symlink escaping the scope must be refused, got {err:?}"
        );
        assert!(mover.moves.lock().unwrap().is_empty(), "no move on a symlink escape");
        assert!(audit.recorded().await.is_empty(), "not even audited");
        assert!(src.exists(), "source untouched");
        assert!(
            std::fs::read_dir(&secret).unwrap().next().is_none(),
            "nothing was written into the symlink target"
        );
    }
}

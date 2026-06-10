//! The trusted given-rule registry: the only source of action schemas the
//! predict-before-act path may prove anything about.
//!
//! A raw [`ActionSchema`] is forgeable, its `action` and `provenance` are just
//! fields, so the world-model interpreter must never be driven from one that
//! came from a behaviour or the model. This module holds the built-in
//! given rules and hands them out wrapped in a [`TrustedActionSchema`] whose
//! only constructor is private here. Code elsewhere can obtain one solely
//! through [`lookup`], keyed by the invoked tool/action id, so the type itself
//! is the proof that a schema was registry-resolved.
//!
//! Only `Provenance::Given` rules live here. Learned rules are induced,
//! approved, and admitted through a separate (later) path; `lookup` never
//! returns one.
//!
//! Until the gate path calls [`lookup`], it has no non-test caller, so the
//! module allows dead code; the allowance goes away once the gate resolves a
//! schema here.
#![allow(dead_code)]

use crate::effect_model::InverseClass;
use crate::world::{compensation_of, ActionSchema, Effect, Predicate, Provenance};

/// What a decided action would do and how to undo it, surfaced so the agent's
/// audited proposals are visible (logged today; the activity view and executor
/// later). The effects and their compensation are in the schema's bind-name
/// vocabulary, not resolved ids, so they are content-free (the operands live in
/// the action's arguments). `compensation` is `None` for an irreversible action
/// (no derivable inverse).
///
/// Deliberately no idempotency / dedup key here: an executor's at-least-once
/// dedup key needs decision identity (a crash replay of one decision matches,
/// but a genuinely new decision with the same operands does not) and a
/// collision-resistant, version-stable digest over a canonical encoding. Those
/// are executor-design decisions that have to be made against the real durable
/// replay path, so the key is built with the executor, not minted speculatively
/// on this public contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionPlan {
    /// The effects the action would apply.
    pub effects: Vec<Effect>,
    /// The compensation that undoes them, or `None` if irreversible.
    pub compensation: Option<Vec<Effect>>,
}

/// Build the [`ExecutionPlan`] for an invoked action: its registry-resolved
/// effects and their compensation (if reversible). `None` for an unregistered
/// action (the gate cannot prove or plan one). Depends only on the action id:
/// the plan describes what the action's schema does, independent of the
/// operands.
pub(crate) fn plan_for(tool: &str) -> Option<ExecutionPlan> {
    let trusted = lookup(tool)?;
    let effects = trusted.schema().effects.clone();
    let compensation = compensation_of(&effects);
    Some(ExecutionPlan {
        effects,
        compensation,
    })
}

/// An [`ActionSchema`] the registry vouches for. Its single field is private
/// and it has no public constructor, so it can only be produced by [`lookup`]
/// in this module, never forged from untrusted input elsewhere in the crate.
pub(crate) struct TrustedActionSchema {
    schema: ActionSchema,
}

impl TrustedActionSchema {
    /// The wrapped schema, for the slice builder and interpreter to read. The
    /// accessor is crate-internal so reading the schema never lets other code
    /// reconstruct the trust token from it.
    pub(crate) fn schema(&self) -> &ActionSchema {
        &self.schema
    }

    /// Whether this action is reversible (Foundation B1): its effect sequence
    /// has a derivable compensation in the same world-model DSL. This grounds
    /// the gate's "reversible" predicate, which was previously assumed: an
    /// action with no compensation is irreversible, so it is high-impact and
    /// must always be confirmed (never lifted to autonomous preview). Reversible
    /// is defined conservatively (see [`compensation_of`]): an effect that needs
    /// prior state to undo (a field set, a node removal) is not auto-invertible,
    /// so a schema containing one is irreversible unless it later declares an
    /// explicit compensation.
    pub(crate) fn is_reversible(&self) -> bool {
        compensation_of(&self.schema.effects).is_some()
    }

    /// The declared reversibility class of the schema's non-graph effect, if any
    /// (reversible-receipts-and-the-effect-model.md §3.2). A pure-graph schema has
    /// none (its reversibility is the op-id self-inverse, read via
    /// [`is_reversible`](Self::is_reversible)); a non-graph schema carries it on
    /// its `Effect::External`, and `resolved_action_kind` maps the three-way class
    /// to the audit kind.
    pub(crate) fn declared_inverse_class(&self) -> Option<InverseClass> {
        self.schema.effects.iter().find_map(|e| match e {
            Effect::External { class, .. } => Some(*class),
            _ => None,
        })
    }
}

/// The production given-rule table: the single source of truth for which action
/// ids `lookup` resolves to a `Given` schema, each paired with its schema
/// builder. `lookup` dispatches every production `Given` rule through this table
/// and [`given_actions`] derives from it, so the set the CY-R4 canary-zero-FP
/// scan and the EM-R9 reversibility-proof CI iterate is exactly the set `lookup`
/// can resolve as `Given` (no silent under-cover when a rule is added: a new
/// production rule cannot resolve without a table entry, which the invariants
/// then cover by construction). The honeytool (never provable) and a
/// `#[cfg(test)]` fixture are deliberately NOT here; a future approved-`Learned`
/// admission path must feed the same derived set so the invariants keep covering
/// the whole acceptance domain, not only `Given`.
const PRODUCTION_GIVEN_RULES: &[(&str, GivenRuleBuilder)] =
    &[("graph.write", graph_write_link_schema)];

/// Builds a registered given-rule schema (one table entry's value).
type GivenRuleBuilder = fn() -> ActionSchema;

/// The canonical production given-rule action ids, derived from
/// [`PRODUCTION_GIVEN_RULES`] so they can never drift from what `lookup`
/// resolves. The CI invariants iterate this, not a hand-kept constant.
pub(crate) fn given_actions() -> impl Iterator<Item = &'static str> {
    PRODUCTION_GIVEN_RULES.iter().map(|(id, _)| *id)
}

/// The honeytool action id (canary-honeytools.md §2): an attractive bait
/// capability shaped like what an injection reaches for. Honest behaviour
/// proposes only `graph.write` / `graph.query` (by exhaustion of the behaviour
/// set), so a proposal of this id is deterministic proof of hijack. It is NOT a
/// [`PRODUCTION_GIVEN_RULES`] entry (it is never a provable rule); it is
/// registered only so [`is_honeytool`] has a single source, and a behaviour must list it in its
/// declared `tools` scope for the model to see it as available (a deployment
/// step, not built here).
const HONEYTOOL_ACTION: &str = "export_all_secrets";

/// Resolve the given-rule schema for an invoked action/tool id, or `None` if
/// no rule is registered. With no rule the predict-before-act path cannot
/// prove the action, so the gate keeps its conservative cap rather than lift
/// it.
pub(crate) fn lookup(action_id: &str) -> Option<TrustedActionSchema> {
    // Every production `Given` rule resolves through the table, so the canary /
    // reversibility CI (which iterates the same table) cannot under-cover.
    if let Some((_, build)) = PRODUCTION_GIVEN_RULES
        .iter()
        .find(|(id, _)| *id == action_id)
    {
        return Some(TrustedActionSchema { schema: build() });
    }
    let schema = match action_id {
        // The honeytool: registered so tool identity has a single source, but
        // tagged `Honeytool` so it is never provable (predict refuses it) and the
        // engine freezes on `is_honeytool` before the gate. Empty effects.
        a if a == HONEYTOOL_ACTION => ActionSchema {
            action: HONEYTOOL_ACTION.to_string(),
            preconditions: vec![],
            effects: vec![],
            provenance: Provenance::Honeytool,
        },
        // A registered but irreversible action, for the gate's
        // irreversible-always-confirms tests (a `SetField` cannot be inverted
        // from itself, so the schema has no derivable compensation).
        #[cfg(test)]
        "test.irreversible" => ActionSchema {
            action: "test.irreversible".to_string(),
            preconditions: vec![],
            effects: vec![Effect::SetField {
                bind: "x".to_string(),
                field: "f".to_string(),
                value: "v".to_string(),
            }],
            provenance: Provenance::Given,
        },
        _ => return None,
    };
    Some(TrustedActionSchema { schema })
}

/// Whether `tool` is a honeytool (canary-honeytools.md §2): a registered bait
/// action tagged [`Provenance::Honeytool`]. A pure name predicate, consulted at
/// the engine's `Propose` arm before the read branch and before the gate (the
/// ordering is mandatory: a honeytool named `graph.query` would otherwise execute
/// as a gate-exempt read, and any other name would hit the low-signal
/// `ToolOutOfScope`). A hit is a deterministic, high-signal hijack trip. Honest
/// runs never name it (a tool is a capability shown to the model, never echoed
/// from graph content), so zero false positives.
pub(crate) fn is_honeytool(tool: &str) -> bool {
    lookup(tool).is_some_and(|t| matches!(t.schema().provenance, Provenance::Honeytool))
}

/// Whether a schema's reversibility is *structurally proven* by its preconditions
/// (reversible-receipts-and-the-effect-model.md EM-R9). A rule classified
/// reversible must carry the precondition that makes its inverse exact; validating
/// it here at registry level catches a malformed reversible rule before it ships,
/// the layer above predict's per-invocation refusal. Today the one auto-reversible
/// effect is `AssertEdge`: its op-id self-inverse `RetractEdge` is exact only if
/// the edge was absent before, so each `AssertEdge` must be guarded by a matching
/// `Not(EdgeExists)`; without it the assert could be a no-op on a pre-existing edge
/// whose inverse retract would then wrongly remove it. A non-reversible schema
/// carries no obligation (vacuously true). When a new reversible effect type gains
/// a rule, its proof obligation is added here.
pub(crate) fn reversibility_proof_holds(schema: &ActionSchema) -> bool {
    if compensation_of(&schema.effects).is_none() {
        return true;
    }
    schema.effects.iter().all(|effect| match effect {
        Effect::AssertEdge { from, edge, to } => schema
            .preconditions
            .iter()
            .any(|p| is_edge_absence_proof(p, from, edge, to)),
        _ => true,
    })
}

/// Whether `p` is exactly `Not(EdgeExists { from, edge, to })`, the absence proof
/// that makes an `AssertEdge` a strict create (and so its retract inverse exact).
fn is_edge_absence_proof(p: &Predicate, from: &str, edge: &str, to: &str) -> bool {
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
}

/// The given rule for linking a file to the project it belongs to
/// (`FILE_PART_OF`). It proves the real invariant before the link may be
/// asserted: both nodes exist, the file's path lies under the project's root
/// (so an unrelated file/project pair cannot be linked), and the edge is not
/// already present. It creates a single edge, no node, so the bounded slice
/// can represent its full effect.
fn graph_write_link_schema() -> ActionSchema {
    ActionSchema {
        action: "graph.write".to_string(),
        preconditions: vec![
            Predicate::NodeExists {
                bind: "file".to_string(),
                label: "File".to_string(),
            },
            Predicate::NodeExists {
                bind: "project".to_string(),
                label: "Project".to_string(),
            },
            // The file must actually belong to the project: its path lies
            // under the project root. Without this the rule would prove only
            // that two unrelated nodes exist and authorise a corrupt link.
            Predicate::PathUnderField {
                inner: "file".to_string(),
                inner_field: "path".to_string(),
                outer: "project".to_string(),
                outer_field: "root_path".to_string(),
            },
            Predicate::Not(Box::new(Predicate::EdgeExists {
                from: "file".to_string(),
                edge: "FILE_PART_OF".to_string(),
                to: "project".to_string(),
            })),
        ],
        effects: vec![Effect::AssertEdge {
            from: "file".to_string(),
            edge: "FILE_PART_OF".to_string(),
            to: "project".to_string(),
        }],
        provenance: Provenance::Given,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_a_given_rule_for_a_known_action() {
        let trusted = lookup("graph.write").expect("graph.write is registered");
        assert_eq!(trusted.schema().action, "graph.write");
        // The registry only ever vouches for given rules.
        assert!(matches!(trusted.schema().provenance, Provenance::Given));
    }

    #[test]
    fn an_unknown_action_has_no_rule() {
        assert!(lookup("fs.delete").is_none());
        assert!(lookup("").is_none());
    }

    #[test]
    fn the_honeytool_is_registered_recognised_and_not_a_given_rule() {
        // is_honeytool fires for the bait and nothing else.
        assert!(is_honeytool(HONEYTOOL_ACTION));
        assert!(!is_honeytool("graph.write"));
        assert!(!is_honeytool("unknown"));
        // It is registered with empty effects and Honeytool provenance.
        let t = lookup(HONEYTOOL_ACTION).expect("the honeytool resolves");
        assert!(t.schema().effects.is_empty());
        assert!(matches!(t.schema().provenance, Provenance::Honeytool));
        // It is NOT a given rule, so the canary zero-FP invariant never scans it
        // and the proof path never treats it as provable.
        assert!(!given_actions().any(|a| a == HONEYTOOL_ACTION));
    }

    #[test]
    fn every_given_rule_proves_its_reversibility() {
        // EM-R9: a registry rule classified reversible must structurally carry the
        // precondition that makes its inverse exact. Validated over the canonical
        // given set (extend with approved-Learned when that path lands), so a
        // malformed reversible rule fails CI before it ships.
        for action in given_actions() {
            let trusted = lookup(action).expect("a listed given action resolves");
            assert!(
                reversibility_proof_holds(trusted.schema()),
                "given rule {action} is reversible but lacks its reversibility-proof precondition"
            );
        }
    }

    #[test]
    fn reversibility_proof_requires_edge_absence() {
        // A reversible AssertEdge without its Not(EdgeExists) proof fails; with it
        // passes; a non-reversible schema is vacuously fine.
        let edge = |from: &str, to: &str| Effect::AssertEdge {
            from: from.into(),
            edge: "FILE_PART_OF".into(),
            to: to.into(),
        };
        let absent = Predicate::Not(Box::new(Predicate::EdgeExists {
            from: "file".into(),
            edge: "FILE_PART_OF".into(),
            to: "proj".into(),
        }));
        let with_proof = ActionSchema {
            action: "graph.write".into(),
            preconditions: vec![absent],
            effects: vec![edge("file", "proj")],
            provenance: Provenance::Given,
        };
        assert!(reversibility_proof_holds(&with_proof));

        let without_proof = ActionSchema {
            action: "graph.write".into(),
            preconditions: vec![],
            effects: vec![edge("file", "proj")],
            provenance: Provenance::Given,
        };
        assert!(!reversibility_proof_holds(&without_proof));

        let irreversible = ActionSchema {
            action: "x".into(),
            preconditions: vec![],
            effects: vec![Effect::SetField {
                bind: "n".into(),
                field: "f".into(),
                value: "v".into(),
            }],
            provenance: Provenance::Given,
        };
        assert!(reversibility_proof_holds(&irreversible));
    }

    #[test]
    fn every_listed_given_action_resolves() {
        // `given_actions` derives from `PRODUCTION_GIVEN_RULES`, the same table
        // `lookup` dispatches production given rules through, so the resolved set
        // and the scanned set cannot drift: a new production rule resolves only by
        // a table entry, which this (and the canary / reversibility invariants)
        // then cover. Confirm each derived id resolves to its own given schema.
        for action in given_actions() {
            let trusted = lookup(action).unwrap_or_else(|| panic!("{action} must resolve"));
            assert_eq!(trusted.schema().action.as_str(), action);
            assert!(matches!(trusted.schema().provenance, Provenance::Given));
        }
    }

    #[test]
    fn no_given_schema_mentions_a_canary_id() {
        // CY-R4 (canary-honeytools.md §3): the structural canary is zero-false-
        // positive only if no trusted schema's acceptance domain can itself name a
        // reserved canary id, so an honest proposal proven against a Given rule can
        // never bind or bake one. Scan the whole schema (action id, bind names,
        // labels, fields, literal values) via its Debug form: the derive includes
        // every field and the reserved token has no escape characters, so a
        // substring search is a complete check. A future approved-Learned schema
        // must be added to the derived given set so it is covered too.
        for action in given_actions() {
            let trusted = lookup(action).expect("a listed given action resolves");
            let dump = format!("{:?}", trusted.schema());
            assert!(
                !dump.contains(crate::canary::RESERVED_CANARY_PREFIX),
                "given schema {action} must not mention a reserved canary id"
            );
        }
    }

    #[test]
    fn plan_for_carries_effects_and_compensation() {
        let plan = plan_for("graph.write").expect("graph.write is registered");
        assert_eq!(
            plan.effects,
            vec![Effect::AssertEdge {
                from: "file".to_string(),
                edge: "FILE_PART_OF".to_string(),
                to: "project".to_string(),
            }]
        );
        // Reversible: the compensation retracts what the effect asserts.
        assert_eq!(
            plan.compensation,
            Some(vec![Effect::RetractEdge {
                from: "file".to_string(),
                edge: "FILE_PART_OF".to_string(),
                to: "project".to_string(),
            }])
        );
        // An unregistered action has no plan.
        assert!(plan_for("fs.delete").is_none());
    }

    #[test]
    fn reversibility_is_grounded_in_derivable_compensation() {
        // The built-in link rule asserts one edge, which inverts cleanly, so it
        // is reversible (and may be lifted to autonomous preview when proven).
        assert!(lookup("graph.write").unwrap().is_reversible());
        // A schema whose effect cannot be inverted from itself alone (a field
        // set needs the prior value) is irreversible: the gate must always
        // confirm it.
        let irreversible = TrustedActionSchema {
            schema: ActionSchema {
                action: "x".to_string(),
                preconditions: vec![],
                effects: vec![Effect::SetField {
                    bind: "a".to_string(),
                    field: "f".to_string(),
                    value: "v".to_string(),
                }],
                provenance: Provenance::Given,
            },
        };
        assert!(!irreversible.is_reversible());
    }

    #[test]
    fn the_built_in_rule_creates_no_node() {
        // Node-level mutations are refused by the slice builder, so a given
        // rule must not contain one or it could never be sliced.
        let trusted = lookup("graph.write").unwrap();
        assert!(!trusted
            .schema()
            .effects
            .iter()
            .any(|e| matches!(e, Effect::AssertNode { .. } | Effect::RetractNode { .. })));
    }
}

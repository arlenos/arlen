//! The `ai_working_set` shape-only introspection surface (ai-transparency-surface.md AIT-R1).
//!
//! Answers "what does the AI know right now?" with the **shape** of the agent's
//! reach, never its content. The transparency surface (Gap 3, "we are not
//! Recall") must let a user inspect what the AI can hold without itself
//! re-surfacing the user's data, so this returns only configuration-derived
//! shape: the live loop status and, per **enabled** behaviour, its name, kind
//! and declared read scope (the global KG read tier it needs). It never returns
//! a node, a field or any slice content.
//!
//! Scope of the honest "now-available" shape (AIT-R1 v1): the agent does not
//! hold a persistent, introspectable KG slice - it builds an ephemeral slice
//! per gate decision inside a bounded loop and discards it, so there is no
//! durable per-entity node count to report truthfully. What is always available
//! is the **configured reach** (which behaviours are enabled and the read tier
//! each declares) plus the **live status**, which is exactly this. A live
//! held-slice node-count deepening needs an engine ingestion hook (the same
//! shape as the deferred Reads feed, which must never show a false "nothing
//! read"); it lands with that hook.
//!
//! When the master `[ai] enabled` switch is off, nothing is enabled, so the
//! behaviour list is empty - the honest "the AI is off; it can hold nothing".
//!
//! The derivation ([`working_set_shape`]) is pure over a [`LoadOutcome`], so it
//! is unit-tested without a daemon or a bus.

use serde::Serialize;

use crate::behaviour::{BehaviourKind, ReadScope};
use crate::loader::LoadOutcome;

/// The shape of one enabled behaviour: identity + the reach it declares.
/// Configuration, not user data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BehaviourShape {
    /// The behaviour's manifest name.
    pub name: String,
    /// `"workflow"` or `"agent"`.
    pub kind: String,
    /// The declared global KG read tier
    /// (`"minimal"`|`"session"`|`"project"`|`"time"`|`"full"`).
    pub read_scope: String,
}

/// The agent's current working-set shape: the live loop status plus the
/// configured reach of every enabled behaviour. Shape only, never content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorkingSetShape {
    /// The live loop status (`"subscribing"`|`"idle"`|`"busy"`).
    pub status: String,
    /// One entry per enabled behaviour; empty when the master switch is off.
    pub behaviours: Vec<BehaviourShape>,
}

/// The wire string for a behaviour kind.
fn kind_str(kind: BehaviourKind) -> &'static str {
    match kind {
        BehaviourKind::Workflow => "workflow",
        BehaviourKind::Agent => "agent",
    }
}

/// The manifest-vocabulary name of a read scope.
fn read_scope_str(scope: ReadScope) -> &'static str {
    match scope {
        ReadScope::Minimal => "minimal",
        ReadScope::Session => "session",
        ReadScope::Project => "project",
        ReadScope::Time => "time",
        ReadScope::Full => "full",
    }
}

/// Derive the working-set shape from the live loop status and the loaded
/// behaviour set. Only **enabled** behaviours contribute (a loaded-but-disabled
/// behaviour is not part of the agent's reach), so the master switch being off
/// yields an empty list. Pure.
pub fn working_set_shape(status: &str, outcome: &LoadOutcome) -> WorkingSetShape {
    let behaviours = outcome
        .loaded
        .iter()
        .filter(|lb| lb.status.is_enabled())
        .map(|lb| {
            let m = &lb.behaviour.manifest;
            BehaviourShape {
                name: m.name.clone(),
                kind: kind_str(m.kind).to_string(),
                read_scope: read_scope_str(m.reads).to_string(),
            }
        })
        .collect();
    WorkingSetShape {
        status: status.to_string(),
        behaviours,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::behaviour::parse;
    use crate::loader::{DisableReason, LoadedBehaviour, Provenance, Status};
    use std::path::PathBuf;

    /// A minimal valid workflow SKILL.md with the given name and read scope.
    fn workflow_md(name: &str, reads: &str) -> String {
        format!(
            "---\nname: {name}\ndescription: test\nkind: workflow\nreads: {reads}\n\
             trigger:\n  type: event\n  event: file.opened\nhandler: noop\n---\nbody\n"
        )
    }

    /// A minimal valid agent SKILL.md with the given name and read scope.
    fn agent_md(name: &str, reads: &str) -> String {
        format!(
            "---\nname: {name}\ndescription: test\nkind: agent\nreads: {reads}\n\
             trigger:\n  type: event\n  event: file.opened\n\
             budget:\n  max_steps: 4\n  max_tokens: 1000\n  max_wall_ms: 5000\n\
             terminal:\n  done: silent\n---\nbody\n"
        )
    }

    fn loaded(md: &str, status: Status) -> LoadedBehaviour {
        LoadedBehaviour {
            behaviour: parse(md).expect("fixture parses"),
            provenance: Provenance::BuiltIn,
            dir: PathBuf::from("/test"),
            status,
        }
    }

    #[test]
    fn only_enabled_behaviours_appear_in_the_shape() {
        let outcome = LoadOutcome {
            loaded: vec![
                loaded(&workflow_md("auto-tag", "project"), Status::Enabled),
                loaded(
                    &agent_md("meeting-prep", "full"),
                    Status::Disabled(DisableReason::NotEnabledInSettings),
                ),
            ],
            errors: vec![],
        };
        let shape = working_set_shape("idle", &outcome);
        assert_eq!(shape.status, "idle");
        assert_eq!(shape.behaviours.len(), 1);
        assert_eq!(shape.behaviours[0].name, "auto-tag");
        assert_eq!(shape.behaviours[0].kind, "workflow");
        assert_eq!(shape.behaviours[0].read_scope, "project");
    }

    #[test]
    fn master_off_yields_an_empty_shape() {
        // Nothing loaded-enabled: the honest "the AI is off; it holds nothing".
        let outcome = LoadOutcome::default();
        let shape = working_set_shape("subscribing", &outcome);
        assert!(shape.behaviours.is_empty());
        assert_eq!(shape.status, "subscribing");
    }

    #[test]
    fn shape_serialises_to_shape_only_json() {
        let outcome = LoadOutcome {
            loaded: vec![loaded(&agent_md("notes", "session"), Status::Enabled)],
            errors: vec![],
        };
        let json = serde_json::to_string(&working_set_shape("busy", &outcome)).unwrap();
        assert!(json.contains("\"status\":\"busy\""));
        assert!(json.contains("\"name\":\"notes\""));
        assert!(json.contains("\"kind\":\"agent\""));
        assert!(json.contains("\"read_scope\":\"session\""));
    }
}

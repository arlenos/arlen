//! Behaviour status for the agent observability view (harness A6).
//!
//! Loads the same behaviour set the agent daemon would act on, through the
//! shared `arlen_ai_agent::loader::load_configured`, and reports each
//! behaviour's enablement and trust. Read-only: this never enables, disables,
//! or runs anything; it shows what the daemon resolved from the trusted
//! config and the behaviour directories.

use arlen_ai_agent::behaviour::BehaviourKind;
use arlen_ai_agent::loader::{load_configured, DisableReason, Provenance, Status};
use serde::Serialize;

/// One behaviour's status as the observability view renders it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BehaviourStatus {
    /// Manifest name.
    name: String,
    /// Manifest one-line description.
    description: String,
    /// `agent` (a bounded LLM loop) or `workflow` (a deterministic handler).
    kind: String,
    /// Where it was loaded from, which sets its trust tier.
    provenance: String,
    /// Whether the trusted config has it enabled for dispatch.
    enabled: bool,
    /// Why it is disabled, when it is.
    disabled_reason: Option<String>,
    /// The declared read scope, for display.
    reads: String,
}

fn kind_label(kind: &BehaviourKind) -> &'static str {
    match kind {
        BehaviourKind::Agent => "agent",
        BehaviourKind::Workflow => "workflow",
    }
}

fn provenance_label(provenance: &Provenance) -> &'static str {
    match provenance {
        Provenance::BuiltIn => "built-in",
        Provenance::User => "user",
        Provenance::ThirdParty => "third-party",
    }
}

fn disabled_reason_label(reason: &DisableReason) -> &'static str {
    match reason {
        DisableReason::NotEnabledInSettings => "not enabled in settings",
        DisableReason::DuplicateName => "duplicate name, disabled fail-closed",
    }
}

/// List every discoverable agent behaviour with its enablement and trust,
/// exactly as the agent daemon resolves them. Read-only.
#[tauri::command]
pub fn ai_behaviours() -> Vec<BehaviourStatus> {
    load_configured()
        .loaded
        .iter()
        .map(|b| {
            let (enabled, disabled_reason) = match &b.status {
                Status::Enabled => (true, None),
                Status::Disabled(reason) => {
                    (false, Some(disabled_reason_label(reason).to_string()))
                }
            };
            BehaviourStatus {
                name: b.behaviour.manifest.name.clone(),
                description: b.behaviour.manifest.description.clone(),
                kind: kind_label(&b.behaviour.manifest.kind).to_string(),
                provenance: provenance_label(&b.provenance).to_string(),
                enabled,
                disabled_reason,
                reads: format!("{:?}", b.behaviour.manifest.reads),
            }
        })
        .collect()
}

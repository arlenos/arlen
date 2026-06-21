//! The Arlen consent broker: the typed request model + severity classification
//! for the one trusted-path consent surface every system prompt routes through
//! (system-dialog-plan.md, ordering #9).
//!
//! This is the FOUNDATION slice (piece 1): the typed [`ConsentRequest`], the
//! [`SeverityTier`] derived by REUSING the existing
//! [`arlen_ai_core::capability::Capability::decide`] (not a reinvented tier
//! scheme), and the load-bearing identity invariant ([`AttestedRequester`]: the
//! requester shown in the dialog == the attested app id == the grant recipient,
//! one value end to end - the macOS TCC CVE-2025-31250 lesson). The trusted
//! Wayland surface, the dialog UI, the grant-mint into the KG/audit, the
//! multi-request queue, and the migration of the existing prompts onto this
//! broker are the later pieces of the strand.

#![warn(missing_docs)]

use arlen_ai_core::capability::{ActionDecision, ActionKind, Capability};
use serde::{Deserialize, Serialize};

pub mod grant;
pub mod queue;

pub use grant::{mint_grant, ConsentGrant};
pub use queue::{ConsentQueue, Enqueued, PendingRequest, RequestId};

/// The class of system request seeking consent. The broker is the ONE surface
/// for all of these (system-dialog-plan.md): they share the trusted path, the
/// severity classification and the grant store, differing only in the rendered
/// dialog and the copy. The class never overrides the severity (that is
/// [`classify`]'s job); it selects which polymorphic dialog renders.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConsentClass {
    /// A capability grant (graph / event-bus / filesystem / ... scope).
    CapabilityGrant,
    /// Access to the user's app data or files.
    AppData,
    /// Installing or removing a package / app.
    Install,
    /// A destructive action (permanent delete, irrecoverable overwrite).
    Destructive,
    /// Sending a message or data to an external recipient.
    ExternalSend,
    /// Network access to a host the app did not declare.
    NetworkAccess,
    /// Running a confined foreign program (Wine / `exec`).
    ExecConfined,
    /// An action requiring elevated privilege (polkit / sudo).
    ElevatedPrivilege,
    /// An xdg desktop-portal access request routed to this backend.
    Portal,
    /// A notification action surfaced as an explicit decision.
    NotificationAction,
    /// An AI-agent action awaiting confirmation.
    AgentAction,
}

impl ConsentClass {
    /// A stable lowercase key for this class, used in logs, the wire form and
    /// the deterministic revocation handle. Stable across releases (do not
    /// rename) so a persisted grant's handle keeps matching.
    pub fn as_key(self) -> &'static str {
        match self {
            ConsentClass::CapabilityGrant => "capability_grant",
            ConsentClass::AppData => "app_data",
            ConsentClass::Install => "install",
            ConsentClass::Destructive => "destructive",
            ConsentClass::ExternalSend => "external_send",
            ConsentClass::NetworkAccess => "network_access",
            ConsentClass::ExecConfined => "exec_confined",
            ConsentClass::ElevatedPrivilege => "elevated_privilege",
            ConsentClass::Portal => "portal",
            ConsentClass::NotificationAction => "notification_action",
            ConsentClass::AgentAction => "agent_action",
        }
    }
}

/// The attested identity of the requester: the SINGLE value that is BOTH shown
/// in the dialog AND recorded as the grant recipient.
///
/// Constructed only from the kernel-attested app id (SO_PEERCRED ->
/// `path_to_app_id`), never from a requester-supplied display string. The macOS
/// TCC CVE-2025-31250 spoof was exactly two divergent fields (it rendered one
/// app's name while writing the grant for another), so this type carries one
/// value and exposes it under both roles - they are identical by construction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttestedRequester {
    app_id: String,
}

impl AttestedRequester {
    /// Wrap the kernel-attested app id. Callers MUST pass the value resolved
    /// from SO_PEERCRED via `path_to_app_id`, never a self-declared name.
    pub fn new(attested_app_id: impl Into<String>) -> Self {
        Self {
            app_id: attested_app_id.into(),
        }
    }

    /// The attested app id - the value SHOWN to the user in the dialog.
    pub fn display_id(&self) -> &str {
        &self.app_id
    }

    /// The attested app id - the value the minted grant is RECORDED against.
    /// Identical to [`AttestedRequester::display_id`] by construction; the two
    /// methods exist to make the "one value, both roles" invariant explicit at
    /// every call site rather than risk two separate fields.
    pub fn grant_recipient(&self) -> &str {
        &self.app_id
    }
}

/// A typed consent request raised by a daemon or app over IPC.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsentRequest {
    /// Who is asking (attested; the shown identity == the grant recipient).
    pub requester: AttestedRequester,
    /// The request class - selects the rendered dialog, never the severity.
    pub class: ConsentClass,
    /// The impact kind, reused verbatim from the AI decision engine: a
    /// high-impact kind always confirms (the non-configurable override).
    pub kind: ActionKind,
    /// Whether this request was triggered by external / untrusted content.
    /// Always escalates to high-stakes (the prompt-injection containment).
    pub triggered_by_external_content: bool,
    /// A plain-language risk/outcome summary - NOT the resource ("uses your
    /// data plan", "permanently deletes 3 files"), the anti-fatigue copy rule.
    pub summary: String,
    /// The concrete scope / target of the action, when there is a useful
    /// detail line to show.
    pub scope: Option<String>,
}

/// The severity tier the broker resolves a request to. EXACTLY the three
/// outcomes of [`Capability::decide`], generalised from "AI action" to "any
/// system request" - NOT a separate severity scheme. Distinct from a
/// notification's urgency `Priority` (which decides *when* to interrupt).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SeverityTier {
    /// No dialog: the grant is silent (still recorded in the KG/audit).
    /// `decide` -> [`ActionDecision::Proceed`].
    Silent,
    /// A standard modal: the action, its kind, the concrete risk, who is asking
    /// and the reversibility. `decide` -> [`ActionDecision::Propose`] /
    /// [`ActionDecision::PreviewThenExecute`].
    Standard,
    /// A high-stakes, polymorphic confirmation (destructive / external-send /
    /// external-triggered / irreversible). `decide` ->
    /// [`ActionDecision::RequireConfirmation`].
    HighStakes,
}

impl SeverityTier {
    /// Map a [`Capability::decide`] outcome to the consent tier. The single
    /// place the three decision outcomes become the three consent tiers.
    pub fn from_decision(decision: ActionDecision) -> Self {
        match decision {
            ActionDecision::Proceed => SeverityTier::Silent,
            ActionDecision::Propose | ActionDecision::PreviewThenExecute => SeverityTier::Standard,
            ActionDecision::RequireConfirmation => SeverityTier::HighStakes,
        }
    }

    /// Whether this tier renders a dialog at all (Silent does not).
    pub fn shows_dialog(self) -> bool {
        !matches!(self, SeverityTier::Silent)
    }
}

/// Classify a request into its severity tier by REUSING the caller's
/// [`Capability`] decision engine - the high-impact + external-content
/// overrides and the per-application action mode. The broker never reinvents
/// the tiering.
///
/// The decision is made for the SAME attested identity that is shown and
/// granted ([`AttestedRequester::grant_recipient`]), so the severity, the
/// displayed requester and the grant recipient are all keyed off one value.
pub fn classify(request: &ConsentRequest, capability: &Capability) -> SeverityTier {
    let decision = capability.decide(
        request.requester.grant_recipient(),
        request.kind,
        request.triggered_by_external_content,
    );
    SeverityTier::from_decision(decision)
}

/// The resolved outcome of a consent interaction, returned to the requester.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "outcome")]
pub enum ConsentOutcome {
    /// Allowed for this one occurrence; no grant is recorded.
    AllowedOnce,
    /// Allowed and remembered: a revocable grant is minted for the recipient
    /// (the KG-grant mint is a later piece; this is the decision the UI
    /// returns).
    AllowedRemembered,
    /// Denied.
    Denied,
}

impl ConsentOutcome {
    /// Whether a remembered, revocable grant should be minted for this outcome.
    pub fn mints_grant(self) -> bool {
        matches!(self, ConsentOutcome::AllowedRemembered)
    }

    /// Whether the action may proceed.
    pub fn allowed(self) -> bool {
        matches!(
            self,
            ConsentOutcome::AllowedOnce | ConsentOutcome::AllowedRemembered
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arlen_ai_core::capability::{AccessTier, ActionPermissions, BaselineMode};

    fn request(app: &str, kind: ActionKind, external: bool) -> ConsentRequest {
        ConsentRequest {
            requester: AttestedRequester::new(app),
            class: ConsentClass::CapabilityGrant,
            kind,
            triggered_by_external_content: external,
            summary: "test".to_string(),
            scope: None,
        }
    }

    /// A capability whose `app` is autonomous over a Suggest-by-default base.
    fn cap_with_autonomous(app: &str) -> Capability {
        Capability::new(
            AccessTier::Minimal,
            ActionPermissions::new(BaselineMode::Suggest, [app.to_string()]),
        )
    }

    fn cap_default(mode: BaselineMode) -> Capability {
        Capability::new(AccessTier::Minimal, ActionPermissions::new(mode, Vec::<String>::new()))
    }

    #[test]
    fn ordinary_autonomous_is_silent() {
        let cap = cap_with_autonomous("org.arlen.files");
        let req = request("org.arlen.files", ActionKind::Ordinary, false);
        assert_eq!(classify(&req, &cap), SeverityTier::Silent);
        assert!(!classify(&req, &cap).shows_dialog());
    }

    #[test]
    fn ordinary_suggest_is_standard() {
        let cap = cap_default(BaselineMode::Suggest);
        let req = request("org.arlen.mail", ActionKind::Ordinary, false);
        assert_eq!(classify(&req, &cap), SeverityTier::Standard);
    }

    #[test]
    fn ordinary_supervised_is_standard() {
        let cap = cap_default(BaselineMode::Supervised);
        let req = request("org.arlen.mail", ActionKind::Ordinary, false);
        assert_eq!(classify(&req, &cap), SeverityTier::Standard);
    }

    #[test]
    fn high_impact_is_high_stakes_regardless_of_mode() {
        // Even an autonomous app's permanent delete must confirm.
        let cap = cap_with_autonomous("org.arlen.files");
        for kind in [
            ActionKind::PermanentDelete,
            ActionKind::SendExternalMessage,
            ActionKind::ElevatedPrivilege,
            ActionKind::Irreversible,
        ] {
            let req = request("org.arlen.files", kind, false);
            assert_eq!(classify(&req, &cap), SeverityTier::HighStakes, "{kind:?}");
        }
    }

    #[test]
    fn external_triggered_ordinary_is_high_stakes() {
        // An autonomous app would otherwise be Silent; external content escalates.
        let cap = cap_with_autonomous("org.arlen.files");
        let req = request("org.arlen.files", ActionKind::Ordinary, true);
        assert_eq!(classify(&req, &cap), SeverityTier::HighStakes);
    }

    #[test]
    fn shown_identity_equals_grant_recipient() {
        // The load-bearing invariant: one attested value, both roles.
        let r = AttestedRequester::new("org.arlen.files");
        assert_eq!(r.display_id(), r.grant_recipient());
        // And `classify` decides for that same recipient (not a separate field).
        let cap = cap_with_autonomous("org.arlen.files");
        let req = request("org.arlen.files", ActionKind::Ordinary, false);
        assert_eq!(req.requester.grant_recipient(), "org.arlen.files");
        assert_eq!(classify(&req, &cap), SeverityTier::Silent);
    }

    #[test]
    fn outcome_helpers() {
        assert!(ConsentOutcome::AllowedRemembered.mints_grant());
        assert!(!ConsentOutcome::AllowedOnce.mints_grant());
        assert!(ConsentOutcome::AllowedOnce.allowed());
        assert!(!ConsentOutcome::Denied.allowed());
    }
}

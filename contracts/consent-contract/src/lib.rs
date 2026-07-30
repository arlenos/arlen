//! The consent-broker WIRE CONTRACT (system-dialog-plan.md, ordering #9): the
//! request body a requester frames, the request class, the intake result it
//! reads back, and the resolved outcome. These cross the intake socket between
//! the broker and any requester (a daemon, an app, the ai-engine-daemon's gate),
//! so they live in one shared crate rather than being mirrored - the broker
//! re-exports them at their original paths, and a client deps this crate instead
//! of the whole broker.
//!
//! The LOAD-BEARING identity rule lives in the broker, not here: [`RequestBody`]
//! structurally carries NO requester field, so a client cannot ask on another
//! app's behalf (the macOS TCC CVE-2025-31250 spoof is unrepresentable); the
//! broker fills the requester from the SO_PEERCRED-attested peer.

#![warn(missing_docs)]

use serde::{Deserialize, Serialize};

/// The impact axis a [`RequestBody`] carries, re-exported so a consumer building
/// or reading a request gets it from this contract crate rather than depending
/// on the AI core directly.
pub use arlen_ai_core::capability::ActionKind;

/// The class of system request seeking consent. The broker is the ONE surface
/// for all of these (system-dialog-plan.md): they share the trusted path, the
/// severity classification and the grant store, differing only in the rendered
/// dialog and the copy. The class never overrides the severity (that is the
/// broker's `classify`'s job); it selects which polymorphic dialog renders.
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
    /// An xdg desktop-portal access request routed to this backend that is not
    /// one of the explicit capture classes below (file chooser, open uri).
    Portal,
    /// A camera capture request (per-device designation, revocable).
    Camera,
    /// A microphone capture request (distinct indicator, revocable).
    Microphone,
    /// A screen capture / screencast request (source picker, revocable). The
    /// wire form is pinned to the one-word `screencast` (the freedesktop portal
    /// config key), matching `as_key` so the wire form and the revocation-handle
    /// key never diverge (snake_case would otherwise give `screen_cast`).
    #[serde(rename = "screencast")]
    ScreenCast,
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
            ConsentClass::Camera => "camera",
            ConsentClass::Microphone => "microphone",
            ConsentClass::ScreenCast => "screencast",
            ConsentClass::NotificationAction => "notification_action",
            ConsentClass::AgentAction => "agent_action",
        }
    }
}

/// The wire request a client sends to the broker. It carries the action's
/// class, impact and scope - but NOT the requester: the broker fills that from
/// the attested peer, so identity cannot be spoofed over the wire.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestBody {
    /// The request class (selects the rendered dialog).
    pub class: ConsentClass,
    /// The impact kind (drives the severity classification).
    pub kind: ActionKind,
    /// Whether this was triggered by external / untrusted content.
    #[serde(default)]
    pub triggered_by_external_content: bool,
    /// The plain-language risk/outcome summary.
    pub summary: String,
    /// The concrete scope / target, when there is one.
    #[serde(default)]
    pub scope: Option<String>,
    /// External-send only: the named recipient the data leaves Arlen to.
    #[serde(default)]
    pub recipient: Option<String>,
    /// External-send only: a short preview of the content that would leave Arlen.
    #[serde(default)]
    pub preview: Option<String>,
    /// Destructive only: the named targets (each with a human-readable size) the
    /// action affects, so the dialog can list what would be lost.
    #[serde(default)]
    pub targets: Vec<ConsentTarget>,
    /// Destructive only: the total size affected, shown beside the targets.
    #[serde(default)]
    pub total: Option<String>,
    /// Trusted-intermediary only: the app this request is made ON BEHALF OF.
    /// A mediator daemon (e.g. the xdg portal handling a ScreenCast the app
    /// cannot reach the broker for directly) sets this to the app it has already
    /// authenticated. The broker HONORS it ONLY when the SO_PEERCRED-attested
    /// peer is an allowlisted trusted intermediary; for any other peer it is
    /// IGNORED and the grant is attributed to the attested peer (fail-safe). The
    /// grant's subject then becomes the named app; the mediator is recorded in
    /// audit but never shown as the grantee.
    #[serde(default)]
    pub on_behalf_of: Option<String>,
}

/// One named target of a destructive request, with a human-readable size (e.g.
/// `"840 MB"`), for the informed-consent preview.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsentTarget {
    /// The target's display name (a file name).
    pub name: String,
    /// The target's human-readable size (e.g. `"840 MB"`).
    pub size: String,
}

/// The wire reply the requester reads back over the intake socket: a single
/// frame carrying the final disposition (silent grant, or the user's decision).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum IntakeResult {
    /// Granted without a dialog.
    SilentGranted,
    /// The user resolved the dialog with this outcome.
    Decided {
        /// The user's decision.
        outcome: ConsentOutcome,
    },
}

/// The resolved outcome of a consent interaction, returned to the requester.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "outcome")]
pub enum ConsentOutcome {
    /// Allowed for this one occurrence; no grant is recorded.
    AllowedOnce,
    /// Allowed and remembered: a revocable grant is minted for the recipient.
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

    /// The two predicates the whole consent contract turns on, over every
    /// outcome. Neither had a test: mutation testing found each could be
    /// replaced with a constant `true` or `false`, and `allowed -> true` means a
    /// DENIAL reads as permission to proceed while `mints_grant -> true` turns a
    /// one-off approval into a standing revocable grant.
    ///
    /// Written as an exhaustive table so a new outcome has to state what it means
    /// on both axes rather than inheriting whichever answer the match falls to.
    #[test]
    fn each_outcome_says_what_it_permits_and_what_it_records() {
        // (outcome, may proceed, mints a remembered grant)
        let table = [
            (ConsentOutcome::AllowedOnce, true, false),
            (ConsentOutcome::AllowedRemembered, true, true),
            (ConsentOutcome::Denied, false, false),
        ];
        for (outcome, allowed, mints) in table {
            assert_eq!(outcome.allowed(), allowed, "{outcome:?} may-proceed");
            assert_eq!(outcome.mints_grant(), mints, "{outcome:?} mints-grant");
        }
        // Stated separately because it is the property that matters most: nothing
        // that is refused may proceed, and nothing refused leaves a grant behind.
        assert!(!ConsentOutcome::Denied.allowed());
        assert!(!ConsentOutcome::Denied.mints_grant());
    }

    #[test]
    fn capture_class_keys_are_stable() {
        // The key is a load-bearing component of the deterministic revocation
        // handle (grant.rs), so a per-class capture grant only stays revocable
        // if these keys never drift. Locked here.
        assert_eq!(ConsentClass::Camera.as_key(), "camera");
        assert_eq!(ConsentClass::Microphone.as_key(), "microphone");
        assert_eq!(ConsentClass::ScreenCast.as_key(), "screencast");
        // The generic Portal class stays distinct from the explicit capture ones.
        assert_eq!(ConsentClass::Portal.as_key(), "portal");
    }

    #[test]
    fn capture_classes_round_trip_over_serde() {
        for class in [
            ConsentClass::Camera,
            ConsentClass::Microphone,
            ConsentClass::ScreenCast,
        ] {
            let wire = serde_json::to_string(&class).unwrap();
            // snake_case derive: the wire form matches the stable key.
            assert_eq!(wire, format!("\"{}\"", class.as_key()));
            let back: ConsentClass = serde_json::from_str(&wire).unwrap();
            assert_eq!(back, class);
        }
    }
}

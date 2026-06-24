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

use arlen_ai_core::capability::ActionKind;
use serde::{Deserialize, Serialize};

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

//! The consent seam (`pi-agent-adoption.md` §A Confirm verb / §C): a gate
//! `RequireConfirmation` is resolved DAEMON-SIDE by driving the trusted-path
//! consent surface (the #9 consent-broker), never by handing a raw `Confirm`
//! down to the engine (a raw engine->shell prompt would be spoofable). The
//! dispatcher calls this on a gate `Confirm` and maps the answer to `Allow`
//! (approved) or `Deny` (denied), so the engine only ever sees a resolved
//! decision.
//!
//! The daemon binary wires the real consent-broker client (the requester side of
//! the broker's intake-then-await protocol); the fail-closed [`DeniedConsent`]
//! default denies until that client lands, so a high-impact action is never
//! silently approved when no consent surface is reachable.

use ai_engine_contract::ConfirmAnswer;
use async_trait::async_trait;

/// Drives the trusted-path consent surface for a confirmation and blocks for the
/// user's answer. Maps to the §A `Confirm` verb (daemon <-> consent surface).
#[async_trait]
pub trait ConsentDriver: Send + Sync {
    /// Ask the user to confirm `tool_name` (the gate's `prompt` is the question;
    /// `external_triggered` escalates the surface's presentation). Returns the
    /// user's answer; MUST fail closed to [`ConfirmAnswer::Denied`] on any error,
    /// timeout, or unreachable surface - a confirmation that cannot be obtained is
    /// a denial, never a silent approval.
    async fn confirm(
        &self,
        tool_name: &str,
        prompt: &str,
        external_triggered: bool,
    ) -> ConfirmAnswer;
}

/// The fail-closed default: no consent surface is wired yet, so every
/// confirmation is `Denied`. The daemon binary swaps in the real consent-broker
/// client at the cutover; until then a high-impact / externally-triggered action
/// that reaches a `Confirm` is denied, never silently allowed.
pub struct DeniedConsent;

#[async_trait]
impl ConsentDriver for DeniedConsent {
    async fn confirm(&self, _tool_name: &str, _prompt: &str, _external_triggered: bool) -> ConfirmAnswer {
        ConfirmAnswer::Denied
    }
}

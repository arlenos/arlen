//! The fprintd-backed [`FactorVerifier`] for the fingerprint factor
//! (lockscreen-plan.md LS-R5: "fingerprint as a CONVENIENCE factor gated to
//! screen-unlock"). Feature-gated behind `fprintd` and exercised on hardware: it
//! drives the `net.reactivated.Fprint` D-Bus conversation against a real reader.
//!
//! A fingerprint match returns [`Factor::Fingerprint`], which the tier core
//! classifies CONVENIENCE - so it only ever warm-unlocks a running session and
//! can NEVER release the home/FDE key (the Linux enrollment path is spoof/replay
//! exposed, no SDCP; see [`crate::tier`]). This verifier cannot widen that: it
//! only produces a `Fingerprint` factor, and the tier boundary is enforced
//! downstream regardless of what any verifier returns.
//!
//! Synchronous, like [`crate::pam_verifier`]: it uses zbus's blocking API so the
//! sync [`FactorVerifier::verify`] can run the whole claim -> verify -> release
//! conversation on its own thread (the daemon calls each verification on a
//! dedicated / blocking thread). The status-string interpretation - the part
//! that decides match vs no-match vs keep-waiting - is a pure function tested
//! here; the live D-Bus flow is metal.

#![cfg(feature = "fprintd")]

use std::time::Duration;

use zbus::blocking::{Connection, Proxy};

use crate::auth::{FactorVerifier, Presentation, VerifyError};
use crate::tier::Factor;

/// The fprintd well-known bus name.
const FPRINT_SERVICE: &str = "net.reactivated.Fprint";
/// The Manager object + interface.
const MANAGER_PATH: &str = "/net/reactivated/Fprint/Manager";
const MANAGER_IFACE: &str = "net.reactivated.Fprint.Manager";
/// The Device interface (the object path comes from `GetDefaultDevice`).
const DEVICE_IFACE: &str = "net.reactivated.Fprint.Device";

/// "any" tells fprintd to match against any of the user's enrolled fingers.
const ANY_FINGER: &str = "any";

/// How long to wait for the user to present a finger before giving up. fprintd
/// drivers usually self-time-out, but a bounded wait keeps a never-touched
/// reader from pinning the verification thread forever.
const VERIFY_DEADLINE: Duration = Duration::from_secs(30);

/// A [`FactorVerifier`] backed by fprintd. Convenience-grade by construction.
pub struct FprintdVerifier;

impl FprintdVerifier {
    /// A verifier against the system fprintd.
    pub fn new() -> Self {
        Self
    }
}

impl Default for FprintdVerifier {
    fn default() -> Self {
        Self::new()
    }
}

/// What a single `VerifyStatus(result, done)` signal means for the flow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VerifyProgress {
    /// A non-terminal status (retry / swipe-too-short / centre your finger):
    /// keep waiting for the next signal.
    Continue,
    /// A terminal match: the fingerprint verified.
    Matched,
    /// A terminal no-match: the wrong finger / not enrolled.
    NoMatch,
    /// A terminal failure (reader disconnected, internal error, or an
    /// unrecognised terminal status): fail closed as a backend fault.
    Failed,
}

/// Interpret one fprintd `VerifyStatus(result, done)` signal.
///
/// `done == false` is always [`VerifyProgress::Continue`] (a transient
/// retry/quality status); the verification is not over. On `done == true`, only
/// the exact `verify-match` is a [`VerifyProgress::Matched`]; `verify-no-match`
/// is [`VerifyProgress::NoMatch`]; ANY other terminal status (disconnected,
/// unknown-error, or one we do not recognise) fails closed as
/// [`VerifyProgress::Failed`] rather than being mistaken for a match.
pub(crate) fn verify_progress(result: &str, done: bool) -> VerifyProgress {
    if !done {
        return VerifyProgress::Continue;
    }
    match result {
        "verify-match" => VerifyProgress::Matched,
        "verify-no-match" => VerifyProgress::NoMatch,
        _ => VerifyProgress::Failed,
    }
}

impl FactorVerifier for FprintdVerifier {
    fn verify(&self, presentation: &Presentation) -> Result<Factor, VerifyError> {
        let user = match presentation {
            Presentation::Fingerprint { user } => *user,
            // This verifier handles only the fingerprint factor; anything else
            // is refused (fail closed) rather than silently accepted.
            _ => {
                return Err(VerifyError::Backend(
                    "FprintdVerifier handles only the fingerprint factor".to_string(),
                ))
            }
        };
        self.verify_fingerprint(user)
    }
}

impl FprintdVerifier {
    /// The metal flow: claim the default device for `user`, start a verify, pump
    /// `VerifyStatus` signals to a verdict, then always stop+release the device.
    fn verify_fingerprint(&self, user: &str) -> Result<Factor, VerifyError> {
        let conn = Connection::system()
            .map_err(|e| VerifyError::Backend(format!("fprintd: system bus: {e}")))?;

        let manager = Proxy::new(&conn, FPRINT_SERVICE, MANAGER_PATH, MANAGER_IFACE)
            .map_err(|e| VerifyError::Backend(format!("fprintd: manager: {e}")))?;
        let device_path: zbus::zvariant::OwnedObjectPath = manager
            .call("GetDefaultDevice", &())
            .map_err(|e| VerifyError::Backend(format!("fprintd: no device: {e}")))?;

        let device = Proxy::new(&conn, FPRINT_SERVICE, &device_path, DEVICE_IFACE)
            .map_err(|e| VerifyError::Backend(format!("fprintd: device: {e}")))?;

        device
            .call::<_, _, ()>("Claim", &(user))
            .map_err(|e| VerifyError::Backend(format!("fprintd: claim: {e}")))?;

        // Whatever happens below, release the device before returning.
        let outcome = self.run_verify(&device);
        let _ = device.call::<_, _, ()>("VerifyStop", &());
        let _ = device.call::<_, _, ()>("Release", &());
        outcome
    }

    /// Subscribe to `VerifyStatus`, start the verify, and pump signals to a
    /// verdict (bounded by [`VERIFY_DEADLINE`]).
    fn run_verify(&self, device: &Proxy<'_>) -> Result<Factor, VerifyError> {
        let mut signals = device
            .receive_signal("VerifyStatus")
            .map_err(|e| VerifyError::Backend(format!("fprintd: subscribe: {e}")))?;

        device
            .call::<_, _, ()>("VerifyStart", &(ANY_FINGER))
            .map_err(|e| VerifyError::Backend(format!("fprintd: verify start: {e}")))?;

        let deadline = std::time::Instant::now() + VERIFY_DEADLINE;
        for msg in signals.by_ref() {
            let (result, done): (String, bool) = msg
                .body()
                .deserialize()
                .map_err(|e| VerifyError::Backend(format!("fprintd: bad signal: {e}")))?;
            match verify_progress(&result, done) {
                VerifyProgress::Matched => return Ok(Factor::Fingerprint),
                VerifyProgress::NoMatch => return Err(VerifyError::BadCredential),
                VerifyProgress::Failed => {
                    return Err(VerifyError::Backend(format!("fprintd: {result}")))
                }
                VerifyProgress::Continue => {
                    if std::time::Instant::now() >= deadline {
                        return Err(VerifyError::Backend("fprintd: verify timed out".to_string()));
                    }
                }
            }
        }
        Err(VerifyError::Backend("fprintd: signal stream ended".to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_not_done_status_keeps_waiting() {
        // Transient quality/retry statuses are never terminal.
        assert_eq!(verify_progress("verify-retry-scan", false), VerifyProgress::Continue);
        assert_eq!(verify_progress("verify-swipe-too-short", false), VerifyProgress::Continue);
        // Even a "match"-looking string is not terminal until done=true.
        assert_eq!(verify_progress("verify-match", false), VerifyProgress::Continue);
    }

    #[test]
    fn only_an_exact_done_match_is_a_match() {
        assert_eq!(verify_progress("verify-match", true), VerifyProgress::Matched);
        assert_eq!(verify_progress("verify-no-match", true), VerifyProgress::NoMatch);
    }

    #[test]
    fn any_other_terminal_status_fails_closed() {
        // A reader fault, an internal error, or an unrecognised terminal status
        // must fail closed as a backend fault, never be read as a match.
        assert_eq!(verify_progress("verify-disconnected", true), VerifyProgress::Failed);
        assert_eq!(verify_progress("verify-unknown-error", true), VerifyProgress::Failed);
        assert_eq!(verify_progress("something-new-from-fprintd", true), VerifyProgress::Failed);
    }
}

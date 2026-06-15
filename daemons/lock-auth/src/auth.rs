//! The auth composition: verify a presented factor, then let [`tier`] decide
//! what the verified factor may unlock, and produce the content-free audit
//! facts. This is the one backend behind BOTH the greeter and the lock screen
//! (greeter-onboarding-plan.md GR-R1, lockscreen-plan.md LS-R1): build the
//! verify -> tier -> audit composition once, render it twice.
//!
//! Verifying a credential (a password against PAM, a FIDO2 assertion, a
//! fingerprint match) is the backend's job, behind [`FactorVerifier`]; deciding
//! what a *verified* factor of a given tier may unlock is [`tier::evaluate`]'s
//! job. [`authenticate`] wires the two together fail-closed:
//!
//! - a credential that does not verify (wrong password, failed assertion,
//!   backend error) never reaches the tier core: it is a [`AuthOutcome::Denied`]
//!   and advances the failure counter, nothing more;
//! - only a VERIFIED strong factor on a cold session can reach
//!   [`AuthOutcome::KeyRelease`], because that is the only path
//!   [`tier::evaluate`] yields it, and the tier core is the sole classifier.
//!
//! The real [`FactorVerifier`] (PAM for the password, `pam_systemd_home` for the
//! homed key release, `libfido2`/`fprintd` for the device factors) is metal
//! plumbing built and verified on hardware; this composition is pure so the
//! security-relevant wiring is exhaustively unit-tested without any of it.

use crate::tier::{evaluate, Factor, SessionState, TierPolicy, UnlockOutcome};

/// Which surface is authenticating. A coarse audit label; it never carries
/// account content. The greeter authenticates a cold boot / login; the lock
/// screen re-authenticates a running session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    /// The login greeter (cold boot or a fresh session).
    Greeter,
    /// The lock screen of a running session.
    LockScreen,
}

impl Surface {
    /// A stable lowercase key for the audit subject.
    pub fn as_key(&self) -> &'static str {
        match self {
            Surface::Greeter => "greeter",
            Surface::LockScreen => "lock-screen",
        }
    }
}

/// A factor a surface presents for verification, with the credential material
/// borrowed only for the duration of the call. The password bytes live here
/// until [`FactorVerifier::verify`] consumes them; they never enter the
/// [`AuthOutcome`] or the [`AuthEvent`].
///
/// The device factors (FIDO2 / fingerprint / proximity) carry the result the
/// device subsystem already produced - the token confirmed user verification,
/// fprintd matched, the companion channel reported proximity - because those
/// checks happen in their own backends, not here. The verifier still gets the
/// final say (it may reject a malformed or replayed presentation), but the tier
/// the factor maps to is fixed by [`tier::Factor`], not by any surface.
pub enum Presentation<'a> {
    /// A password to verify against PAM for `user`.
    Password {
        /// The target account.
        user: &'a str,
        /// The password to verify. Borrowed, never retained past verification.
        password: &'a str,
    },
    /// A FIDO2 / passkey assertion for `user`. `user_verified` is whether the
    /// token confirmed a PIN or on-token biometric (the strong-vs-convenience
    /// distinction the `uv=false` hazard turns on).
    Fido2 {
        /// The target account.
        user: &'a str,
        /// Whether the token confirmed user verification.
        user_verified: bool,
    },
    /// A fingerprint match reported by fprintd for `user`. Convenience only.
    Fingerprint {
        /// The target account.
        user: &'a str,
    },
    /// A companion-device proximity signal for `user`. Convenience only.
    Proximity {
        /// The target account.
        user: &'a str,
        /// Whether the proximity channel is distance-bounded (anti-relay).
        relay_defended: bool,
    },
}

impl Presentation<'_> {
    /// The target account this presentation is for. A coarse account identifier
    /// kept on the audit facts; never a secret.
    pub fn user(&self) -> &str {
        match self {
            Presentation::Password { user, .. }
            | Presentation::Fido2 { user, .. }
            | Presentation::Fingerprint { user, .. }
            | Presentation::Proximity { user, .. } => user,
        }
    }

    /// The coarse factor kind, derivable even when verification fails (so a
    /// failed attempt can be audited with which factor was tried).
    pub fn kind(&self) -> FactorKind {
        match self {
            Presentation::Password { .. } => FactorKind::Password,
            Presentation::Fido2 { .. } => FactorKind::Fido2,
            Presentation::Fingerprint { .. } => FactorKind::Fingerprint,
            Presentation::Proximity { .. } => FactorKind::Proximity,
        }
    }
}

/// A coarse, content-free factor label for the audit log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FactorKind {
    /// The account password (PAM).
    Password,
    /// A FIDO2 / passkey token.
    Fido2,
    /// A fingerprint (fprintd).
    Fingerprint,
    /// A companion-device proximity unlock.
    Proximity,
}

impl FactorKind {
    /// A stable lowercase key for the audit subject.
    pub fn as_key(&self) -> &'static str {
        match self {
            FactorKind::Password => "password",
            FactorKind::Fido2 => "fido2",
            FactorKind::Fingerprint => "fingerprint",
            FactorKind::Proximity => "proximity",
        }
    }
}

/// Why a credential failed to verify. Both variants fail closed (a denial);
/// neither ever reaches the tier core, so neither can release the key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerifyError {
    /// The credential was wrong: a bad password, a failed/forged FIDO2
    /// assertion, no fingerprint match.
    BadCredential,
    /// The verification backend was unavailable or errored (PAM/fprintd/token
    /// fault). Treated as a denial - an unavailable verifier must never become
    /// an unlock.
    Backend(String),
}

/// Verifies a presented credential. The boundary between this crate's pure
/// decision core and the metal plumbing (PAM, libfido2, fprintd): an
/// implementation does the actual cryptographic / biometric check and, on
/// success, returns the [`tier::Factor`] the credential proved. On any failure
/// the credential proved nothing and nothing may unlock.
///
/// Synchronous on purpose: PAM's conversation is blocking, and the daemon runs
/// each verification on its own task / thread, so the trait stays simple and
/// the composition stays a plain function.
pub trait FactorVerifier {
    /// Verify `presentation`. On `Ok`, the returned [`Factor`] is the
    /// tier-classified factor the credential proved (the backend decides
    /// `user_verified` for FIDO2 from the token's response, never the UI). On
    /// `Err`, the credential did not prove a factor.
    fn verify(&self, presentation: &Presentation) -> Result<Factor, VerifyError>;
}

/// What the surface should do with the attempt - the [`tier::UnlockOutcome`]
/// plus the failure modes that never reach the tier core.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthOutcome {
    /// A verified strong factor on a cold session: release the home/FDE key.
    /// The only outcome that crosses the key boundary.
    KeyRelease,
    /// Unlock the warm session (the key is already loaded).
    WarmUnlock,
    /// Refused. Nothing is unlocked; the caller must obtain a strong factor or
    /// retry the credential.
    Denied(RefuseReason),
}

/// Why an attempt was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefuseReason {
    /// The credential did not verify (wrong password / failed assertion).
    BadCredential,
    /// The verification backend was unavailable; fail closed.
    BackendUnavailable,
    /// The credential verified but a STRONG factor is required (cold session,
    /// the strong window elapsed, or too many failures) and the verified factor
    /// is only convenience-grade. Maps from [`tier::DenyReason::StrongAuthRequired`].
    StrongAuthRequired,
}

/// The content-free facts recorded for one attempt. Carries the coarse account
/// id, the surface, the factor kind and a coarse outcome label - never the
/// password, never the FIDO2 assertion, never the released key. The integration
/// layer maps this onto the audit ledger (the account stays out of the always-
/// recorded Structural tier; only the surface + factor + outcome do).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthEvent {
    /// The target account (a coarse identifier, not a secret).
    pub account: String,
    /// Which surface asked.
    pub surface: Surface,
    /// Which factor was attempted.
    pub factor: FactorKind,
    /// A stable coarse outcome label: `key-release`, `warm-unlock`,
    /// `denied:bad-credential`, `denied:backend-unavailable`,
    /// `denied:strong-auth-required`.
    pub outcome: &'static str,
    /// Whether this attempt released the home/FDE key (the security-critical
    /// fact a reviewer most wants surfaced).
    pub released_key: bool,
}

/// The full result of one attempt: what to do, the post-attempt state to
/// persist, and the audit facts to record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthResult {
    /// What the surface should do.
    pub outcome: AuthOutcome,
    /// The session state to persist for the next attempt (advances the failure
    /// counter on a denial, resets the strong window on a strong success).
    pub next_state: SessionState,
    /// The content-free audit facts.
    pub audit: AuthEvent,
}

/// The coarse outcome label for a [`AuthOutcome`], used in the audit facts.
fn outcome_label(outcome: &AuthOutcome) -> &'static str {
    match outcome {
        AuthOutcome::KeyRelease => "key-release",
        AuthOutcome::WarmUnlock => "warm-unlock",
        AuthOutcome::Denied(RefuseReason::BadCredential) => "denied:bad-credential",
        AuthOutcome::Denied(RefuseReason::BackendUnavailable) => "denied:backend-unavailable",
        AuthOutcome::Denied(RefuseReason::StrongAuthRequired) => "denied:strong-auth-required",
    }
}

/// Authenticate one presented factor and decide what it unlocks.
///
/// The fail-closed pipeline:
/// 1. `verifier.verify(presentation)` - a credential check. On `Err` the
///    attempt is `Denied` (bad credential or unavailable backend) and the
///    failure counter advances; the tier core is never consulted, so a failed
///    verification can never unlock anything.
/// 2. On a verified [`Factor`], `tier::evaluate(state, policy)` decides: a
///    strong factor releases the key on a cold session or warm-unlocks; a
///    convenience factor warm-unlocks only when no strong factor is required.
/// 3. The post-attempt [`SessionState`] is advanced (a strong success resets
///    the strong window, any success clears the failure streak, a denial bumps
///    the counter) and the content-free [`AuthEvent`] is produced.
///
/// The KEY invariant, asserted by the tests: the ONLY way `outcome` is
/// `KeyRelease` is a successfully verified factor that `tier::evaluate` deemed
/// strong on a cold session; no verification failure and no convenience factor
/// can reach it.
pub fn authenticate(
    verifier: &dyn FactorVerifier,
    surface: Surface,
    presentation: &Presentation,
    state: &SessionState,
    policy: &TierPolicy,
) -> AuthResult {
    let account = presentation.user().to_string();
    let factor_kind = presentation.kind();

    let (outcome, next_state) = match verifier.verify(presentation) {
        Err(VerifyError::BadCredential) => (
            AuthOutcome::Denied(RefuseReason::BadCredential),
            state.record_failure(),
        ),
        Err(VerifyError::Backend(_)) => (
            AuthOutcome::Denied(RefuseReason::BackendUnavailable),
            // A backend fault is not a wrong-password event; still fail closed
            // and advance the failure counter so a flapping backend cannot be
            // used to keep a session permanently in the convenience tier.
            state.record_failure(),
        ),
        Ok(factor) => {
            let unlock = evaluate(&factor, state, policy);
            let outcome = match unlock {
                UnlockOutcome::KeyRelease => AuthOutcome::KeyRelease,
                UnlockOutcome::WarmUnlock => AuthOutcome::WarmUnlock,
                UnlockOutcome::Denied(_) => {
                    AuthOutcome::Denied(RefuseReason::StrongAuthRequired)
                }
            };
            (outcome, state.after_attempt(&factor, &unlock))
        }
    };

    let label = outcome_label(&outcome);
    let released_key = matches!(outcome, AuthOutcome::KeyRelease);
    AuthResult {
        outcome,
        next_state,
        audit: AuthEvent {
            account,
            surface,
            factor: factor_kind,
            outcome: label,
            released_key,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    const HOUR: Duration = Duration::from_secs(3600);

    fn warm() -> SessionState {
        SessionState {
            home_key_loaded: true,
            since_strong_auth: HOUR,
            failed_attempts: 0,
        }
    }

    fn cold() -> SessionState {
        SessionState {
            home_key_loaded: false,
            since_strong_auth: HOUR,
            failed_attempts: 0,
        }
    }

    /// A mock verifier: maps a presentation to a fixed result, so the
    /// composition (not the real PAM/FIDO backend) is what the tests exercise.
    struct MockVerifier {
        result: Result<Factor, VerifyError>,
    }

    impl FactorVerifier for MockVerifier {
        fn verify(&self, _p: &Presentation) -> Result<Factor, VerifyError> {
            self.result.clone()
        }
    }

    fn verifying(factor: Factor) -> MockVerifier {
        MockVerifier { result: Ok(factor) }
    }

    fn rejecting(err: VerifyError) -> MockVerifier {
        MockVerifier { result: Err(err) }
    }

    #[test]
    fn a_bad_password_is_denied_and_advances_failures_without_touching_the_tier() {
        let v = rejecting(VerifyError::BadCredential);
        let p = Presentation::Password {
            user: "alice",
            password: "wrong",
        };
        let r = authenticate(&v, Surface::LockScreen, &p, &cold(), &TierPolicy::default());
        assert_eq!(r.outcome, AuthOutcome::Denied(RefuseReason::BadCredential));
        assert!(!r.audit.released_key);
        assert_eq!(r.audit.outcome, "denied:bad-credential");
        assert_eq!(r.audit.account, "alice");
        assert_eq!(r.audit.factor, FactorKind::Password);
        assert_eq!(r.next_state.failed_attempts, 1, "a denial bumps the counter");
        assert!(!r.next_state.home_key_loaded, "a bad password never loads the key");
    }

    #[test]
    fn a_backend_fault_fails_closed_as_a_denial() {
        let v = rejecting(VerifyError::Backend("pam: conversation error".into()));
        let p = Presentation::Password {
            user: "bob",
            password: "irrelevant",
        };
        let r = authenticate(&v, Surface::LockScreen, &p, &cold(), &TierPolicy::default());
        assert_eq!(
            r.outcome,
            AuthOutcome::Denied(RefuseReason::BackendUnavailable)
        );
        assert!(!r.audit.released_key);
        assert_eq!(r.audit.outcome, "denied:backend-unavailable");
        assert_eq!(r.next_state.failed_attempts, 1);
    }

    #[test]
    fn a_verified_password_releases_the_key_on_a_cold_session() {
        let v = verifying(Factor::Password);
        let p = Presentation::Password {
            user: "carol",
            password: "correct horse",
        };
        let r = authenticate(&v, Surface::LockScreen, &p, &cold(), &TierPolicy::default());
        assert_eq!(r.outcome, AuthOutcome::KeyRelease);
        assert!(r.audit.released_key);
        assert_eq!(r.audit.outcome, "key-release");
        assert!(r.next_state.home_key_loaded, "the key is now loaded");
        assert_eq!(r.next_state.since_strong_auth, Duration::ZERO);
        assert_eq!(r.next_state.failed_attempts, 0);
    }

    #[test]
    fn a_verified_password_warm_unlocks_when_the_key_is_loaded() {
        let v = verifying(Factor::Password);
        let p = Presentation::Password {
            user: "dave",
            password: "x",
        };
        let r = authenticate(&v, Surface::LockScreen, &p, &warm(), &TierPolicy::default());
        assert_eq!(r.outcome, AuthOutcome::WarmUnlock);
        assert!(!r.audit.released_key, "a warm session is not a key release");
        assert_eq!(r.audit.outcome, "warm-unlock");
        assert_eq!(r.next_state.since_strong_auth, Duration::ZERO, "strong re-auth resets the window");
    }

    #[test]
    fn a_verified_fingerprint_warm_unlocks_only_when_strong_is_not_required() {
        let v = verifying(Factor::Fingerprint);
        let p = Presentation::Fingerprint { user: "erin" };
        // Warm + inside the window: convenience works.
        let r = authenticate(&v, Surface::LockScreen, &p, &warm(), &TierPolicy::default());
        assert_eq!(r.outcome, AuthOutcome::WarmUnlock);
        assert_eq!(r.audit.factor, FactorKind::Fingerprint);
        // Cold: strong required, convenience refused.
        let r = authenticate(&v, Surface::LockScreen, &p, &cold(), &TierPolicy::default());
        assert_eq!(
            r.outcome,
            AuthOutcome::Denied(RefuseReason::StrongAuthRequired)
        );
        assert_eq!(r.audit.outcome, "denied:strong-auth-required");
        assert_eq!(r.next_state.failed_attempts, 1);
    }

    #[test]
    fn a_non_user_verified_fido2_token_is_convenience_and_cannot_release_the_key() {
        // A token reporting uv=false maps to a convenience factor; on a cold
        // session it is refused, so a stolen token alone never releases the key.
        let v = verifying(Factor::Fido2 {
            user_verified: false,
        });
        let p = Presentation::Fido2 {
            user: "frank",
            user_verified: false,
        };
        let r = authenticate(&v, Surface::LockScreen, &p, &cold(), &TierPolicy::default());
        assert_eq!(
            r.outcome,
            AuthOutcome::Denied(RefuseReason::StrongAuthRequired)
        );
        assert!(!r.audit.released_key);
    }

    #[test]
    fn a_user_verified_fido2_token_releases_the_key_on_a_cold_session() {
        let v = verifying(Factor::Fido2 {
            user_verified: true,
        });
        let p = Presentation::Fido2 {
            user: "grace",
            user_verified: true,
        };
        let r = authenticate(&v, Surface::LockScreen, &p, &cold(), &TierPolicy::default());
        assert_eq!(r.outcome, AuthOutcome::KeyRelease);
        assert!(r.audit.released_key);
    }

    #[test]
    fn no_verification_failure_and_no_convenience_factor_ever_releases_the_key() {
        // The composition-level invariant mirroring the tier-core invariant:
        // across every failed verification and every convenience factor, in
        // every state, authenticate never yields KeyRelease.
        let policy = TierPolicy::default();
        let failures = [
            VerifyError::BadCredential,
            VerifyError::Backend("x".into()),
        ];
        for err in &failures {
            for st in [cold(), warm()] {
                let v = MockVerifier {
                    result: Err(err.clone()),
                };
                let p = Presentation::Password {
                    user: "u",
                    password: "p",
                };
                let r = authenticate(&v, Surface::LockScreen, &p, &st, &policy);
                assert!(!r.audit.released_key);
                assert_ne!(r.outcome, AuthOutcome::KeyRelease);
            }
        }
        let convenience = [
            Factor::Fido2 {
                user_verified: false,
            },
            Factor::Fingerprint,
            Factor::Proximity {
                relay_defended: true,
            },
            Factor::Proximity {
                relay_defended: false,
            },
        ];
        for factor in convenience {
            for key_loaded in [true, false] {
                for fails in [0u32, 5, u32::MAX] {
                    let st = SessionState {
                        home_key_loaded: key_loaded,
                        since_strong_auth: HOUR,
                        failed_attempts: fails,
                    };
                    let v = verifying(factor);
                    let p = Presentation::Fingerprint { user: "u" };
                    let r = authenticate(&v, Surface::LockScreen, &p, &st, &policy);
                    assert_ne!(
                        r.outcome,
                        AuthOutcome::KeyRelease,
                        "convenience factor {factor:?} released the key in {st:?}"
                    );
                }
            }
        }
    }
}

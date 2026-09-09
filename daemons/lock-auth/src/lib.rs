//! Arlen lock-screen + greeter shared auth backend (lockscreen-plan.md LS-R1).
//!
//! The lock screen and the greeter are two surfaces over ONE auth backend, built
//! once. This crate is the security spine of that backend: the factor-tier
//! enforcement ([`tier`]). The PAM bridge, the concrete factor backends
//! (password / FIDO2 / fingerprint / proximity) and the audit emission build on
//! top of this pure decision core; none of them may move the tier boundary, which
//! is enforced HERE, in code, not in any UI.
//!
//! The boundary (lockscreen-plan.md Decided 2, validated against Apple + Debian):
//! convenience factors (fingerprint, companion-proximity) unlock a WARM session
//! only and never release the systemd-homed / LUKS2 home key; only a STRONG factor
//! (a password, or a user-verification-confirmed FIDO2 token) releases the key,
//! and it is force-required after reboot, after a bounded time window, and after
//! too many failed attempts. A convenience factor can never cross that line.
//!
//! **THE GREETER RUNS `evaluate` SINCE 9 SEPTEMBER**, and holds a `SessionState`
//! across attempts - which is what this note used to say was missing. The
//! decision is taken at the one instant it can be a gate: greetd separates
//! proving the credential from starting the session, so the greeter asks after
//! `Authenticated` and before `start_session`, and a refusal cancels.
//!
//! **WHAT THAT DOES AND DOES NOT DO, because the difference matters more than
//! the wiring.** At the greeter the session is cold and a password is strong, so
//! the outcome is always `KeyRelease` today - and `KeyRelease` and `WarmUnlock`
//! lead to the same call. **Nothing in this tree acts on the distinction**: the
//! home key is released by greetd's own PAM stack (systemd-homed) as a side
//! effect of authenticating, not because this crate said `KeyRelease`. So the
//! key boundary is currently enforced by PAM being the only path, not by the
//! tier.
//!
//! What the greeter's wiring DOES buy: the failure count now advances across
//! attempts, and there is one place - not several - where a factor is weighed
//! before a session starts. **The tier has never refused anything**, because the
//! only factor that reaches it is a password, which is strong. The convenience
//! factors have no backend (`greeter_factor_begin` answers `not-connected` to
//! everything), so `StrongAuthRequired` is unreachable today.
//!
//! The tier becomes load-bearing when there is a warm session to unlock WITHOUT
//! re-running PAM - which is the lock screen, and that surface does not exist.
//!
//! `authenticate` still has no production caller: the greeter cannot use it,
//! because greetd owns the PAM conversation and calling it would verify the
//! credential a second time.

pub mod audit;
pub mod auth;
#[cfg(feature = "fprintd")]
pub mod fprintd_verifier;
#[cfg(feature = "greetd")]
pub mod greetd_client;
#[cfg(feature = "pam")]
pub mod pam_verifier;
pub mod tier;

#[cfg(feature = "fprintd")]
pub use fprintd_verifier::FprintdVerifier;
#[cfg(feature = "greetd")]
pub use greetd_client::{AuthStep, GreetdClient, GreetdError, GREETD_SOCK_ENV};
#[cfg(feature = "pam")]
pub use pam_verifier::{PamVerifier, DEFAULT_HOMED_SERVICE};

pub use audit::auth_audit_event;
pub use auth::{
    authenticate, AuthEvent, AuthOutcome, AuthResult, FactorKind, FactorVerifier, Presentation,
    RefuseReason, Surface, VerifyError,
};
pub use tier::{evaluate, DenyReason, Factor, SessionState, Tier, TierPolicy, UnlockOutcome};

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

pub mod auth;
pub mod tier;

pub use auth::{
    authenticate, AuthEvent, AuthOutcome, AuthResult, FactorKind, FactorVerifier, Presentation,
    RefuseReason, Surface, VerifyError,
};
pub use tier::{evaluate, DenyReason, Factor, SessionState, Tier, TierPolicy, UnlockOutcome};

//! The factor-tier enforcement: the one place the convenience-vs-strong boundary
//! is decided (lockscreen-plan.md Decided 2).
//!
//! [`evaluate`] takes a factor that the PAM / backend layer has ALREADY VERIFIED
//! (the cryptographic / biometric check succeeded) plus the current
//! [`SessionState`] and the [`TierPolicy`], and decides what that verified factor
//! is allowed to unlock: release the home/FDE key, unlock a warm session, or be
//! refused because a strong factor is required. Verifying a factor is the
//! backend's job; deciding what a verified factor of a given TIER may unlock is
//! this core's job, and it is the security spine, so it is pure and exhaustively
//! tested.
//!
//! The invariant, asserted by [`tests::no_convenience_factor_ever_releases_the_key`]:
//! **no convenience factor, in any state, ever yields [`UnlockOutcome::KeyRelease`].**

use std::time::Duration;

/// An authentication factor, as the tier layer sees it AFTER the backend verified
/// it. The closed set keeps a new factor from silently defaulting into the strong
/// tier: adding one is a deliberate [`Factor::tier`] arm, never an accident.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Factor {
    /// The account password (PAM). Strong: releases the key.
    Password,
    /// A FIDO2 / passkey token. Strong ONLY when user verification (a PIN or an
    /// on-token biometric) was confirmed: a token alone, with `user_verified`
    /// false, is the systemd-homed `uv=false` hazard (a stolen token would be the
    /// key), so it is treated as convenience-grade, never a key releaser.
    Fido2 {
        /// Whether the token confirmed user verification (PIN / on-token biometric).
        user_verified: bool,
    },
    /// A fingerprint (fprintd). Convenience only: the Linux enrollment path is
    /// spoof/replay exposed (no SDCP), so it gates the warm screen, never the key.
    Fingerprint,
    /// A companion-device proximity unlock. Convenience only; never the key. A
    /// proximity signal that is not relay-defended is not even load-bearing, but
    /// either way it cannot cross the key boundary, so the tier is the same.
    Proximity {
        /// Whether the proximity channel has anti-relay distance-bounding.
        relay_defended: bool,
    },
}

/// The two tiers; the boundary between them is the home/FDE key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    /// Releases the systemd-homed / LUKS2 home key. Password or UV-confirmed FIDO2.
    Strong,
    /// Unlocks a warm session only; never releases the key. Fingerprint, proximity,
    /// and a non-user-verified FIDO2 token.
    Convenience,
}

impl Factor {
    /// The tier this verified factor belongs to. The ONLY place a factor is
    /// classified; everything downstream keys off this.
    pub fn tier(&self) -> Tier {
        match self {
            Factor::Password => Tier::Strong,
            Factor::Fido2 {
                user_verified: true,
            } => Tier::Strong,
            // A token without user verification is the `uv=false` hazard: not strong.
            Factor::Fido2 {
                user_verified: false,
            } => Tier::Convenience,
            Factor::Fingerprint => Tier::Convenience,
            Factor::Proximity { .. } => Tier::Convenience,
        }
    }
}

/// The state the unlock decision is made against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionState {
    /// Whether the home/FDE key is currently loaded in the kernel (a WARM session).
    /// False after a reboot or after the key was discarded on deactivate: then the
    /// session is COLD and only a strong factor can re-release the key.
    pub home_key_loaded: bool,
    /// Time since the last STRONG authentication. Past [`TierPolicy::strong_window`]
    /// a strong factor is force-required again, even on a warm session.
    pub since_strong_auth: Duration,
    /// Consecutive failed unlock attempts since the last success. At or past
    /// [`TierPolicy::max_failed_attempts`] a strong factor is force-required.
    pub failed_attempts: u32,
}

/// The tunable thresholds for when a strong factor is force-required (the Apple
/// model: at startup, every N hours, and after K failed attempts). Loaded from
/// the lock-screen config by the integration layer; the defaults are conservative.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TierPolicy {
    /// Re-require a strong factor once this long has elapsed since the last strong
    /// auth, even on a warm session.
    pub strong_window: Duration,
    /// Re-require a strong factor once failed attempts reach this count.
    pub max_failed_attempts: u32,
}

impl Default for TierPolicy {
    /// Conservative defaults in the spirit of Apple's documented model (a strong
    /// factor every 48 hours, and after 5 failed attempts). The integration layer
    /// overrides these from config.
    fn default() -> Self {
        Self {
            strong_window: Duration::from_secs(48 * 60 * 60),
            max_failed_attempts: 5,
        }
    }
}

/// What a verified factor is allowed to do, given the state and policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnlockOutcome {
    /// A strong factor on a COLD session: release the home/FDE key. The only
    /// outcome that crosses the key boundary, and only a strong factor reaches it.
    KeyRelease,
    /// Unlock the warm screen/session (the key is already loaded): a strong factor
    /// on a warm session, or a convenience factor when strong auth is not required.
    WarmUnlock,
    /// Refused. The caller must obtain a strong factor; nothing is unlocked.
    Denied(DenyReason),
}

/// Why an unlock was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DenyReason {
    /// A convenience factor was offered but a STRONG factor is required: the
    /// session is cold (key not loaded), the strong-auth window elapsed, or there
    /// were too many failed attempts. The convenience tier cannot satisfy this.
    StrongAuthRequired,
}

/// Whether the state demands a strong factor right now (cold key, the window
/// elapsed, or too many failures). A strong factor always satisfies it; a
/// convenience factor never can.
fn strong_auth_required(state: &SessionState, policy: &TierPolicy) -> bool {
    !state.home_key_loaded
        || state.since_strong_auth >= policy.strong_window
        || state.failed_attempts >= policy.max_failed_attempts
}

/// Decide what the VERIFIED `factor` may unlock in `state` under `policy`.
///
/// - A STRONG factor always succeeds: it releases the key on a cold session, or
///   warm-unlocks when the key is already loaded.
/// - A CONVENIENCE factor warm-unlocks only when no strong factor is required;
///   when one is required (cold key / window elapsed / too many failures) it is
///   refused. A convenience factor never releases the key, in any state.
pub fn evaluate(factor: &Factor, state: &SessionState, policy: &TierPolicy) -> UnlockOutcome {
    match factor.tier() {
        Tier::Strong => {
            if state.home_key_loaded {
                UnlockOutcome::WarmUnlock
            } else {
                UnlockOutcome::KeyRelease
            }
        }
        Tier::Convenience => {
            if strong_auth_required(state, policy) {
                UnlockOutcome::Denied(DenyReason::StrongAuthRequired)
            } else {
                UnlockOutcome::WarmUnlock
            }
        }
    }
}

impl SessionState {
    /// A COLD session: no home key, the strong-auth window maximally elapsed, no
    /// failures. The state after a reboot or a deactivate key-discard, in which
    /// [`evaluate`] forces a strong factor before anything unlocks.
    pub fn cold() -> Self {
        SessionState {
            home_key_loaded: false,
            // Maximally elapsed so the window check forces strong for any finite
            // policy window; `home_key_loaded == false` already forces it, this is
            // belt-and-braces for a warm-then-cold transition.
            since_strong_auth: Duration::MAX,
            failed_attempts: 0,
        }
    }

    /// The discrete state transition after a single unlock attempt with `factor`
    /// that [`evaluate`] resolved to `outcome`. This advances only the DISCRETE
    /// counters: advancing `since_strong_auth` by wall-clock time is the
    /// integration layer's job (it samples a clock before each [`evaluate`]); this
    /// resets that window only on a strong success.
    ///
    /// The security-relevant rules:
    /// - a STRONG success ([`UnlockOutcome::KeyRelease`], or a [`Tier::Strong`]
    ///   [`UnlockOutcome::WarmUnlock`]) loads the key (key-release) or keeps it,
    ///   restarts the strong-auth window, and clears the failure streak;
    /// - a CONVENIENCE [`UnlockOutcome::WarmUnlock`] does NOT restart the
    ///   strong-auth window, does NOT load the key, AND does NOT clear the failure
    ///   streak - only a STRONG factor clears it. The failure counter exists to
    ///   force a strong factor after K failed attempts (Apple's model resets the
    ///   failed-passcode count only on a successful passcode, never on Touch ID);
    ///   letting a convenience touch reset it would let an interleaved biometric
    ///   keep the K-attempt brute-force throttle from ever tripping. A convenience
    ///   success only ever happens on an already-warm session, so it never affects
    ///   the cold-session key-release escalation either way;
    /// - a [`UnlockOutcome::Denied`] attempt advances the failure counter
    ///   (saturating) and changes nothing else, so K consecutive denials force a
    ///   strong factor.
    pub fn after_attempt(&self, factor: &Factor, outcome: &UnlockOutcome) -> SessionState {
        match outcome {
            UnlockOutcome::KeyRelease => SessionState {
                home_key_loaded: true,
                since_strong_auth: Duration::ZERO,
                failed_attempts: 0,
            },
            UnlockOutcome::WarmUnlock => {
                if factor.tier() == Tier::Strong {
                    SessionState {
                        home_key_loaded: true,
                        since_strong_auth: Duration::ZERO,
                        failed_attempts: 0,
                    }
                } else {
                    // Convenience proof-of-presence keeps the key and window as
                    // they were and leaves the failure streak intact: only a
                    // strong factor clears it (the throttle-integrity rule above).
                    SessionState {
                        home_key_loaded: self.home_key_loaded,
                        since_strong_auth: self.since_strong_auth,
                        failed_attempts: self.failed_attempts,
                    }
                }
            }
            UnlockOutcome::Denied(_) => SessionState {
                home_key_loaded: self.home_key_loaded,
                since_strong_auth: self.since_strong_auth,
                failed_attempts: self.failed_attempts.saturating_add(1),
            },
        }
    }

    /// The state transition after an attempt whose credential never VERIFIED
    /// (a wrong password, a failed FIDO2 assertion, a backend error), so no
    /// factor was ever classified. It is a denial: advance the failure counter
    /// (saturating) and change nothing else, exactly like a tier-denied attempt.
    /// Kept distinct from [`after_attempt`], which requires a verified factor the
    /// failed path never produced.
    pub fn record_failure(&self) -> SessionState {
        SessionState {
            home_key_loaded: self.home_key_loaded,
            since_strong_auth: self.since_strong_auth,
            failed_attempts: self.failed_attempts.saturating_add(1),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn factor_tiers_are_classified_correctly() {
        assert_eq!(Factor::Password.tier(), Tier::Strong);
        assert_eq!(
            Factor::Fido2 { user_verified: true }.tier(),
            Tier::Strong
        );
        // The uv=false hazard: a token alone is NOT strong.
        assert_eq!(
            Factor::Fido2 {
                user_verified: false
            }
            .tier(),
            Tier::Convenience
        );
        assert_eq!(Factor::Fingerprint.tier(), Tier::Convenience);
        assert_eq!(
            Factor::Proximity {
                relay_defended: true
            }
            .tier(),
            Tier::Convenience
        );
    }

    #[test]
    fn a_strong_factor_releases_the_key_on_a_cold_session() {
        let p = TierPolicy::default();
        assert_eq!(evaluate(&Factor::Password, &cold(), &p), UnlockOutcome::KeyRelease);
        assert_eq!(
            evaluate(&Factor::Fido2 { user_verified: true }, &cold(), &p),
            UnlockOutcome::KeyRelease
        );
    }

    #[test]
    fn a_strong_factor_warm_unlocks_when_the_key_is_loaded() {
        let p = TierPolicy::default();
        assert_eq!(evaluate(&Factor::Password, &warm(), &p), UnlockOutcome::WarmUnlock);
    }

    #[test]
    fn a_convenience_factor_warm_unlocks_only_when_strong_is_not_required() {
        let p = TierPolicy::default();
        assert_eq!(
            evaluate(&Factor::Fingerprint, &warm(), &p),
            UnlockOutcome::WarmUnlock
        );
        assert_eq!(
            evaluate(
                &Factor::Proximity {
                    relay_defended: true
                },
                &warm(),
                &p
            ),
            UnlockOutcome::WarmUnlock
        );
    }

    #[test]
    fn a_convenience_factor_is_refused_when_strong_is_required() {
        let p = TierPolicy::default();
        // Cold session (post-reboot / key discarded): strong required.
        assert_eq!(
            evaluate(&Factor::Fingerprint, &cold(), &p),
            UnlockOutcome::Denied(DenyReason::StrongAuthRequired)
        );
        // Warm but the strong-auth window elapsed.
        let stale = SessionState {
            since_strong_auth: p.strong_window,
            ..warm()
        };
        assert_eq!(
            evaluate(&Factor::Fingerprint, &stale, &p),
            UnlockOutcome::Denied(DenyReason::StrongAuthRequired)
        );
        // Warm but too many failed attempts.
        let locked_out = SessionState {
            failed_attempts: p.max_failed_attempts,
            ..warm()
        };
        assert_eq!(
            evaluate(
                &Factor::Proximity {
                    relay_defended: true
                },
                &locked_out,
                &p
            ),
            UnlockOutcome::Denied(DenyReason::StrongAuthRequired)
        );
    }

    #[test]
    fn a_non_user_verified_fido2_token_never_releases_the_key() {
        let p = TierPolicy::default();
        // On a cold session a bare token is refused (it is convenience-grade), so a
        // stolen token alone can never release the home key (the uv=false hazard).
        assert_eq!(
            evaluate(
                &Factor::Fido2 {
                    user_verified: false
                },
                &cold(),
                &p
            ),
            UnlockOutcome::Denied(DenyReason::StrongAuthRequired)
        );
    }

    #[test]
    fn no_convenience_factor_ever_releases_the_key() {
        // The crate invariant: across every factor classified Convenience and every
        // state, the outcome is never KeyRelease. This is the boundary the whole
        // design rests on, asserted exhaustively over the state space.
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
        let policies = [
            TierPolicy::default(),
            TierPolicy {
                strong_window: Duration::ZERO,
                max_failed_attempts: 0,
            },
            TierPolicy {
                strong_window: Duration::from_secs(u64::MAX / 2),
                max_failed_attempts: u32::MAX,
            },
        ];
        for factor in &convenience {
            assert_eq!(factor.tier(), Tier::Convenience);
            for key_loaded in [true, false] {
                for &fails in &[0u32, 1, 4, 5, 100, u32::MAX] {
                    for &since in &[Duration::ZERO, HOUR, Duration::from_secs(u64::MAX / 2)] {
                        for policy in &policies {
                            let state = SessionState {
                                home_key_loaded: key_loaded,
                                since_strong_auth: since,
                                failed_attempts: fails,
                            };
                            assert_ne!(
                                evaluate(factor, &state, policy),
                                UnlockOutcome::KeyRelease,
                                "convenience factor {factor:?} released the key in {state:?}"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn cold_forces_a_strong_factor() {
        let p = TierPolicy::default();
        let cold = SessionState::cold();
        assert!(!cold.home_key_loaded);
        // A convenience factor is refused on a cold session.
        assert_eq!(
            evaluate(&Factor::Fingerprint, &cold, &p),
            UnlockOutcome::Denied(DenyReason::StrongAuthRequired)
        );
        // A strong factor releases the key.
        assert_eq!(
            evaluate(&Factor::Password, &cold, &p),
            UnlockOutcome::KeyRelease
        );
    }

    #[test]
    fn a_key_release_loads_the_key_and_clears_the_window_and_failures() {
        let before = SessionState {
            home_key_loaded: false,
            since_strong_auth: Duration::from_secs(999_999),
            failed_attempts: 4,
        };
        let after = before.after_attempt(&Factor::Password, &UnlockOutcome::KeyRelease);
        assert!(after.home_key_loaded);
        assert_eq!(after.since_strong_auth, Duration::ZERO);
        assert_eq!(after.failed_attempts, 0);
    }

    #[test]
    fn a_denied_attempt_advances_the_failure_counter_and_saturates() {
        let s = warm();
        let after = s.after_attempt(
            &Factor::Password,
            &UnlockOutcome::Denied(DenyReason::StrongAuthRequired),
        );
        assert_eq!(after.failed_attempts, 1);
        assert_eq!(after.home_key_loaded, s.home_key_loaded);
        assert_eq!(after.since_strong_auth, s.since_strong_auth);
        // Saturates rather than overflowing.
        let maxed = SessionState {
            failed_attempts: u32::MAX,
            ..warm()
        };
        let after = maxed.after_attempt(
            &Factor::Fingerprint,
            &UnlockOutcome::Denied(DenyReason::StrongAuthRequired),
        );
        assert_eq!(after.failed_attempts, u32::MAX);
    }

    #[test]
    fn record_failure_advances_the_counter_and_saturates_without_a_factor() {
        let s = warm();
        let after = s.record_failure();
        assert_eq!(after.failed_attempts, 1);
        assert_eq!(after.home_key_loaded, s.home_key_loaded);
        assert_eq!(after.since_strong_auth, s.since_strong_auth);
        let maxed = SessionState {
            failed_attempts: u32::MAX,
            ..warm()
        };
        assert_eq!(maxed.record_failure().failed_attempts, u32::MAX);
    }

    #[test]
    fn a_convenience_success_keeps_the_window_key_flag_and_failure_streak() {
        let s = SessionState {
            home_key_loaded: true,
            since_strong_auth: HOUR,
            failed_attempts: 3,
        };
        let after = s.after_attempt(&Factor::Fingerprint, &UnlockOutcome::WarmUnlock);
        assert_eq!(
            after.failed_attempts, 3,
            "convenience does NOT clear the failure streak - only a strong factor does"
        );
        assert_eq!(
            after.since_strong_auth, HOUR,
            "convenience never restarts the strong-auth window"
        );
        assert!(after.home_key_loaded, "convenience never loads the key");
    }

    #[test]
    fn a_convenience_success_cannot_reset_the_brute_force_throttle() {
        // The throttle-integrity rule: interleaving a fingerprint between password
        // guesses must NOT keep the failure streak from reaching the lockout. Drive
        // the streak up with denials, slip a convenience success in between, and
        // confirm the count still climbs to force strong.
        let p = TierPolicy::default();
        let mut s = warm();
        for _ in 0..(p.max_failed_attempts - 1) {
            s = s.after_attempt(
                &Factor::Password,
                &UnlockOutcome::Denied(DenyReason::StrongAuthRequired),
            );
            // A convenience touch in between must not rescue the streak.
            s = s.after_attempt(&Factor::Fingerprint, &UnlockOutcome::WarmUnlock);
        }
        // One more denial reaches the cap.
        s = s.after_attempt(
            &Factor::Password,
            &UnlockOutcome::Denied(DenyReason::StrongAuthRequired),
        );
        assert!(s.failed_attempts >= p.max_failed_attempts);
        assert_eq!(
            evaluate(&Factor::Fingerprint, &s, &p),
            UnlockOutcome::Denied(DenyReason::StrongAuthRequired),
            "the throttle tripped despite interleaved convenience unlocks"
        );
    }

    #[test]
    fn a_strong_warm_reauth_restarts_the_window() {
        let s = SessionState {
            home_key_loaded: true,
            since_strong_auth: Duration::from_secs(999_999),
            failed_attempts: 2,
        };
        let after = s.after_attempt(&Factor::Password, &UnlockOutcome::WarmUnlock);
        assert!(after.home_key_loaded);
        assert_eq!(after.since_strong_auth, Duration::ZERO);
        assert_eq!(after.failed_attempts, 0);
    }

    #[test]
    fn k_denials_drive_a_warm_session_to_force_strong_and_a_strong_auth_recovers() {
        let p = TierPolicy::default();
        // Start warm and inside the window: convenience works.
        let mut s = SessionState {
            home_key_loaded: true,
            since_strong_auth: HOUR,
            failed_attempts: 0,
        };
        assert_eq!(
            evaluate(&Factor::Fingerprint, &s, &p),
            UnlockOutcome::WarmUnlock
        );
        // Drive `max_failed_attempts` denials through the transition.
        for _ in 0..p.max_failed_attempts {
            s = s.after_attempt(
                &Factor::Fingerprint,
                &UnlockOutcome::Denied(DenyReason::StrongAuthRequired),
            );
        }
        // Now convenience is force-refused even though the key is still loaded.
        assert_eq!(
            evaluate(&Factor::Fingerprint, &s, &p),
            UnlockOutcome::Denied(DenyReason::StrongAuthRequired)
        );
        // A strong warm re-auth clears the failures and convenience works again.
        let outcome = evaluate(&Factor::Password, &s, &p);
        s = s.after_attempt(&Factor::Password, &outcome);
        assert_eq!(
            evaluate(&Factor::Fingerprint, &s, &p),
            UnlockOutcome::WarmUnlock
        );
    }
}

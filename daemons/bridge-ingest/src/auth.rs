// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

//! BR-4's decision core: what a bridge's credential is and when it must be
//! renewed, with no opinion yet on where the long-lived secret lives.
//!
//! `bridge-architecture.md` §5 puts one `AuthAdapter` contract over three
//! concrete strategies and a broker behind it, so a bridge process only ever
//! holds a fresh, scoped, short-lived credential and never the raw long-lived
//! secret. The broker is the freedesktop Secret Service, shared with
//! `connections-plan.md`, and it is deliberately absent here: talking to a
//! keyring is I/O and policy, while the part that is easy to get quietly wrong
//! is the arithmetic about when a credential is still good. That part is here,
//! pure, so it can be tested without a keyring, a bridge or a clock.

use serde::{Deserialize, Serialize};

/// How a bridge authenticates upstream.
///
/// Closed on purpose: a bridge is data, and the privileged side runs no
/// per-bridge code, so a strategy the host does not know how to broker must be
/// unrepresentable rather than fall through to something permissive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AuthStrategy {
    /// A long-lived key the broker holds and hands over unchanged. It cannot be
    /// scoped or shortened, so the containment is the broker holding it rather
    /// than the bridge storing it.
    StaticApiKey,
    /// RFC 6749: the broker keeps the refresh token, the bridge sees only a
    /// short-lived access token. The canonical split and the only strategy
    /// where "short-lived" is more than a wish.
    Oauth2Refresh,
    /// No secret at all: the peer is pinned by identity, like the browser
    /// bridge's exact signed extension id in `allowed_origins`.
    PinnedIdentity,
}

impl AuthStrategy {
    /// Whether this strategy yields credentials that expire and can be renewed.
    ///
    /// Drives whether an expiry is meaningful: a static key that "expires" is a
    /// broker configuration error, not a refresh cue, and a pinned identity
    /// carries no credential to age.
    pub fn renewable(self) -> bool {
        matches!(self, AuthStrategy::Oauth2Refresh)
    }
}

/// How long before a credential's expiry it should be renewed.
///
/// Renewing exactly at expiry loses the race by construction: the check, the
/// call and the upstream's own clock all sit between the decision and the use.
/// Thirty seconds is comfortably longer than a slow refresh round trip and far
/// shorter than any access-token lifetime worth issuing, so it never renews
/// continuously and never hands over a token that dies mid-request.
pub const REFRESH_SKEW_MICROS: i64 = 30 * 1_000_000;

/// A credential as the bridge sees it: the value plus when it stops working.
///
/// The value is deliberately opaque here - this type decides timing, not
/// transport - and there is no `Display`, so it cannot be logged by accident.
#[derive(Clone, PartialEq, Eq)]
pub struct ScopedCredential {
    value: String,
    expires_at_micros: Option<i64>,
}

impl std::fmt::Debug for ScopedCredential {
    /// Redacted. A bridge's credential ends up in error paths and health
    /// reports, and the one place a short-lived token becomes long-lived is a
    /// log file.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScopedCredential")
            .field("value", &"<redacted>")
            .field("expires_at_micros", &self.expires_at_micros)
            .finish()
    }
}

/// What the host should do with a credential before using it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freshness {
    /// Good to use as-is.
    Usable,
    /// Still valid but inside the renewal window: use it, and renew alongside.
    RenewSoon,
    /// Past its expiry. Not usable; a request with it would fail upstream.
    Expired,
}

impl ScopedCredential {
    /// A credential that expires.
    pub fn expiring(value: impl Into<String>, expires_at_micros: i64) -> Self {
        Self {
            value: value.into(),
            expires_at_micros: Some(expires_at_micros),
        }
    }

    /// A credential with no expiry: a static key, or a pinned identity's
    /// stand-in. Never renewable, so it is never `RenewSoon`.
    pub fn perpetual(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            expires_at_micros: None,
        }
    }

    /// The secret itself. Named so a reader sees what is being handled.
    pub fn secret(&self) -> &str {
        &self.value
    }

    /// When it stops working, if it does.
    pub fn expires_at_micros(&self) -> Option<i64> {
        self.expires_at_micros
    }

    /// Whether this credential can be used at `now_micros`, and whether a
    /// renewal should be started.
    ///
    /// The clock is a parameter rather than read here so the decision is a pure
    /// function of its inputs and a test does not have to wait.
    pub fn freshness(&self, now_micros: i64) -> Freshness {
        match self.expires_at_micros {
            None => Freshness::Usable,
            Some(expiry) if now_micros >= expiry => Freshness::Expired,
            Some(expiry) if now_micros >= expiry.saturating_sub(REFRESH_SKEW_MICROS) => {
                Freshness::RenewSoon
            }
            Some(_) => Freshness::Usable,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_700_000_000_000_000;

    #[test]
    fn a_credential_well_before_its_expiry_is_simply_usable() {
        let c = ScopedCredential::expiring("t", NOW + 3600 * 1_000_000);
        assert_eq!(c.freshness(NOW), Freshness::Usable);
    }

    /// The window exists so a renewal starts before the token dies, not after.
    /// A credential valid for another ten seconds is still usable AND already
    /// overdue for renewal; conflating those is how a request goes out with a
    /// token that expires in flight.
    #[test]
    fn inside_the_skew_it_is_usable_and_due_for_renewal() {
        let expiry = NOW + 10 * 1_000_000;
        let c = ScopedCredential::expiring("t", expiry);
        assert_eq!(c.freshness(NOW), Freshness::RenewSoon);
        // And one microsecond before the window opens it is not yet due.
        let c = ScopedCredential::expiring("t", NOW + REFRESH_SKEW_MICROS + 1);
        assert_eq!(c.freshness(NOW), Freshness::Usable);
    }

    #[test]
    fn at_and_past_the_expiry_it_is_expired_not_merely_due() {
        let c = ScopedCredential::expiring("t", NOW);
        assert_eq!(c.freshness(NOW), Freshness::Expired, "at the instant it lapses");
        assert_eq!(c.freshness(NOW + 1), Freshness::Expired);
    }

    #[test]
    fn a_perpetual_credential_is_usable_at_any_time_and_never_due() {
        let c = ScopedCredential::perpetual("k");
        for t in [i64::MIN, 0, NOW, i64::MAX] {
            assert_eq!(c.freshness(t), Freshness::Usable, "at {t}");
        }
    }

    /// An expiry near the start of the epoch must not wrap when the skew is
    /// subtracted; saturating arithmetic keeps the answer sane rather than
    /// flipping a lapsed credential to usable.
    #[test]
    fn an_expiry_near_the_epoch_floor_does_not_wrap() {
        let c = ScopedCredential::expiring("t", i64::MIN + 5);
        assert_eq!(c.freshness(0), Freshness::Expired);
    }

    #[test]
    fn only_the_refresh_strategy_is_renewable() {
        assert!(AuthStrategy::Oauth2Refresh.renewable());
        assert!(!AuthStrategy::StaticApiKey.renewable());
        assert!(!AuthStrategy::PinnedIdentity.renewable());
    }

    /// The secret must not reach a log through the derived formatter, because
    /// the whole point of a short-lived token is undone by one that persists in
    /// a log file.
    #[test]
    fn the_debug_form_does_not_carry_the_secret() {
        let rendered = format!("{:?}", ScopedCredential::expiring("super-secret-token", NOW));
        assert!(!rendered.contains("super-secret-token"), "{rendered}");
        assert!(rendered.contains("redacted"), "{rendered}");
    }

    #[test]
    fn a_strategy_round_trips_through_its_wire_form() {
        for s in [
            AuthStrategy::StaticApiKey,
            AuthStrategy::Oauth2Refresh,
            AuthStrategy::PinnedIdentity,
        ] {
            let text = toml::to_string(&Wrapper { strategy: s }).unwrap();
            let back: Wrapper = toml::from_str(&text).unwrap();
            assert_eq!(back.strategy, s, "{text}");
        }
    }

    #[derive(Serialize, Deserialize)]
    struct Wrapper {
        strategy: AuthStrategy,
    }
}

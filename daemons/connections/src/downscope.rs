//! Token downscoping (CONN-R2, connections-plan.md §2 property 2).
//!
//! The authority holds the full credential; it hands an app only a short-lived,
//! downscoped derived token, never the full credential. Two modes: an RFC 8693
//! token exchange (mint a narrowed access token at the provider's STS) where the
//! upstream supports it, else broker-proxy (the app never sees a token; the daemon
//! makes the outbound call with the full credential).
//!
//! RFC 8693 standardises the wire format but does NOT guarantee the issued token
//! is less privileged, so the broker ENFORCES strict monotonic attenuation: a
//! derived token's scope must be a subset of the granted scope, never add, never
//! exceed (the subtract-only property of ocap/Macaroons). This module is the pure
//! enforcement core; the HTTP exchange, the proxy path, and on-exit revocation
//! build on it.

use crate::broker::CredentialKind;

/// How the daemon delivers a credential to an app, per credential kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownscopeMode {
    /// RFC 8693 token exchange: mint a short-lived downscoped access token at the
    /// provider STS and hand it to the app (OAuth refresh tokens are narrowable).
    RfcExchange,
    /// Broker-proxy: the app never sees a token; the daemon makes the outbound
    /// call with the full credential (an opaque API key or webhook secret the
    /// upstream cannot narrow).
    Proxy,
}

/// Classify the delivery mode from the credential kind. An OAuth refresh token is
/// narrowable via RFC 8693; an opaque API key or webhook secret is not, so it is
/// broker-proxied (the app never receives it).
pub fn mode_for(kind: CredentialKind) -> DownscopeMode {
    match kind {
        CredentialKind::OAuthRefreshToken => DownscopeMode::RfcExchange,
        CredentialKind::ApiKey | CredentialKind::WebhookSecret => DownscopeMode::Proxy,
    }
}

/// A short-lived downscoped token handed to an app (the RFC 8693 exchange result).
/// The token bytes are a secret, so they are redacted from `Debug` and zeroized on
/// drop, exactly like the stored credential.
#[derive(Clone, PartialEq, Eq)]
pub struct DerivedToken {
    /// The short-lived access token.
    pub access_token: String,
    /// The scope the token actually carries (a subset of the granted scope).
    pub scope: Vec<String>,
    /// Expiry, epoch micros. The daemon refuses to hand out an already-expired
    /// token and revokes on process exit before this.
    pub expires_at_micros: i64,
}

impl std::fmt::Debug for DerivedToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DerivedToken")
            .field("access_token", &format_args!("<redacted {} bytes>", self.access_token.len()))
            .field("scope", &self.scope)
            .field("expires_at_micros", &self.expires_at_micros)
            .finish()
    }
}

impl Drop for DerivedToken {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.access_token.zeroize();
    }
}

/// The strict-attenuation guard on a mint. The derived token's scope MUST be a
/// subset of the `granted` scope (subtract-only: never add, never exceed), so a
/// provider that returned a broader scope than requested (RFC 8693 does not forbid
/// it) is refused. An EMPTY derived scope against a non-empty grant is also refused
/// (several OAuth providers read an empty scope as "all default scopes", which
/// would amplify - the CONN-R1 review's deferred hazard). Returns the token only
/// when the invariant holds, else `None` (the caller fails the handout closed).
pub fn enforce_attenuation(derived: DerivedToken, granted: &[String]) -> Option<DerivedToken> {
    if !derived.scope.iter().all(|s| granted.contains(s)) {
        return None;
    }
    if derived.scope.is_empty() && !granted.is_empty() {
        return None;
    }
    Some(derived)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token(scope: &[&str]) -> DerivedToken {
        DerivedToken {
            access_token: "short-lived-xyz".to_string(),
            scope: scope.iter().map(|s| s.to_string()).collect(),
            expires_at_micros: 0,
        }
    }

    fn owned(scope: &[&str]) -> Vec<String> {
        scope.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn mode_follows_credential_kind() {
        assert_eq!(mode_for(CredentialKind::OAuthRefreshToken), DownscopeMode::RfcExchange);
        assert_eq!(mode_for(CredentialKind::ApiKey), DownscopeMode::Proxy);
        assert_eq!(mode_for(CredentialKind::WebhookSecret), DownscopeMode::Proxy);
    }

    #[test]
    fn a_subset_derived_scope_is_accepted() {
        let granted = owned(&["repo", "read:user"]);
        let out = enforce_attenuation(token(&["read:user"]), &granted);
        assert!(out.is_some());
        assert_eq!(out.unwrap().scope, vec!["read:user".to_string()]);
    }

    #[test]
    fn equal_scope_is_accepted() {
        let granted = owned(&["repo"]);
        assert!(enforce_attenuation(token(&["repo"]), &granted).is_some());
    }

    #[test]
    fn a_scope_beyond_the_grant_is_refused() {
        // The provider returned a scope not in the grant -> amplification refused.
        let granted = owned(&["read:user"]);
        assert!(enforce_attenuation(token(&["repo"]), &granted).is_none());
        // Partial overlap still refused (one token is outside the grant).
        assert!(enforce_attenuation(token(&["read:user", "repo"]), &granted).is_none());
    }

    #[test]
    fn an_empty_derived_scope_for_a_nonempty_grant_is_refused() {
        // Empty scope is read as "all default scopes" by some providers -> refuse.
        let granted = owned(&["repo"]);
        assert!(enforce_attenuation(token(&[]), &granted).is_none());
    }

    #[test]
    fn an_empty_grant_and_empty_derived_scope_is_accepted() {
        // A name-only grant with an empty derived scope is consistent (no widening).
        assert!(enforce_attenuation(token(&[]), &[]).is_some());
    }

    #[test]
    fn debug_redacts_the_token() {
        let out = format!("{:?}", token(&["repo"]));
        assert!(!out.contains("short-lived-xyz"));
        assert!(out.contains("redacted"));
    }
}

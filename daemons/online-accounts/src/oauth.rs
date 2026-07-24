//! OA-R2: the RFC-8252 loopback OAuth flow's pure building blocks.
//!
//! The daemon drives an OAuth authorization-code flow through the system
//! browser (no embedded web-view). It generates a PKCE-S256 verifier and
//! challenge plus a CSRF `state`, opens the browser at the built authorization
//! URL, receives the code on a loopback redirect, and exchanges it for tokens.
//! This module is the pure, deterministic half (PKCE per RFC 7636 and the
//! authorization URL per RFC 6749 §4.1.1), so the security-critical crypto and
//! URL construction are unit-tested. The loopback listener and the token HTTP
//! exchange (which need the human-gated client IDs to run live) are the wiring
//! half.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use sha2::{Digest, Sha256};

/// A PKCE (RFC 7636) verifier + its S256 challenge. The verifier is a
/// high-entropy secret kept by the daemon; the challenge goes in the
/// authorization request, and the verifier is presented at the token exchange
/// so the authorization server can bind the two - defeating an intercepted
/// authorization code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PkcePair {
    /// The `code_verifier` (43 chars, base64url of 32 random bytes). Secret.
    pub verifier: String,
    /// The `code_challenge` = base64url(SHA256(verifier)), sent in the auth URL.
    pub challenge: String,
}

impl PkcePair {
    /// A fresh pair from 32 CSPRNG bytes. Fails only if the OS RNG does.
    pub fn generate() -> Result<Self, getrandom::Error> {
        let mut seed = [0u8; 32];
        getrandom::getrandom(&mut seed)?;
        let verifier = URL_SAFE_NO_PAD.encode(seed);
        let challenge = challenge_for(&verifier);
        Ok(Self { verifier, challenge })
    }
}

/// The S256 `code_challenge` for a `code_verifier`: `base64url(SHA256(v))`,
/// unpadded (RFC 7636 §4.2). Deterministic, so it is testable against the RFC
/// test vector.
pub fn challenge_for(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(digest)
}

/// A fresh CSRF `state` (base64url of 16 random bytes), echoed back on the
/// redirect and compared to defeat cross-site request forgery of the callback.
pub fn random_state() -> Result<String, getrandom::Error> {
    let mut seed = [0u8; 16];
    getrandom::getrandom(&mut seed)?;
    Ok(URL_SAFE_NO_PAD.encode(seed))
}

/// The inputs to an authorization-code request (RFC 6749 §4.1.1) with PKCE.
#[derive(Debug, Clone)]
pub struct AuthRequest<'a> {
    /// The provider's authorization endpoint URL (the base, no query).
    pub authorization_endpoint: &'a str,
    /// The registered OAuth client id (human-gated per provider).
    pub client_id: &'a str,
    /// The loopback redirect URI (`http://127.0.0.1:<port>/`), RFC 8252 §7.3.
    pub redirect_uri: &'a str,
    /// The space-delimited requested scopes.
    pub scope: &'a str,
    /// The CSRF `state` from [`random_state`].
    pub state: &'a str,
    /// The PKCE S256 `code_challenge`.
    pub code_challenge: &'a str,
}

impl AuthRequest<'_> {
    /// Build the full authorization URL the daemon opens in the system browser.
    /// Query values are percent-encoded (RFC 3986); `response_type=code` and
    /// `code_challenge_method=S256` are fixed.
    pub fn url(&self) -> String {
        let sep = if self.authorization_endpoint.contains('?') {
            '&'
        } else {
            '?'
        };
        format!(
            "{base}{sep}response_type=code\
             &client_id={client}\
             &redirect_uri={redirect}\
             &scope={scope}\
             &state={state}\
             &code_challenge={challenge}\
             &code_challenge_method=S256",
            base = self.authorization_endpoint,
            sep = sep,
            client = enc(self.client_id),
            redirect = enc(self.redirect_uri),
            scope = enc(self.scope),
            state = enc(self.state),
            challenge = enc(self.code_challenge),
        )
    }
}

/// Percent-encode a query value. `NON_ALPHANUMERIC` over-encodes the unreserved
/// `-._~` too, which servers still decode correctly, so it is always safe.
fn enc(value: &str) -> String {
    utf8_percent_encode(value, NON_ALPHANUMERIC).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The RFC 7636 Appendix B test vector pins the S256 derivation exactly.
    #[test]
    fn challenge_matches_the_rfc7636_vector() {
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert_eq!(
            challenge_for(verifier),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    /// A generated pair has a 43-char verifier whose challenge re-derives.
    #[test]
    fn generated_pair_is_self_consistent() {
        let p = PkcePair::generate().unwrap();
        assert_eq!(p.verifier.len(), 43, "base64url of 32 bytes, unpadded");
        assert_eq!(challenge_for(&p.verifier), p.challenge);
        // No padding / URL-unsafe chars leak into either field.
        for s in [&p.verifier, &p.challenge] {
            assert!(!s.contains('='), "unpadded");
            assert!(!s.contains('+') && !s.contains('/'), "url-safe alphabet");
        }
        // Two generations differ (entropy).
        assert_ne!(PkcePair::generate().unwrap().verifier, p.verifier);
    }

    #[test]
    fn auth_url_has_the_fixed_params_and_encodes_values() {
        let req = AuthRequest {
            authorization_endpoint: "https://accounts.example.com/authorize",
            client_id: "client-123",
            redirect_uri: "http://127.0.0.1:41739/",
            scope: "openid email calendar.readonly",
            state: "xyz",
            code_challenge: "abc-def",
        };
        let url = req.url();
        assert!(url.starts_with("https://accounts.example.com/authorize?response_type=code"));
        assert!(url.contains("&code_challenge_method=S256"));
        assert!(url.contains("&client_id=client%2D123"));
        // The redirect's `:` and `/` are encoded (a valid query value).
        assert!(url.contains("&redirect_uri=http%3A%2F%2F127%2E0%2E0%2E1%3A41739%2F"));
        // Spaces in scope are encoded, not left literal.
        assert!(url.contains("&scope=openid%20email%20calendar%2Ereadonly"));
        assert!(!url.contains(' '));
    }

    #[test]
    fn auth_url_appends_when_the_endpoint_already_has_a_query() {
        let req = AuthRequest {
            authorization_endpoint: "https://p.example.com/auth?hd=example.com",
            client_id: "c",
            redirect_uri: "http://127.0.0.1:1/",
            scope: "s",
            state: "st",
            code_challenge: "ch",
        };
        // A pre-existing query means the params join with `&`, not `?`.
        assert!(req.url().contains("example.com&response_type=code"));
    }

    #[test]
    fn state_is_urlsafe_and_nonempty() {
        let s = random_state().unwrap();
        assert!(!s.is_empty());
        assert!(!s.contains('=') && !s.contains('+') && !s.contains('/'));
    }
}

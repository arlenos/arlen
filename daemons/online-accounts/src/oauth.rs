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
use percent_encoding::{utf8_percent_encode, AsciiSet, CONTROLS};
use serde::Deserialize;
use sha2::{Digest, Sha256};

/// Everything that is NOT an RFC 3986 unreserved char (ALPHA / DIGIT / `-` /
/// `.` / `_` / `~`) is percent-encoded. This leaves OAuth constants like
/// `grant_type` and values like `refresh_token` verbatim while encoding the
/// reserved/unsafe chars (`:` `/` `?` `&` `=` space, etc.) that would otherwise
/// break the query or form body.
const NON_UNRESERVED: &AsciiSet = &CONTROLS
    .add(b' ').add(b'!').add(b'"').add(b'#').add(b'$').add(b'%').add(b'&')
    .add(b'\'').add(b'(').add(b')').add(b'*').add(b'+').add(b',').add(b'/')
    .add(b':').add(b';').add(b'<').add(b'=').add(b'>').add(b'?').add(b'@')
    .add(b'[').add(b'\\').add(b']').add(b'^').add(b'`').add(b'{').add(b'|').add(b'}')
    .add(b'\x7f');

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

/// Percent-encode a query / form value, encoding everything that is not an RFC
/// 3986 unreserved char (so OAuth constants + tokens pass through verbatim, the
/// reserved chars are escaped).
fn enc(value: &str) -> String {
    utf8_percent_encode(value, NON_UNRESERVED).to_string()
}

/// The `application/x-www-form-urlencoded` body for the authorization-code
/// token exchange (RFC 6749 §4.1.3) with PKCE: the daemon POSTs this to the
/// token endpoint, presenting the same `code_verifier` whose challenge it sent
/// in the auth URL. Values are percent-encoded.
pub fn authorization_code_form(
    code: &str,
    redirect_uri: &str,
    client_id: &str,
    code_verifier: &str,
) -> String {
    form(&[
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", redirect_uri),
        ("client_id", client_id),
        ("code_verifier", code_verifier),
    ])
}

/// The token-endpoint body for a refresh (RFC 6749 §6): swap a (rotated)
/// refresh token for a fresh access token without user interaction.
pub fn refresh_form(refresh_token: &str, client_id: &str) -> String {
    form(&[
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
        ("client_id", client_id),
    ])
}

/// Build an `application/x-www-form-urlencoded` body from ordered params.
fn form(params: &[(&str, &str)]) -> String {
    params
        .iter()
        .map(|(k, v)| format!("{}={}", enc(k), enc(v)))
        .collect::<Vec<_>>()
        .join("&")
}

/// A successful token response (RFC 6749 §5.1). `refresh_token` is present on
/// the initial exchange and (for a rotating provider) on each refresh; `scope`
/// is present when the granted scope differs from the request.
#[derive(Debug, Clone, Deserialize)]
pub struct TokenResponse {
    /// The bearer access token.
    pub access_token: String,
    /// The token type, normally `Bearer`.
    #[serde(default)]
    pub token_type: String,
    /// Lifetime of the access token in seconds, if the provider states it.
    #[serde(default)]
    pub expires_in: Option<u64>,
    /// The refresh token, if issued/rotated.
    #[serde(default)]
    pub refresh_token: Option<String>,
    /// The granted scope, if it differs from the request.
    #[serde(default)]
    pub scope: Option<String>,
}

/// A token-endpoint failure: either a structured provider error (RFC 6749
/// §5.2) or a body that is neither a success nor a recognisable error.
#[derive(Debug, thiserror::Error)]
pub enum TokenError {
    /// The provider returned an OAuth error object.
    #[error("oauth error '{error}'{}", .description.as_deref().map(|d| format!(": {d}")).unwrap_or_default())]
    Provider {
        /// The RFC 6749 §5.2 error code (e.g. `invalid_grant`).
        error: String,
        /// The optional human-readable description.
        description: Option<String>,
    },
    /// The body could not be parsed as a token success or error response.
    #[error("malformed token response: {0}")]
    Malformed(String),
}

/// Parse a token-endpoint response body. A success object (with a non-empty
/// `access_token`) becomes a [`TokenResponse`]; an error object becomes
/// [`TokenError::Provider`]; anything else is [`TokenError::Malformed`] - never
/// a silently-accepted empty token.
pub fn parse_token_response(body: &str) -> Result<TokenResponse, TokenError> {
    if let Ok(ok) = serde_json::from_str::<TokenResponse>(body) {
        if !ok.access_token.is_empty() {
            return Ok(ok);
        }
    }
    #[derive(Deserialize)]
    struct ErrResp {
        error: String,
        error_description: Option<String>,
    }
    match serde_json::from_str::<ErrResp>(body) {
        Ok(e) => Err(TokenError::Provider {
            error: e.error,
            description: e.error_description,
        }),
        Err(e) => Err(TokenError::Malformed(e.to_string())),
    }
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
        assert!(url.contains("&client_id=client-123"));
        // The redirect's `:` and `/` are encoded (a valid query value).
        assert!(url.contains("&redirect_uri=http%3A%2F%2F127.0.0.1%3A41739%2F"));
        // Spaces in scope are encoded, not left literal.
        assert!(url.contains("&scope=openid%20email%20calendar.readonly"));
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

    #[test]
    fn authorization_code_form_carries_the_grant_and_verifier() {
        let body = authorization_code_form("the-code", "http://127.0.0.1:9/", "cid", "verif-1");
        assert!(body.starts_with("grant_type=authorization_code"));
        assert!(body.contains("&code=the-code"));
        assert!(body.contains("&code_verifier=verif-1"));
        assert!(body.contains("&client_id=cid"));
        assert!(body.contains("&redirect_uri=http%3A%2F%2F127.0.0.1%3A9%2F"));
    }

    #[test]
    fn refresh_form_is_the_refresh_grant() {
        let body = refresh_form("rt-abc", "cid");
        assert_eq!(body, "grant_type=refresh_token&refresh_token=rt-abc&client_id=cid");
    }

    #[test]
    fn parse_token_response_reads_a_success() {
        let ok = parse_token_response(
            r#"{"access_token":"at-1","token_type":"Bearer","expires_in":3600,"refresh_token":"rt-1"}"#,
        )
        .unwrap();
        assert_eq!(ok.access_token, "at-1");
        assert_eq!(ok.token_type, "Bearer");
        assert_eq!(ok.expires_in, Some(3600));
        assert_eq!(ok.refresh_token.as_deref(), Some("rt-1"));

        // A response without a refresh token (a non-rotating refresh) is fine.
        let no_rt = parse_token_response(r#"{"access_token":"at-2","token_type":"Bearer"}"#).unwrap();
        assert!(no_rt.refresh_token.is_none());
    }

    #[test]
    fn parse_token_response_surfaces_a_provider_error() {
        let err = parse_token_response(
            r#"{"error":"invalid_grant","error_description":"code expired"}"#,
        )
        .unwrap_err();
        match err {
            TokenError::Provider { error, description } => {
                assert_eq!(error, "invalid_grant");
                assert_eq!(description.as_deref(), Some("code expired"));
            }
            other => panic!("expected Provider, got {other:?}"),
        }
    }

    #[test]
    fn parse_token_response_rejects_garbage_and_empty_tokens() {
        assert!(matches!(
            parse_token_response("not json"),
            Err(TokenError::Malformed(_))
        ));
        // An access_token that is present-but-empty is not accepted.
        assert!(parse_token_response(r#"{"access_token":""}"#).is_err());
    }
}

//! OA-R2: the authorization-code flow orchestration.
//!
//! Ties the pure pieces together: generate a PKCE pair + a CSRF `state`, build
//! the authorization URL (with the loopback receiver's redirect URI), open the
//! system browser at it, receive the redirect and check the state, then
//! exchange the code for tokens. The two side effects - opening a browser and
//! the HTTPS token POST - are behind the [`Browser`] and [`TokenExchanger`]
//! seams, so the flow logic (sequence, state binding, error propagation) is
//! unit-tested against the REAL loopback receiver plus mocks, and the concrete
//! browser/HTTP implementations are a thin, dep-bearing follow-up.

use crate::loopback::{LoopbackReceiver, RecvError};
use crate::oauth::{
    authorization_code_form, random_state, AuthRequest, PkcePair, TokenResponse,
};

/// Opens a URL in the user's system browser (the concrete impl spawns e.g.
/// `xdg-open`; kept behind a trait so the flow is testable without a browser).
pub trait Browser {
    /// Open `url`. Returns a message on failure.
    fn open(&self, url: &str) -> Result<(), String>;
}

/// Performs the HTTPS token-endpoint POST (`application/x-www-form-urlencoded`
/// body) and returns the parsed response. Behind a trait so the flow is
/// testable without an HTTP client / a live provider.
pub trait TokenExchanger {
    /// POST `form_body` to `token_endpoint` and parse the token response.
    fn exchange(&self, token_endpoint: &str, form_body: &str) -> Result<TokenResponse, String>;
}

/// A provider's OAuth endpoints + the registered client (client id + scope are
/// the human-gated per-provider config).
#[derive(Debug, Clone)]
pub struct ProviderConfig<'a> {
    /// The authorization endpoint (where the browser is sent).
    pub authorization_endpoint: &'a str,
    /// The token endpoint (where the code is exchanged).
    pub token_endpoint: &'a str,
    /// The registered OAuth client id.
    pub client_id: &'a str,
    /// The space-delimited requested scopes.
    pub scope: &'a str,
}

/// An authorization-flow failure.
#[derive(Debug, thiserror::Error)]
pub enum FlowError {
    /// The OS RNG failed generating the PKCE verifier / state.
    #[error("rng: {0}")]
    Rng(getrandom::Error),
    /// Binding / reading the loopback receiver failed.
    #[error("loopback: {0}")]
    Loopback(#[from] RecvError),
    /// The browser could not be opened.
    #[error("browser: {0}")]
    Browser(String),
    /// The token exchange failed (transport or provider error).
    #[error("token exchange: {0}")]
    Exchange(String),
}

impl From<getrandom::Error> for FlowError {
    fn from(e: getrandom::Error) -> Self {
        FlowError::Rng(e)
    }
}

impl From<std::io::Error> for FlowError {
    fn from(e: std::io::Error) -> Self {
        FlowError::Loopback(RecvError::Io(e))
    }
}

/// Run the authorization-code flow for `provider` using the already-bound
/// `receiver` (its port sets the redirect URI). Returns the token response.
///
/// The PKCE `code_verifier` never leaves this call except as the token-exchange
/// binding, and the `state` binds the redirect to this specific request (a
/// mismatched state is a [`RecvError::StateMismatch`]).
pub fn authorize(
    provider: &ProviderConfig,
    receiver: &LoopbackReceiver,
    browser: &dyn Browser,
    exchanger: &dyn TokenExchanger,
) -> Result<TokenResponse, FlowError> {
    let pkce = PkcePair::generate()?;
    let state = random_state()?;
    let redirect_uri = receiver.redirect_uri()?;

    let auth_url = AuthRequest {
        authorization_endpoint: provider.authorization_endpoint,
        client_id: provider.client_id,
        redirect_uri: &redirect_uri,
        scope: provider.scope,
        state: &state,
        code_challenge: &pkce.challenge,
    }
    .url();

    browser.open(&auth_url).map_err(FlowError::Browser)?;
    let code = receiver.recv(&state)?;

    let form = authorization_code_form(&code, &redirect_uri, provider.client_id, &pkce.verifier);
    exchanger
        .exchange(provider.token_endpoint, &form)
        .map_err(FlowError::Exchange)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::io::Write;
    use std::net::TcpStream;

    /// Extract a query-param value from `url` (`...&name=VALUE&...` or `?name=`).
    fn param<'a>(url: &'a str, name: &str) -> Option<&'a str> {
        let q = url.split_once('?')?.1;
        q.split('&')
            .find_map(|p| p.strip_prefix(&format!("{name}=")))
    }

    /// A browser that COMPLETES the redirect: given the auth URL it pulls the
    /// state out and connects to the loopback port to POST the redirect (as a
    /// real browser would), so the single-threaded flow's `recv` picks it up.
    struct CompletingBrowser {
        port: u16,
        code: String,
        opened: RefCell<Option<String>>,
    }
    impl Browser for CompletingBrowser {
        fn open(&self, url: &str) -> Result<(), String> {
            *self.opened.borrow_mut() = Some(url.to_string());
            let state = param(url, "state").ok_or("no state in url")?;
            let mut c = TcpStream::connect(("127.0.0.1", self.port)).map_err(|e| e.to_string())?;
            write!(c, "GET /?code={}&state={} HTTP/1.1\r\n\r\n", self.code, state)
                .map_err(|e| e.to_string())?;
            Ok(())
        }
    }

    struct CannedExchanger {
        response: Result<TokenResponse, String>,
        seen_form: RefCell<Option<String>>,
    }
    impl TokenExchanger for CannedExchanger {
        fn exchange(&self, _endpoint: &str, form_body: &str) -> Result<TokenResponse, String> {
            *self.seen_form.borrow_mut() = Some(form_body.to_string());
            self.response.clone()
        }
    }

    fn provider() -> ProviderConfig<'static> {
        ProviderConfig {
            authorization_endpoint: "https://accounts.example.com/authorize",
            token_endpoint: "https://accounts.example.com/token",
            client_id: "cid",
            scope: "openid email",
        }
    }

    fn token() -> TokenResponse {
        crate::oauth::parse_token_response(
            r#"{"access_token":"at","token_type":"Bearer","expires_in":3600,"refresh_token":"rt"}"#,
        )
        .unwrap()
    }

    #[test]
    fn a_full_flow_returns_the_token_and_binds_the_verifier() {
        let receiver = LoopbackReceiver::bind().unwrap();
        let port = receiver.port().unwrap();
        let browser = CompletingBrowser {
            port,
            code: "the-code".into(),
            opened: RefCell::new(None),
        };
        let exchanger = CannedExchanger {
            response: Ok(token()),
            seen_form: RefCell::new(None),
        };

        let tokens = authorize(&provider(), &receiver, &browser, &exchanger).unwrap();
        assert_eq!(tokens.access_token, "at");
        assert_eq!(tokens.refresh_token.as_deref(), Some("rt"));

        // The browser was opened at the built auth URL (S256 + our redirect).
        let opened = browser.opened.borrow().clone().unwrap();
        assert!(opened.starts_with("https://accounts.example.com/authorize?response_type=code"));
        assert!(opened.contains("&code_challenge_method=S256"));
        // The exchange presented the code + a code_verifier (PKCE binding).
        let form = exchanger.seen_form.borrow().clone().unwrap();
        assert!(form.contains("grant_type=authorization_code"));
        assert!(form.contains("&code=the-code"));
        assert!(form.contains("&code_verifier="));
    }

    #[test]
    fn a_token_exchange_error_propagates() {
        let receiver = LoopbackReceiver::bind().unwrap();
        let port = receiver.port().unwrap();
        let browser = CompletingBrowser {
            port,
            code: "c".into(),
            opened: RefCell::new(None),
        };
        let exchanger = CannedExchanger {
            response: Err("invalid_grant".into()),
            seen_form: RefCell::new(None),
        };
        match authorize(&provider(), &receiver, &browser, &exchanger) {
            Err(FlowError::Exchange(e)) => assert_eq!(e, "invalid_grant"),
            other => panic!("expected Exchange error, got {other:?}"),
        }
    }
}

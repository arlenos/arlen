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
    authorization_code_form, parse_token_response, random_state, AuthRequest, PkcePair,
    TokenResponse,
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

/// The concrete [`Browser`]: opens the authorization URL in the user's default
/// browser via `xdg-open`. No embedded web-view (RFC 8252 §8.12): the system
/// browser is where the user's provider session + password manager live, and it
/// keeps the app out of the credential exchange.
pub struct SystemBrowser;

/// The command + args to open `url` in the system browser. `xdg-open` is the
/// desktop-agnostic launcher; pulled out so the argv is unit-tested (the spawn
/// itself is I/O).
fn open_argv(url: &str) -> [String; 2] {
    ["xdg-open".to_string(), url.to_string()]
}

impl Browser for SystemBrowser {
    fn open(&self, url: &str) -> Result<(), String> {
        let argv = open_argv(url);
        std::process::Command::new(&argv[0])
            .arg(&argv[1])
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("could not launch {}: {e}", argv[0]))
    }
}

/// The concrete [`TokenExchanger`]: POSTs the form to the provider's HTTPS token
/// endpoint via a blocking reqwest client (the flow runs on a blocking thread,
/// so a blocking client avoids a nested async runtime). The production client is
/// HTTPS-only with a bounded timeout; a token exchange is a short, foreground
/// request. Both the success and the RFC 6749 §5.2 error body are handled by
/// [`parse_token_response`], so the HTTP status is not separately branched.
pub struct HttpExchanger {
    client: reqwest::blocking::Client,
}

impl HttpExchanger {
    /// Build an HTTPS-only client for real token endpoints.
    pub fn new() -> Result<Self, String> {
        let client = reqwest::blocking::Client::builder()
            .https_only(true)
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self { client })
    }

    /// A client without the HTTPS-only guard, for a local plain-HTTP mock.
    #[cfg(test)]
    fn insecure_for_test() -> Self {
        Self {
            client: reqwest::blocking::Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .unwrap(),
        }
    }
}

impl TokenExchanger for HttpExchanger {
    fn exchange(&self, token_endpoint: &str, form_body: &str) -> Result<TokenResponse, String> {
        let resp = self
            .client
            .post(token_endpoint)
            .header(
                reqwest::header::CONTENT_TYPE,
                "application/x-www-form-urlencoded",
            )
            .header(reqwest::header::ACCEPT, "application/json")
            .body(form_body.to_string())
            .send()
            .map_err(|e| format!("token endpoint POST: {e}"))?;
        let body = resp.text().map_err(|e| format!("read token response: {e}"))?;
        parse_token_response(&body).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::io::Write;
    use std::net::TcpStream;

    /// A one-shot mock token endpoint: reads the full request (headers + the
    /// Content-Length body, so the socket closes cleanly), then replies with
    /// `body` as JSON. Returns the bound port.
    fn spawn_token_server(body: &'static str) -> u16 {
        use std::io::Read;
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut head = Vec::new();
            let mut byte = [0u8; 1];
            while !head.ends_with(b"\r\n\r\n") && head.len() < 8192 {
                if stream.read(&mut byte).unwrap_or(0) == 0 {
                    return;
                }
                head.push(byte[0]);
            }
            let text = String::from_utf8_lossy(&head).to_ascii_lowercase();
            let clen: usize = text
                .lines()
                .find_map(|l| l.strip_prefix("content-length:"))
                .and_then(|v| v.trim().parse().ok())
                .unwrap_or(0);
            let mut b = vec![0u8; clen];
            let _ = stream.read_exact(&mut b);
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(resp.as_bytes());
        });
        port
    }

    #[test]
    fn the_http_exchanger_posts_the_form_and_parses_the_token() {
        let port =
            spawn_token_server(r#"{"access_token":"live-at","token_type":"Bearer","expires_in":3599}"#);
        let ex = HttpExchanger::insecure_for_test();
        let tokens = ex
            .exchange(
                &format!("http://127.0.0.1:{port}/token"),
                "grant_type=authorization_code&code=x&client_id=c",
            )
            .unwrap();
        assert_eq!(tokens.access_token, "live-at");
        assert_eq!(tokens.expires_in, Some(3599));
    }

    #[test]
    fn the_browser_opens_the_url_with_xdg_open() {
        let argv = open_argv("https://accounts.example.com/authorize?x=1");
        assert_eq!(argv[0], "xdg-open");
        assert_eq!(argv[1], "https://accounts.example.com/authorize?x=1");
    }

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

//! Session authentication for the ai-engine-daemon (`pi-agent-adoption.md` trust
//! boundary).
//!
//! The daemon does not trust the agent engine (pi) or its plugins. At
//! `SessionInit` the daemon mints an opaque, unguessable session token and binds
//! it to (a) the session's server-side grant derived from the init and (b) the
//! SO_PEERCRED-attested pid of the engine process. Every subsequent verb
//! (Authorize/Execute/Report) must present that token AND arrive from the bound
//! pid, so the daemon resolves the grant server-side rather than believing
//! anything the engine sends. A compromised engine is therefore bounded to
//! exactly its granted authority, no more.

use ai_engine_contract::{CapabilityContext, ReadTier, SessionInit};
use std::collections::HashMap;

/// Bytes of CSPRNG entropy in a session token (256 bits, hex-encoded).
const TOKEN_BYTES: usize = 32;

/// An opaque, unguessable session handle minted by the daemon at `SessionInit`.
/// It is only ever compared for equality; it carries no authority by itself
/// (the daemon also checks the calling pid), so leaking it to a different
/// process does not grant that process the session.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SessionToken(String);

impl SessionToken {
    /// The token as the wire string the engine echoes on each verb.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The server-side authority a session is bound to, derived from its
/// `SessionInit`. The engine never sees more than this, and the daemon enforces
/// it on every call rather than trusting the engine's claims.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionGrant {
    /// The coarse tool authority granted to the session.
    pub capability_context: CapabilityContext,
    /// The active-project anchor bounding graph reads (the GAP-21 fix).
    pub project_anchor: Option<String>,
    /// How much of the graph the session may read.
    pub read_tier: ReadTier,
    /// The SO_PEERCRED-attested pid of the engine process bound at creation.
    /// A verb whose calling pid differs is rejected even with a valid token.
    pub pid: u32,
}

/// Why resolving a session grant failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionError {
    /// No session exists for the presented token.
    UnknownToken,
    /// The token exists but the calling pid is not the one it was bound to.
    PidMismatch,
}

/// The daemon's in-memory table of live sessions. Not `Clone` (a single owner,
/// behind the daemon's lock); tokens are the only handles.
#[derive(Default)]
pub struct SessionStore {
    sessions: HashMap<String, SessionGrant>,
}

impl SessionStore {
    /// An empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Mint a fresh token for a new session, binding the init's grant to `pid`
    /// (the SO_PEERCRED-attested engine process). Returns the token the engine
    /// must echo on every subsequent verb.
    pub fn create(&mut self, init: &SessionInit, pid: u32) -> Result<SessionToken, CsprngError> {
        let token = mint_token()?;
        self.sessions.insert(
            token.0.clone(),
            SessionGrant {
                capability_context: init.capability_context.clone(),
                project_anchor: init.project_anchor.clone(),
                read_tier: init.read_tier,
                pid,
            },
        );
        Ok(token)
    }

    /// Resolve the grant for a verb call, fail-closed: the token must exist AND
    /// the calling `pid` must match the pid the session was bound to. This is
    /// the server-side authority bound - a stolen token from another process
    /// (different pid) is refused.
    pub fn grant_for(&self, token: &SessionToken, pid: u32) -> Result<&SessionGrant, SessionError> {
        let grant = self.sessions.get(&token.0).ok_or(SessionError::UnknownToken)?;
        if grant.pid != pid {
            return Err(SessionError::PidMismatch);
        }
        Ok(grant)
    }

    /// End a session (idempotent). After this the token resolves to
    /// `UnknownToken`.
    pub fn end(&mut self, token: &SessionToken) {
        self.sessions.remove(&token.0);
    }

    /// The number of live sessions.
    pub fn len(&self) -> usize {
        self.sessions.len()
    }

    /// Whether there are no live sessions.
    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }
}

/// A CSPRNG failure while minting a token (fail-closed: no token is issued).
#[derive(Debug, Clone)]
pub struct CsprngError(pub String);

impl std::fmt::Display for CsprngError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "session token CSPRNG failure: {}", self.0)
    }
}

impl std::error::Error for CsprngError {}

/// Mint a 256-bit CSPRNG token, hex-encoded.
fn mint_token() -> Result<SessionToken, CsprngError> {
    let mut bytes = [0u8; TOKEN_BYTES];
    getrandom::getrandom(&mut bytes).map_err(|e| CsprngError(e.to_string()))?;
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    Ok(SessionToken(hex))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn init() -> SessionInit {
        SessionInit {
            system_prompt: "p".into(),
            behaviour: None,
            capability_context: CapabilityContext {
                generic_tools: vec!["bash".into()],
                proxy_tools: vec!["graph.read".into()],
            },
            project_anchor: Some("proj-1".into()),
            read_tier: ReadTier::Standard,
        }
    }

    #[test]
    fn a_session_resolves_only_for_its_bound_pid() {
        let mut store = SessionStore::new();
        let token = store.create(&init(), 4242).unwrap();

        // Right token + right pid: the bound grant.
        let grant = store.grant_for(&token, 4242).unwrap();
        assert_eq!(grant.project_anchor.as_deref(), Some("proj-1"));
        assert_eq!(grant.read_tier, ReadTier::Standard);
        assert_eq!(grant.capability_context.proxy_tools, vec!["graph.read".to_string()]);

        // Right token, WRONG pid (a stolen token from another process): refused.
        assert_eq!(store.grant_for(&token, 9999), Err(SessionError::PidMismatch));
    }

    #[test]
    fn an_unknown_token_is_refused() {
        let store = SessionStore::new();
        let fake = SessionToken("deadbeef".into());
        assert_eq!(store.grant_for(&fake, 4242), Err(SessionError::UnknownToken));
    }

    #[test]
    fn ending_a_session_revokes_the_token() {
        let mut store = SessionStore::new();
        let token = store.create(&init(), 1).unwrap();
        assert!(store.grant_for(&token, 1).is_ok());
        store.end(&token);
        assert_eq!(store.grant_for(&token, 1), Err(SessionError::UnknownToken));
        assert!(store.is_empty());
        // End is idempotent.
        store.end(&token);
    }

    #[test]
    fn tokens_are_distinct_and_long() {
        let mut store = SessionStore::new();
        let a = store.create(&init(), 1).unwrap();
        let b = store.create(&init(), 1).unwrap();
        assert_ne!(a, b, "each session gets a fresh token");
        assert_eq!(a.as_str().len(), TOKEN_BYTES * 2, "256-bit hex token");
        assert!(a.as_str().chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(store.len(), 2);
    }
}

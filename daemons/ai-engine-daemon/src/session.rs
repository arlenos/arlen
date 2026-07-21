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
use arlen_permissions::identity::pid_start_time;
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

    /// Reconstruct a token from the wire string the engine echoed on a verb.
    /// The token carries no authority by itself (the store also checks the pid),
    /// so reconstructing one the store never minted simply resolves to
    /// `UnknownToken`.
    pub fn from_wire(raw: String) -> Self {
        SessionToken(raw)
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
    /// Whether the whole run was started by external content (HIGH-2, from the
    /// supervisor-set [`SessionInit::externally_triggered`]). The gate ORs it with
    /// each call's own flag (escalate-only), so an externally-originated session
    /// escalates every action regardless of the engine's per-call claim.
    pub externally_triggered: bool,
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
    /// Per-token start-time of the bound pid (recycle-proofing). Kept parallel to
    /// `sessions` so a session's authority grant stays a pure value, while binding
    /// integrity (is this pid still the process it was bound to?) lives in the
    /// store. A bare pid compare admits a same-uid process that reuses the bound
    /// pid after the engine exits; comparing the pid's start-time as well rejects
    /// it, since a recycled pid belongs to a different process with a different
    /// start-time.
    start_times: HashMap<String, u64>,
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
        self.bind(token.clone(), init, pid);
        Ok(token)
    }

    /// Bind a pre-minted `token` to a session's grant + `pid`. The supervisor
    /// mints the token before spawning the engine (so it can pass it in the
    /// child's env) and binds it here once the spawned pid is known.
    pub fn bind(&mut self, token: SessionToken, init: &SessionInit, pid: u32) {
        // Capture the bound pid's start-time so a later verb from a recycled pid
        // (a different process that reused this pid after the engine exited) is
        // rejected even with a valid token. A read failure here (the just-spawned
        // pid already gone) stores a sentinel that no live pid's start-time can
        // match, so the session is dead-on-arrival - fail-closed, never fail-open.
        let start_time = pid_start_time(pid).unwrap_or(u64::MAX);
        self.bind_checked(token, init, pid, start_time);
    }

    /// Bind with an explicit start-time (the pure core; [`bind`] reads the live
    /// start-time and delegates here). Separated so the recycle-proofing is
    /// unit-testable without a live pid.
    fn bind_checked(&mut self, token: SessionToken, init: &SessionInit, pid: u32, start_time: u64) {
        self.start_times.insert(token.0.clone(), start_time);
        self.sessions.insert(
            token.0,
            SessionGrant {
                capability_context: init.capability_context.clone(),
                project_anchor: init.project_anchor.clone(),
                read_tier: init.read_tier,
                externally_triggered: init.externally_triggered,
                pid,
            },
        );
    }

    /// Resolve the grant for a verb call, fail-closed: the token must exist AND
    /// the calling `pid` must match the pid the session was bound to. This is
    /// the server-side authority bound - a stolen token from another process
    /// (different pid) is refused.
    pub fn grant_for(&self, token: &SessionToken, pid: u32) -> Result<&SessionGrant, SessionError> {
        // Re-read the calling pid's start-time now; a pid recycled to a different
        // process has a different start-time. A read failure (the pid already gone)
        // is fail-closed: no live process is presenting, so nothing is authorized.
        let now_start = pid_start_time(pid).map_err(|_| SessionError::PidMismatch)?;
        self.grant_for_checked(token, pid, now_start)
    }

    /// Resolve with an explicit current start-time (the pure core; [`grant_for`]
    /// reads the live start-time and delegates here). Fail-closed: the token must
    /// exist, the calling pid must match, AND that pid's start-time must equal the
    /// one bound at creation, else the pid was reused by another process after the
    /// engine exited. Separated so the recycle-proofing is unit-testable.
    fn grant_for_checked(
        &self,
        token: &SessionToken,
        pid: u32,
        now_start: u64,
    ) -> Result<&SessionGrant, SessionError> {
        let grant = self.sessions.get(&token.0).ok_or(SessionError::UnknownToken)?;
        if grant.pid != pid {
            return Err(SessionError::PidMismatch);
        }
        // A missing start-time entry cannot happen while `sessions` holds the token
        // (bind writes both); a sentinel that no live start-time matches keeps it
        // fail-closed if it ever did.
        let bound_start = self.start_times.get(&token.0).copied().unwrap_or(u64::MAX);
        if now_start != bound_start {
            return Err(SessionError::PidMismatch);
        }
        Ok(grant)
    }

    /// End a session (idempotent). After this the token resolves to
    /// `UnknownToken`.
    pub fn end(&mut self, token: &SessionToken) {
        self.sessions.remove(&token.0);
        self.start_times.remove(&token.0);
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

impl SessionToken {
    /// Mint a fresh 256-bit CSPRNG token (the supervisor mints one before
    /// spawning the engine, to pass in its env, then binds it via
    /// [`SessionStore::bind`]).
    pub fn mint() -> Result<SessionToken, CsprngError> {
        mint_token()
    }
}

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
            externally_triggered: false,
        }
    }

    #[test]
    fn a_session_resolves_only_for_its_bound_pid_and_start_time() {
        let mut store = SessionStore::new();
        let token = SessionToken::mint().unwrap();
        // Bind to pid 4242 with start-time 100 (explicit, no live pid needed).
        store.bind_checked(token.clone(), &init(), 4242, 100);

        // Right token + right pid + right start-time: the bound grant.
        let grant = store.grant_for_checked(&token, 4242, 100).unwrap();
        assert_eq!(grant.project_anchor.as_deref(), Some("proj-1"));
        assert_eq!(grant.read_tier, ReadTier::Standard);
        assert_eq!(grant.capability_context.proxy_tools, vec!["graph.read".to_string()]);

        // Right token, WRONG pid (a stolen token from another process): refused.
        assert_eq!(store.grant_for_checked(&token, 9999, 100), Err(SessionError::PidMismatch));
    }

    #[test]
    fn a_recycled_pid_with_a_different_start_time_is_refused() {
        // The recycle-proofing: after the engine exits, a same-uid process reuses
        // the bound pid number. Even holding a valid token AND matching the pid
        // number, a different process (different start-time) is rejected.
        let mut store = SessionStore::new();
        let token = SessionToken::mint().unwrap();
        store.bind_checked(token.clone(), &init(), 4242, 100);
        // Same token, same pid number, but the pid now has a different start-time.
        assert_eq!(store.grant_for_checked(&token, 4242, 200), Err(SessionError::PidMismatch));
        // The genuine engine (matching start-time) still resolves.
        assert!(store.grant_for_checked(&token, 4242, 100).is_ok());
    }

    #[test]
    fn an_unknown_token_is_refused() {
        let store = SessionStore::new();
        let fake = SessionToken("deadbeef".into());
        assert_eq!(store.grant_for_checked(&fake, 4242, 100), Err(SessionError::UnknownToken));
    }

    #[test]
    fn ending_a_session_revokes_the_token_and_its_start_time() {
        let mut store = SessionStore::new();
        let token = SessionToken::mint().unwrap();
        store.bind_checked(token.clone(), &init(), 1, 50);
        assert!(store.grant_for_checked(&token, 1, 50).is_ok());
        store.end(&token);
        assert_eq!(store.grant_for_checked(&token, 1, 50), Err(SessionError::UnknownToken));
        assert!(store.is_empty());
        assert!(store.start_times.is_empty(), "the start-time entry is cleared too");
        // End is idempotent.
        store.end(&token);
    }

    #[test]
    fn tokens_are_distinct_and_long() {
        let mut store = SessionStore::new();
        let a = SessionToken::mint().unwrap();
        let b = SessionToken::mint().unwrap();
        store.bind_checked(a.clone(), &init(), 1, 10);
        store.bind_checked(b.clone(), &init(), 1, 10);
        assert_ne!(a, b, "each session gets a fresh token");
        assert_eq!(a.as_str().len(), TOKEN_BYTES * 2, "256-bit hex token");
        assert!(a.as_str().chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(store.len(), 2);
    }

    #[test]
    fn create_and_resolve_over_the_live_pid_path() {
        // Exercises the real I/O path (bind + grant_for read /proc start-time) using
        // the test process's own live pid, so the start-time read succeeds and the
        // round-trip resolves. A pid whose number differs is refused.
        let mut store = SessionStore::new();
        let me = std::process::id();
        let token = store.create(&init(), me).unwrap();
        assert!(store.grant_for(&token, me).is_ok());
        assert!(store.grant_for(&token, me.wrapping_add(1)).is_err());
    }
}

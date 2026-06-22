//! The verb-dispatch core (`pi-agent-adoption.md` §A).
//!
//! Every contract verb from the engine resolves its session grant FIRST -
//! [`SessionStore::grant_for`], which requires both a valid token and the
//! calling pid the session was bound to - and only then reaches the gate /
//! executor / reporter seams. A verb with no valid session fails closed
//! (Authorize -> Deny, Execute -> Error, Report -> Block), so the engine can
//! never act around the session bound. The three seams are traits the daemon
//! binary wires to the real Rust in Phase 1 (Gate -> `Capability::decide`,
//! Executor -> the trusted privileged-tool runner, Reporter -> audit +
//! compensation + S17/S18 screening); here they are mockable so the
//! security-routing is unit-tested without any of that machinery.

use crate::session::{SessionGrant, SessionStore, SessionToken};
use ai_engine_contract::{
    Authorize, AuthorizeDecision, Call, ContractCall, ContractError, Execute, ExecuteOutcome, Report,
    ReportAck, Reply, ScreenVerdict, SessionInit,
};
use async_trait::async_trait;

/// Decides whether a proposed tool call is allowed. Maps to `Capability::decide`.
#[async_trait]
pub trait Gate: Send + Sync {
    /// Authorize `req` for a session holding `grant`.
    async fn authorize(&self, req: &Authorize, grant: &SessionGrant) -> AuthorizeDecision;
}

/// Runs a PRIVILEGED tool in trusted Rust (the engine never touches the KG/OS).
#[async_trait]
pub trait Executor: Send + Sync {
    /// Execute `req` for a session holding `grant`.
    async fn execute(&self, req: &Execute, grant: &SessionGrant) -> ExecuteOutcome;
}

/// Records a tool result (audit + compensation) and screens its content.
#[async_trait]
pub trait Reporter: Send + Sync {
    /// Report `req` for a session holding `grant`, returning the screen verdict.
    async fn report(&self, req: &Report, grant: &SessionGrant) -> ReportAck;
}

/// Routes the contract verbs through the session bound and the seams. All
/// methods take `&self` (so it is shared as an `Arc` across per-connection
/// tasks); the session store is behind a `Mutex` whose guard is always dropped
/// BEFORE awaiting a seam (never held across an `.await`), and the bound grant
/// is cloned out under the lock.
pub struct Dispatcher<G, E, R> {
    gate: G,
    executor: E,
    reporter: R,
    sessions: std::sync::Mutex<SessionStore>,
}

impl<G: Gate, E: Executor, R: Reporter> Dispatcher<G, E, R> {
    /// Build a dispatcher over the three seams with an empty session store.
    pub fn new(gate: G, executor: E, reporter: R) -> Self {
        Self { gate, executor, reporter, sessions: std::sync::Mutex::new(SessionStore::new()) }
    }

    /// Mint a session for an authenticated engine process (pid is the
    /// SO_PEERCRED-attested value). Returns the token the engine echoes.
    pub fn init_session(
        &self,
        init: &SessionInit,
        pid: u32,
    ) -> Result<SessionToken, crate::session::CsprngError> {
        self.sessions.lock().unwrap().create(init, pid)
    }

    /// Bind a pre-minted token to a session (the supervisor minted it before
    /// spawning the engine and learned the spawned pid afterward).
    pub fn bind_session(&self, token: SessionToken, init: &SessionInit, pid: u32) {
        self.sessions.lock().unwrap().bind(token, init, pid);
    }

    /// End a session (idempotent).
    pub fn end_session(&self, token: &SessionToken) {
        self.sessions.lock().unwrap().end(token);
    }

    /// The number of live sessions (for supervision/diagnostics).
    pub fn session_count(&self) -> usize {
        self.sessions.lock().unwrap().len()
    }

    /// Resolve the bound grant for `(token, pid)`, cloning it out under the lock
    /// so the guard is released before any seam await.
    fn resolve(&self, token: &SessionToken, pid: u32) -> Option<SessionGrant> {
        self.sessions.lock().unwrap().grant_for(token, pid).ok().cloned()
    }

    /// Authorize a tool call. A verb with no valid session for `(token, pid)`
    /// is denied without consulting the gate.
    pub async fn authorize(
        &self,
        token: &SessionToken,
        pid: u32,
        req: &Authorize,
    ) -> AuthorizeDecision {
        match self.resolve(token, pid) {
            Some(grant) => self.gate.authorize(req, &grant).await,
            None => AuthorizeDecision::Deny {
                reason: "no valid session for this caller".to_string(),
            },
        }
    }

    /// Execute a privileged tool. No valid session fails closed as a permission
    /// error; the executor is never reached.
    pub async fn execute(&self, token: &SessionToken, pid: u32, req: &Execute) -> ExecuteOutcome {
        match self.resolve(token, pid) {
            Some(grant) => self.executor.execute(req, &grant).await,
            None => ExecuteOutcome::Error {
                code: ContractError::PermissionDenied,
                message: "no valid session for this caller".to_string(),
            },
        }
    }

    /// Report a tool result. No valid session fails closed by BLOCKING the
    /// content (it never re-enters the engine's context) and skipping audit.
    pub async fn report(&self, token: &SessionToken, pid: u32, req: &Report) -> ReportAck {
        match self.resolve(token, pid) {
            Some(grant) => self.reporter.report(req, &grant).await,
            None => ReportAck { screen: ScreenVerdict::Block },
        }
    }

    /// Route one wire [`ContractCall`] from a connection whose SO_PEERCRED pid
    /// is `pid` to the matching verb, returning the wire [`Reply`].
    pub async fn handle_call(&self, call: ContractCall, pid: u32) -> Reply {
        let token = SessionToken::from_wire(call.token);
        match call.call {
            Call::Authorize(req) => Reply::Authorize(self.authorize(&token, pid, &req).await),
            Call::Execute(req) => Reply::Execute(self.execute(&token, pid, &req).await),
            Call::Report(req) => Reply::Report(self.report(&token, pid, &req).await),
            Call::EndSession => {
                self.end_session(&token);
                Reply::Ack
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ai_engine_contract::{CapabilityContext, ReadTier};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    /// A gate that records calls + echoes the bound grant's project anchor into
    /// its decision reason, so a test can prove the right grant was threaded.
    struct SpyGate {
        calls: Arc<AtomicUsize>,
        decision: AuthorizeDecision,
    }
    #[async_trait]
    impl Gate for SpyGate {
        async fn authorize(&self, _req: &Authorize, grant: &SessionGrant) -> AuthorizeDecision {
            self.calls.fetch_add(1, Ordering::SeqCst);
            match &self.decision {
                AuthorizeDecision::Deny { .. } => AuthorizeDecision::Deny {
                    reason: format!("anchor={:?}", grant.project_anchor),
                },
                other => other.clone(),
            }
        }
    }

    struct SpyExecutor {
        calls: Arc<AtomicUsize>,
    }
    #[async_trait]
    impl Executor for SpyExecutor {
        async fn execute(&self, _req: &Execute, _grant: &SessionGrant) -> ExecuteOutcome {
            self.calls.fetch_add(1, Ordering::SeqCst);
            ExecuteOutcome::Ok { result: serde_json::json!({"ran": true}) }
        }
    }

    struct SpyReporter {
        calls: Arc<AtomicUsize>,
    }
    #[async_trait]
    impl Reporter for SpyReporter {
        async fn report(&self, _req: &Report, _grant: &SessionGrant) -> ReportAck {
            self.calls.fetch_add(1, Ordering::SeqCst);
            ReportAck { screen: ScreenVerdict::Clean }
        }
    }

    fn init() -> SessionInit {
        SessionInit {
            system_prompt: "p".into(),
            behaviour: None,
            capability_context: CapabilityContext { generic_tools: vec![], proxy_tools: vec![] },
            project_anchor: Some("proj-9".into()),
            read_tier: ReadTier::Minimal,
        }
    }

    type TestRig = (Dispatcher<SpyGate, SpyExecutor, SpyReporter>, Arc<AtomicUsize>, Arc<AtomicUsize>, Arc<AtomicUsize>);

    fn dispatcher() -> TestRig {
        let g = Arc::new(AtomicUsize::new(0));
        let e = Arc::new(AtomicUsize::new(0));
        let r = Arc::new(AtomicUsize::new(0));
        let d = Dispatcher::new(
            SpyGate { calls: g.clone(), decision: AuthorizeDecision::Allow },
            SpyExecutor { calls: e.clone() },
            SpyReporter { calls: r.clone() },
        );
        (d, g, e, r)
    }

    #[tokio::test]
    async fn a_valid_session_reaches_each_seam() {
        let (d, g, e, r) = dispatcher();
        let token = d.init_session(&init(), 100).unwrap();

        let dec = d.authorize(&token, 100, &Authorize {
            tool_name: "bash".into(),
            tool_input: serde_json::json!({}),
            external_triggered: false,
        }).await;
        assert_eq!(dec, AuthorizeDecision::Allow);
        assert_eq!(g.load(Ordering::SeqCst), 1);

        let out = d.execute(&token, 100, &Execute {
            tool_name: "graph.read".into(),
            tool_input: serde_json::json!({}),
        }).await;
        assert!(matches!(out, ExecuteOutcome::Ok { .. }));
        assert_eq!(e.load(Ordering::SeqCst), 1);

        let ack = d.report(&token, 100, &Report {
            tool_name: "graph.read".into(),
            tool_call_id: "c1".into(),
            result: serde_json::json!({}),
            is_error: false,
        }).await;
        assert_eq!(ack.screen, ScreenVerdict::Clean);
        assert_eq!(r.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn the_bound_grant_is_threaded_to_the_gate() {
        let g = Arc::new(AtomicUsize::new(0));
        let d = Dispatcher::new(
            SpyGate { calls: g.clone(), decision: AuthorizeDecision::Deny { reason: String::new() } },
            SpyExecutor { calls: Arc::new(AtomicUsize::new(0)) },
            SpyReporter { calls: Arc::new(AtomicUsize::new(0)) },
        );
        let token = d.init_session(&init(), 100).unwrap();
        let dec = d.authorize(&token, 100, &Authorize {
            tool_name: "bash".into(),
            tool_input: serde_json::json!({}),
            external_triggered: false,
        }).await;
        // The gate saw the session's bound grant (project anchor proj-9).
        assert_eq!(dec, AuthorizeDecision::Deny { reason: "anchor=Some(\"proj-9\")".into() });
    }

    #[tokio::test]
    async fn no_valid_session_fails_closed_without_touching_seams() {
        let (d, g, e, r) = dispatcher();
        let token = d.init_session(&init(), 100).unwrap();
        let bad = SessionToken_for_test();

        // Unknown token -> Deny / Error / Block, seams untouched.
        let dec = d.authorize(&bad, 100, &authz()).await;
        assert!(matches!(dec, AuthorizeDecision::Deny { .. }));
        let out = d.execute(&bad, 100, &exec()).await;
        assert!(matches!(out, ExecuteOutcome::Error { code: ContractError::PermissionDenied, .. }));
        let ack = d.report(&bad, 100, &report()).await;
        assert_eq!(ack.screen, ScreenVerdict::Block);

        // A real token from the WRONG pid is likewise refused at every verb.
        let dec = d.authorize(&token, 999, &authz()).await;
        assert!(matches!(dec, AuthorizeDecision::Deny { .. }));
        let out = d.execute(&token, 999, &exec()).await;
        assert!(matches!(out, ExecuteOutcome::Error { .. }));
        let ack = d.report(&token, 999, &report()).await;
        assert_eq!(ack.screen, ScreenVerdict::Block);

        assert_eq!(g.load(Ordering::SeqCst), 0, "gate never reached");
        assert_eq!(e.load(Ordering::SeqCst), 0, "executor never reached");
        assert_eq!(r.load(Ordering::SeqCst), 0, "reporter never reached");
    }

    #[tokio::test]
    async fn ending_a_session_fails_subsequent_verbs_closed() {
        let (d, _g, _e, _r) = dispatcher();
        let token = d.init_session(&init(), 100).unwrap();
        d.end_session(&token);
        assert!(matches!(d.authorize(&token, 100, &authz()).await, AuthorizeDecision::Deny { .. }));
    }

    // Helpers building throwaway requests.
    fn authz() -> Authorize {
        Authorize { tool_name: "bash".into(), tool_input: serde_json::json!({}), external_triggered: false }
    }
    fn exec() -> Execute {
        Execute { tool_name: "graph.read".into(), tool_input: serde_json::json!({}) }
    }
    fn report() -> Report {
        Report { tool_name: "graph.read".into(), tool_call_id: "c".into(), result: serde_json::json!({}), is_error: false }
    }
    #[allow(non_snake_case)]
    fn SessionToken_for_test() -> SessionToken {
        // A token value the store never minted.
        crate::session::SessionToken::from_wire("0".repeat(64))
    }
}

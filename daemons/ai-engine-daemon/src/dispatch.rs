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

use crate::consent::{ConsentDriver, DeniedConsent};
use crate::session::{SessionGrant, SessionStore, SessionToken};
use arlen_run_consent_token::RUN_COMMAND_TOOL;
use ai_engine_contract::{
    Authorize, AuthorizeDecision, Call, ConfirmAnswer, ContractCall, ContractError, Execute,
    ExecuteOutcome, Report, ReportAck, Reply, ScreenVerdict, SessionInit,
};
use async_trait::async_trait;
use std::sync::Arc;

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

/// Mints the per-action consent token for `run_command` when its `Confirm` is
/// approved. `run_command` runs in a SEPARATE process (the terminal-run MCP
/// server), so the daemon's in-memory execution proof cannot be verified there; a
/// public-key-verifiable biscuit bound to the exact command + args crosses that
/// boundary instead. The daemon binary wires the real, keypair-backed minter; the
/// dispatch stays decoupled from key custody. A `None` return fails closed (no
/// credential -> the MCP server refuses to run).
pub trait ConsentMinter: Send + Sync {
    /// Mint a consent token binding exactly `command` + `args`, or `None` if
    /// minting is unavailable (then the run cannot be authorized at the MCP
    /// boundary).
    fn mint_run(&self, command: &str, args: &[String]) -> Option<String>;
}

/// Extract `(command, args)` from a `run_command` tool input, matching the
/// terminal-run MCP server's own parse EXACTLY so the daemon's minted digest and
/// the server's verified digest agree: a non-empty string `command`, and `args`
/// absent (empty) or an array of strings. Any deviation returns `None` (no token
/// is minted; the MCP server, parsing the same input, also refuses).
fn extract_run_argv(tool_input: &serde_json::Value) -> Option<(String, Vec<String>)> {
    let command = tool_input
        .get("command")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())?
        .to_string();
    let args = match tool_input.get("args") {
        None => Vec::new(),
        Some(v) => {
            let arr = v.as_array()?;
            let mut out = Vec::with_capacity(arr.len());
            for a in arr {
                out.push(a.as_str()?.to_string());
            }
            out
        }
    };
    Some((command, args))
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
    consent: Arc<dyn ConsentDriver>,
    /// Mints the `run_command` consent biscuit on an approved Confirm. `None` when
    /// no keypair is wired (then a `run_command` Allow carries no credential and the
    /// MCP server fails closed).
    consent_minter: Option<Arc<dyn ConsentMinter>>,
    sessions: std::sync::Mutex<SessionStore>,
    /// Live one-time execution proofs (HIGH-1): Authorize mints, Execute consumes.
    proofs: std::sync::Mutex<crate::execution_proof::ProofStore>,
    /// Monotonic epoch for proof TTLs (elapsed-since-start, immune to clock jumps).
    proof_epoch: std::time::Instant,
}

/// How long an execution proof is valid after Authorize (the engine executes
/// immediately, so a minute is generous; a stale proof forces a re-Authorize).
const PROOF_TTL_MS: u64 = 60_000;

impl<G: Gate, E: Executor, R: Reporter> Dispatcher<G, E, R> {
    /// Build a dispatcher over the three seams with an empty session store. The
    /// consent surface defaults to the fail-closed [`DeniedConsent`]; the daemon
    /// binary swaps in the real consent-broker client via [`Self::with_consent`].
    pub fn new(gate: G, executor: E, reporter: R) -> Self {
        Self {
            gate,
            executor,
            reporter,
            consent: Arc::new(DeniedConsent),
            consent_minter: None,
            sessions: std::sync::Mutex::new(SessionStore::new()),
            proofs: std::sync::Mutex::new(crate::execution_proof::ProofStore::new()),
            proof_epoch: std::time::Instant::now(),
        }
    }

    /// Monotonic milliseconds since the dispatcher started (proof TTL clock).
    fn now_ms(&self) -> u64 {
        self.proof_epoch.elapsed().as_millis() as u64
    }

    /// Mint a one-time execution proof for an admitted call, bound to the tool,
    /// the exact args, and the session. Returns `None` if the CSPRNG is
    /// unavailable (fail-closed: no proof means Execute is refused). Also sweeps
    /// expired proofs so the store stays bounded.
    fn mint_proof(&self, tool_name: &str, tool_input: &serde_json::Value, session: &str) -> Option<String> {
        let handle = crate::execution_proof::new_handle().ok()?;
        let args_hash = crate::execution_proof::hash_args(tool_input);
        let now = self.now_ms();
        let mut store = self.proofs.lock().unwrap();
        store.sweep(now);
        // Flood backstop: if the store is at capacity after sweeping expired proofs,
        // refuse to mint (this call gets no proof and its Execute fails closed)
        // rather than grow unbounded. Only reachable by an authenticated engine
        // spamming admitted Authorize without executing.
        if store.len() >= crate::execution_proof::MAX_PROOFS {
            return None;
        }
        store.mint(
            handle.clone(),
            tool_name.to_string(),
            args_hash,
            session.to_string(),
            now + PROOF_TTL_MS,
        );
        Some(handle)
    }

    /// Wire the trusted-path consent surface used to resolve a gate `Confirm`
    /// (the requester side of the #9 consent-broker). Without this, a `Confirm`
    /// resolves through the fail-closed default (every confirmation denied).
    pub fn with_consent(mut self, consent: Arc<dyn ConsentDriver>) -> Self {
        self.consent = consent;
        self
    }

    /// Wire the `run_command` consent-token minter. Without it, an approved
    /// `run_command` Confirm carries no consent credential, so the terminal-run MCP
    /// server (which verifies the biscuit at its boundary) fails closed.
    pub fn with_consent_minter(mut self, minter: Arc<dyn ConsentMinter>) -> Self {
        self.consent_minter = Some(minter);
        self
    }

    /// The credential an admitted call carries in its `Allow`/`Modify` proof.
    /// `run_command` is a proxy tool run by the SEPARATE terminal-run MCP server,
    /// which the daemon's in-memory proof cannot reach, so its credential is the
    /// consent biscuit bound to the exact `(command, args)`; an unparseable argv or
    /// an unwired minter yields `None` (the MCP server then fails closed). Every
    /// other tool keeps the daemon-side one-time execution proof.
    ///
    /// `confirmed` is whether this authorization was resolved through a user
    /// Confirm. The `run_command` biscuit is minted ONLY when `confirmed` is true:
    /// the gate always classifies `run_command` as Confirm, so this holds today, and
    /// the guard makes the consent boundary unbypassable even if a future gate ever
    /// returns Allow directly for it. (`timeout_ms`, present in the tool input, is
    /// deliberately NOT part of the consent digest - consent binds the command and
    /// its arguments; the timeout only bounds runtime and is capped at the MCP
    /// boundary.)
    fn proof_for_allow(
        &self,
        tool_name: &str,
        tool_input: &serde_json::Value,
        session: &str,
        confirmed: bool,
    ) -> Option<String> {
        if tool_name == RUN_COMMAND_TOOL {
            if !confirmed {
                return None;
            }
            let minter = self.consent_minter.as_ref()?;
            let (command, args) = extract_run_argv(tool_input)?;
            return minter.mint_run(&command, &args);
        }
        self.mint_proof(tool_name, tool_input, session)
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
    /// is denied without consulting the gate. A gate `Confirm` is resolved
    /// DAEMON-SIDE by driving the trusted-path consent surface (it never reaches
    /// the engine): the user's answer maps to `Allow` (approved) or `Deny`
    /// (denied), so the engine only ever sees a settled decision.
    pub async fn authorize(
        &self,
        token: &SessionToken,
        pid: u32,
        req: &Authorize,
    ) -> AuthorizeDecision {
        let grant = match self.resolve(token, pid) {
            Some(grant) => grant,
            None => {
                return AuthorizeDecision::Deny {
                    reason: "no valid session for this caller".to_string(),
                }
            }
        };
        // The TRUSTED external-origin bit for the consent surface: the OR of the
        // session's supervisor-derived origin and the engine's untrusted per-call
        // flag (escalate-only). The consent broker must see the resolved value, not
        // the engine's self-report - else a prompt-injected engine claiming
        // external=false could corrupt the trusted-path provenance ("you are asked
        // because external content triggered this") or down-classify the dialog.
        let externally_triggered = grant.externally_triggered || req.external_triggered;
        // Resolve the gate decision (driving the consent broker for a Confirm).
        let gate_decision = self.gate.authorize(req, &grant).await;
        // Whether this authorization was resolved through a user Confirm. The
        // run_command consent biscuit is minted ONLY on this path (defense in depth:
        // even if a future gate ever returns Allow directly for run_command - the
        // path-discriminated downgrade the capability map anticipates - the mint is
        // refused, so the consent boundary for the sharp-edge tool is unbypassable).
        let confirmed = matches!(gate_decision, AuthorizeDecision::Confirm { .. });
        let decision = match gate_decision {
            AuthorizeDecision::Confirm { prompt } => {
                match self
                    .consent
                    .confirm(&req.tool_name, &prompt, externally_triggered)
                    .await
                {
                    // A resolved confirm becomes an Allow, so the proof below is
                    // minted ONLY after the user approved (HIGH-1: a Confirm tool
                    // cannot execute without a resolved confirm).
                    ConfirmAnswer::Approved => AuthorizeDecision::Allow { proof: None },
                    ConfirmAnswer::Denied => {
                        return AuthorizeDecision::Deny {
                            reason: format!("{} was not confirmed", req.tool_name),
                        }
                    }
                }
            }
            other => other,
        };
        // Mint the one-time execution proof for any admitted outcome, bound to this
        // session (HIGH-1). Deny / Confirm carry no proof, so they can never reach
        // Execute; Modify binds the daemon-substituted args that will actually run.
        match decision {
            AuthorizeDecision::Allow { .. } => AuthorizeDecision::Allow {
                proof: self.proof_for_allow(&req.tool_name, &req.tool_input, token.as_str(), confirmed),
            },
            AuthorizeDecision::Modify { args, .. } => {
                let proof = self.proof_for_allow(&req.tool_name, &args, token.as_str(), confirmed);
                AuthorizeDecision::Modify { args, proof }
            }
            other => other,
        }
    }

    /// Execute a privileged tool. No valid session fails closed as a permission
    /// error; the executor is never reached.
    pub async fn execute(&self, token: &SessionToken, pid: u32, req: &Execute) -> ExecuteOutcome {
        let grant = match self.resolve(token, pid) {
            Some(grant) => grant,
            None => {
                return ExecuteOutcome::Error {
                    code: ContractError::PermissionDenied,
                    message: "no valid session for this caller".to_string(),
                }
            }
        };
        // HIGH-1 gate enforcement: Execute REQUIRES a valid, unconsumed, matching
        // one-time proof that Authorize minted. No proof (or a mismatched / reused /
        // expired one) means the gate never admitted THIS exact call, so refuse
        // before the executor - the gate's reversibility / confirm / deny logic
        // cannot be skipped by calling Execute directly.
        let proof = match &req.proof {
            Some(p) => p,
            None => {
                return ExecuteOutcome::Error {
                    code: ContractError::PermissionDenied,
                    message: "execute requires an authorization proof".to_string(),
                }
            }
        };
        let args_hash = crate::execution_proof::hash_args(&req.tool_input);
        let now = self.now_ms();
        let consumed = self.proofs.lock().unwrap().consume(
            proof,
            &req.tool_name,
            &args_hash,
            token.as_str(),
            now,
        );
        if consumed.is_err() {
            return ExecuteOutcome::Error {
                code: ContractError::PermissionDenied,
                message: "authorization proof invalid, expired, or already used".to_string(),
            };
        }
        self.executor.execute(req, &grant).await
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

/// Verifies a `(session token, pid)` pair for the completion egress. Object-safe
/// (no `G`/`E`/`R`) so the pi completion listener can hold an
/// `Arc<dyn SessionVerifier>` without the dispatcher's Gate/Executor/Reporter type
/// parameters.
pub trait SessionVerifier: Send + Sync {
    /// True iff `token` is a live session bound to `pid` - the same check the
    /// contract socket's tool verbs use, so the completion egress admits exactly
    /// the attested, session-bound engine.
    fn verify_session(&self, token: &SessionToken, pid: u32) -> bool;
}

impl<G: Gate, E: Executor, R: Reporter> SessionVerifier for Dispatcher<G, E, R> {
    fn verify_session(&self, token: &SessionToken, pid: u32) -> bool {
        self.resolve(token, pid).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ai_engine_contract::{CapabilityContext, ReadTier};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::sync::Mutex as StdMutex;

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
            externally_triggered: false,
        }
    }

    type TestRig = (Dispatcher<SpyGate, SpyExecutor, SpyReporter>, Arc<AtomicUsize>, Arc<AtomicUsize>, Arc<AtomicUsize>);

    fn dispatcher() -> TestRig {
        let g = Arc::new(AtomicUsize::new(0));
        let e = Arc::new(AtomicUsize::new(0));
        let r = Arc::new(AtomicUsize::new(0));
        let d = Dispatcher::new(
            SpyGate { calls: g.clone(), decision: AuthorizeDecision::Allow { proof: None } },
            SpyExecutor { calls: e.clone() },
            SpyReporter { calls: r.clone() },
        );
        (d, g, e, r)
    }

    #[tokio::test]
    async fn a_valid_session_reaches_each_seam() {
        let (d, g, e, r) = dispatcher();
        let token = d.init_session(&init(), std::process::id()).unwrap();

        let dec = d.authorize(&token, std::process::id(), &Authorize {
            tool_name: "bash".into(),
            tool_input: serde_json::json!({}),
            external_triggered: false,
        }).await;
        // An admitted authorize mints a one-time proof (HIGH-1).
        let proof = match dec {
            AuthorizeDecision::Allow { proof } => proof,
            other => panic!("expected Allow, got {other:?}"),
        };
        assert!(proof.is_some(), "an admitted authorize mints an execution proof");
        assert_eq!(g.load(Ordering::SeqCst), 1);

        // Execute presents the proof for the SAME tool + args; the executor runs.
        let out = d.execute(&token, std::process::id(), &Execute {
            tool_name: "bash".into(),
            tool_input: serde_json::json!({}),
            proof: proof.clone(),
        }).await;
        assert!(matches!(out, ExecuteOutcome::Ok { .. }));
        assert_eq!(e.load(Ordering::SeqCst), 1);

        // The proof is single-use: a replay is refused and the executor is NOT
        // called again (HIGH-1 enforcement).
        let replay = d.execute(&token, std::process::id(), &Execute {
            tool_name: "bash".into(),
            tool_input: serde_json::json!({}),
            proof,
        }).await;
        assert!(matches!(replay, ExecuteOutcome::Error { code: ContractError::PermissionDenied, .. }));
        assert_eq!(e.load(Ordering::SeqCst), 1, "a consumed proof does not reach the executor");

        let ack = d.report(&token, std::process::id(), &Report {
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
        let token = d.init_session(&init(), std::process::id()).unwrap();
        let dec = d.authorize(&token, std::process::id(), &Authorize {
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
        let token = d.init_session(&init(), std::process::id()).unwrap();
        let bad = SessionToken_for_test();

        // Unknown token -> Deny / Error / Block, seams untouched.
        let dec = d.authorize(&bad, std::process::id(), &authz()).await;
        assert!(matches!(dec, AuthorizeDecision::Deny { .. }));
        let out = d.execute(&bad, std::process::id(), &exec()).await;
        assert!(matches!(out, ExecuteOutcome::Error { code: ContractError::PermissionDenied, .. }));
        let ack = d.report(&bad, std::process::id(), &report()).await;
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
        let token = d.init_session(&init(), std::process::id()).unwrap();
        d.end_session(&token);
        assert!(matches!(d.authorize(&token, std::process::id(), &authz()).await, AuthorizeDecision::Deny { .. }));
    }

    /// A consent surface that records the confirmation it was asked and returns a
    /// scripted answer, so a test can prove the gate `Confirm` was resolved
    /// daemon-side with the right details (and never handed to the engine).
    struct MockConsent {
        approve: bool,
        seen: Arc<StdMutex<Option<(String, String, bool)>>>,
    }
    #[async_trait]
    impl ConsentDriver for MockConsent {
        async fn confirm(&self, tool_name: &str, prompt: &str, external_triggered: bool) -> ConfirmAnswer {
            *self.seen.lock().unwrap() = Some((tool_name.to_string(), prompt.to_string(), external_triggered));
            if self.approve { ConfirmAnswer::Approved } else { ConfirmAnswer::Denied }
        }
    }

    /// A dispatcher whose gate always returns `Confirm`, with `consent` wired in.
    fn confirm_dispatcher(consent: Arc<dyn ConsentDriver>) -> Dispatcher<SpyGate, SpyExecutor, SpyReporter> {
        Dispatcher::new(
            SpyGate {
                calls: Arc::new(AtomicUsize::new(0)),
                decision: AuthorizeDecision::Confirm { prompt: "delete everything?".into() },
            },
            SpyExecutor { calls: Arc::new(AtomicUsize::new(0)) },
            SpyReporter { calls: Arc::new(AtomicUsize::new(0)) },
        )
        .with_consent(consent)
    }

    #[tokio::test]
    async fn a_gate_confirm_with_approved_consent_becomes_allow() {
        let seen = Arc::new(StdMutex::new(None));
        let d = confirm_dispatcher(Arc::new(MockConsent { approve: true, seen: seen.clone() }));
        let token = d.init_session(&init(), std::process::id()).unwrap();
        let dec = d
            .authorize(&token, std::process::id(), &Authorize {
                tool_name: "graph.write".into(),
                tool_input: serde_json::json!({}),
                external_triggered: true,
            })
            .await;
        assert!(matches!(dec, AuthorizeDecision::Allow { proof: Some(_) }), "approved consent resolves Confirm to an Allow with a proof");
        // The consent surface saw the confirmation details; the engine never did.
        let s = seen.lock().unwrap().clone().unwrap();
        assert_eq!(s, ("graph.write".to_string(), "delete everything?".to_string(), true));
    }

    #[tokio::test]
    async fn a_gate_confirm_with_denied_consent_becomes_deny() {
        let d = confirm_dispatcher(Arc::new(MockConsent {
            approve: false,
            seen: Arc::new(StdMutex::new(None)),
        }));
        let token = d.init_session(&init(), std::process::id()).unwrap();
        match d.authorize(&token, std::process::id(), &authz()).await {
            AuthorizeDecision::Deny { reason } => {
                assert!(reason.contains("not confirmed"), "deny reason: {reason}")
            }
            other => panic!("expected Deny, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn the_default_consent_fails_a_confirm_closed() {
        // No with_consent(): the fail-closed DeniedConsent default denies a
        // Confirm, so a high-impact action with no consent surface reachable is
        // never silently allowed.
        let d = Dispatcher::new(
            SpyGate {
                calls: Arc::new(AtomicUsize::new(0)),
                decision: AuthorizeDecision::Confirm { prompt: "ok?".into() },
            },
            SpyExecutor { calls: Arc::new(AtomicUsize::new(0)) },
            SpyReporter { calls: Arc::new(AtomicUsize::new(0)) },
        );
        let token = d.init_session(&init(), std::process::id()).unwrap();
        assert!(
            matches!(d.authorize(&token, std::process::id(), &authz()).await, AuthorizeDecision::Deny { .. }),
            "the default consent surface denies a confirm"
        );
    }

    /// A minter that records the argv it was asked to bind and returns a fixed
    /// token, so a test can prove the biscuit is minted over the exact command+args.
    struct MockMinter {
        seen: Arc<StdMutex<Option<(String, Vec<String>)>>>,
        token: String,
    }
    impl ConsentMinter for MockMinter {
        fn mint_run(&self, command: &str, args: &[String]) -> Option<String> {
            *self.seen.lock().unwrap() = Some((command.to_string(), args.to_vec()));
            Some(self.token.clone())
        }
    }

    /// Build a dispatcher whose gate returns Confirm (the real run_command class),
    /// an approving consent surface, and the given minter - the production path a
    /// run_command travels: gate Confirm -> broker Approved -> Allow -> mint.
    fn confirmed_run_dispatcher(
        minter: Option<Arc<dyn ConsentMinter>>,
    ) -> Dispatcher<SpyGate, SpyExecutor, SpyReporter> {
        let mut d = Dispatcher::new(
            SpyGate { calls: Arc::new(AtomicUsize::new(0)), decision: AuthorizeDecision::Confirm { prompt: "run ls?".into() } },
            SpyExecutor { calls: Arc::new(AtomicUsize::new(0)) },
            SpyReporter { calls: Arc::new(AtomicUsize::new(0)) },
        )
        .with_consent(Arc::new(MockConsent { approve: true, seen: Arc::new(StdMutex::new(None)) }));
        if let Some(m) = minter {
            d = d.with_consent_minter(m);
        }
        d
    }

    #[tokio::test]
    async fn an_approved_run_command_confirm_mints_the_consent_token() {
        let seen = Arc::new(StdMutex::new(None));
        let d = confirmed_run_dispatcher(Some(Arc::new(MockMinter {
            seen: seen.clone(),
            token: "biscuit-xyz".into(),
        })));
        let token = d.init_session(&init(), std::process::id()).unwrap();
        let dec = d.authorize(&token, std::process::id(), &Authorize {
            tool_name: "run_command".into(),
            tool_input: serde_json::json!({"command": "ls", "args": ["-la", "/work"]}),
            external_triggered: false,
        }).await;
        // After the user approved, the run_command Allow carries the biscuit, NOT a
        // daemon in-memory proof handle (the MCP server, a separate process, verifies
        // the biscuit).
        assert_eq!(dec, AuthorizeDecision::Allow { proof: Some("biscuit-xyz".into()) });
        // The token was minted over the exact command + args the caller proposed.
        assert_eq!(
            *seen.lock().unwrap(),
            Some(("ls".to_string(), vec!["-la".to_string(), "/work".to_string()]))
        );
    }

    #[tokio::test]
    async fn a_run_command_allow_without_a_minter_carries_no_credential() {
        // Confirmed by the user, but no minter wired -> no consent credential -> the
        // MCP server fails closed.
        let d = confirmed_run_dispatcher(None);
        let token = d.init_session(&init(), std::process::id()).unwrap();
        let dec = d.authorize(&token, std::process::id(), &Authorize {
            tool_name: "run_command".into(),
            tool_input: serde_json::json!({"command": "ls"}),
            external_triggered: false,
        }).await;
        assert_eq!(dec, AuthorizeDecision::Allow { proof: None });
    }

    #[tokio::test]
    async fn a_run_command_allow_direct_from_the_gate_mints_nothing() {
        // Defense in depth: even a gate that WRONGLY returns Allow directly for
        // run_command (bypassing the consent broker) mints no biscuit, so the MCP
        // server fails closed - the consent boundary for the sharp-edge tool is not
        // bypassable by a future gate misclassification.
        let seen = Arc::new(StdMutex::new(None));
        let d = Dispatcher::new(
            SpyGate { calls: Arc::new(AtomicUsize::new(0)), decision: AuthorizeDecision::Allow { proof: None } },
            SpyExecutor { calls: Arc::new(AtomicUsize::new(0)) },
            SpyReporter { calls: Arc::new(AtomicUsize::new(0)) },
        )
        .with_consent_minter(Arc::new(MockMinter { seen: seen.clone(), token: "biscuit-xyz".into() }));
        let token = d.init_session(&init(), std::process::id()).unwrap();
        let dec = d.authorize(&token, std::process::id(), &Authorize {
            tool_name: "run_command".into(),
            tool_input: serde_json::json!({"command": "ls"}),
            external_triggered: false,
        }).await;
        assert_eq!(dec, AuthorizeDecision::Allow { proof: None });
        assert_eq!(
            *seen.lock().unwrap(),
            None,
            "the minter is never consulted for an unconfirmed run_command"
        );
    }

    #[test]
    fn extract_run_argv_matches_the_mcp_parse() {
        assert_eq!(
            extract_run_argv(&serde_json::json!({"command":"ls","args":["-la"]})),
            Some(("ls".to_string(), vec!["-la".to_string()]))
        );
        // args absent -> empty (the MCP server defaults the same way).
        assert_eq!(
            extract_run_argv(&serde_json::json!({"command":"ls"})),
            Some(("ls".to_string(), vec![]))
        );
        // Fail-closed cases the MCP server also refuses: empty / missing command,
        // a non-string arg, a non-array args.
        assert_eq!(extract_run_argv(&serde_json::json!({"command":""})), None);
        assert_eq!(extract_run_argv(&serde_json::json!({"args":["x"]})), None);
        assert_eq!(extract_run_argv(&serde_json::json!({"command":"ls","args":[1]})), None);
        assert_eq!(extract_run_argv(&serde_json::json!({"command":"ls","args":"x"})), None);
    }

    // Helpers building throwaway requests.
    fn authz() -> Authorize {
        Authorize { tool_name: "bash".into(), tool_input: serde_json::json!({}), external_triggered: false }
    }
    fn exec() -> Execute {
        Execute { tool_name: "graph.read".into(), tool_input: serde_json::json!({}), proof: None }
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

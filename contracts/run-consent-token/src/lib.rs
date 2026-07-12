//! The per-action consent token for `run_command` (ai-act-layer-plan.md, the
//! biscuit per-action tie-in).
//!
//! `run_command` is the sharp edge: always-Confirm, confined, never-autonomous. It
//! runs in a SEPARATE process (the terminal-run MCP server), so the ai-engine
//! daemon's in-memory execution proof cannot be verified there. Instead the daemon
//! MINTS a public-key-verifiable Biscuit token when the gate's `Confirm` for a
//! `run_command` is resolved-approved by the consent broker, and the MCP server
//! VERIFIES it at its boundary before running the command. The daemon holds the
//! signing key; the server needs only the root public key.
//!
//! The token binds the EXACT command + args (a sha256 digest) and a TTL, so:
//!  - a token minted for one command cannot authorize a DIFFERENT command (the
//!    digest mismatches at verify), and
//!  - it expires (a single short-lived approval, never a standing grant).
//!
//! It mirrors the Connections capability-token datalog: parameterized facts (no
//! caller string is ever interpolated into datalog source), a `trusting authority`-
//! scoped allow policy (so a holder cannot append a block to re-broaden), and a
//! fail-closed TTL check (a verifier that supplies no `time` fact denies).

use std::collections::HashMap;

use biscuit_auth::builder::{AuthorizerBuilder, BiscuitBuilder, Term};
use biscuit_auth::{Biscuit, KeyPair, PublicKey};
use sha2::{Digest, Sha256};

/// The fixed tool this token authorizes. `run_command` is the only per-action
/// consent-token act today; a future act would carry its own tool string.
pub const RUN_COMMAND_TOOL: &str = "run_command";

/// A failure minting or verifying a consent token.
#[derive(Debug, thiserror::Error)]
pub enum ConsentTokenError {
    /// A Biscuit build / parse / signature error (a malformed or wrong-key token).
    #[error("biscuit error: {0}")]
    Biscuit(String),
    /// A caller-supplied value was out of range.
    #[error("invalid input: {0}")]
    InvalidInput(&'static str),
}

impl From<biscuit_auth::error::Token> for ConsentTokenError {
    fn from(e: biscuit_auth::error::Token) -> Self {
        ConsentTokenError::Biscuit(e.to_string())
    }
}

/// The sha256 hex digest binding the exact command + its args. Lengths are mixed in
/// (length-delimited) so `["a", "bc"]` and `["ab", "c"]` never collide, and the arg
/// COUNT is bound too. Mint and verify compute it identically, so a token
/// authorizes exactly one `(command, args)` pair and nothing else.
pub fn run_digest(command: &str, args: &[String]) -> String {
    let mut h = Sha256::new();
    h.update((command.len() as u64).to_le_bytes());
    h.update(command.as_bytes());
    h.update((args.len() as u64).to_le_bytes());
    for a in args {
        h.update((a.len() as u64).to_le_bytes());
        h.update(a.as_bytes());
    }
    let out = h.finalize();
    let mut hex = String::with_capacity(out.len() * 2);
    for b in out {
        hex.push_str(&format!("{b:02x}"));
    }
    hex
}

/// Mint a consent token authorizing exactly `command` + `args` until
/// `expiry_unix` (unix seconds), signed by the daemon's `root` keypair. Called
/// once the consent broker approves the `run_command` Confirm. `expiry_unix` must
/// be positive.
pub fn mint_run_consent(
    root: &KeyPair,
    command: &str,
    args: &[String],
    expiry_unix: i64,
) -> Result<String, ConsentTokenError> {
    if expiry_unix <= 0 {
        return Err(ConsentTokenError::InvalidInput("expiry not positive"));
    }
    let digest = run_digest(command, args);

    let mut builder: BiscuitBuilder = Biscuit::builder();
    // Authority facts, parameterized (never interpolated into datalog source).
    let mut params: HashMap<String, Term> = HashMap::new();
    params.insert("tool".to_string(), Term::Str(RUN_COMMAND_TOOL.to_string()));
    params.insert("digest".to_string(), Term::Str(digest));
    builder = builder.code_with_params(
        "tool({tool}); run_digest({digest});",
        params,
        HashMap::new(),
    )?;
    // The TTL check. A verifier that supplies no `time` fact leaves it unsatisfiable
    // (fails closed). The expiry bound is a parameter, not interpolated source.
    let mut tp: HashMap<String, Term> = HashMap::new();
    tp.insert("expiry".to_string(), Term::Date(expiry_unix as u64));
    builder = builder.code_with_params(
        "check if time($t), $t <= {expiry};",
        tp,
        HashMap::new(),
    )?;

    let token = builder.build(root)?;
    Ok(token.to_base64()?)
}

/// Verify that `token_b64` (signed by the mint's `root_public`) authorizes running
/// exactly `command` + `args` at `now_unix`.
///
/// Fails closed on every adverse condition: a malformed base64 or a bad signature
/// is an `Err` (the caller must refuse); a well-formed token that does not
/// authorize this exact command, or that has expired, is `Ok(false)`. Only a token
/// whose authority binds this run's `(tool, digest)` and whose TTL check passes at
/// `now_unix` is `Ok(true)`. The authorizer supplies `time(now)`, `req_tool` and
/// `req_digest`, and one `trusting authority`-scoped allow policy, so the match
/// reads only the token's own authority block (a later attenuation block cannot
/// re-broaden it).
pub fn verify_run_consent(
    token_b64: &str,
    root_public: &PublicKey,
    command: &str,
    args: &[String],
    now_unix: i64,
) -> Result<bool, ConsentTokenError> {
    if now_unix < 0 {
        return Err(ConsentTokenError::InvalidInput("negative time"));
    }
    // Parse + verify the signature against the root public key. A bad signature or
    // malformed token is a hard error, never a soft deny.
    let token = Biscuit::from_base64(token_b64, root_public)?;
    let digest = run_digest(command, args);

    let mut params: HashMap<String, Term> = HashMap::new();
    params.insert("now".to_string(), Term::Date(now_unix as u64));
    params.insert("reqtool".to_string(), Term::Str(RUN_COMMAND_TOOL.to_string()));
    params.insert("reqdigest".to_string(), Term::Str(digest));
    let source = "time({now}); req_tool({reqtool}); req_digest({reqdigest}); \
                  allow if req_tool($t), tool($t), req_digest($d), run_digest($d) trusting authority;";
    let authorizer: AuthorizerBuilder =
        AuthorizerBuilder::new().code_with_params(source, params, HashMap::new())?;

    let mut built = authorizer.build(&token)?;
    match built.authorize() {
        Ok(_) => Ok(true),
        // A denied or expired token is an authorization outcome, not a fault.
        Err(_) => Ok(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_minted_token_authorizes_exactly_its_command() {
        let root = KeyPair::new();
        let token = mint_run_consent(&root, "ls", &args(&["-la", "/work"]), 10_000).unwrap();
        // Same command + args + a time before expiry -> authorized.
        assert_eq!(
            verify_run_consent(&token, &root.public(), "ls", &args(&["-la", "/work"]), 5_000).unwrap(),
            true
        );
    }

    #[test]
    fn a_token_does_not_authorize_a_different_command() {
        let root = KeyPair::new();
        let token = mint_run_consent(&root, "ls", &args(&["-la"]), 10_000).unwrap();
        // A different command / different args -> the digest mismatches -> denied.
        assert_eq!(verify_run_consent(&token, &root.public(), "rm", &args(&["-rf", "/"]), 5_000).unwrap(), false);
        assert_eq!(verify_run_consent(&token, &root.public(), "ls", &args(&["-la", "/etc"]), 5_000).unwrap(), false);
        assert_eq!(verify_run_consent(&token, &root.public(), "ls", &args(&[]), 5_000).unwrap(), false);
    }

    #[test]
    fn an_expired_token_is_denied() {
        let root = KeyPair::new();
        let token = mint_run_consent(&root, "ls", &args(&["-la"]), 1_000).unwrap();
        // now (2000) is past the expiry (1000) -> the TTL check fails -> denied.
        assert_eq!(verify_run_consent(&token, &root.public(), "ls", &args(&["-la"]), 2_000).unwrap(), false);
    }

    #[test]
    fn a_token_signed_by_a_different_key_is_a_hard_error() {
        let root = KeyPair::new();
        let attacker = KeyPair::new();
        let token = mint_run_consent(&root, "ls", &args(&["-la"]), 10_000).unwrap();
        // Verifying under the wrong root public key is a signature failure (Err),
        // never a soft deny - a forged token cannot even parse.
        assert!(verify_run_consent(&token, &attacker.public(), "ls", &args(&["-la"]), 5_000).is_err());
    }

    #[test]
    fn a_malformed_token_is_a_hard_error() {
        let root = KeyPair::new();
        assert!(verify_run_consent("not-a-token", &root.public(), "ls", &args(&[]), 5_000).is_err());
    }

    #[test]
    fn mint_rejects_a_non_positive_expiry() {
        let root = KeyPair::new();
        assert!(matches!(
            mint_run_consent(&root, "ls", &args(&[]), 0),
            Err(ConsentTokenError::InvalidInput(_))
        ));
    }

    #[test]
    fn run_digest_is_injective_over_the_command_and_arg_boundaries() {
        // The length-delimiting means a shifted boundary changes the digest.
        assert_ne!(run_digest("a", &args(&["bc"])), run_digest("ab", &args(&["c"])));
        assert_ne!(run_digest("ls", &args(&["a", "b"])), run_digest("ls", &args(&["ab"])));
        // Identical inputs are identical.
        assert_eq!(run_digest("ls", &args(&["-la"])), run_digest("ls", &args(&["-la"])));
    }
}

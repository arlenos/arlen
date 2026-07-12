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
//!
//! Two properties the digest binding alone does not give, seeded here so the
//! follow-up verify slice can rely on them without a token-shape change:
//!  - Every mint carries a fresh random `nonce(...)` authority fact. It is not
//!    checked today (the same approved command re-runs within its TTL, bounded by
//!    a short expiry), but a future single-use verifier can key a seen-set on it.
//!  - `command` and `args` are length- and count-capped, and an interior NUL is
//!    refused: `execve` truncates an argv string at NUL, so a NUL past the first
//!    byte would make the executed command differ from the digested one. Refusing
//!    it keeps the mint digest and the exec-time digest identical.

use std::collections::HashMap;

use std::path::PathBuf;

use biscuit_auth::builder::{AuthorizerBuilder, BiscuitBuilder, Term};
use biscuit_auth::{Algorithm, Biscuit, KeyPair, PublicKey};
use sha2::{Digest, Sha256};

/// The fixed tool this token authorizes. `run_command` is the only per-action
/// consent-token act today; a future act would carry its own tool string.
pub const RUN_COMMAND_TOOL: &str = "run_command";

/// The rendezvous file where the AI-engine daemon publishes its consent-token root
/// PUBLIC key (hex) and the terminal-run MCP server reads it:
/// `$XDG_STATE_HOME|$HOME/.local/state` + `arlen/ai-engine/run-consent-root.pub`, or
/// `None` when neither env var is set. This ONE resolver is the single source of
/// truth for both sides, so the publish path and the read path cannot drift; a
/// mismatch anyway fails closed (the reader finds no key and refuses). A public key
/// is not a secret, so the file is world-readable.
pub fn published_public_key_path() -> Option<PathBuf> {
    let base = std::env::var("XDG_STATE_HOME")
        .ok()
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .filter(|s| !s.is_empty())
                .map(|h| PathBuf::from(h).join(".local/state"))
        })?;
    Some(base.join("arlen/ai-engine/run-consent-root.pub"))
}

/// Parse a published hex public key back into a Biscuit [`PublicKey`]. The verify
/// side (the MCP server) reads the rendezvous file and calls this; it lives here so
/// the mint and verify sides share one encoding. A malformed hex string or a
/// non-Ed25519 key is a hard error - the verifier must refuse, never fall back to an
/// unverified run.
pub fn public_key_from_hex(hex: &str) -> Result<PublicKey, ConsentTokenError> {
    let hex = hex.trim();
    if !hex.len().is_multiple_of(2) {
        return Err(ConsentTokenError::InvalidInput("odd-length public-key hex"));
    }
    let mut bytes = Vec::with_capacity(hex.len() / 2);
    let raw = hex.as_bytes();
    let mut i = 0;
    while i < raw.len() {
        let hi = (raw[i] as char)
            .to_digit(16)
            .ok_or(ConsentTokenError::InvalidInput("non-hex byte in public key"))?;
        let lo = (raw[i + 1] as char)
            .to_digit(16)
            .ok_or(ConsentTokenError::InvalidInput("non-hex byte in public key"))?;
        bytes.push(((hi << 4) | lo) as u8);
        i += 2;
    }
    PublicKey::from_bytes(&bytes, Algorithm::Ed25519)
        .map_err(|e| ConsentTokenError::Biscuit(e.to_string()))
}

/// The longest accepted command (the executable name or path).
const MAX_COMMAND_LEN: usize = 4096;

/// The longest accepted single argument. Args can legitimately be large (an
/// inline payload), so the per-arg cap is generous; it exists only to bound the
/// token the daemon signs and the MCP server parses, not to constrain content.
const MAX_ARG_LEN: usize = 128 * 1024;

/// The most arguments a token may bind. Args come from a model proposal upstream,
/// so the count is capped at the mint boundary rather than trusted.
const MAX_ARGS: usize = 1024;

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

/// Validate a `(command, args)` pair at the mint boundary: the command must be
/// non-empty, control-free (a control byte in an executable name is never
/// legitimate) and within [`MAX_COMMAND_LEN`]; every argument must be free of an
/// interior NUL (which `execve` would truncate at, skewing the exec-time digest)
/// and within [`MAX_ARG_LEN`]; the arg count must be within [`MAX_ARGS`]. Args are
/// otherwise opaque data bound by the digest, so no charset is imposed on them.
fn validate_run(command: &str, args: &[String]) -> Result<(), ConsentTokenError> {
    if command.is_empty() {
        return Err(ConsentTokenError::InvalidInput("empty command"));
    }
    if command.len() > MAX_COMMAND_LEN {
        return Err(ConsentTokenError::InvalidInput("command too long"));
    }
    if command.bytes().any(|b| b.is_ascii_control()) {
        return Err(ConsentTokenError::InvalidInput("command has a control byte"));
    }
    if args.len() > MAX_ARGS {
        return Err(ConsentTokenError::InvalidInput("too many arguments"));
    }
    for a in args {
        if a.len() > MAX_ARG_LEN {
            return Err(ConsentTokenError::InvalidInput("argument too long"));
        }
        if a.bytes().any(|b| b == 0) {
            return Err(ConsentTokenError::InvalidInput("argument has a NUL byte"));
        }
    }
    Ok(())
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
    validate_run(command, args)?;
    let digest = run_digest(command, args);

    // A fresh 128-bit random uniqueness tag. Not checked at verify today; seeded so
    // a future single-use verifier can key a seen-set on it without a shape change.
    let mut raw = [0u8; 16];
    getrandom::getrandom(&mut raw)
        .map_err(|_| ConsentTokenError::InvalidInput("nonce entropy unavailable"))?;
    let mut nonce = String::with_capacity(raw.len() * 2);
    for b in raw {
        nonce.push_str(&format!("{b:02x}"));
    }

    let mut builder: BiscuitBuilder = Biscuit::builder();
    // Authority facts, parameterized (never interpolated into datalog source).
    let mut params: HashMap<String, Term> = HashMap::new();
    params.insert("tool".to_string(), Term::Str(RUN_COMMAND_TOOL.to_string()));
    params.insert("digest".to_string(), Term::Str(digest));
    params.insert("nonce".to_string(), Term::Str(nonce));
    builder = builder.code_with_params(
        "tool({tool}); run_digest({digest}); nonce({nonce});",
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
    fn two_mints_of_the_same_command_differ_but_both_authorize() {
        let root = KeyPair::new();
        let a = mint_run_consent(&root, "ls", &args(&["-la"]), 10_000).unwrap();
        let b = mint_run_consent(&root, "ls", &args(&["-la"]), 10_000).unwrap();
        // Distinct tokens: the random nonce makes each mint unique (future single-use).
        assert_ne!(a, b);
        // The nonce is not part of the digest, so both still authorize the command.
        assert!(verify_run_consent(&a, &root.public(), "ls", &args(&["-la"]), 5_000).unwrap());
        assert!(verify_run_consent(&b, &root.public(), "ls", &args(&["-la"]), 5_000).unwrap());
    }

    #[test]
    fn mint_rejects_an_empty_command() {
        let root = KeyPair::new();
        assert!(matches!(
            mint_run_consent(&root, "", &args(&["-la"]), 10_000),
            Err(ConsentTokenError::InvalidInput(_))
        ));
    }

    #[test]
    fn mint_rejects_a_control_byte_in_the_command() {
        let root = KeyPair::new();
        assert!(matches!(
            mint_run_consent(&root, "ls\n", &args(&[]), 10_000),
            Err(ConsentTokenError::InvalidInput(_))
        ));
    }

    #[test]
    fn mint_rejects_an_interior_nul_in_an_argument() {
        let root = KeyPair::new();
        // execve would truncate "foo\0bar" to "foo", skewing the exec-time digest.
        assert!(matches!(
            mint_run_consent(&root, "ls", &args(&["foo\0bar"]), 10_000),
            Err(ConsentTokenError::InvalidInput(_))
        ));
    }

    #[test]
    fn mint_rejects_too_many_arguments() {
        let root = KeyPair::new();
        let many: Vec<String> = (0..MAX_ARGS + 1).map(|i| i.to_string()).collect();
        assert!(matches!(
            mint_run_consent(&root, "ls", &many, 10_000),
            Err(ConsentTokenError::InvalidInput(_))
        ));
    }

    #[test]
    fn mint_rejects_an_over_long_argument() {
        let root = KeyPair::new();
        let big = "x".repeat(MAX_ARG_LEN + 1);
        assert!(matches!(
            mint_run_consent(&root, "ls", &args(&[&big]), 10_000),
            Err(ConsentTokenError::InvalidInput(_))
        ));
    }

    #[test]
    fn a_public_key_hex_round_trips_and_verifies() {
        let root = KeyPair::new();
        let bytes = root.public().to_bytes();
        let mut hex = String::with_capacity(bytes.len() * 2);
        for b in &bytes {
            hex.push_str(&format!("{b:02x}"));
        }
        let parsed = public_key_from_hex(&hex).unwrap();
        assert_eq!(parsed.to_bytes(), bytes);
        // A token minted by the daemon verifies under the hex-parsed public key the
        // MCP server would read from the rendezvous file.
        let token = mint_run_consent(&root, "ls", &args(&["-la"]), 10_000).unwrap();
        assert!(verify_run_consent(&token, &parsed, "ls", &args(&["-la"]), 5_000).unwrap());
    }

    #[test]
    fn a_malformed_public_key_hex_is_rejected() {
        assert!(public_key_from_hex("not-hex").is_err()); // non-hex
        assert!(public_key_from_hex("abc").is_err()); // odd length
        assert!(public_key_from_hex("").is_err()); // wrong length for Ed25519
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

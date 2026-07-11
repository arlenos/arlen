//! The destination-scoped capability token (connections-plan.md, the
//! credential-injection substrate).
//!
//! A trusted proxy presents one of these tokens to prove it is authorized to
//! reach a specific network host before the daemon hands out a credential. The
//! token is a Biscuit: it is public-key-verifiable (any holder of the root
//! public key can check it, the private key never leaves the minter), and it is
//! monotonically attenuable (a holder can append caveats that only ever narrow
//! authority, never broaden it).
//!
//! A minted token carries a set of allowed hosts, an expiry as unix seconds, a
//! nonce, and its connection id. Verification asks: may this token reach host H
//! at time T (and, when a connection is required, is the token bound to that
//! connection)? It answers yes only when H is in the allowed set, T is at or
//! before the expiry, and the token's connection matches the requested one.
//! Everything fails closed: a bad signature, a malformed token, a disallowed
//! host, a wrong connection or an expired token all deny.
//!
//! ## Why the attenuation cannot broaden
//!
//! The allowed hosts live as `host(...)` facts in the token's authority block
//! (block 0). The verify authorizer's allow policy is scoped `trusting
//! authority`, and Biscuit's default trust for an authorizer policy is already
//! only the authority block plus the authorizer itself, never a later appended
//! block. So a holder who appends a block cannot make the policy honour a `host`
//! fact it smuggles in: that fact lives in an untrusted block. Attenuation only
//! ever adds a check (a caveat that must also pass), and a check can only remove
//! authority. Restricting the hosts to a subset appends `check if
//! requested_host($h), [$h in subset]`, so a host dropped by the subset now
//! fails the check and is denied. This is the subtract-only property the plan
//! mandates, enforced by Biscuit's block-scoping rather than by our own code.
//!
//! ## Residual limits the caller must honour (from the adversarial review)
//!
//! Host comparison is EXACT bytewise equality: `API.Github.com`, a trailing dot,
//! or a punycode look-alike do not match an allowed `host` fact. Normalization
//! (lowercasing, IDNA/punycode canonicalization, trailing-dot stripping) is the
//! CALLER's responsibility, and it must normalize the SAME way at mint and at
//! verify, or a legitimate request is denied. The module compares raw bytes on
//! purpose so the security-relevant match is unambiguous.
//!
//! The TTL is enforced as a check inside the token, so an accepted token's expiry
//! rests on it having been minted by [`mint_token`] (which always appends the
//! check). Since [`verify_token`] also checks the root signature, only the sole
//! key-holder can mint an accepted token, so a TTL-less token cannot arise in
//! practice; there is no separate authorizer-side expiry check.

use biscuit_auth::builder::{AuthorizerBuilder, BiscuitBuilder, BlockBuilder, Term};
use biscuit_auth::{Biscuit, KeyPair, PublicKey};
use std::collections::HashMap;
use thiserror::Error;

/// The longest accepted host string. Bounds a client-supplied or minted host so
/// an oversized value cannot bloat the token or a datalog fact. Comfortably
/// above the DNS name limit of 253 octets.
const MAX_HOST_LEN: usize = 255;

/// The longest accepted nonce string. A nonce is a short uniqueness tag, not a
/// payload; this cap keeps it from being abused as a smuggling channel.
const MAX_NONCE_LEN: usize = 128;

/// The largest allowed-host set a mint or an attenuation may carry. Keeps the
/// token small and the authorizer's work bounded (the plan wants it to fit
/// anywhere).
const MAX_HOSTS: usize = 64;

/// What went wrong minting, verifying or attenuating a capability token. The
/// verify path never distinguishes "denied" through this type: an unauthorized
/// token is reported as `Ok(false)`, and this error is reserved for a token that
/// could not even be parsed or a caller input that was rejected before any
/// cryptographic work.
#[derive(Debug, Error)]
pub enum CapabilityError {
    /// A host, nonce or connection id was empty, over-long or otherwise
    /// unacceptable as a datalog string term.
    #[error("invalid capability input: {0}")]
    InvalidInput(&'static str),
    /// The allowed-host set was empty or exceeded [`MAX_HOSTS`]. An empty set is
    /// refused at mint time because a token that authorizes nothing is a
    /// configuration error, not a security posture we want to serialize.
    #[error("invalid host set: {0}")]
    InvalidHostSet(&'static str),
    /// The Biscuit library rejected an operation (build, serialize, parse,
    /// append). A parse or signature failure surfaces here so the caller must
    /// treat it as a hard error, never as a soft "unauthorized".
    #[error("biscuit error: {0}")]
    Biscuit(#[from] biscuit_auth::error::Token),
}

/// Validate a single string term (host, nonce, connection id): non-empty, within
/// the cap and free of the NUL and control bytes that have no place in a datalog
/// string. The charset is otherwise left open because a host may legitimately
/// carry dots, hyphens and, for an internationalized name, punycode.
fn validate_term(value: &str, max_len: usize, what: &'static str) -> Result<(), CapabilityError> {
    if value.is_empty() {
        return Err(CapabilityError::InvalidInput(what));
    }
    if value.len() > max_len {
        return Err(CapabilityError::InvalidInput(what));
    }
    if value.bytes().any(|b| b.is_ascii_control()) {
        return Err(CapabilityError::InvalidInput(what));
    }
    Ok(())
}

/// Validate and normalize an allowed-host set: non-empty, within [`MAX_HOSTS`],
/// each host a valid term. Returns the hosts owned so the caller can move them
/// into the datalog builder. Duplicates are left as-is (they are harmless in a
/// membership test).
fn validate_hosts(hosts: &[String]) -> Result<Vec<String>, CapabilityError> {
    if hosts.is_empty() {
        return Err(CapabilityError::InvalidHostSet("empty host set"));
    }
    if hosts.len() > MAX_HOSTS {
        return Err(CapabilityError::InvalidHostSet("too many hosts"));
    }
    for h in hosts {
        validate_term(h, MAX_HOST_LEN, "host")?;
    }
    Ok(hosts.to_vec())
}

/// Mint a destination-scoped capability token.
///
/// The authority block, signed by `root`, carries: one `host(<h>)` fact per
/// allowed host, the `connection(<id>)` and `nonce(<n>)` context facts, and a
/// TTL check `check if time($t), $t <= <expiry>`. The TTL is enforced as a check
/// inside the token so an expired token fails regardless of the verifying
/// authorizer's own policy, and it fails closed if a verifier ever forgets to
/// supply the current time (no `time` fact means the check cannot succeed).
///
/// `expiry_unix` is an absolute expiry in unix seconds; a non-positive value is
/// rejected. Returns the URL-safe base64 serialization of the token.
pub fn mint_token(
    root: &KeyPair,
    connection_id: &str,
    allowed_hosts: &[String],
    expiry_unix: i64,
    nonce: &str,
) -> Result<String, CapabilityError> {
    validate_term(connection_id, 128, "connection id")?;
    validate_term(nonce, MAX_NONCE_LEN, "nonce")?;
    let hosts = validate_hosts(allowed_hosts)?;
    if expiry_unix <= 0 {
        return Err(CapabilityError::InvalidInput("expiry not positive"));
    }

    let mut builder: BiscuitBuilder = Biscuit::builder();

    // Context facts. The connection id and nonce are parameterized so no caller
    // string is ever interpolated into datalog source directly.
    let mut params: HashMap<String, Term> = HashMap::new();
    params.insert("conn".to_string(), Term::Str(connection_id.to_string()));
    params.insert("nonce".to_string(), Term::Str(nonce.to_string()));
    builder = builder.code_with_params(
        "connection({conn}); nonce({nonce});",
        params,
        HashMap::new(),
    )?;

    // One host fact per allowed host, each a parameter (never interpolated).
    for (i, host) in hosts.iter().enumerate() {
        let mut hp: HashMap<String, Term> = HashMap::new();
        let key = format!("h{i}");
        hp.insert(key.clone(), Term::Str(host.clone()));
        builder = builder.code_with_params(format!("host({{{key}}});"), hp, HashMap::new())?;
    }

    // The TTL check. `time` is a Biscuit date (unix seconds); a verifier that
    // supplies no `time` fact leaves this check unsatisfiable, so it fails
    // closed. The expiry bound is a parameter, not interpolated source.
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

/// Verify that a token authorizes reaching `requested_host` at `now_unix`.
///
/// Fails closed on every adverse condition: a malformed base64 or a bad
/// signature is an `Err` (the caller must not proceed); a well-formed token that
/// does not authorize the host, or that has expired, returns `Ok(false)`. Only a
/// token whose authority allows the host and whose TTL check passes at `now_unix`
/// returns `Ok(true)`.
///
/// The authorizer supplies `time(now)`, `requested_host(host)`, and (when a
/// connection is required) `requested_connection(conn)`, plus one allow policy
/// scoped `trusting authority`, so the membership tests read only the authority
/// block's own facts. Biscuit's default deny means any unmatched or failed check
/// ends as an `Err` from `authorize`, which we fold to `Ok(false)`.
///
/// `requested_connection` binds the token to the connection whose credential is
/// about to be released: when `Some(conn)`, the token's authority-block
/// `connection` fact must equal `conn`, so a token minted for connection A cannot
/// unlock connection B even when the two connections share a destination host.
/// When `None`, only the host and TTL are checked (the pure destination-scope
/// property); the delivery path always passes `Some`.
pub fn verify_token(
    token_b64: &str,
    root_public: &PublicKey,
    requested_host: &str,
    requested_connection: Option<&str>,
    now_unix: i64,
) -> Result<bool, CapabilityError> {
    validate_term(requested_host, MAX_HOST_LEN, "host")?;
    if let Some(conn) = requested_connection {
        validate_term(conn, 128, "connection id")?;
    }
    if now_unix < 0 {
        return Err(CapabilityError::InvalidInput("negative time"));
    }

    // Parse and verify the signature against the root public key. A bad
    // signature or a malformed token is a hard error, never a soft deny.
    let token = Biscuit::from_base64(token_b64, root_public)?;

    let mut params: HashMap<String, Term> = HashMap::new();
    params.insert("now".to_string(), Term::Date(now_unix as u64));
    params.insert("req".to_string(), Term::Str(requested_host.to_string()));

    // The allow policy always requires the host; when a connection is required it
    // also requires the authority-block `connection` fact to equal the requested
    // connection. `trusting authority` keeps both membership tests blind to any
    // fact a later attenuation block might carry, so a holder cannot re-broaden.
    // The TTL check inside the token consumes the same `time` fact.
    let source = match requested_connection {
        Some(conn) => {
            params.insert("reqconn".to_string(), Term::Str(conn.to_string()));
            "time({now}); requested_host({req}); requested_connection({reqconn}); \
             allow if requested_host($h), host($h), connection($c), requested_connection($c) trusting authority;"
        }
        None => {
            "time({now}); requested_host({req}); \
             allow if requested_host($h), host($h) trusting authority;"
        }
    };

    let authorizer: AuthorizerBuilder =
        AuthorizerBuilder::new().code_with_params(source, params, HashMap::new())?;

    let mut built = authorizer.build(&token)?;
    match built.authorize() {
        Ok(_) => Ok(true),
        // A denied or expired token is an authorization outcome, not a fault.
        Err(_) => Ok(false),
    }
}

/// Attenuate a token by restricting its allowed hosts to `subset`.
///
/// Appends a check `check if requested_host($h), [$h == s0 || $h == s1 || ...]`
/// referencing the subset. Because the check must also pass at verify time, a
/// host outside the subset is now denied. The append is monotonic: it can only
/// add a constraint, never remove one, and it cannot introduce a host the
/// original did not allow (a host in `subset` that the authority never granted
/// simply never matches a `host` fact, so it stays unreachable). The caller
/// passes the subset it wants; passing a host outside the original set does not
/// broaden authority, it only writes a caveat that can never be satisfied for
/// that host.
///
/// Returns the URL-safe base64 of the attenuated token. `subset` must be
/// non-empty and within [`MAX_HOSTS`].
pub fn attenuate_token(
    token_b64: &str,
    root_public: &PublicKey,
    subset: &[String],
) -> Result<String, CapabilityError> {
    let hosts = validate_hosts(subset)?;

    // Parse and verify before attenuating: we only ever narrow a token we could
    // have verified, so a malformed or wrong-key token is rejected up front.
    let token = Biscuit::from_base64(token_b64, root_public)?;

    // Build the disjunction `$h == {s0} || $h == {s1} || ...` over the subset,
    // each host a parameter. The check reads the authorizer's requested_host
    // fact, so it constrains the request, not the token's own host set.
    let mut params: HashMap<String, Term> = HashMap::new();
    let mut clauses: Vec<String> = Vec::with_capacity(hosts.len());
    for (i, host) in hosts.iter().enumerate() {
        let key = format!("s{i}");
        params.insert(key.clone(), Term::Str(host.clone()));
        clauses.push(format!("$h == {{{key}}}"));
    }
    let source = format!("check if requested_host($h), {};", clauses.join(" || "));

    let block: BlockBuilder =
        BlockBuilder::new().code_with_params(source, params, HashMap::new())?;
    let attenuated = token.append(block)?;
    Ok(attenuated.to_base64()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hosts(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    // A fixed far-future expiry (well past any test run) in unix seconds.
    const FAR_FUTURE: i64 = 4_102_444_800; // 2100-01-01
    const NOW: i64 = 1_700_000_000; // 2023-11-14, before FAR_FUTURE

    #[test]
    fn mint_then_verify_allowed_host_passes() {
        let root = KeyPair::new();
        let token = mint_token(
            &root,
            "github",
            &hosts(&["api.github.com", "uploads.github.com"]),
            FAR_FUTURE,
            "nonce-1",
        )
        .unwrap();

        assert!(verify_token(&token, &root.public(), "api.github.com", None, NOW).unwrap());
        assert!(verify_token(&token, &root.public(), "uploads.github.com", None, NOW).unwrap());
    }

    #[test]
    fn connection_binding_is_enforced_when_requested() {
        // A token minted for "github" is bound to that connection: with the same
        // authorized host, it verifies for connection "github" but NOT for a
        // different connection - the cross-connection replay the review flagged.
        let root = KeyPair::new();
        let token = mint_token(&root, "github", &hosts(&["shared.example"]), FAR_FUTURE, "n").unwrap();
        assert!(verify_token(&token, &root.public(), "shared.example", Some("github"), NOW).unwrap());
        assert!(!verify_token(&token, &root.public(), "shared.example", Some("gitlab"), NOW).unwrap());
        // With no connection required, the host-only check still passes (the pure
        // destination-scope property the other tests rely on).
        assert!(verify_token(&token, &root.public(), "shared.example", None, NOW).unwrap());
    }

    #[test]
    fn disallowed_host_fails() {
        let root = KeyPair::new();
        let token = mint_token(&root, "github", &hosts(&["api.github.com"]), FAR_FUTURE, "n").unwrap();

        // A host the token never granted is denied, not errored.
        assert!(!verify_token(&token, &root.public(), "evil.example.com", None, NOW).unwrap());
        // A near miss (subdomain confusion) is also denied: membership is exact.
        assert!(!verify_token(&token, &root.public(), "api.github.com.evil.com", None, NOW).unwrap());
        assert!(!verify_token(&token, &root.public(), "github.com", None, NOW).unwrap());
    }

    #[test]
    fn expired_token_fails() {
        let root = KeyPair::new();
        let expiry = 1_600_000_000; // 2020-09-13
        let token = mint_token(&root, "github", &hosts(&["api.github.com"]), expiry, "n").unwrap();

        // Now is after expiry: denied.
        let after = expiry + 1;
        assert!(!verify_token(&token, &root.public(), "api.github.com", None, after).unwrap());
        // Exactly at expiry: still valid (the check is `<=`).
        assert!(verify_token(&token, &root.public(), "api.github.com", None, expiry).unwrap());
        // Before expiry: valid.
        assert!(verify_token(&token, &root.public(), "api.github.com", None, expiry - 1).unwrap());
    }

    #[test]
    fn wrong_root_public_key_fails() {
        let root = KeyPair::new();
        let attacker = KeyPair::new();
        let token = mint_token(&root, "github", &hosts(&["api.github.com"]), FAR_FUTURE, "n").unwrap();

        // Verifying with a different root public key must fail the signature
        // check and surface as an error, never as a soft allow.
        let err = verify_token(&token, &attacker.public(), "api.github.com", None, NOW);
        assert!(matches!(err, Err(CapabilityError::Biscuit(_))));
    }

    #[test]
    fn tampered_or_garbage_token_fails_closed() {
        let root = KeyPair::new();

        // Pure garbage base64.
        assert!(verify_token("not-a-real-token", &root.public(), "api.github.com", None, NOW).is_err());
        // Empty string.
        assert!(verify_token("", &root.public(), "api.github.com", None, NOW).is_err());

        // A valid token with one byte flipped in its base64 payload.
        let token = mint_token(&root, "github", &hosts(&["api.github.com"]), FAR_FUTURE, "n").unwrap();
        let mut bytes = token.into_bytes();
        let last = bytes.len() - 1;
        // Flip the final char to a different valid base64 char to corrupt the signature.
        bytes[last] = if bytes[last] == b'A' { b'B' } else { b'A' };
        let tampered = String::from_utf8(bytes).unwrap();
        assert!(verify_token(&tampered, &root.public(), "api.github.com", None, NOW).is_err());
    }

    #[test]
    fn attenuation_narrows_and_cannot_rebroaden() {
        let root = KeyPair::new();
        let token = mint_token(
            &root,
            "github",
            &hosts(&["api.github.com", "uploads.github.com", "raw.github.com"]),
            FAR_FUTURE,
            "n",
        )
        .unwrap();

        // Restrict to a subset of the original hosts.
        let narrowed =
            attenuate_token(&token, &root.public(), &hosts(&["api.github.com"])).unwrap();

        // The retained host still verifies.
        assert!(verify_token(&narrowed, &root.public(), "api.github.com", None, NOW).unwrap());
        // A host removed by attenuation now fails, even though the original token allowed it.
        assert!(!verify_token(&narrowed, &root.public(), "uploads.github.com", None, NOW).unwrap());
        assert!(!verify_token(&narrowed, &root.public(), "raw.github.com", None, NOW).unwrap());

        // Attenuation cannot re-broaden to a host the original never allowed:
        // naming it in the subset writes a caveat that never matches a granted
        // host fact, so the host stays unreachable.
        let attempted_broaden =
            attenuate_token(&token, &root.public(), &hosts(&["new.evil.com"])).unwrap();
        assert!(!verify_token(&attempted_broaden, &root.public(), "new.evil.com", None, NOW).unwrap());
        // And it cannot even reach the original hosts, since the caveat now
        // only permits new.evil.com, which is not granted: the token authorizes nothing.
        assert!(!verify_token(&attempted_broaden, &root.public(), "api.github.com", None, NOW).unwrap());
    }

    #[test]
    fn double_attenuation_stays_monotonic() {
        let root = KeyPair::new();
        let token = mint_token(
            &root,
            "github",
            &hosts(&["a.example.com", "b.example.com", "c.example.com"]),
            FAR_FUTURE,
            "n",
        )
        .unwrap();

        let step1 =
            attenuate_token(&token, &root.public(), &hosts(&["a.example.com", "b.example.com"]))
                .unwrap();
        // Second attenuation narrows further to just a.example.com.
        let step2 = attenuate_token(&step1, &root.public(), &hosts(&["a.example.com"])).unwrap();

        assert!(verify_token(&step2, &root.public(), "a.example.com", None, NOW).unwrap());
        // b was dropped by the second step, c by the first: both denied.
        assert!(!verify_token(&step2, &root.public(), "b.example.com", None, NOW).unwrap());
        assert!(!verify_token(&step2, &root.public(), "c.example.com", None, NOW).unwrap());

        // A second step cannot re-widen to c, dropped in the first step.
        let widen_attempt =
            attenuate_token(&step1, &root.public(), &hosts(&["c.example.com"])).unwrap();
        assert!(!verify_token(&widen_attempt, &root.public(), "c.example.com", None, NOW).unwrap());
    }

    #[test]
    fn mint_rejects_bad_input() {
        let root = KeyPair::new();
        // Empty host set.
        assert!(matches!(
            mint_token(&root, "github", &[], FAR_FUTURE, "n"),
            Err(CapabilityError::InvalidHostSet(_))
        ));
        // Empty connection id.
        assert!(matches!(
            mint_token(&root, "", &hosts(&["a.com"]), FAR_FUTURE, "n"),
            Err(CapabilityError::InvalidInput(_))
        ));
        // Non-positive expiry.
        assert!(matches!(
            mint_token(&root, "github", &hosts(&["a.com"]), 0, "n"),
            Err(CapabilityError::InvalidInput(_))
        ));
        // Control char in a host.
        assert!(matches!(
            mint_token(&root, "github", &hosts(&["a\n.com"]), FAR_FUTURE, "n"),
            Err(CapabilityError::InvalidInput(_))
        ));
    }
}

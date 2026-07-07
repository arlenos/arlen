//! The Authorize->Execute one-time proof (HIGH-1, `pi-gate-class-registry.md` gate
//! enforcement). The gate CLASSIFIES at Authorize but the daemon must also ENFORCE
//! at Execute, or a caller can skip Authorize and call Execute directly (the review
//! found `Dispatcher::execute` calls the executor with no gate). The fix is the
//! standard capability pattern: Authorize mints a short-lived, single-use PROOF
//! bound to `(tool_name, args_hash, session)` and returns it; Execute must present a
//! valid, unconsumed, matching proof before the executor runs.
//!
//! - **Allow** class -> a proof is minted immediately.
//! - **Confirm** class -> a proof is minted ONLY after the consent broker resolves
//!   the confirm (so a Confirm tool cannot run without a resolved confirm).
//! - **Deny** / unknown -> no proof (so it can never run).
//!
//! The proof binds the exact args (`args_hash`), so an attacker cannot Authorize
//! benign args and Execute malicious ones; it binds the `session`, so one session's
//! proof cannot run another's; it is single-use (consumed on Execute) and expires,
//! so it cannot be replayed. This module is the pure store; the dispatch wiring
//! (mint at Authorize, consume at Execute) and the contract fields compose on it.

use std::collections::HashMap;

use sha2::{Digest, Sha256};

use crate::session::CsprngError;

/// Bytes of CSPRNG entropy in a proof handle (256 bits, hex-encoded, unguessable).
const HANDLE_BYTES: usize = 32;

/// Why a one-time proof could not be consumed at Execute. All are refusals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProofError {
    /// No such handle: never minted, or already consumed (single-use), or swept.
    Unknown,
    /// The handle exists but its binding does not match this Execute's
    /// `(tool_name, args_hash, session)` - a proof minted for one call cannot
    /// execute a different tool, different args, or a different session.
    Mismatch,
    /// The handle exists and matches but its TTL has passed (replay window closed).
    Expired,
}

/// The canonical hash of a tool call's arguments. Binds a proof to the exact
/// `tool_input` so a proof minted for benign args cannot execute different ones.
/// Uses [`canonical_encode`] (object keys sorted, strings/keys length-prefixed) so
/// Authorize and Execute hash identical args identically REGARDLESS of the
/// `serde_json` map backend (default `BTreeMap` or a `preserve_order` `IndexMap`)
/// or the engine's key ordering - not relying on `serde_json`'s serialization order.
pub fn hash_args(tool_input: &serde_json::Value) -> [u8; 32] {
    let mut bytes = Vec::new();
    canonical_encode(tool_input, &mut bytes);
    Sha256::digest(&bytes).into()
}

/// Append an order-independent, unambiguous byte encoding of `value` to `out`.
/// Object keys are sorted; strings and keys are length-prefixed so distinct values
/// (e.g. `{"ab":1}` vs `{"a":"b1"}`) can never collide by concatenation.
fn canonical_encode(value: &serde_json::Value, out: &mut Vec<u8>) {
    use serde_json::Value;
    match value {
        Value::Null => out.extend_from_slice(b"n"),
        Value::Bool(b) => out.extend_from_slice(if *b { b"t" } else { b"f" }),
        Value::Number(n) => {
            let s = n.to_string();
            out.extend_from_slice(format!("d{}:", s.len()).as_bytes());
            out.extend_from_slice(s.as_bytes());
        }
        Value::String(s) => {
            out.extend_from_slice(format!("s{}:", s.len()).as_bytes());
            out.extend_from_slice(s.as_bytes());
        }
        Value::Array(a) => {
            out.extend_from_slice(format!("a{}:", a.len()).as_bytes());
            for v in a {
                canonical_encode(v, out);
            }
        }
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.extend_from_slice(format!("o{}:", keys.len()).as_bytes());
            for k in keys {
                out.extend_from_slice(format!("k{}:", k.len()).as_bytes());
                out.extend_from_slice(k.as_bytes());
                canonical_encode(&map[k], out);
            }
        }
    }
}

/// A hard cap on live proofs: a flood backstop so a misbehaving-but-authenticated
/// engine spamming admitted Authorize without ever executing cannot grow the store
/// unbounded within the TTL window. Generous vs any real per-session authorize rate.
pub const MAX_PROOFS: usize = 4096;

/// Mint an unguessable 256-bit proof handle (hex). Mirrors the session-token
/// CSPRNG; fails closed if the OS RNG is unavailable (no proof -> no execute).
pub fn new_handle() -> Result<String, CsprngError> {
    let mut bytes = [0u8; HANDLE_BYTES];
    getrandom::getrandom(&mut bytes).map_err(|e| CsprngError(e.to_string()))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// One minted, not-yet-consumed proof.
struct Proof {
    tool_name: String,
    args_hash: [u8; 32],
    session: String,
    expires_at_ms: u64,
}

/// The daemon's in-memory store of live execution proofs, keyed by handle. Single
/// owner (behind the dispatcher); not `Clone`.
#[derive(Default)]
pub struct ProofStore {
    proofs: HashMap<String, Proof>,
}

impl ProofStore {
    /// An empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a minted proof under `handle`, bound to `(tool_name, args_hash,
    /// session)` and expiring at `expires_at_ms`. The caller mints the CSPRNG
    /// `handle` ([`new_handle`]) and computes the expiry from its clock, so the
    /// store stays pure and testable.
    pub fn mint(
        &mut self,
        handle: String,
        tool_name: String,
        args_hash: [u8; 32],
        session: String,
        expires_at_ms: u64,
    ) {
        self.proofs.insert(
            handle,
            Proof {
                tool_name,
                args_hash,
                session,
                expires_at_ms,
            },
        );
    }

    /// Validate and CONSUME the proof for an Execute. On success the proof is
    /// removed (single-use). A mismatched presentation does NOT consume the proof
    /// (a legitimate later Execute with the right binding can still use it); an
    /// expired one is removed. `now_ms` is the caller's monotonic clock.
    pub fn consume(
        &mut self,
        handle: &str,
        tool_name: &str,
        args_hash: &[u8; 32],
        session: &str,
        now_ms: u64,
    ) -> Result<(), ProofError> {
        let proof = self.proofs.get(handle).ok_or(ProofError::Unknown)?;
        if proof.tool_name != tool_name
            || &proof.args_hash != args_hash
            || proof.session != session
        {
            return Err(ProofError::Mismatch);
        }
        if now_ms > proof.expires_at_ms {
            self.proofs.remove(handle);
            return Err(ProofError::Expired);
        }
        self.proofs.remove(handle);
        Ok(())
    }

    /// Drop every expired proof (bounded memory). Called periodically / at mint.
    pub fn sweep(&mut self, now_ms: u64) {
        self.proofs.retain(|_, p| now_ms <= p.expires_at_ms);
    }

    /// The number of live proofs (for tests / introspection).
    pub fn len(&self) -> usize {
        self.proofs.len()
    }

    /// Whether the store is empty.
    pub fn is_empty(&self) -> bool {
        self.proofs.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn h(v: serde_json::Value) -> [u8; 32] {
        hash_args(&v)
    }

    #[test]
    fn a_minted_proof_is_consumed_exactly_once() {
        let mut s = ProofStore::new();
        let args = json!({"cypher": "CREATE (n)"});
        s.mint("H".into(), "graph.write".into(), h(args.clone()), "sess".into(), 100);
        // First Execute with the right binding succeeds.
        assert_eq!(s.consume("H", "graph.write", &h(args.clone()), "sess", 50), Ok(()));
        // A replay of the same handle fails (single-use, removed).
        assert_eq!(
            s.consume("H", "graph.write", &h(args), "sess", 50),
            Err(ProofError::Unknown)
        );
    }

    #[test]
    fn a_proof_binds_tool_args_and_session() {
        let mut s = ProofStore::new();
        let args = json!({"cypher": "CREATE (n)"});
        s.mint("H".into(), "graph.write".into(), h(args.clone()), "sess".into(), 100);
        // Wrong tool, wrong args, wrong session all mismatch (and do NOT consume).
        assert_eq!(
            s.consume("H", "graph.set_field", &h(args.clone()), "sess", 50),
            Err(ProofError::Mismatch)
        );
        assert_eq!(
            s.consume("H", "graph.write", &h(json!({"cypher": "DELETE (n)"})), "sess", 50),
            Err(ProofError::Mismatch)
        );
        assert_eq!(
            s.consume("H", "graph.write", &h(args.clone()), "other", 50),
            Err(ProofError::Mismatch)
        );
        // The right binding still works afterwards (mismatch didn't consume it).
        assert_eq!(s.consume("H", "graph.write", &h(args), "sess", 50), Ok(()));
    }

    #[test]
    fn an_expired_proof_is_refused() {
        let mut s = ProofStore::new();
        let args = json!({});
        s.mint("H".into(), "fs.move".into(), h(args.clone()), "sess".into(), 100);
        assert_eq!(
            s.consume("H", "fs.move", &h(args), "sess", 101),
            Err(ProofError::Expired)
        );
        assert!(s.is_empty(), "an expired proof is removed on consume");
    }

    #[test]
    fn an_unminted_handle_is_unknown() {
        let mut s = ProofStore::new();
        assert_eq!(
            s.consume("nope", "graph.write", &h(json!({})), "sess", 0),
            Err(ProofError::Unknown)
        );
    }

    #[test]
    fn sweep_drops_expired_proofs() {
        let mut s = ProofStore::new();
        s.mint("A".into(), "t".into(), [0; 32], "s".into(), 10);
        s.mint("B".into(), "t".into(), [0; 32], "s".into(), 100);
        s.sweep(50);
        assert_eq!(s.len(), 1, "only the still-live proof survives");
    }

    #[test]
    fn hash_args_is_key_order_independent_and_unambiguous() {
        // Two objects with the same entries in different key order hash equally,
        // so a re-serialized-but-equal args set is not falsely refused.
        let a = json!({"x": 1, "y": 2});
        let b = json!({"y": 2, "x": 1});
        assert_eq!(hash_args(&a), hash_args(&b));
        // Concatenation-ambiguous shapes stay distinct (length-prefixing).
        assert_ne!(hash_args(&json!({"ab": 1})), hash_args(&json!({"a": "b1"})));
        assert_ne!(hash_args(&json!("12")), hash_args(&json!(12)));
        assert_ne!(hash_args(&json!([1, 2])), hash_args(&json!([12])));
    }

    #[test]
    fn new_handle_is_unguessable_and_distinct() {
        let a = new_handle().unwrap();
        let b = new_handle().unwrap();
        assert_eq!(a.len(), HANDLE_BYTES * 2, "256-bit hex");
        assert_ne!(a, b);
    }
}

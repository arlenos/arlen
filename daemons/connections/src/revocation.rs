//! On-process-exit and expiry revocation of derived tokens (CONN-R2,
//! connections-plan.md §2 property 2: "on process exit the derived token is
//! revoked").
//!
//! The daemon tracks each derived token it mints by the caller PID that received
//! it and its expiry. When that process exits (a monitor detects the exit) the
//! registry revokes all its tokens; expired tokens are swept the same way. The
//! registry is the pure bookkeeping: it marks a token dead and yields the ids to
//! revoke upstream (the provider revoke call is the network delivery built on
//! top). A revoked or expired token is never considered live, so a leaked derived
//! token stops working the moment its owner is gone.

use std::collections::HashMap;

/// One derived token's registry record.
struct Record {
    caller_pid: i32,
    expires_at_micros: i64,
    revoked: bool,
}

/// The derived-token revocation registry, keyed by an opaque token id.
#[derive(Default)]
pub struct RevocationRegistry {
    tokens: HashMap<String, Record>,
}

impl RevocationRegistry {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a freshly-minted derived token owned by `caller_pid`, keyed by an
    /// opaque `token_id`, expiring at `expires_at_micros`. Re-registering a token
    /// id replaces the record (a fresh mint for the same handle).
    pub fn register(&mut self, token_id: String, caller_pid: i32, expires_at_micros: i64) {
        self.tokens.insert(
            token_id,
            Record {
                caller_pid,
                expires_at_micros,
                revoked: false,
            },
        );
    }

    /// Whether a token is live: known, not revoked, and not past `now_micros`.
    /// An unknown token is never live (fail closed).
    pub fn is_live(&self, token_id: &str, now_micros: i64) -> bool {
        self.tokens
            .get(token_id)
            .is_some_and(|r| !r.revoked && r.expires_at_micros > now_micros)
    }

    /// Revoke every token owned by `pid` (its process exited). Returns the ids
    /// newly revoked, so the caller can revoke them upstream at the provider.
    pub fn revoke_for_pid(&mut self, pid: i32) -> Vec<String> {
        let mut revoked = Vec::new();
        for (id, r) in self.tokens.iter_mut() {
            if r.caller_pid == pid && !r.revoked {
                r.revoked = true;
                revoked.push(id.clone());
            }
        }
        revoked
    }

    /// Mark every token expired at `now_micros` revoked, returning their ids.
    pub fn revoke_expired(&mut self, now_micros: i64) -> Vec<String> {
        let mut expired = Vec::new();
        for (id, r) in self.tokens.iter_mut() {
            if !r.revoked && r.expires_at_micros <= now_micros {
                r.revoked = true;
                expired.push(id.clone());
            }
        }
        expired
    }

    /// Drop revoked records to bound memory. Call after the upstream revoke so the
    /// registry does not grow without bound over the session.
    pub fn forget_revoked(&mut self) {
        self.tokens.retain(|_, r| !r.revoked);
    }

    /// The number of tracked (not-yet-forgotten) tokens.
    pub fn len(&self) -> usize {
        self.tokens.len()
    }

    /// Whether the registry tracks no tokens.
    pub fn is_empty(&self) -> bool {
        self.tokens.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_registered_token_is_live_until_expiry() {
        let mut reg = RevocationRegistry::new();
        reg.register("t1".into(), 100, 1_000);
        assert!(reg.is_live("t1", 500));
        assert!(!reg.is_live("t1", 1_000)); // at expiry
        assert!(!reg.is_live("t1", 2_000)); // past expiry
        assert!(!reg.is_live("unknown", 0)); // unknown never live
    }

    #[test]
    fn revoke_for_pid_revokes_only_that_pids_tokens() {
        let mut reg = RevocationRegistry::new();
        reg.register("a".into(), 100, 9_999);
        reg.register("b".into(), 100, 9_999);
        reg.register("c".into(), 200, 9_999);
        let mut revoked = reg.revoke_for_pid(100);
        revoked.sort();
        assert_eq!(revoked, vec!["a".to_string(), "b".to_string()]);
        assert!(!reg.is_live("a", 0));
        assert!(!reg.is_live("b", 0));
        assert!(reg.is_live("c", 0)); // another pid's token untouched
        // Revoking again yields nothing (already revoked).
        assert!(reg.revoke_for_pid(100).is_empty());
    }

    #[test]
    fn revoke_expired_sweeps_past_deadline() {
        let mut reg = RevocationRegistry::new();
        reg.register("old".into(), 100, 1_000);
        reg.register("new".into(), 100, 5_000);
        let expired = reg.revoke_expired(2_000);
        assert_eq!(expired, vec!["old".to_string()]);
        assert!(!reg.is_live("old", 2_000));
        assert!(reg.is_live("new", 2_000));
    }

    #[test]
    fn forget_revoked_bounds_memory() {
        let mut reg = RevocationRegistry::new();
        reg.register("a".into(), 100, 9_999);
        reg.register("b".into(), 100, 9_999);
        reg.revoke_for_pid(100);
        assert_eq!(reg.len(), 2);
        reg.forget_revoked();
        assert!(reg.is_empty());
    }
}

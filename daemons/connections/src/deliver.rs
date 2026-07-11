//! The credential-to-egress DELIVERY composition (connections-plan.md §7b).
//!
//! This is the credential-injection boundary: the only path by which a RAW stored
//! credential leaves the daemon, and it leaves ONLY to the trusted net-guard (the
//! generalised ai-proxy) so it can inject the key into an outbound request at the
//! egress boundary. Everywhere else the daemon downscopes or proxies; here, for an
//! un-attenuable API key / webhook secret (Proxy mode), the raw key must reach the
//! net-guard to be injected.
//!
//! Confused-deputy defense (the Secretless gap the plan calls out): a raw key is
//! released ONLY when BOTH hold, checked BEFORE the credential is ever loaded so a
//! failed check is never even an oracle on which connections hold a credential:
//!
//! 1. the caller is an attested egress authoriser (`caller` in the net-guard
//!    allowlist - the kernel/bus-attested `ai-proxy` id), AND
//! 2. the caller presents a request-bound, destination-scoped Biscuit capability
//!    token that verifies for the requested destination host under the daemon root
//!    public key and is not expired ([`crate::capability::verify_token`]).
//!
//! Only then is the credential loaded, checked to be Proxy mode (an OAuth refresh
//! token is narrowable and goes the RFC 8693 path, never raw egress), audited
//! BEFORE it is returned (S13 audit-before-act, fail-closed so no raw key ever
//! leaves un-audited), and handed back zeroize-on-drop.
//!
//! Host normalization is done HERE, at both mint and verify, so the exact-bytewise
//! comparison in [`crate::capability`] never rejects a legitimate host over case or
//! a trailing dot (the documented residual): [`mint_egress_capability`] normalizes
//! every allowed host and [`deliver_egress_credential`] normalizes the requested
//! host with the SAME [`normalize_host`], so a token minted here always verifies
//! here for the same host written differently.

use audit_proto::AuditSink;
use biscuit_auth::{KeyPair, PublicKey};
use zeroize::Zeroizing;

use crate::audit::credential_egress_event;
use crate::broker::ConnectionId;
use crate::capability::{mint_token, verify_token, CapabilityError};
use crate::downscope::{mode_for, DownscopeMode};
use crate::store::{CredentialStore, StoreError};

/// Why an egress delivery was refused. Every variant is fail-closed: no raw
/// credential is ever returned on any error path.
#[derive(Debug, thiserror::Error)]
pub enum DeliverError {
    /// The attested caller is not an allowlisted egress authoriser. The raw
    /// credential injection path is reachable ONLY by the net-guard.
    #[error("caller is not an allowlisted egress authoriser")]
    NotAuthorizedCaller,
    /// The presented capability token did not verify for the requested destination
    /// host (bad signature, wrong root, expired, or host not in the token scope).
    #[error("capability token rejected")]
    TokenRejected,
    /// The connection id is malformed.
    #[error("unknown connection")]
    UnknownConnection,
    /// The requested destination host is empty after normalization.
    #[error("empty destination host")]
    EmptyHost,
    /// Authorized, but no credential is stored for the connection.
    #[error("no credential stored")]
    NoCredential,
    /// The credential is not a Proxy-mode secret (an OAuth refresh token must take
    /// the RFC 8693 exchange path, never raw egress injection).
    #[error("credential is not injectable at the egress boundary")]
    NotProxyMode,
    /// The credential store failed.
    #[error("store: {0}")]
    Store(String),
    /// The audit ledger was unreachable, so the release was refused rather than a
    /// raw key returned un-audited (fail closed).
    #[error("audit unavailable")]
    AuditUnavailable,
    /// Minting the capability token failed (bad host/nonce/expiry inputs).
    #[error("mint: {0}")]
    Mint(CapabilityError),
}

/// Normalize a host for capability matching: trim, lowercase, and strip a single
/// trailing dot (the FQDN root label). Applied identically at mint and verify so
/// the exact-bytewise token comparison never rejects a host over case or a trailing
/// dot. It does NOT do IDNA/punycode canonicalization; a caller that accepts
/// unicode hosts must punycode them before this, consistently at mint and verify.
pub fn normalize_host(host: &str) -> String {
    let trimmed = host.trim().trim_end_matches('.');
    trimmed.to_ascii_lowercase()
}

/// Mint a destination-scoped capability token for a connection's egress, binding
/// the request to `allowed_hosts` (each normalized), a TTL, and a nonce. The token
/// is handed to the app/net-guard and presented back at
/// [`deliver_egress_credential`]. The daemon holds the private half; any verifier
/// checks it with only the root public key.
pub fn mint_egress_capability(
    root: &KeyPair,
    connection: &str,
    allowed_hosts: &[String],
    expiry_unix: i64,
    nonce: &str,
) -> Result<String, DeliverError> {
    let normalized: Vec<String> = allowed_hosts.iter().map(|h| normalize_host(h)).collect();
    mint_token(root, connection, &normalized, expiry_unix, nonce).map_err(DeliverError::Mint)
}

/// The inputs the transport resolves for one egress-credential delivery. `caller`
/// is the KERNEL/BUS-ATTESTED net-guard id (never a client-supplied value);
/// `presented_token` is the base64 Biscuit the caller relays; `destination_host` is
/// the host of the outbound request the caller is about to make.
pub struct EgressRequest<'a> {
    /// The attested caller id (must be an allowlisted egress authoriser).
    pub caller: &'a str,
    /// The connection whose credential is requested (e.g. `anthropic`).
    pub connection: &'a str,
    /// The base64 capability token the caller presents.
    pub presented_token: &'a str,
    /// The destination host of the outbound request.
    pub destination_host: &'a str,
    /// Current unix time (seconds), for the token expiry check.
    pub now_unix: i64,
}

/// Deliver the raw Proxy-mode credential to an attested, capability-bearing egress
/// authoriser. See the module docs for the full gate. On success the raw secret is
/// returned zeroize-on-drop; on any failure nothing is returned (fail closed), and
/// the outcome is audited (a release before it is returned; a denial best-effort).
pub async fn deliver_egress_credential(
    req: &EgressRequest<'_>,
    proxy_allowlist: &[String],
    root_public: &PublicKey,
    store: &CredentialStore,
    audit: &dyn AuditSink,
) -> Result<Zeroizing<Vec<u8>>, DeliverError> {
    // (1) Caller must be an allowlisted egress authoriser. A non-net-guard caller
    // is refused before any token work or store read.
    if !proxy_allowlist.iter().any(|a| a == req.caller) {
        best_effort_deny(audit, req.caller, req.connection).await;
        return Err(DeliverError::NotAuthorizedCaller);
    }

    // (2) The requested host, normalized the same way the token was minted.
    let host = normalize_host(req.destination_host);
    if host.is_empty() {
        best_effort_deny(audit, req.caller, req.connection).await;
        return Err(DeliverError::EmptyHost);
    }

    // (3) The presented capability token must verify for that host under the root
    // public key and be unexpired. A bad signature/garbage token is an Err from
    // verify (fail closed); an unauthorized/expired token is Ok(false).
    match verify_token(req.presented_token, root_public, &host, req.now_unix) {
        Ok(true) => {}
        _ => {
            best_effort_deny(audit, req.caller, req.connection).await;
            return Err(DeliverError::TokenRejected);
        }
    }

    // Both gates passed: only NOW resolve + load the credential (no oracle before
    // the decision, mirroring the store's authorize-then-load ordering).
    let Some(connection_id) = ConnectionId::new(req.connection) else {
        best_effort_deny(audit, req.caller, req.connection).await;
        return Err(DeliverError::UnknownConnection);
    };
    let credential = match store.get(&connection_id) {
        Ok(Some(c)) => c,
        Ok(None) => {
            best_effort_audit(audit, req.caller, req.connection, "no-credential").await;
            return Err(DeliverError::NoCredential);
        }
        Err(StoreError::Vault(e)) => {
            best_effort_audit(audit, req.caller, req.connection, "error").await;
            return Err(DeliverError::Store(e.to_string()));
        }
        Err(StoreError::Codec(e)) => {
            best_effort_audit(audit, req.caller, req.connection, "error").await;
            return Err(DeliverError::Store(e));
        }
    };

    // (4) Only Proxy-mode secrets inject raw at the egress boundary. An OAuth
    // refresh token is narrowable and must take the RFC 8693 exchange path.
    if mode_for(credential.kind) != DownscopeMode::Proxy {
        best_effort_deny(audit, req.caller, req.connection).await;
        return Err(DeliverError::NotProxyMode);
    }

    // (5) Audit the release BEFORE returning. A down ledger refuses the release, so
    // no raw key ever leaves un-audited.
    audit
        .submit(credential_egress_event(req.caller, req.connection, "granted"))
        .await
        .map_err(|_| DeliverError::AuditUnavailable)?;

    Ok(Zeroizing::new(credential.secret.clone()))
}

/// Audit a denial best-effort: a denial is not a release, so a down ledger must not
/// turn it into an error.
async fn best_effort_deny(audit: &dyn AuditSink, caller: &str, connection: &str) {
    best_effort_audit(audit, caller, connection, "denied").await;
}

async fn best_effort_audit(audit: &dyn AuditSink, caller: &str, connection: &str, outcome: &str) {
    let _ = audit
        .submit(credential_egress_event(caller, connection, outcome))
        .await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use audit_proto::sink::MockAuditSink;
    use biscuit_auth::KeyPair;

    use crate::broker::CredentialKind;
    use crate::store::Credential;

    const FAR_FUTURE: i64 = 4_102_444_800; // 2100-01-01
    const NOW: i64 = 1_000_000_000;

    fn store_with(kind: CredentialKind, secret: &[u8]) -> (tempfile::TempDir, CredentialStore) {
        let tmp = tempfile::TempDir::new().unwrap();
        let s = CredentialStore::new([7u8; 32], tmp.path().join("connections"));
        s.put(
            &ConnectionId::new("anthropic").unwrap(),
            &Credential {
                kind,
                secret: secret.to_vec(),
            },
        )
        .unwrap();
        (tmp, s)
    }

    fn proxy() -> Vec<String> {
        vec!["ai-proxy".to_string()]
    }

    fn token(root: &KeyPair, hosts: &[&str]) -> String {
        let hosts: Vec<String> = hosts.iter().map(|h| h.to_string()).collect();
        mint_egress_capability(root, "anthropic", &hosts, FAR_FUTURE, "nonce-1").unwrap()
    }

    #[tokio::test]
    async fn an_attested_proxy_with_a_valid_token_gets_the_raw_key() {
        let root = KeyPair::new();
        let (_t, store) = store_with(CredentialKind::ApiKey, b"sk-ant-secret");
        let audit = MockAuditSink::accepting();
        let tok = token(&root, &["api.anthropic.com"]);
        let out = deliver_egress_credential(
            &EgressRequest {
                caller: "ai-proxy",
                connection: "anthropic",
                presented_token: &tok,
                destination_host: "api.anthropic.com",
                now_unix: NOW,
            },
            &proxy(),
            &root.public(),
            &store,
            &audit,
        )
        .await
        .unwrap();
        assert_eq!(&out[..], b"sk-ant-secret");
        let recorded = audit.recorded().await;
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].structural.subject, "connection.egress");
        assert_eq!(recorded[0].structural.outcome, "granted");
    }

    #[tokio::test]
    async fn host_case_and_trailing_dot_still_match() {
        // Minted for api.anthropic.com; requested as API.Anthropic.Com. - normalizes
        // to the same host, so it verifies (the residual is closed at this layer).
        let root = KeyPair::new();
        let (_t, store) = store_with(CredentialKind::ApiKey, b"k");
        let audit = MockAuditSink::accepting();
        let tok = token(&root, &["API.Anthropic.Com"]);
        let out = deliver_egress_credential(
            &EgressRequest {
                caller: "ai-proxy",
                connection: "anthropic",
                presented_token: &tok,
                destination_host: "api.anthropic.com.",
                now_unix: NOW,
            },
            &proxy(),
            &root.public(),
            &store,
            &audit,
        )
        .await
        .unwrap();
        assert_eq!(&out[..], b"k");
    }

    #[tokio::test]
    async fn a_non_proxy_caller_is_refused_and_never_loads() {
        let root = KeyPair::new();
        let (_t, store) = store_with(CredentialKind::ApiKey, b"k");
        let audit = MockAuditSink::accepting();
        let tok = token(&root, &["api.anthropic.com"]);
        let err = deliver_egress_credential(
            &EgressRequest {
                caller: "com.evil.app",
                connection: "anthropic",
                presented_token: &tok,
                destination_host: "api.anthropic.com",
                now_unix: NOW,
            },
            &proxy(),
            &root.public(),
            &store,
            &audit,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, DeliverError::NotAuthorizedCaller));
        assert_eq!(audit.recorded().await[0].structural.outcome, "denied");
    }

    #[tokio::test]
    async fn a_token_for_a_different_host_is_rejected() {
        let root = KeyPair::new();
        let (_t, store) = store_with(CredentialKind::ApiKey, b"k");
        let audit = MockAuditSink::accepting();
        // Token authorizes only api.anthropic.com; the request targets evil.com.
        let tok = token(&root, &["api.anthropic.com"]);
        let err = deliver_egress_credential(
            &EgressRequest {
                caller: "ai-proxy",
                connection: "anthropic",
                presented_token: &tok,
                destination_host: "evil.com",
                now_unix: NOW,
            },
            &proxy(),
            &root.public(),
            &store,
            &audit,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, DeliverError::TokenRejected));
    }

    #[tokio::test]
    async fn a_token_under_a_foreign_root_is_rejected() {
        let root = KeyPair::new();
        let attacker = KeyPair::new();
        let (_t, store) = store_with(CredentialKind::ApiKey, b"k");
        let audit = MockAuditSink::accepting();
        // Minted under the attacker key, verified under the daemon root -> rejected.
        let tok = token(&attacker, &["api.anthropic.com"]);
        let err = deliver_egress_credential(
            &EgressRequest {
                caller: "ai-proxy",
                connection: "anthropic",
                presented_token: &tok,
                destination_host: "api.anthropic.com",
                now_unix: NOW,
            },
            &proxy(),
            &root.public(),
            &store,
            &audit,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, DeliverError::TokenRejected));
    }

    #[tokio::test]
    async fn an_expired_token_is_rejected() {
        let root = KeyPair::new();
        let (_t, store) = store_with(CredentialKind::ApiKey, b"k");
        let audit = MockAuditSink::accepting();
        // Mint with a near-past expiry, verify well after it.
        let tok =
            mint_egress_capability(&root, "anthropic", &["api.anthropic.com".to_string()], NOW, "n")
                .unwrap();
        let err = deliver_egress_credential(
            &EgressRequest {
                caller: "ai-proxy",
                connection: "anthropic",
                presented_token: &tok,
                destination_host: "api.anthropic.com",
                now_unix: NOW + 10_000,
            },
            &proxy(),
            &root.public(),
            &store,
            &audit,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, DeliverError::TokenRejected));
    }

    #[tokio::test]
    async fn an_oauth_credential_is_not_injectable_raw() {
        let root = KeyPair::new();
        let (_t, store) = store_with(CredentialKind::OAuthRefreshToken, b"refresh");
        let audit = MockAuditSink::accepting();
        let tok = token(&root, &["api.anthropic.com"]);
        let err = deliver_egress_credential(
            &EgressRequest {
                caller: "ai-proxy",
                connection: "anthropic",
                presented_token: &tok,
                destination_host: "api.anthropic.com",
                now_unix: NOW,
            },
            &proxy(),
            &root.public(),
            &store,
            &audit,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, DeliverError::NotProxyMode));
    }

    #[tokio::test]
    async fn a_down_ledger_refuses_the_release() {
        let root = KeyPair::new();
        let (_t, store) = store_with(CredentialKind::ApiKey, b"k");
        let audit = MockAuditSink::failing();
        let tok = token(&root, &["api.anthropic.com"]);
        let err = deliver_egress_credential(
            &EgressRequest {
                caller: "ai-proxy",
                connection: "anthropic",
                presented_token: &tok,
                destination_host: "api.anthropic.com",
                now_unix: NOW,
            },
            &proxy(),
            &root.public(),
            &store,
            &audit,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, DeliverError::AuditUnavailable));
    }

    #[tokio::test]
    async fn an_authorized_request_with_no_stored_credential_is_distinct() {
        let root = KeyPair::new();
        let tmp = tempfile::TempDir::new().unwrap();
        let store = CredentialStore::new([9u8; 32], tmp.path().join("connections"));
        let audit = MockAuditSink::accepting();
        let tok = token(&root, &["api.anthropic.com"]);
        let err = deliver_egress_credential(
            &EgressRequest {
                caller: "ai-proxy",
                connection: "anthropic",
                presented_token: &tok,
                destination_host: "api.anthropic.com",
                now_unix: NOW,
            },
            &proxy(),
            &root.public(),
            &store,
            &audit,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, DeliverError::NoCredential));
    }
}

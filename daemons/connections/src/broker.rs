//! The powerbox authorization core (connections-plan.md §2, property 1).
//!
//! An app never browses the credential store. It *requests* "a credential for
//! GitHub" at some scope; the broker checks the app's standing capability GRANT
//! and decides. This module is the pure decision: it maps a request plus the set
//! of grants to a [`BrokerDecision`], and it never touches a stored credential
//! (the store, the daemon socket, and the peer-auth are separate layers built on
//! top). Being pure, the whole authorization property is unit-tested without a
//! socket, a keystore, or a filesystem.
//!
//! The security property it enforces is **strict monotonic attenuation**: a
//! handout's scope can only SUBTRACT from the grant's ceiling, never add and
//! never exceed it (the subtract-only property of ocap/Macaroons the plan
//! mandates). Everything fails closed: no grant, an unknown connection, or a
//! requested scope that is not a subset of the ceiling all deny.

use serde::{Deserialize, Serialize};

/// A named integration connection (a credential class), e.g. `github` or
/// `google-drive`. It is the stable key under which the store seals a
/// credential and the identifier an app names in a request. The charset is
/// restricted so a connection id can never traverse a path or inject into a
/// wire form: lowercase ASCII alphanumerics plus `.`, `-`, `_`, non-empty.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ConnectionId(String);

/// The longest accepted connection id. Mirrors the vault's own record-id cap so
/// a valid `ConnectionId` is always a storable record id (a longer id would parse
/// into a live grant that could never resolve a credential, a silent dead grant),
/// and so an oversized client-supplied connection cannot reach the audit label
/// un-bounded.
const MAX_CONNECTION_ID_LEN: usize = 128;

impl ConnectionId {
    /// Build a connection id, rejecting an empty, over-long, or out-of-charset
    /// string (fail-closed: an invalid id is never coerced, it is refused).
    pub fn new(raw: &str) -> Option<Self> {
        if raw.is_empty()
            || raw.len() > MAX_CONNECTION_ID_LEN
            || !raw
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'-' | b'_'))
        {
            return None;
        }
        Some(Self(raw.to_string()))
    }

    /// The validated id as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The kind of credential a connection holds. It classifies the secret so the
/// downscoping broker (CONN-R2) knows whether an RFC 8693 exchange applies (an
/// OAuth refresh token) or the broker must proxy the call (an opaque API key or
/// webhook secret the upstream cannot narrow).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialKind {
    /// An OAuth 2.0 refresh token: downscopable via RFC 8693 token exchange.
    OAuthRefreshToken,
    /// An opaque API key: not narrowable, so the broker proxies the call.
    ApiKey,
    /// A webhook signing secret: not narrowable, broker-proxied.
    WebhookSecret,
}

/// An app's standing capability grant: which connection it may access and the
/// maximum scope it may ever request (the ceiling the broker attenuates
/// against). One app may hold several grants (one per connection). The scope
/// vocabulary is the connection's own (OAuth scope strings for an OAuth
/// provider, method names for an API), compared as opaque tokens here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionGrant {
    /// The kernel-attested app id the daemon resolves from the peer, never a
    /// value the requesting app supplies.
    pub app_id: String,
    /// The connection this grant authorizes.
    pub connection_id: ConnectionId,
    /// The ceiling: the maximum set of scope tokens a handout may carry. A
    /// request may ask for any subset, never a superset.
    pub max_scope: Vec<String>,
    /// The per-connection egress endpoint allowlist (CONN-R3): the hosts this app
    /// may reach for this connection. The daemon binds a minted capability token to
    /// exactly these hosts, so the destination scope is DECLARATIVE (from the
    /// trusted config), never a value the requesting app chooses. Empty means no
    /// host is authorized (fail-closed: no egress token can be minted).
    #[serde(default)]
    pub allowed_hosts: Vec<String>,
}

/// The declared egress allowlist for one `(app_id, connection)` pair, or `None`
/// when the app holds no grant for that connection. The mint path binds a token to
/// exactly this host set, so the destination scope comes from the trusted grant
/// config, never from the requesting app.
pub fn allowed_hosts_for<'a>(
    grants: &'a [ConnectionGrant],
    app_id: &str,
    connection: &ConnectionId,
) -> Option<&'a [String]> {
    grants
        .iter()
        .find(|g| g.app_id == app_id && &g.connection_id == connection)
        .map(|g| g.allowed_hosts.as_slice())
}

/// An app's request for a scoped credential handout.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialRequest {
    /// The requesting app (the daemon fills this from the attested peer, so the
    /// broker decision keys on identity the app cannot forge).
    pub app_id: String,
    /// The connection the app wants a credential for.
    pub connection_id: ConnectionId,
    /// The scope tokens requested. Empty means "the grant's full ceiling" (the
    /// app asks for everything it is entitled to, still bounded by the grant).
    pub requested_scope: Vec<String>,
}

/// Why a request was denied. Content-free enough to audit without leaking the
/// scope vocabulary of a connection the caller has no grant for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DenyReason {
    /// The app holds no grant for this connection.
    NoGrant,
    /// The requested scope is not a subset of the grant's ceiling (an attempt to
    /// amplify beyond the capability).
    ScopeExceedsCeiling,
}

/// The broker's decision for a request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "decision")]
pub enum BrokerDecision {
    /// Authorized: hand out a credential scoped to exactly `scope` (the
    /// attenuated set, always a subset of the grant ceiling). The store/mint
    /// layer then seals or exchanges the real credential to this scope.
    Grant {
        /// The connection to hand a credential for.
        connection_id: ConnectionId,
        /// The granted scope: the request's subset of the ceiling, or the full
        /// ceiling when the request asked for everything (empty request scope).
        scope: Vec<String>,
    },
    /// Refused, with a reason.
    Deny {
        /// Why the request was refused.
        reason: DenyReason,
    },
}

/// The powerbox authorization: does `request.app_id` hold a grant for
/// `request.connection_id`, and is the requested scope within that grant's
/// ceiling? Pure and fail-closed:
///
/// - no matching grant for (app, connection) -> [`DenyReason::NoGrant`];
/// - an empty requested scope -> granted the full ceiling (the app asked for
///   everything it is entitled to);
/// - a non-empty requested scope that is a subset of the ceiling -> granted that
///   subset (strict attenuation, subtract-only);
/// - any requested token outside the ceiling -> [`DenyReason::ScopeExceedsCeiling`]
///   (amplification is refused; the broker never hands out more than the grant).
///
/// The match keys on the attested `app_id` and the validated `connection_id`, so
/// a request cannot name another app's grant or an unparsed connection.
///
/// The empty-request-scope -> full-ceiling behaviour is a conscious query-surface
/// choice (a caller asks for everything it is entitled to), acceptable for
/// CONN-R1 which returns only the scope. The CONN-R2 downscoping MUST guard an
/// empty resolved scope: several OAuth providers read an empty RFC 8693 scope as
/// "all default scopes", so a name-only grant must not exchange to a maximally
/// scoped token. When two grants share an (app, connection) the first wins (a
/// duplicate tightening entry is silently shadowed); config authoring should keep
/// one grant per pair.
pub fn broker_decide(request: &CredentialRequest, grants: &[ConnectionGrant]) -> BrokerDecision {
    let Some(grant) = grants
        .iter()
        .find(|g| g.app_id == request.app_id && g.connection_id == request.connection_id)
    else {
        return BrokerDecision::Deny {
            reason: DenyReason::NoGrant,
        };
    };

    // Empty request scope means "the full ceiling I am entitled to".
    if request.requested_scope.is_empty() {
        return BrokerDecision::Grant {
            connection_id: grant.connection_id.clone(),
            scope: grant.max_scope.clone(),
        };
    }

    // Strict attenuation: every requested token must be within the ceiling.
    if request
        .requested_scope
        .iter()
        .all(|s| grant.max_scope.contains(s))
    {
        BrokerDecision::Grant {
            connection_id: grant.connection_id.clone(),
            scope: request.requested_scope.clone(),
        }
    } else {
        BrokerDecision::Deny {
            reason: DenyReason::ScopeExceedsCeiling,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn(s: &str) -> ConnectionId {
        ConnectionId::new(s).unwrap()
    }

    fn grant(app: &str, connection: &str, scope: &[&str]) -> ConnectionGrant {
        ConnectionGrant {
            app_id: app.to_string(),
            connection_id: conn(connection),
            max_scope: scope.iter().map(|s| s.to_string()).collect(),
            allowed_hosts: vec![],
        }
    }

    #[test]
    fn allowed_hosts_are_looked_up_per_app_and_connection() {
        let grants = vec![
            ConnectionGrant {
                app_id: "com.example.app".to_string(),
                connection_id: conn("anthropic"),
                max_scope: vec![],
                allowed_hosts: vec!["api.anthropic.com".to_string()],
            },
            ConnectionGrant {
                app_id: "com.other.app".to_string(),
                connection_id: conn("anthropic"),
                max_scope: vec![],
                allowed_hosts: vec!["evil.example".to_string()],
            },
        ];
        // Matches on BOTH app and connection: the first app's hosts, never the
        // other app's (an app can only mint for its own grant).
        assert_eq!(
            allowed_hosts_for(&grants, "com.example.app", &conn("anthropic")),
            Some(&["api.anthropic.com".to_string()][..])
        );
        // No grant for this connection -> None (no token can be minted).
        assert_eq!(allowed_hosts_for(&grants, "com.example.app", &conn("github")), None);
        // No grant for this app -> None.
        assert_eq!(allowed_hosts_for(&grants, "com.stranger", &conn("anthropic")), None);
    }

    fn request(app: &str, connection: &str, scope: &[&str]) -> CredentialRequest {
        CredentialRequest {
            app_id: app.to_string(),
            connection_id: conn(connection),
            requested_scope: scope.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn connection_id_validates_charset() {
        assert!(ConnectionId::new("github").is_some());
        assert!(ConnectionId::new("google-drive.v2_1").is_some());
        assert!(ConnectionId::new("").is_none());
        assert!(ConnectionId::new("Bad").is_none()); // uppercase
        assert!(ConnectionId::new("a/b").is_none()); // separator
        assert!(ConnectionId::new("a b").is_none()); // space
        assert!(ConnectionId::new(&"a".repeat(128)).is_some()); // at the cap
        assert!(ConnectionId::new(&"a".repeat(129)).is_none()); // over the cap
    }

    #[test]
    fn no_grant_denies() {
        let grants = [grant("com.example.app", "github", &["repo"])];
        // Different app.
        let d = broker_decide(&request("com.other.app", "github", &["repo"]), &grants);
        assert_eq!(d, BrokerDecision::Deny { reason: DenyReason::NoGrant });
        // Different connection.
        let d = broker_decide(&request("com.example.app", "gitlab", &["repo"]), &grants);
        assert_eq!(d, BrokerDecision::Deny { reason: DenyReason::NoGrant });
    }

    #[test]
    fn empty_request_scope_grants_full_ceiling() {
        let grants = [grant("com.example.app", "github", &["repo", "read:user"])];
        let d = broker_decide(&request("com.example.app", "github", &[]), &grants);
        match d {
            BrokerDecision::Grant { scope, .. } => {
                assert_eq!(scope, vec!["repo".to_string(), "read:user".to_string()]);
            }
            other => panic!("expected grant, got {other:?}"),
        }
    }

    #[test]
    fn subset_request_is_attenuated() {
        let grants = [grant("com.example.app", "github", &["repo", "read:user", "gist"])];
        let d = broker_decide(&request("com.example.app", "github", &["read:user"]), &grants);
        match d {
            BrokerDecision::Grant { scope, .. } => assert_eq!(scope, vec!["read:user".to_string()]),
            other => panic!("expected grant, got {other:?}"),
        }
    }

    #[test]
    fn scope_beyond_ceiling_is_refused() {
        let grants = [grant("com.example.app", "github", &["read:user"])];
        // Asks for a scope the grant does not include -> amplification refused.
        let d = broker_decide(&request("com.example.app", "github", &["repo"]), &grants);
        assert_eq!(
            d,
            BrokerDecision::Deny { reason: DenyReason::ScopeExceedsCeiling }
        );
        // Partial overlap still refused (one token is outside the ceiling).
        let d = broker_decide(
            &request("com.example.app", "github", &["read:user", "repo"]),
            &grants,
        );
        assert_eq!(
            d,
            BrokerDecision::Deny { reason: DenyReason::ScopeExceedsCeiling }
        );
    }

    #[test]
    fn a_request_cannot_target_another_apps_grant() {
        // Two apps, each with its own github grant at different ceilings.
        let grants = [
            grant("com.a.app", "github", &["repo"]),
            grant("com.b.app", "github", &["admin:org"]),
        ];
        // App A asking for B's higher scope is refused (keyed on attested app_id).
        let d = broker_decide(&request("com.a.app", "github", &["admin:org"]), &grants);
        assert_eq!(
            d,
            BrokerDecision::Deny { reason: DenyReason::ScopeExceedsCeiling }
        );
    }
}

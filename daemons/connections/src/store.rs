//! The credential store and the authorized-handout composition
//! (connections-plan.md §2).
//!
//! The store seals each connection's credential at rest under the daemon master
//! (via the shared [`arlen_secret_vault::Vault`], keyed by connection id). The
//! composition [`authorize_and_load`] is the security-critical ordering: it runs
//! the powerbox [`broker_decide`] FIRST and loads the sealed credential ONLY on a
//! grant, so a denied request never touches a credential it has no capability
//! for. The raw credential still never reaches a client; the daemon downscopes it
//! to the granted scope (CONN-R2) or proxies the call, handing out a scoped token
//! rather than the credential.

use arlen_secret_vault::{Vault, VaultError};
use serde::{Deserialize, Serialize};

use crate::broker::{
    broker_decide, BrokerDecision, ConnectionGrant, ConnectionId, CredentialKind,
    CredentialRequest, DenyReason,
};

/// A stored credential: its kind plus the raw secret bytes (an OAuth refresh
/// token, an API key, or a webhook secret). Sealed at rest by the vault and never
/// serialized to a client; the broker hands out a scoped token derived from it.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Credential {
    /// What kind of secret this is (drives whether CONN-R2 can RFC 8693 exchange
    /// it or must proxy the call).
    pub kind: CredentialKind,
    /// The raw secret bytes.
    pub secret: Vec<u8>,
}

impl Drop for Credential {
    /// Zeroize the secret when the credential is dropped, so a decrypted token is
    /// not left in freed heap after a handout (including the failed-audit path
    /// where the handout is loaded then discarded).
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.secret.zeroize();
    }
}

impl std::fmt::Debug for Credential {
    /// Redact the secret: Debug must never print credential bytes (they could
    /// reach a log or an assertion message).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credential")
            .field("kind", &self.kind)
            .field("secret", &format_args!("<redacted {} bytes>", self.secret.len()))
            .finish()
    }
}

/// A store failure.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// The underlying vault failed (io, decrypt, corrupt).
    #[error("vault: {0}")]
    Vault(#[from] VaultError),
    /// A stored record could not be (de)serialized to a credential.
    #[error("credential encode/decode: {0}")]
    Codec(String),
}

/// The credential store: the daemon's connection credentials sealed under the
/// master, keyed by connection id.
pub struct CredentialStore {
    vault: Vault,
}

impl CredentialStore {
    /// A store keyed by `master` (the daemon master secret) with records under
    /// `dir`. A `ConnectionId` is always a valid vault record id (its charset is
    /// a subset), so the vault's own path-safety check never rejects one.
    pub fn new(master: [u8; 32], dir: impl Into<std::path::PathBuf>) -> Self {
        Self {
            vault: Vault::new(master, dir),
        }
    }

    /// Seal and persist a connection's credential. The serialized scratch buffer
    /// carries the plaintext secret, so it is zeroized on drop.
    pub fn put(&self, id: &ConnectionId, credential: &Credential) -> Result<(), StoreError> {
        let bytes = zeroize::Zeroizing::new(
            serde_json::to_vec(credential).map_err(|e| StoreError::Codec(e.to_string()))?,
        );
        self.vault.store(id.as_str(), &bytes)?;
        Ok(())
    }

    /// Load a connection's credential. `Ok(None)` when none is stored; a decrypt
    /// or decode failure is an error (fail closed), never `None`. The decrypted
    /// scratch buffer holds the plaintext secret, so it is zeroized on drop; the
    /// returned `Credential` zeroizes its secret on drop too.
    pub fn get(&self, id: &ConnectionId) -> Result<Option<Credential>, StoreError> {
        match self.vault.load(id.as_str())? {
            Some(bytes) => {
                let bytes = zeroize::Zeroizing::new(bytes);
                let cred = serde_json::from_slice(&bytes)
                    .map_err(|e| StoreError::Codec(e.to_string()))?;
                Ok(Some(cred))
            }
            None => Ok(None),
        }
    }

    /// Remove a connection's credential (idempotent).
    pub fn remove(&self, id: &ConnectionId) -> Result<(), StoreError> {
        self.vault.remove(id.as_str())?;
        Ok(())
    }
}

/// An authorized handout: the connection, the loaded credential, and the scope
/// the broker granted. The daemon downscopes the credential to `scope` (CONN-R2)
/// before anything crosses to the requesting app.
#[derive(Debug)]
pub struct AuthorizedHandout {
    /// The connection the credential is for.
    pub connection_id: ConnectionId,
    /// The loaded sealed credential (daemon-internal; never handed to the app).
    pub credential: Credential,
    /// The scope the broker granted (a subset of the app's grant ceiling).
    pub scope: Vec<String>,
}

/// Why a handout was refused.
#[derive(Debug, thiserror::Error)]
pub enum HandoutError {
    /// The broker refused the request (no grant or scope beyond the ceiling).
    #[error("denied: {0:?}")]
    Denied(DenyReason),
    /// The request was authorized but no credential is stored for the connection.
    #[error("no credential stored for the connection")]
    NoCredential,
    /// The store failed loading the credential.
    #[error("store: {0}")]
    Store(#[from] StoreError),
}

/// Authorize a request against the standing grants and, ONLY on a grant, load the
/// sealed credential. The ordering is the security property: the powerbox
/// decision gates the store read, so a denied request never causes the credential
/// to be loaded (no load-then-check, no oracle on which connections hold a
/// credential). On a grant it returns the credential plus the attenuated scope
/// for the daemon to downscope against.
pub fn authorize_and_load(
    request: &CredentialRequest,
    grants: &[ConnectionGrant],
    store: &CredentialStore,
) -> Result<AuthorizedHandout, HandoutError> {
    match broker_decide(request, grants) {
        BrokerDecision::Grant { connection_id, scope } => {
            let credential = store
                .get(&connection_id)?
                .ok_or(HandoutError::NoCredential)?;
            Ok(AuthorizedHandout {
                connection_id,
                credential,
                scope,
            })
        }
        BrokerDecision::Deny { reason } => Err(HandoutError::Denied(reason)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn(s: &str) -> ConnectionId {
        ConnectionId::new(s).unwrap()
    }

    fn store() -> (tempfile::TempDir, CredentialStore) {
        let tmp = tempfile::TempDir::new().unwrap();
        let s = CredentialStore::new([5u8; 32], tmp.path().join("connections"));
        (tmp, s)
    }

    fn cred(secret: &[u8]) -> Credential {
        Credential {
            kind: CredentialKind::OAuthRefreshToken,
            secret: secret.to_vec(),
        }
    }

    fn grant(app: &str, connection: &str, scope: &[&str]) -> ConnectionGrant {
        ConnectionGrant {
            app_id: app.to_string(),
            connection_id: conn(connection),
            max_scope: scope.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn request(app: &str, connection: &str, scope: &[&str]) -> CredentialRequest {
        CredentialRequest {
            app_id: app.to_string(),
            connection_id: conn(connection),
            requested_scope: scope.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn put_then_get_round_trips() {
        let (_tmp, s) = store();
        let c = cred(b"refresh-xyz");
        s.put(&conn("github"), &c).unwrap();
        assert_eq!(s.get(&conn("github")).unwrap(), Some(c));
    }

    #[test]
    fn get_absent_is_none() {
        let (_tmp, s) = store();
        assert_eq!(s.get(&conn("github")).unwrap(), None);
    }

    #[test]
    fn remove_is_idempotent() {
        let (_tmp, s) = store();
        s.put(&conn("github"), &cred(b"x")).unwrap();
        s.remove(&conn("github")).unwrap();
        assert_eq!(s.get(&conn("github")).unwrap(), None);
        s.remove(&conn("github")).unwrap();
    }

    #[test]
    fn authorize_and_load_grants_then_loads() {
        let (_tmp, s) = store();
        s.put(&conn("github"), &cred(b"refresh-xyz")).unwrap();
        let grants = [grant("com.example.app", "github", &["repo", "read:user"])];
        let out = authorize_and_load(
            &request("com.example.app", "github", &["read:user"]),
            &grants,
            &s,
        )
        .unwrap();
        assert_eq!(out.scope, vec!["read:user".to_string()]);
        assert_eq!(out.credential.secret, b"refresh-xyz");
    }

    #[test]
    fn a_denied_request_never_loads_the_credential() {
        // No grant for this app: the broker denies, and the credential (which
        // DOES exist) is never loaded - the decision gates the read.
        let (_tmp, s) = store();
        s.put(&conn("github"), &cred(b"refresh-xyz")).unwrap();
        let grants = [grant("com.other.app", "github", &["repo"])];
        let err = authorize_and_load(
            &request("com.example.app", "github", &["repo"]),
            &grants,
            &s,
        )
        .unwrap_err();
        assert!(matches!(err, HandoutError::Denied(DenyReason::NoGrant)));
    }

    #[test]
    fn scope_beyond_ceiling_is_denied_before_load() {
        let (_tmp, s) = store();
        s.put(&conn("github"), &cred(b"refresh-xyz")).unwrap();
        let grants = [grant("com.example.app", "github", &["read:user"])];
        let err = authorize_and_load(
            &request("com.example.app", "github", &["repo"]),
            &grants,
            &s,
        )
        .unwrap_err();
        assert!(matches!(
            err,
            HandoutError::Denied(DenyReason::ScopeExceedsCeiling)
        ));
    }

    #[test]
    fn authorized_but_no_credential_stored_is_distinct() {
        // The grant authorizes, but nothing is stored yet: distinct from a denial.
        let (_tmp, s) = store();
        let grants = [grant("com.example.app", "github", &["repo"])];
        let err = authorize_and_load(
            &request("com.example.app", "github", &["repo"]),
            &grants,
            &s,
        )
        .unwrap_err();
        assert!(matches!(err, HandoutError::NoCredential));
    }
}

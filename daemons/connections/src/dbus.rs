//! The D-Bus interface (`org.arlen.Connections1`): the app-facing surface of the
//! credential broker.
//!
//! A caller requests authorization for a connection; the daemon resolves the
//! KERNEL-ATTESTED app id (never a client-supplied value, exactly as the account
//! daemon does over the bus), checks the capability grant via
//! [`crate::serve::serve_request`], and audits the decision. `request_credential`
//! returns the granted scope (the authorization + audit surface).
//!
//! The credential-to-egress DELIVERY (connections-plan.md §7b) adds two methods:
//! an app calls [`mint_egress_capability`](ConnectionsDaemon::mint_egress_capability)
//! to get a destination-scoped Biscuit token bound to its grant's declared host
//! allowlist, and the trusted net-guard (the ai-proxy) calls
//! [`fetch_egress_credential`](ConnectionsDaemon::fetch_egress_credential),
//! presenting that token, to receive the raw Proxy-mode credential it injects at
//! the egress boundary. Both resolve the attested caller; the fetch composition
//! ([`crate::deliver`]) is the injection boundary and is the piece that got the
//! adversarial review.

use std::time::{SystemTime, UNIX_EPOCH};

use audit_proto::LedgerAuditSink;
use zbus::message::Header;
use zbus::Connection;

use crate::broker::{allowed_hosts_for, ConnectionId};
use crate::config;
use crate::deliver::{deliver_egress_credential, mint_egress_capability, DeliverError, EgressRequest};
use crate::root::RootKeypair;
use crate::serve::{serve_request, ServeError};
use crate::store::CredentialStore;

/// The attested caller ids allowed to fetch a raw credential for egress injection:
/// the generalised ai-proxy net-guard. The confused-deputy defense rests on this
/// list plus the presented capability token, so only the egress authoriser can pull
/// a raw key.
const EGRESS_AUTHORISERS: &[&str] = &["ai-proxy"];

/// How long a minted egress capability token is valid. Short-lived (bearer, bound
/// to a request): the net-guard must present it within this window.
const EGRESS_TOKEN_TTL_SECS: i64 = 300;

/// The Connections daemon object served on the bus: the sealed credential store,
/// the audit sink, and the capability-token root keypair.
pub struct ConnectionsDaemon {
    store: CredentialStore,
    audit: LedgerAuditSink,
    root: RootKeypair,
}

impl ConnectionsDaemon {
    /// Build the daemon over its store, audit sink, and capability-token root
    /// keypair.
    pub fn new(store: CredentialStore, audit: LedgerAuditSink, root: RootKeypair) -> Self {
        Self { store, audit, root }
    }
}

#[zbus::interface(name = "org.arlen.Connections1")]
impl ConnectionsDaemon {
    /// Request authorization for a connection credential at `scope`. Resolves the
    /// attested caller, checks its capability grant, confirms a credential is
    /// stored, and audits the decision; returns the granted scope (a subset of
    /// the caller's ceiling, or the full ceiling for an empty request). Fail-closed:
    /// an unresolvable caller, a denied grant, a missing credential, or a down
    /// ledger all error, and no credential is ever returned.
    async fn request_credential(
        &self,
        connection: String,
        scope: Vec<String>,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] conn: &Connection,
    ) -> zbus::fdo::Result<Vec<String>> {
        let caller = resolve_caller_app_id_guarded(&header, conn)
            .await
            .map_err(zbus::fdo::Error::AccessDenied)?;
        let grants = config::load().grants();
        match serve_request(&caller, &connection, scope, &grants, &self.store, &self.audit).await {
            Ok(handout) => Ok(handout.scope),
            Err(ServeError::Denied(_)) | Err(ServeError::UnknownConnection) => {
                Err(zbus::fdo::Error::AccessDenied("not authorized".into()))
            }
            Err(ServeError::NoCredential) => {
                Err(zbus::fdo::Error::Failed("no credential stored".into()))
            }
            Err(ServeError::AuditUnavailable) => {
                Err(zbus::fdo::Error::Failed("audit unavailable".into()))
            }
            Err(ServeError::Store(e)) => Err(zbus::fdo::Error::Failed(e)),
        }
    }

    /// Mint a short-lived, destination-scoped capability token for the caller's
    /// egress on `connection`. Resolves the attested caller, looks up ITS grant's
    /// declared host allowlist (the destination scope is from the trusted config,
    /// never the caller's choice), and binds a Biscuit token to exactly those hosts
    /// with a fresh nonce and a short TTL under the daemon root key. The caller
    /// hands this token to the net-guard, which presents it at
    /// [`fetch_egress_credential`](Self::fetch_egress_credential). Fail-closed: no
    /// grant, an empty host allowlist, or an unresolvable caller all deny. Minting
    /// releases no credential (only the fetch does, which is the audited boundary).
    async fn mint_egress_capability(
        &self,
        connection: String,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] conn: &Connection,
    ) -> zbus::fdo::Result<String> {
        let caller = resolve_caller_app_id_guarded(&header, conn)
            .await
            .map_err(zbus::fdo::Error::AccessDenied)?;
        let Some(connection_id) = ConnectionId::new(&connection) else {
            return Err(zbus::fdo::Error::AccessDenied("not authorized".into()));
        };
        let grants = config::load().grants();
        // The declared host allowlist for THIS caller's grant. No grant, or an empty
        // allowlist, is fail-closed: no egress token is mintable.
        let hosts = match allowed_hosts_for(&grants, &caller, &connection_id) {
            Some(h) if !h.is_empty() => h.to_vec(),
            _ => return Err(zbus::fdo::Error::AccessDenied("not authorized".into())),
        };
        let expiry = now_unix().saturating_add(EGRESS_TOKEN_TTL_SECS);
        let nonce = fresh_nonce().map_err(|e| zbus::fdo::Error::Failed(format!("nonce: {e}")))?;
        mint_egress_capability(self.root.keypair(), &connection, &hosts, expiry, &nonce)
            .map_err(|_| zbus::fdo::Error::Failed("mint failed".into()))
    }

    /// Fetch the raw Proxy-mode credential for `connection` to inject at the egress
    /// boundary. Reachable ONLY by an allowlisted egress authoriser (the ai-proxy)
    /// that presents a valid `capability_token` scoped to `destination_host`; the
    /// composition ([`crate::deliver`]) verifies both, confirms the credential is
    /// Proxy-mode, audits the release before returning, and fails closed on any
    /// gap. Returns the raw secret bytes for injection; the caller must never
    /// persist or log them.
    async fn fetch_egress_credential(
        &self,
        connection: String,
        capability_token: String,
        destination_host: String,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] conn: &Connection,
    ) -> zbus::fdo::Result<Vec<u8>> {
        let caller = resolve_caller_app_id_guarded(&header, conn)
            .await
            .map_err(zbus::fdo::Error::AccessDenied)?;
        let allowlist: Vec<String> = EGRESS_AUTHORISERS.iter().map(|s| s.to_string()).collect();
        let req = EgressRequest {
            caller: &caller,
            connection: &connection,
            presented_token: &capability_token,
            destination_host: &destination_host,
            now_unix: now_unix(),
        };
        match deliver_egress_credential(&req, &allowlist, &self.root.public(), &self.store, &self.audit)
            .await
        {
            Ok(secret) => Ok(secret.to_vec()),
            // Authorization failures collapse to one AccessDenied: no oracle on
            // which of caller / token / mode / connection failed.
            Err(DeliverError::NotAuthorizedCaller)
            | Err(DeliverError::TokenRejected)
            | Err(DeliverError::NotProxyMode)
            | Err(DeliverError::UnknownConnection)
            | Err(DeliverError::EmptyHost) => {
                Err(zbus::fdo::Error::AccessDenied("not authorized".into()))
            }
            Err(DeliverError::NoCredential) => {
                Err(zbus::fdo::Error::Failed("no credential stored".into()))
            }
            Err(DeliverError::AuditUnavailable) => {
                Err(zbus::fdo::Error::Failed("audit unavailable".into()))
            }
            Err(DeliverError::Store(e)) => Err(zbus::fdo::Error::Failed(e)),
            Err(DeliverError::Mint(_)) => Err(zbus::fdo::Error::Failed("internal".into())),
        }
    }
}

/// The current unix time in seconds, for token expiry. A clock before the epoch
/// yields 0 (fail toward already-expired, never a far-future token).
fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// A fresh 16-byte CSPRNG nonce, hex-encoded (32 ASCII chars, control-char-free and
/// within the capability layer's nonce cap). Each minted token carries a distinct
/// nonce for uniqueness/correlation. NOTE: the nonce is not currently VERIFIED
/// (there is no consumed-nonce ledger), so it is not an anti-replay control within
/// the TTL; the replay window is bounded by the short TTL + the ai-proxy-only fetch
/// gate. A single-use consumed-nonce set is the follow-up if tighter replay defense
/// is needed.
fn fresh_nonce() -> Result<String, getrandom::Error> {
    let mut bytes = [0u8; 16];
    getrandom::getrandom(&mut bytes)?;
    let mut s = String::with_capacity(32);
    for b in bytes {
        use std::fmt::Write;
        let _ = write!(s, "{b:02x}");
    }
    Ok(s)
}

/// Resolve the caller's app id with a PID-reuse guard (mirrors the account
/// daemon): the bus-attested sender resolves to a pid, the pid's exe resolves to
/// an app id, and the pid start time is captured before and after so a pid
/// recycled to a different process between the bus attesting it and the exe read
/// fails closed. The app id is the connection's attested identity, never a value
/// the caller supplies.
async fn resolve_caller_app_id_guarded(
    header: &Header<'_>,
    connection: &Connection,
) -> Result<String, String> {
    use arlen_permissions::identity::{app_id_from_pid, pid_start_time};
    let sender = header.sender().ok_or_else(|| "no sender".to_string())?;
    let proxy = zbus::fdo::DBusProxy::new(connection)
        .await
        .map_err(|e| format!("DBusProxy: {e}"))?;
    let pid = proxy
        .get_connection_unix_process_id(sender.clone().into())
        .await
        .map_err(|e| format!("caller pid: {e}"))?;
    let before = pid_start_time(pid).map_err(|e| format!("pid start: {e}"))?;
    let app_id = app_id_from_pid(pid).map_err(|e| format!("app id: {e}"))?;
    let after = pid_start_time(pid).map_err(|e| format!("pid start: {e}"))?;
    if before != after {
        return Err("pid recycled during resolution".to_string());
    }
    Ok(app_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_nonce_is_fresh_hex_within_the_cap() {
        let a = fresh_nonce().unwrap();
        let b = fresh_nonce().unwrap();
        assert_eq!(a.len(), 32);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b, "two nonces must differ");
    }

    #[test]
    fn now_is_positive() {
        assert!(now_unix() > 1_700_000_000);
    }
}

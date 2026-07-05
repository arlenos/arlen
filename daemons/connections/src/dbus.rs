//! The D-Bus interface (`org.arlen.Connections1`): the app-facing surface of the
//! credential broker.
//!
//! A caller requests authorization for a connection; the daemon resolves the
//! KERNEL-ATTESTED app id (never a client-supplied value, exactly as the account
//! daemon does over the bus), checks the capability grant via
//! [`crate::serve::serve_request`], and audits the decision. For CONN-R1 the
//! method returns the granted scope (the authorization + audit surface, closing
//! the GAP-15 audit); the downscoped-token DELIVERY that hands the app a usable
//! token without ever exposing the raw credential is CONN-R2.

use audit_proto::LedgerAuditSink;
use zbus::message::Header;
use zbus::Connection;

use crate::config;
use crate::serve::{serve_request, ServeError};
use crate::store::CredentialStore;

/// The Connections daemon object served on the bus: the sealed credential store
/// and the audit sink.
pub struct ConnectionsDaemon {
    store: CredentialStore,
    audit: LedgerAuditSink,
}

impl ConnectionsDaemon {
    /// Build the daemon over its store and audit sink.
    pub fn new(store: CredentialStore, audit: LedgerAuditSink) -> Self {
        Self { store, audit }
    }
}

#[zbus::interface(name = "org.arlen.Connections1")]
impl ConnectionsDaemon {
    /// Request authorization for a connection credential at `scope`. Resolves the
    /// attested caller, checks its capability grant, confirms a credential is
    /// stored, and audits the decision; returns the granted scope (a subset of
    /// the caller's ceiling, or the full ceiling for an empty request). The
    /// downscoped-token delivery is CONN-R2; this is the authorization + audit
    /// surface. Fail-closed: an unresolvable caller, a denied grant, a missing
    /// credential, or a down ledger all error, and no credential is ever
    /// returned.
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

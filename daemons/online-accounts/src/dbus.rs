//! The `org.arlen.Accounts1` D-Bus surface, mediated by the per-app capability
//! gate (online-accounts-plan.md).
//!
//! Every method resolves the CALLER's Arlen identity from the connection and
//! consults the [`AccessGate`]: an app sees and reaches only the accounts it was
//! granted. The identity is the existing F3 `path_to_app_id` model knowledge and
//! installd key on - but here, over a message bus, the attested PID comes from
//! the bus daemon's `GetConnectionUnixProcessID` (there is no peer socket to read
//! `SO_PEERCRED` from, as the raw-socket daemons do), then the same
//! `path_to_app_id` chain resolves `/proc/<pid>/exe`. Same trust, bus-attested.
//!
//! This is sound ONLY because the daemon serves on the session BUS (see `main`):
//! the bus authoritatively stamps the sender and answers the PID query, so a
//! caller cannot forge another connection's identity. A peer-to-peer (busless)
//! variant would not have that guarantee and must not copy this resolution.

use zbus::interface;

use crate::config::{AccountConfig, Service};
use crate::gate::{Access, AccessGate};
use crate::vault::Vault;

/// The accounts daemon's served object: the loaded account set, gated per-caller,
/// plus the token vault the gated handout reads from.
pub struct AccountsDaemon {
    accounts: Vec<AccountConfig>,
    vault: Vault,
}

impl AccountsDaemon {
    /// A daemon over the loaded accounts and the token vault. The vault holds
    /// the AEAD-encrypted tokens; `GetAccessToken` reads it only after the gate
    /// admits the caller.
    pub fn new(accounts: Vec<AccountConfig>, vault: Vault) -> Self {
        Self { accounts, vault }
    }
}

#[interface(name = "org.arlen.Accounts1")]
impl AccountsDaemon {
    /// The accounts the CALLER's app was granted - never the full set. An app with
    /// no grant gets an empty list (no shared-DB enumeration; the structural fix
    /// for what GOA/KDE expose). An unresolvable caller is treated as ungranted
    /// (fail-closed). Each entry is `(id, provider, identity, presentation)`.
    async fn list_accounts(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> Vec<(String, String, String, String)> {
        let Ok(caller) = resolve_caller_app_id(&header, connection).await else {
            return Vec::new();
        };
        AccessGate::new(&self.accounts)
            .granted_accounts(&caller)
            .into_iter()
            .map(|a| {
                (
                    a.id.clone(),
                    a.provider.clone(),
                    a.identity.clone(),
                    a.presentation.clone().unwrap_or_default(),
                )
            })
            .collect()
    }

    /// Hand out an access token for `(account_id, service)` to the calling app,
    /// gated on its per-app grant - the Arlen differentiator over GOA/KDE, where
    /// any app reads the shared keyring. Returns `(token, scope)`; refuses with
    /// `AccessDenied` when the caller is unresolved, holds no grant for this
    /// account+service, or the service name is unknown, and `Failed` when the
    /// grant exists but no token is stored yet (the OAuth flow that populates the
    /// vault is OA-R2).
    ///
    /// PID-reuse guard: the caller's `(pid, start_time)` is captured and
    /// re-verified across the `/proc`-based identity resolution, so a recycled
    /// PID cannot be resolved to a different app between the bus attesting the
    /// PID and the exe read (the knowledge-daemon pattern the metadata-only
    /// `list_accounts` could defer but a token handout must not).
    async fn get_access_token(
        &self,
        account_id: String,
        service: String,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> zbus::fdo::Result<(String, String)> {
        let Ok(caller) = resolve_caller_app_id_guarded(&header, connection).await else {
            return Err(zbus::fdo::Error::AccessDenied("unresolved caller".into()));
        };
        let Some(service) = Service::parse(&service) else {
            return Err(zbus::fdo::Error::AccessDenied("unknown service".into()));
        };
        let scope = match AccessGate::new(&self.accounts).access(&caller, &account_id, service) {
            Access::Granted { scope } => scope.unwrap_or_default(),
            Access::Refused => {
                return Err(zbus::fdo::Error::AccessDenied(
                    "no grant for this app on this account and service".into(),
                ))
            }
        };
        // The grant is held; read the token from the vault. A vault error or a
        // non-UTF-8 record fails closed (no token leaks, no panic).
        match self.vault.load(&account_id) {
            Ok(Some(bytes)) => match String::from_utf8(bytes) {
                Ok(token) => Ok((token, scope)),
                Err(_) => Err(zbus::fdo::Error::Failed("stored token is not valid UTF-8".into())),
            },
            Ok(None) => Err(zbus::fdo::Error::Failed(
                "no token stored for this account yet".into(),
            )),
            Err(_) => Err(zbus::fdo::Error::Failed("vault read failed".into())),
        }
    }
}

/// Resolve the calling app's Arlen identity from the D-Bus connection.
///
/// The session bus daemon attests the sender's PID (`GetConnectionUnixProcessID`,
/// not a client-supplied value), and `app_id_from_pid` resolves `/proc/<pid>/exe`
/// through the F3 `path_to_app_id` chain - the SAME identity model the knowledge
/// daemon and installd use, so the account gate keys on one model. Any failure
/// (no sender, bus error, unresolvable binary) is an `Err`, which every caller
/// treats as ungranted (fail-closed).
///
/// Residual (documented, low for metadata enumeration): a sub-millisecond
/// PID-reuse window between the bus attesting the PID and reading `/proc`. The
/// `GetAccessToken` slice, which hands out an actual token, must close it with a
/// `pid_start_time` capture-and-recheck (the knowledge-daemon pattern); here it
/// only exposes the granted accounts' metadata, so it is deferred.
async fn resolve_caller_app_id(
    header: &zbus::message::Header<'_>,
    connection: &zbus::Connection,
) -> Result<String, String> {
    let sender = header
        .sender()
        .ok_or_else(|| "no sender in message".to_string())?;
    let proxy = zbus::fdo::DBusProxy::new(connection)
        .await
        .map_err(|e| format!("DBusProxy: {e}"))?;
    let pid = proxy
        .get_connection_unix_process_id(sender.clone().into())
        .await
        .map_err(|e| format!("get caller pid: {e}"))?;
    arlen_permissions::identity::app_id_from_pid(pid).map_err(|e| format!("resolve app id: {e}"))
}

/// Resolve the caller's app id with a PID-reuse guard, for the token handout.
///
/// Captures the caller PID's start time, resolves the app id (a `/proc/<pid>/exe`
/// read), then re-captures the start time: if the PID was recycled to a
/// different process between the bus attesting it and the exe read, the start
/// time differs and resolution fails closed. Metadata-only methods can use the
/// unguarded [`resolve_caller_app_id`]; a token handout must not.
async fn resolve_caller_app_id_guarded(
    header: &zbus::message::Header<'_>,
    connection: &zbus::Connection,
) -> Result<String, String> {
    use arlen_permissions::identity::{app_id_from_pid, pid_start_time};
    let sender = header
        .sender()
        .ok_or_else(|| "no sender in message".to_string())?;
    let proxy = zbus::fdo::DBusProxy::new(connection)
        .await
        .map_err(|e| format!("DBusProxy: {e}"))?;
    let pid = proxy
        .get_connection_unix_process_id(sender.clone().into())
        .await
        .map_err(|e| format!("get caller pid: {e}"))?;
    let start_before = pid_start_time(pid).map_err(|e| format!("pid start time: {e}"))?;
    let app_id = app_id_from_pid(pid).map_err(|e| format!("resolve app id: {e}"))?;
    let start_after = pid_start_time(pid).map_err(|e| format!("pid start time: {e}"))?;
    if start_before != start_after {
        return Err("pid recycled during resolution".to_string());
    }
    Ok(app_id)
}

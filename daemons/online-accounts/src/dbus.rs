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

use crate::config::AccountConfig;
use crate::gate::AccessGate;

/// The accounts daemon's served object: the loaded account set, gated per-caller.
pub struct AccountsDaemon {
    accounts: Vec<AccountConfig>,
}

impl AccountsDaemon {
    /// A daemon over the loaded accounts.
    pub fn new(accounts: Vec<AccountConfig>) -> Self {
        Self { accounts }
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

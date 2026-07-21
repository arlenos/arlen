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
//! caller cannot forge ANOTHER connection's identity. A peer-to-peer (busless)
//! variant would not have that guarantee and must not copy this resolution.
//!
//! Residual (the same ambient limit every `/proc/exe` identity model in the repo
//! carries, the documented F3 gap): the attested PID is the connection's, but a
//! granted app could `exec` a different binary into the same PID after connecting
//! (the PID and its start time are unchanged by `exec`), or pass its bus
//! connection to a child. So the resolution is unforgeable against a *different*
//! connection, not against in-process `exec` on the *same* one; the per-request
//! `pid_start_time` recheck closes only PID *recycling* during resolution, not
//! same-PID `exec`. The eventual close is the same inode-attestation F3 work.

use crate::presence::PeerRegistry;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use audit_proto::{AuditSink, LedgerAuditSink};
use zbus::interface;

use crate::attenuate::{attenuate, AttenuationError};
use crate::audit::credential_handout_event;
use crate::config::{self, AccountConfig, Service};
use crate::gate::{Access, AccessGate};
use crate::launcher::{self, RcloneMount};
use crate::vault::Vault;
use std::collections::HashMap;

/// The accounts daemon's served object: the account-config directory (re-read
/// per call so grant changes take effect immediately, see [`current_accounts`])
/// plus the token vault the gated handout reads from.
pub struct AccountsDaemon {
    accounts_dir: PathBuf,
    vault: Vault,
    /// Content-free audit of the credential handout (GAP-2). One fresh one-shot
    /// connection per submit, against the canonical ingest socket.
    audit: LedgerAuditSink,
    /// Live caller presence (bus name -> app id), recorded on each admitted call,
    /// so an account-change signal is unicast only to granted apps' connections.
    peers: Arc<Mutex<PeerRegistry>>,
    /// Live confined rclone mounts, keyed by account id (one drive per account,
    /// shared by every granted caller). `Mount` inserts here, `Unmount` removes +
    /// tears down. Held only across the quick insert/remove, never an await.
    mounts: Arc<Mutex<HashMap<String, RcloneMount>>>,
}

impl AccountsDaemon {
    /// A daemon over the account-config directory and the token vault. The vault
    /// holds the AEAD-encrypted tokens; `GetAccessToken` reads it only after the
    /// gate admits the caller.
    pub fn new(accounts_dir: PathBuf, vault: Vault, peers: Arc<Mutex<PeerRegistry>>) -> Self {
        Self {
            accounts_dir,
            vault,
            audit: LedgerAuditSink::at_default_socket(),
            peers,
            mounts: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Record the calling connection's bus name against its resolved app id, so a
    /// later account change can be unicast to it if the app is granted. A poisoned
    /// lock is swallowed - a missed presence record only means a signal may not
    /// reach one live connection, never a leak.
    fn record_peer(&self, header: &zbus::message::Header<'_>, app_id: &str) {
        if let Some(sender) = header.sender() {
            if let Ok(mut peers) = self.peers.lock() {
                peers.record(sender.to_string(), app_id);
            }
        }
    }

    /// The current account set, re-read from disk on every call. A capability
    /// daemon must honour a grant change the instant it is saved: a grant
    /// **revoked** by editing the config would otherwise keep working until the
    /// daemon restarted (a real gap). Re-reading per call has no staleness window
    /// (stronger than a watched cache) at the cost of a few small TOML reads,
    /// negligible for the infrequent capability calls. A config that became
    /// malformed drops that account (fail-closed), so a broken grant denies
    /// rather than serves.
    fn current_accounts(&self) -> Vec<AccountConfig> {
        config::load_accounts(&self.accounts_dir).0
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
        self.record_peer(&header, &caller);
        let accounts = self.current_accounts();
        AccessGate::new(&accounts)
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

    /// Hand out an access token for the account to the calling app at the
    /// service's least-privilege scope, gated on its per-app grant - the Arlen
    /// differentiator over GOA/KDE, where any app reads the shared keyring.
    /// `requested_scope` lets the caller ask for a NARROWER subset of its grant
    /// (CONN-R2 subtract-only attenuation): empty takes the full grant, a request
    /// naming any scope outside the grant is refused with `AccessDenied`
    /// (amplification, the GAP-15 invariant). Returns `(token, scope)` where `scope`
    /// is the attenuated grant actually handed out; refuses with `AccessDenied` when
    /// the caller is unresolved, holds no grant for this account+service, the account
    /// does not offer the service, the service name is unknown, or the request
    /// amplifies, and a single generic `Failed("token unavailable")` for any
    /// post-grant vault outcome (no token yet / read error) so the error channel
    /// does not leak provisioning state.
    ///
    /// Token isolation note: the stored credential is **per account** (one
    /// refresh/access token), so `service` selects the OAuth `scope` handed out,
    /// not a distinct secret per service; the handout returns the account's token
    /// narrowed to the granted scope. The OAuth flow that populates the vault is
    /// OA-R2; until then a granted call returns `Failed` (no token stored yet).
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
        requested_scope: String,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> zbus::fdo::Result<(String, String)> {
        let Ok(caller) = resolve_caller_app_id_guarded(&header, connection).await else {
            return Err(zbus::fdo::Error::AccessDenied("unresolved caller".into()));
        };
        self.record_peer(&header, &caller);
        let Some(service) = Service::parse(&service) else {
            return Err(zbus::fdo::Error::AccessDenied("unknown service".into()));
        };
        let service_key = service.as_key();
        let accounts = self.current_accounts();
        let granted_scope = match AccessGate::new(&accounts).access(&caller, &account_id, service) {
            Access::Granted { scope } => scope.unwrap_or_default(),
            Access::Refused => {
                return Err(zbus::fdo::Error::AccessDenied(
                    "no grant for this app on this account and service".into(),
                ))
            }
        };
        // CONN-R2: downscope to the caller's requested subset, subtract-only. An
        // empty request takes the full grant; a request naming any scope outside
        // the grant is an amplification and is refused (the GAP-15 invariant).
        let scope = match attenuate(&granted_scope, &requested_scope) {
            Ok(scope) => scope,
            Err(AttenuationError::Amplification(_)) => {
                // Record the denied over-reach, content-free. Best-effort: a refusal
                // releases nothing, so a ledger hiccup must not turn the denial into
                // a grant (the fail-closed rule guards releases, not denials).
                let _ = self
                    .audit
                    .submit(credential_handout_event(&caller, service_key, "amplification-refused"))
                    .await;
                return Err(zbus::fdo::Error::AccessDenied(
                    "requested scope exceeds the grant".into(),
                ));
            }
        };
        // The grant is held; read the token from the vault. Every post-grant
        // failure (no token yet, a non-UTF-8 record, a vault I/O/decrypt error)
        // collapses to ONE generic error so a granted caller cannot distinguish
        // "no token provisioned yet" from a transient read error (the error
        // channel leaks no provisioning state); fail-closed, no token leaks, no
        // panic.
        let unavailable = || zbus::fdo::Error::Failed("token unavailable".into());
        let token = match self.vault.load(&account_id) {
            Ok(Some(bytes)) => match String::from_utf8(bytes) {
                Ok(t) => t,
                Err(_) => return Err(unavailable()),
            },
            Ok(None) => return Err(unavailable()),
            Err(_) => return Err(unavailable()),
        };
        // GAP-2: record that a credential was released BEFORE handing it out, so
        // no token leaves the daemon unaudited (S13 fail-closed: if the ledger is
        // unreachable the handout fails rather than slipping through unrecorded).
        // The record is content-free - caller + coarse service + outcome, never
        // the token, the account id, or the credential value.
        if self
            .audit
            .submit(credential_handout_event(&caller, service_key, "granted"))
            .await
            .is_err()
        {
            return Err(unavailable());
        }
        Ok((token, scope))
    }

    /// Mount the account's `Files` drive: a confined rclone under `arlen-run`
    /// (the §0 egress netns + Landlock + seccomp + cgroup), scoped to the
    /// provider host. Refused for an ungranted caller or an account with no
    /// dialable `[files]` backend. Idempotent - an already-mounted account
    /// returns its mount point. S13-audited before the confined subprocess is
    /// spawned (a ledger failure refuses rather than mounting unrecorded).
    /// Returns the mount point path.
    async fn mount(
        &self,
        account_id: String,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> zbus::fdo::Result<String> {
        let Ok(caller) = resolve_caller_app_id_guarded(&header, connection).await else {
            return Err(zbus::fdo::Error::AccessDenied("unresolved caller".into()));
        };
        self.record_peer(&header, &caller);
        let accounts = self.current_accounts();
        // The files backend's password from the vault (a key_file sftp has none).
        // Fail-closed to None on any vault error; rclone then fails at connect
        // (surfaced), never the daemon guessing a credential.
        let secret = self
            .vault
            .load(&account_id)
            .ok()
            .flatten()
            .and_then(|b| String::from_utf8(b).ok());
        let Some((runtime_dir, cache_dir)) = mount_dirs(&account_id) else {
            return Err(zbus::fdo::Error::Failed("no runtime directory".into()));
        };
        let resolved = launcher::resolve_mount(
            &accounts,
            &caller,
            &account_id,
            &runtime_dir,
            &cache_dir,
            secret.as_deref(),
        )
        .map_err(|_| zbus::fdo::Error::AccessDenied("mount refused".into()))?;
        let mount_point = resolved.plan.mount_point.to_string_lossy().into_owned();
        // Idempotent: an already-mounted account returns its point (the grant was
        // re-verified by resolve_mount above). The lock is held only for the check.
        if self.mounts.lock().map(|m| m.contains_key(&account_id)).unwrap_or(false) {
            return Ok(mount_point);
        }
        // S13: a mount launches a credential-bearing confined subprocess; audit it
        // before spawning, and refuse if the ledger cannot record it.
        if self
            .audit
            .submit(credential_handout_event(&caller, "files", "mount"))
            .await
            .is_err()
        {
            return Err(zbus::fdo::Error::Failed("mount unavailable".into()));
        }
        let mount = launcher::spawn_confined_mount(&resolved.paths, &resolved.plan, &resolved.hosts)
            .await
            .map_err(|_| zbus::fdo::Error::Failed("mount failed".into()))?;
        if let Ok(mut mounts) = self.mounts.lock() {
            mounts.insert(account_id, mount);
        }
        Ok(mount_point)
    }

    /// Unmount the account's `Files` drive and stop its confined rclone. Requires
    /// the same `Files` grant, so one app cannot tear down another's drive.
    /// Idempotent - unmounting a not-mounted account succeeds.
    async fn unmount(
        &self,
        account_id: String,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> zbus::fdo::Result<()> {
        let Ok(caller) = resolve_caller_app_id_guarded(&header, connection).await else {
            return Err(zbus::fdo::Error::AccessDenied("unresolved caller".into()));
        };
        self.record_peer(&header, &caller);
        let accounts = self.current_accounts();
        if !matches!(
            AccessGate::new(&accounts).access(&caller, &account_id, Service::Files),
            Access::Granted { .. }
        ) {
            return Err(zbus::fdo::Error::AccessDenied("no grant for this account".into()));
        }
        // Take the handle out under the lock, then tear it down off-lock.
        let mount = self.mounts.lock().ok().and_then(|mut m| m.remove(&account_id));
        match mount {
            Some(m) => m
                .unmount()
                .await
                .map_err(|_| zbus::fdo::Error::Failed("unmount failed".into())),
            None => Ok(()),
        }
    }
}

/// The per-account runtime dir (`XDG_RUNTIME_DIR`) and rclone cache dir
/// (`XDG_CACHE_HOME`, else `~/.cache`, under `arlen/rclone/{id}`) for a confined
/// mount. `None` when neither base is set - the daemon cannot place the mount.
fn mount_dirs(account_id: &str) -> Option<(PathBuf, PathBuf)> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from)?;
    let cache_base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))?;
    let cache = cache_base.join("arlen").join("rclone").join(account_id);
    Some((runtime, cache))
}

/// The management ObjectManager surface (online-accounts-plan.md §3.1), a separate
/// object served at the same `/org/arlen/Accounts1` path (zbus needs one type per
/// interface). The full account tree is exposed ONLY to the Settings management
/// app; every other same-uid caller gets an empty tree (the in-code gate, since a
/// session-bus policy cannot distinguish same-uid callers). Apps enumerate via the
/// caller-filtered `ListAccounts`, not this surface.
pub struct AccountsObjectManager {
    accounts_dir: PathBuf,
}

impl AccountsObjectManager {
    /// Build the manager over the account config dir (reloaded per call, so it
    /// tracks account add/remove without a restart, like the daemon's methods).
    pub fn new(accounts_dir: PathBuf) -> Self {
        Self { accounts_dir }
    }
}

#[zbus::interface(name = "org.freedesktop.DBus.ObjectManager")]
impl AccountsObjectManager {
    /// Return the managed per-account objects, but only for the Settings management
    /// app; a non-management or unresolvable caller gets an empty tree, never a
    /// leak. The per-account property maps are the non-secret metadata, inline.
    ///
    /// Uses the PID-reuse-GUARDED caller resolver, not the unguarded one the
    /// grant-filtered `list_accounts` may use: on a `settings` verdict this returns
    /// the FULL account inventory (unfiltered by grant), so the payoff of winning a
    /// sub-millisecond PID-recycling race would be the whole inventory, strictly
    /// larger than the single-caller metadata `list_accounts` exposes. The guarded
    /// resolver closes that race, the consistent choice for the highest-value
    /// disclosure surface.
    async fn get_managed_objects(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> zbus::fdo::Result<crate::objects::ManagedObjects> {
        let caller = resolve_caller_app_id_guarded(&header, connection)
            .await
            .unwrap_or_default();
        let (configs, _errs) = crate::config::load_accounts(&self.accounts_dir);
        Ok(crate::objects::managed_objects_gated(&caller, &configs))
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

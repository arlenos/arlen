//! The `org.arlen.Power1` D-Bus interface.
//!
//! A read surface over the daemon's latest [`PowerState`] snapshot (PWR-R1):
//! the shell, apps and the SDK query power state on demand instead of forking
//! UPower. The poll loop in `main` updates the shared snapshot; this interface
//! serves it. Actions (`Suspend`/`SetProfile`/…) are added by PWR-R2/R5 and
//! gated by PWR-R7; this read interface is unprivileged.

use std::sync::Arc;

use tokio::sync::RwLock;
use tracing::{info, warn};

use crate::logind::{self, PowerAction};
use crate::power::PowerState;
use crate::profiles;

/// Shared, atomically-swappable latest power snapshot.
pub type SharedState = Arc<RwLock<PowerState>>;

/// The `org.arlen.Power1` object.
///
/// Reads (the properties below) are unprivileged. Actions (`Suspend`,
/// `SetProfile`) are gated by PWR-R7: the daemon holds the logind /
/// power-profiles-daemon trust on its own **system-bus** connection
/// ([`PowerInterface::system_bus`]); a caller reaches an action only with the
/// matching `[system.power]` grant in its profile, resolved from the caller's
/// bus-attested app id.
pub struct PowerInterface {
    state: SharedState,
    /// The system-bus connection used to drive logind / p-p-d for actions.
    /// `None` if the system bus was unavailable at startup, in which case every
    /// action fails closed.
    system_bus: Option<zbus::Connection>,
}

impl PowerInterface {
    /// Wrap the shared snapshot (the poll loop updates it) and the system-bus
    /// connection used for privileged actions.
    pub fn new(state: SharedState, system_bus: Option<zbus::Connection>) -> Self {
        Self { state, system_bus }
    }
}

#[zbus::interface(name = "org.arlen.Power1")]
impl PowerInterface {
    /// True on battery, false on AC.
    #[zbus(property)]
    async fn on_battery(&self) -> bool {
        self.state.read().await.on_battery
    }

    /// Battery charge, 0-100.
    #[zbus(property)]
    async fn percentage(&self) -> u8 {
        self.state.read().await.percentage
    }

    /// Charge state: "charging"|"discharging"|"full"|"empty"|"unknown".
    #[zbus(property)]
    async fn charge_state(&self) -> String {
        self.state.read().await.charge.as_str().to_string()
    }

    /// Seconds to empty (0 when unknown or charging).
    #[zbus(property)]
    async fn time_to_empty_seconds(&self) -> i64 {
        self.state.read().await.time_to_empty_seconds
    }

    /// Seconds to full (0 when unknown or discharging).
    #[zbus(property)]
    async fn time_to_full_seconds(&self) -> i64 {
        self.state.read().await.time_to_full_seconds
    }

    /// Lid state: "open"|"closed"|"none".
    #[zbus(property)]
    async fn lid_state(&self) -> String {
        self.state.read().await.lid.as_str().to_string()
    }

    /// Active power profile: "performance"|"balanced"|"power-saver"|"unknown".
    #[zbus(property)]
    async fn profile(&self) -> String {
        self.state.read().await.profile.clone()
    }

    /// Request a sleep/power action ("suspend"|"hibernate"|"suspend-then-hibernate"
    /// |"hybrid-sleep"|"power-off"|"reboot"). Gated on the caller's
    /// `[system.power] suspend` grant (PWR-R7); the action runs non-interactively
    /// so logind still honours other apps' block inhibitors.
    async fn suspend(
        &self,
        action: String,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> zbus::fdo::Result<()> {
        let power = self.caller_power_grant(&header, connection).await?;
        if !power.suspend {
            return Err(zbus::fdo::Error::AccessDenied(
                "caller lacks the system.power suspend grant".into(),
            ));
        }
        let act = PowerAction::parse(&action)
            .ok_or_else(|| zbus::fdo::Error::InvalidArgs(format!("unknown power action: {action}")))?;
        let bus = self
            .system_bus
            .as_ref()
            .ok_or_else(|| zbus::fdo::Error::Failed("system bus unavailable".into()))?;
        logind::perform(bus, act)
            .await
            .map_err(|e| zbus::fdo::Error::Failed(format!("logind {}: {e}", act.as_str())))?;
        info!(action = act.as_str(), "performed power action");
        Ok(())
    }

    /// Change the active power profile ("performance"|"balanced"|"power-saver").
    /// Gated on the caller's `[system.power] set_profile` grant (PWR-R7).
    async fn set_profile(
        &self,
        profile: String,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> zbus::fdo::Result<()> {
        let power = self.caller_power_grant(&header, connection).await?;
        if !power.set_profile {
            return Err(zbus::fdo::Error::AccessDenied(
                "caller lacks the system.power set_profile grant".into(),
            ));
        }
        let bus = self
            .system_bus
            .as_ref()
            .ok_or_else(|| zbus::fdo::Error::Failed("system bus unavailable".into()))?;
        profiles::set_active_profile(bus, &profile)
            .await
            .map_err(|e| zbus::fdo::Error::Failed(format!("set profile {profile}: {e}")))?;
        info!(profile = %profile, "changed power profile");
        Ok(())
    }
}

impl PowerInterface {
    /// Resolve the caller's `[system.power]` grant, fail-closed.
    ///
    /// The caller's app id comes from the session bus daemon's attested PID
    /// (`GetConnectionUnixProcessID`, not a client value) resolved through the
    /// F3 `path_to_app_id` chain - the same identity model knowledge/installd/
    /// online-accounts use. Any failure (no sender, bus error, unresolvable
    /// binary, no profile) yields an empty grant, so an unprofiled or
    /// unidentifiable caller is denied.
    ///
    /// Residual (documented, low for a per-user power daemon): the sub-millisecond
    /// PID-reuse window between the bus attesting the PID and reading `/proc`. The
    /// blast radius is bounded to actions the user can already take on their own
    /// machine; the `pid_start_time` capture-recheck (the knowledge-daemon
    /// pattern) is the close, deferred with online-accounts'.
    async fn caller_power_grant(
        &self,
        header: &zbus::message::Header<'_>,
        connection: &zbus::Connection,
    ) -> zbus::fdo::Result<arlen_permissions::PowerPermissions> {
        let app_id = match resolve_caller_app_id(header, connection).await {
            Ok(id) => id,
            Err(e) => {
                warn!("power action denied: unresolved caller: {e}");
                return Err(zbus::fdo::Error::AccessDenied("unresolved caller".into()));
            }
        };
        match arlen_permissions::load_profile(&app_id) {
            Ok(profile) => Ok(profile.system.power),
            Err(e) => {
                warn!(app_id = %app_id, "power action denied: no profile: {e}");
                Err(zbus::fdo::Error::AccessDenied("no profile for caller".into()))
            }
        }
    }
}

/// Resolve the calling app's Arlen identity from the D-Bus connection.
///
/// The session bus daemon attests the sender's PID (`GetConnectionUnixProcessID`,
/// not a client-supplied value), and `app_id_from_pid` resolves `/proc/<pid>/exe`
/// through the F3 `path_to_app_id` chain. Any failure is an `Err` (fail-closed).
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

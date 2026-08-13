//! The systemd seam, over the user manager's own D-Bus interface.
//!
//! Two calls, both on `org.freedesktop.systemd1.Manager` at
//! `/org/freedesktop/systemd1`: `StartUnit` to bring a unit up, and `GetUnit`
//! followed by the `MainPID` property to find out what is running.
//!
//! The blocking zbus API, deliberately: [`Systemd`] is a synchronous trait because
//! the decision half it serves is synchronous and pure, and wrapping a runtime
//! around two property reads would add a moving part without removing one. It is
//! the same choice `lock-auth`'s fprintd client made.

use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::OwnedObjectPath;

use crate::supervise::Systemd;

/// The user manager's bus name, object and manager interface.
const SYSTEMD: &str = "org.freedesktop.systemd1";
const MANAGER_PATH: &str = "/org/freedesktop/systemd1";
const MANAGER_IFACE: &str = "org.freedesktop.systemd1.Manager";
const SERVICE_IFACE: &str = "org.freedesktop.systemd1.Service";

/// A client for the `systemd --user` manager of the session this runs in.
pub struct SystemdBus {
    conn: Connection,
}

impl SystemdBus {
    /// Connect to the session bus, which is where the user manager lives.
    ///
    /// A failure here is not a supervisor bug: it is what a run outside a session
    /// looks like (no `DBUS_SESSION_BUS_ADDRESS`), and the caller reports that
    /// rather than registering anything.
    pub fn session() -> Result<Self, String> {
        Connection::session()
            .map(|conn| Self { conn })
            .map_err(|e| format!("no session bus: {e}"))
    }

    fn manager(&self) -> Result<Proxy<'_>, String> {
        Proxy::new(&self.conn, SYSTEMD, MANAGER_PATH, MANAGER_IFACE)
            .map_err(|e| format!("systemd manager proxy: {e}"))
    }
}

impl Systemd for SystemdBus {
    fn start(&self, unit: &str) -> Result<(), String> {
        // "replace" is systemd's ordinary mode and is what `systemctl start`
        // sends; starting an already-running unit is a no-op, which is the
        // property the round relies on to be safe to repeat.
        let _job: OwnedObjectPath = self
            .manager()?
            .call("StartUnit", &(unit, "replace"))
            .map_err(|e| format!("StartUnit({unit}): {e}"))?;
        Ok(())
    }

    fn main_pid(&self, unit: &str) -> Result<u32, String> {
        // `GetUnit` only resolves a unit systemd has LOADED, which is why the
        // round starts the unit first: after a start it is loaded whether or not
        // it stayed up, so a failure here is a real failure and not the ordinary
        // not-yet-touched case.
        let path: OwnedObjectPath = self
            .manager()?
            .call("GetUnit", &(unit,))
            .map_err(|e| format!("GetUnit({unit}): {e}"))?;
        let service = Proxy::new(&self.conn, SYSTEMD, path.as_str(), SERVICE_IFACE)
            .map_err(|e| format!("service proxy for {unit}: {e}"))?;
        // 0 while the unit is between restarts or has given up, which the decision
        // half reads as NotRunning rather than as an error.
        service
            .get_property("MainPID")
            .map_err(|e| format!("MainPID({unit}): {e}"))
    }
}

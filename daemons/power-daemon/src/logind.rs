//! systemd-logind sleep/power actions (system-services-plan.md PWR-R2).
//!
//! `systemd-logind` owns `org.freedesktop.login1` on the **system** bus and is
//! the only sanctioned path to suspend/hibernate/power-off. The power daemon
//! drives it on behalf of callers, who reach it only through the capability
//! gate (PWR-R7) - the daemon holds the logind trust, callers hold a scoped
//! token. Every action is invoked **non-interactively** (`interactive=false`),
//! so logind still honours block inhibitors registered by other apps rather
//! than forcing the action.
//!
//! For lock-before-sleep, the daemon takes a **delay** inhibitor on `sleep`
//! ([`take_delay_inhibitor`]) and watches `PrepareForSleep`; the lock handshake
//! itself is the lock screen's (LS-R2/R3, cross-repo), so this module provides
//! the inhibitor primitive and the action calls, not the lock policy.
//!
//! The action -> D-Bus method mapping is pure and unit-tested; the calls
//! themselves need a live system bus (Tim verifies actual suspend on metal).

use zbus::zvariant::OwnedFd;

/// logind's well-known name, manager object path and interface.
pub const LOGIND_BUS: &str = "org.freedesktop.login1";
pub const LOGIND_PATH: &str = "/org/freedesktop/login1";
pub const LOGIND_MANAGER_IFACE: &str = "org.freedesktop.login1.Manager";

/// A logind power action. Closed set: only these are ever issued, and each
/// maps to exactly one Manager method + its `Can*` probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerAction {
    /// Suspend to RAM.
    Suspend,
    /// Suspend to disk.
    Hibernate,
    /// Suspend to RAM, then hibernate after a delay.
    SuspendThenHibernate,
    /// Suspend to RAM with the system staying able to hibernate.
    HybridSleep,
    /// Power the machine off.
    PowerOff,
    /// Reboot the machine.
    Reboot,
}

impl PowerAction {
    /// The logind `Manager` method that performs the action.
    pub fn method(self) -> &'static str {
        match self {
            PowerAction::Suspend => "Suspend",
            PowerAction::Hibernate => "Hibernate",
            PowerAction::SuspendThenHibernate => "SuspendThenHibernate",
            PowerAction::HybridSleep => "HybridSleep",
            PowerAction::PowerOff => "PowerOff",
            PowerAction::Reboot => "Reboot",
        }
    }

    /// The logind `Manager` `Can*` probe for the action.
    pub fn can_method(self) -> &'static str {
        match self {
            PowerAction::Suspend => "CanSuspend",
            PowerAction::Hibernate => "CanHibernate",
            PowerAction::SuspendThenHibernate => "CanSuspendThenHibernate",
            PowerAction::HybridSleep => "CanHybridSleep",
            PowerAction::PowerOff => "CanPowerOff",
            PowerAction::Reboot => "CanReboot",
        }
    }

    /// The wire string used by the capability gate (PWR-R7) and the D-Bus API.
    pub fn as_str(self) -> &'static str {
        match self {
            PowerAction::Suspend => "suspend",
            PowerAction::Hibernate => "hibernate",
            PowerAction::SuspendThenHibernate => "suspend-then-hibernate",
            PowerAction::HybridSleep => "hybrid-sleep",
            PowerAction::PowerOff => "power-off",
            PowerAction::Reboot => "reboot",
        }
    }

    /// Parse the wire string back to an action (`as_str` inverse).
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "suspend" => PowerAction::Suspend,
            "hibernate" => PowerAction::Hibernate,
            "suspend-then-hibernate" => PowerAction::SuspendThenHibernate,
            "hybrid-sleep" => PowerAction::HybridSleep,
            "power-off" => PowerAction::PowerOff,
            "reboot" => PowerAction::Reboot,
            _ => return None,
        })
    }
}

/// Whether logind reports an action as available.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Availability {
    /// Available now.
    Yes,
    /// Not available (no hardware support, or disabled).
    No,
    /// Available but needs polkit authentication (we invoke non-interactively,
    /// so this means the caller's session is not authorised for it).
    Challenge,
    /// Not applicable on this system.
    NotApplicable,
}

impl Availability {
    /// Map logind's `Can*` reply string.
    pub fn from_logind(s: &str) -> Self {
        match s {
            "yes" => Availability::Yes,
            "challenge" => Availability::Challenge,
            "na" => Availability::NotApplicable,
            _ => Availability::No,
        }
    }

    /// Whether the action can be performed without a polkit challenge.
    pub fn is_available(self) -> bool {
        matches!(self, Availability::Yes)
    }

    /// A short reason string for the unavailable case (used in the error a
    /// caller sees when logind will not perform the action).
    pub fn as_str(self) -> &'static str {
        match self {
            Availability::Yes => "yes",
            Availability::No => "no",
            Availability::Challenge => "requires authentication",
            Availability::NotApplicable => "not applicable",
        }
    }
}

/// Build a `Manager` proxy on the system bus.
async fn manager(conn: &zbus::Connection) -> zbus::Result<zbus::Proxy<'static>> {
    zbus::Proxy::new(conn, LOGIND_BUS, LOGIND_PATH, LOGIND_MANAGER_IFACE).await
}

/// Probe whether logind can perform the action right now.
pub async fn can_perform(conn: &zbus::Connection, action: PowerAction) -> zbus::Result<Availability> {
    let proxy = manager(conn).await?;
    let reply: String = proxy.call(action.can_method(), &()).await?;
    Ok(Availability::from_logind(&reply))
}

/// Perform the action non-interactively, so logind honours other apps' block
/// inhibitors instead of forcing it.
pub async fn perform(conn: &zbus::Connection, action: PowerAction) -> zbus::Result<()> {
    let proxy = manager(conn).await?;
    proxy.call::<_, _, ()>(action.method(), &(false)).await
}

/// Take a **delay** inhibitor on the given lock classes (e.g. `"sleep"` or
/// `"sleep:shutdown"`). The returned fd holds the inhibitor; dropping it
/// releases the lock. Delay mode lets the daemon run its lock-before-sleep
/// work before logind proceeds, without blocking sleep outright.
pub async fn take_delay_inhibitor(
    conn: &zbus::Connection,
    what: &str,
    who: &str,
    why: &str,
) -> zbus::Result<OwnedFd> {
    let proxy = manager(conn).await?;
    proxy.call("Inhibit", &(what, who, why, "delay")).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_method_and_can_method_pair_up() {
        for a in [
            PowerAction::Suspend,
            PowerAction::Hibernate,
            PowerAction::SuspendThenHibernate,
            PowerAction::HybridSleep,
            PowerAction::PowerOff,
            PowerAction::Reboot,
        ] {
            assert_eq!(a.can_method(), format!("Can{}", a.method()));
        }
    }

    #[test]
    fn wire_string_round_trips() {
        for a in [
            PowerAction::Suspend,
            PowerAction::Hibernate,
            PowerAction::SuspendThenHibernate,
            PowerAction::HybridSleep,
            PowerAction::PowerOff,
            PowerAction::Reboot,
        ] {
            assert_eq!(PowerAction::parse(a.as_str()), Some(a));
        }
        assert_eq!(PowerAction::parse("explode"), None);
    }

    #[test]
    fn availability_maps_logind_strings() {
        assert_eq!(Availability::from_logind("yes"), Availability::Yes);
        assert_eq!(Availability::from_logind("no"), Availability::No);
        assert_eq!(Availability::from_logind("challenge"), Availability::Challenge);
        assert_eq!(Availability::from_logind("na"), Availability::NotApplicable);
        assert_eq!(Availability::from_logind("weird"), Availability::No);
        assert!(Availability::Yes.is_available());
        assert!(!Availability::Challenge.is_available());
        // The reason string is set for every variant (the unavailable error path).
        assert_eq!(Availability::Challenge.as_str(), "requires authentication");
        assert_eq!(Availability::No.as_str(), "no");
    }
}

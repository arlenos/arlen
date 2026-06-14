//! Power-profile read over `power-profiles-daemon` (system-services-plan.md PWR-R5).
//!
//! `power-profiles-daemon` owns `net.hadess.PowerProfiles` on the **system**
//! bus and exposes the active profile as the `ActiveProfile` property
//! ("power-saver"|"balanced"|"performance"). The power daemon reads it on each
//! poll and folds it into the published [`crate::power::PowerState`], so the
//! shell shows the profile without each consumer talking to p-p-d.
//!
//! The read is best-effort: a machine without p-p-d (or with it stopped) keeps
//! `profile = "unknown"` rather than failing the snapshot. Setting the profile
//! is a privileged action (`SetProfile`) that lands with the PWR-R7 capability
//! gate, so it is deliberately not exposed here yet; this module is read-only.
//!
//! [`normalize_profile`] is pure so the string normalisation is unit-tested
//! without a live bus.

/// `power-profiles-daemon`'s well-known name, object path and interface.
pub const PPD_BUS: &str = "net.hadess.PowerProfiles";
pub const PPD_PATH: &str = "/net/hadess/PowerProfiles";
pub const PPD_IFACE: &str = "net.hadess.PowerProfiles";

/// The canonical "profile unknown / not available" value.
pub const PROFILE_UNKNOWN: &str = "unknown";

/// Normalise a `power-profiles-daemon` `ActiveProfile` string to Arlen's
/// canonical set. p-p-d already emits exactly these three values; anything
/// unexpected (a future profile, an empty string) maps to `"unknown"` so the
/// snapshot never carries an uninterpreted value.
pub fn normalize_profile(raw: &str) -> String {
    match raw {
        "performance" => "performance".to_string(),
        "balanced" => "balanced".to_string(),
        "power-saver" => "power-saver".to_string(),
        _ => PROFILE_UNKNOWN.to_string(),
    }
}

/// Read the active power profile from `power-profiles-daemon` on the system
/// bus, normalised. Returns `None` when p-p-d is absent or the property read
/// fails, so the caller can fall back to `"unknown"` without failing the
/// whole power snapshot.
pub async fn read_active_profile(conn: &zbus::Connection) -> Option<String> {
    let proxy = zbus::Proxy::new(conn, PPD_BUS, PPD_PATH, PPD_IFACE).await.ok()?;
    let active: String = proxy.get_property("ActiveProfile").await.ok()?;
    Some(normalize_profile(&active))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_the_three_known_profiles() {
        assert_eq!(normalize_profile("performance"), "performance");
        assert_eq!(normalize_profile("balanced"), "balanced");
        assert_eq!(normalize_profile("power-saver"), "power-saver");
    }

    #[test]
    fn unknown_profiles_map_to_unknown() {
        assert_eq!(normalize_profile(""), "unknown");
        assert_eq!(normalize_profile("turbo"), "unknown");
        assert_eq!(normalize_profile("Performance"), "unknown");
    }
}

//! SEN-1 exposure-posture readout: does the radio environment leak a stable
//! hardware identity an observer can track across places?
//!
//! The default-ON, deterministic detector. The daemon reads the raw hardware state
//! (Wi-Fi active vs permanent MAC, the saved-connection cloned-MAC policy, the
//! Bluetooth adapter's Discoverable/Pairable flags) via the graduated read
//! contracts; this module turns each reading into a plain [`Posture`], so the
//! comparisons are one auditable, testable place. Pure - no I/O, no radio.

/// The privacy posture of one exposure surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Posture {
    /// The surface does not leak a stable hardware identity.
    Protected,
    /// The surface broadcasts a stable hardware identity an observer can track.
    Exposed,
    /// The reading is missing or ambiguous, so the posture cannot be determined
    /// (surfaced as-is, never guessed to "protected").
    Unknown,
}

/// Normalize a MAC for comparison: trim and lowercase (hex is case-insensitive), so
/// `AA:BB:...` and `aa:bb:...` compare equal. `None` for an empty/whitespace value.
fn normalize_mac(mac: &str) -> Option<String> {
    let t = mac.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_ascii_lowercase())
    }
}

/// Whether Wi-Fi currently broadcasts the PERMANENT hardware MAC (trackable) or a
/// randomized one. `Exposed` when the active MAC equals the permanent MAC,
/// `Protected` when it differs (randomization is active), `Unknown` when either MAC
/// is missing (the daemon could not read it).
pub fn wifi_mac_posture(active_mac: &str, permanent_mac: &str) -> Posture {
    match (normalize_mac(active_mac), normalize_mac(permanent_mac)) {
        (Some(active), Some(permanent)) => {
            if active == permanent {
                Posture::Exposed
            } else {
                Posture::Protected
            }
        }
        _ => Posture::Unknown,
    }
}

/// The posture implied by a saved connection's NetworkManager `cloned-mac-address`
/// policy (the setting that governs FUTURE activations). `stable`/`random` and an
/// explicit non-permanent MAC randomize (Protected); `permanent` uses the hardware
/// MAC (Exposed); an absent or `preserve`/unrecognized value is ambiguous (Unknown -
/// the remediation writes an explicit `stable`). Case-insensitive.
pub fn cloned_mac_policy_posture(policy: Option<&str>) -> Posture {
    match policy.map(str::trim) {
        Some(p) if p.eq_ignore_ascii_case("stable") || p.eq_ignore_ascii_case("random") => {
            Posture::Protected
        }
        Some(p) if p.eq_ignore_ascii_case("permanent") => Posture::Exposed,
        // An explicit MAC literal (contains a colon) is a set, non-hardware address.
        Some(p) if p.contains(':') => Posture::Protected,
        _ => Posture::Unknown,
    }
}

/// The Bluetooth adapter's exposure: `Exposed` while it is Discoverable (broadcasting
/// its presence so anyone nearby can see + address it), else `Protected`. A pairable-
/// but-not-discoverable adapter is not a broadcast leak (a device must already know
/// the address to initiate), so `pairable` alone does not expose; it is carried so a
/// caller can still surface a persistent-pairable hint.
pub fn bluetooth_posture(discoverable: bool, _pairable: bool) -> Posture {
    if discoverable {
        Posture::Exposed
    } else {
        Posture::Protected
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wifi_exposed_when_active_is_the_permanent_mac() {
        assert_eq!(
            wifi_mac_posture("AA:BB:CC:DD:EE:FF", "aa:bb:cc:dd:ee:ff"),
            Posture::Exposed,
            "case-insensitive equality"
        );
        assert_eq!(
            wifi_mac_posture("02:11:22:33:44:55", "AA:BB:CC:DD:EE:FF"),
            Posture::Protected,
            "a different (randomized) active MAC"
        );
        assert_eq!(wifi_mac_posture("", "AA:BB:CC:DD:EE:FF"), Posture::Unknown);
        assert_eq!(wifi_mac_posture("02:11:22:33:44:55", "  "), Posture::Unknown);
    }

    #[test]
    fn cloned_mac_policy_maps_to_posture() {
        assert_eq!(cloned_mac_policy_posture(Some("stable")), Posture::Protected);
        assert_eq!(cloned_mac_policy_posture(Some("Random")), Posture::Protected);
        assert_eq!(cloned_mac_policy_posture(Some("permanent")), Posture::Exposed);
        assert_eq!(cloned_mac_policy_posture(Some("02:11:22:33:44:55")), Posture::Protected);
        assert_eq!(cloned_mac_policy_posture(None), Posture::Unknown);
        assert_eq!(cloned_mac_policy_posture(Some("preserve")), Posture::Unknown);
    }

    #[test]
    fn bluetooth_exposed_only_while_discoverable() {
        assert_eq!(bluetooth_posture(true, false), Posture::Exposed);
        assert_eq!(bluetooth_posture(true, true), Posture::Exposed);
        assert_eq!(bluetooth_posture(false, true), Posture::Protected);
        assert_eq!(bluetooth_posture(false, false), Posture::Protected);
    }
}

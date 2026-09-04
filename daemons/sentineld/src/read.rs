//! What this machine broadcasts about itself, read off the running system.
//!
//! Split in two on purpose. [`Readings`] is plain data and [`postures`] turns it
//! into the six surface postures with no I/O anywhere near it, so the judgements
//! are testable without a radio, a NetworkManager or a Bluetooth adapter. Only
//! [`read_host`] touches the bus, and every step of it is allowed to fail.
//!
//! A FAILED READING IS A READING. Each field is an `Option` and an absent one
//! becomes `Posture::Unknown`, which the readout carries through to the page as
//! its own line. The alternative - dropping what could not be measured - would
//! leave a privacy page showing only the good news it happened to collect, and
//! somebody would read a short green list as an all-clear.

use arlen_sentinel_detect::exposure::{self, Posture};
use arlen_sentinel_detect::readout::Surface;

/// The raw state of this host's radio identity surface.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Readings {
    /// The MAC the Wi-Fi interface is using right now.
    pub wifi_active_mac: Option<String>,
    /// The MAC burned into the card.
    pub wifi_permanent_mac: Option<String>,
    /// What each saved Wi-Fi profile says about FUTURE activations, one entry per
    /// profile. An empty vector means there are no saved Wi-Fi networks; a `None`
    /// entry means that profile does not say.
    pub saved_mac_policies: Vec<Option<String>>,
    /// Whether any saved Wi-Fi profile is marked hidden. `None` when the saved
    /// connections could not be read at all.
    pub any_hidden_network: Option<bool>,
    /// Whether any saved profile sends this machine's hostname over DHCP.
    pub sends_dhcp_hostname: Option<bool>,
    /// Whether the Bluetooth adapter is discoverable right now.
    pub bluetooth_discoverable: Option<bool>,
    /// Whether it accepts incoming pairing.
    pub bluetooth_pairable: Option<bool>,
    /// BlueZ's LE privacy mode, verbatim from `main.conf`.
    pub ble_privacy: Option<String>,
}

/// The posture of the saved-profile MAC policy, taken across every saved network.
///
/// WORST CASE WINS, and that is the point of aggregating rather than listing. One
/// profile pinned to the hardware address is enough to hand an observer a stable
/// identifier the next time that network is joined, so a single exposed profile
/// makes the surface exposed however many well-behaved ones sit beside it.
///
/// No saved networks at all reads as protected, not unknown: nothing was hidden
/// from the reader, there is simply no stored profile that could join with the
/// permanent address. Saying "could not read" there would invent a doubt.
fn saved_policy_posture(policies: &[Option<String>]) -> Posture {
    let mut worst = Posture::Protected;
    for p in policies {
        match exposure::cloned_mac_policy_posture(p.as_deref()) {
            Posture::Exposed => return Posture::Exposed,
            Posture::Unknown => worst = Posture::Unknown,
            Posture::Protected => {}
        }
    }
    worst
}

/// The six surface postures, from what was measured.
#[must_use]
pub fn postures(r: &Readings) -> Vec<(Surface, Posture)> {
    let wifi = match (&r.wifi_active_mac, &r.wifi_permanent_mac) {
        (Some(a), Some(p)) => exposure::wifi_mac_posture(a, p),
        _ => Posture::Unknown,
    };
    let saved = if r.any_hidden_network.is_none() && r.saved_mac_policies.is_empty() {
        // The connection list itself did not read, so there is nothing to
        // aggregate. Distinct from a machine with no saved networks, which reads
        // the list fine and finds it empty.
        Posture::Unknown
    } else {
        saved_policy_posture(&r.saved_mac_policies)
    };
    let hidden = match r.any_hidden_network {
        Some(h) => exposure::hidden_ssid_posture(h),
        None => Posture::Unknown,
    };
    let hostname = match r.sends_dhcp_hostname {
        Some(s) => exposure::dhcp_hostname_posture(s),
        None => Posture::Unknown,
    };
    let bt = match r.bluetooth_discoverable {
        Some(d) => exposure::bluetooth_posture(d, r.bluetooth_pairable.unwrap_or(false)),
        None => Posture::Unknown,
    };
    let ble = exposure::ble_privacy_posture(r.ble_privacy.as_deref());
    vec![
        (Surface::WifiMac, wifi),
        (Surface::SavedMacPolicy, saved),
        (Surface::HiddenNetwork, hidden),
        (Surface::DhcpHostname, hostname),
        (Surface::BluetoothDiscoverable, bt),
        (Surface::BlerPrivacy, ble),
    ]
}

/// Where BlueZ keeps the LE privacy setting.
pub const BLUEZ_MAIN_CONF: &str = "/etc/bluetooth/main.conf";

/// Pull `Privacy` out of a BlueZ `main.conf`.
///
/// A hand-rolled read of one key rather than a full ini parser, because that is
/// all this needs and a dependency for one lookup is the wrong trade. Comments
/// are skipped, and a commented-out `Privacy` line is NOT a setting: BlueZ ships
/// the file with its defaults commented, so treating `#Privacy = device` as
/// configured would report a machine as protected on the strength of an example.
#[must_use]
pub fn privacy_from_main_conf(text: &str) -> Option<String> {
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim().eq_ignore_ascii_case("Privacy") {
            let v = value.split('#').next().unwrap_or("").trim();
            if !v.is_empty() {
                return Some(v.to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_pinned_profile_exposes_the_surface_whatever_its_neighbours_do() {
        let p = saved_policy_posture(&[
            Some("stable".into()),
            Some("permanent".into()),
            Some("random".into()),
        ]);
        assert_eq!(p, Posture::Exposed);
    }

    #[test]
    fn a_machine_with_no_saved_networks_has_nothing_to_leak() {
        assert_eq!(saved_policy_posture(&[]), Posture::Protected);
    }

    #[test]
    fn a_profile_that_does_not_say_leaves_the_surface_unread() {
        assert_eq!(
            saved_policy_posture(&[Some("stable".into()), None]),
            Posture::Unknown
        );
    }

    #[test]
    fn a_reading_nobody_could_take_becomes_a_line_of_its_own() {
        let all = postures(&Readings::default());
        assert_eq!(all.len(), 6);
        assert!(
            all.iter().all(|(_, p)| *p == Posture::Unknown),
            "nothing measured, nothing claimed"
        );
    }

    #[test]
    fn an_empty_connection_list_that_read_is_not_a_connection_list_that_failed() {
        let read_fine = Readings {
            any_hidden_network: Some(false),
            ..Readings::default()
        };
        let by_surface: Vec<_> = postures(&read_fine)
            .into_iter()
            .filter(|(s, _)| *s == Surface::SavedMacPolicy)
            .collect();
        assert_eq!(by_surface[0].1, Posture::Protected);

        let failed = Readings::default();
        let by_surface: Vec<_> = postures(&failed)
            .into_iter()
            .filter(|(s, _)| *s == Surface::SavedMacPolicy)
            .collect();
        assert_eq!(by_surface[0].1, Posture::Unknown);
    }

    #[test]
    fn a_discoverable_adapter_is_exposed_and_a_quiet_one_is_not() {
        let on = Readings {
            bluetooth_discoverable: Some(true),
            ..Readings::default()
        };
        let off = Readings {
            bluetooth_discoverable: Some(false),
            ..Readings::default()
        };
        let bt = |r: &Readings| {
            postures(r)
                .into_iter()
                .find(|(s, _)| *s == Surface::BluetoothDiscoverable)
                .unwrap()
                .1
        };
        assert_eq!(bt(&on), Posture::Exposed);
        assert_eq!(bt(&off), Posture::Protected);
    }

    #[test]
    fn a_commented_default_is_not_a_setting() {
        let conf = "[General]\n# Privacy = device\nName = arlen\n";
        assert_eq!(privacy_from_main_conf(conf), None);
    }

    #[test]
    fn the_privacy_key_is_read_with_its_trailing_comment_stripped() {
        let conf = "[General]\nPrivacy = device # rotate the address\n";
        assert_eq!(privacy_from_main_conf(conf).as_deref(), Some("device"));
    }

    #[test]
    fn an_empty_privacy_value_is_not_a_setting() {
        assert_eq!(privacy_from_main_conf("Privacy =\n"), None);
    }
}

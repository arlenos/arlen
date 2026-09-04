//! The exposure readout: six surface postures composed into the lines a person
//! reads, and whether each one has a remediation behind it.
//!
//! [`crate::exposure`] answers one surface at a time. This is the layer above:
//! it takes what the daemon measured and produces the list Settings renders,
//! with the two properties that list has to have.
//!
//! **A line names a SURFACE, never a sentence.** The daemon cannot write the
//! prose - every user-facing string in this project comes from the app's own
//! catalogue, so a daemon returning English would be a string no locale can
//! reach and the born-translatable lint would be right to refuse it. So the wire
//! carries a [`Surface`] and its [`Posture`], and the app turns that pair into
//! the sentence in the reader's language.
//!
//! **A surface nobody could read is a line too.** `Posture::Unknown` is carried
//! through rather than dropped or rounded to protected: "we could not read the
//! Bluetooth adapter" and "the Bluetooth adapter is fine" are different things
//! to tell somebody, and a privacy page that shows only the good news it managed
//! to measure is the failure this whole surface exists to avoid.

use crate::exposure::Posture;

/// One exposure surface the readout can speak about.
///
/// A closed set on purpose: the app matches on it to pick a sentence, so a new
/// surface is a compile error there rather than a line that renders as nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Surface {
    /// Whether Wi-Fi currently broadcasts the permanent hardware MAC.
    WifiMac,
    /// What the saved connections say about FUTURE activations.
    SavedMacPolicy,
    /// A saved network marked hidden, which directed-probes its name everywhere.
    HiddenNetwork,
    /// Whether saved connections send the machine's hostname over DHCP.
    DhcpHostname,
    /// Whether the Bluetooth adapter is discoverable right now.
    BluetoothDiscoverable,
    /// Whether BlueZ advertises with a rotating address or its identity address.
    BlerPrivacy,
}

impl Surface {
    /// Every surface, in the order [`compose`] is given them.
    ///
    /// Exists so a caller mapping surfaces to something else - a sentence, a wire
    /// key - can prove it covered all of them rather than trusting a match arm it
    /// wrote by hand.
    pub const ALL: [Surface; 6] = [
        Surface::WifiMac,
        Surface::SavedMacPolicy,
        Surface::HiddenNetwork,
        Surface::DhcpHostname,
        Surface::BluetoothDiscoverable,
        Surface::BlerPrivacy,
    ];
}

/// One line of the posture readout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Line {
    /// Which surface this line is about.
    pub surface: Surface,
    /// What was measured there.
    pub posture: Posture,
    /// Whether the daemon offers a one-click remediation for this line AS IT
    /// STANDS. False for a protected surface (nothing to fix), false for one the
    /// plan marks confirm-first or not one-click safe, and false for a surface
    /// nobody could read - a fix button on a reading that failed would act on a
    /// state nobody established.
    pub fixable: bool,
}

/// Whether an EXPOSED reading of this surface has a one-click safe remediation.
///
/// Straight from `privacy-sentinel-plan.md` §2.3, which splits the remediations
/// into one-click-safe and confirm-first. Only the safe ones get a button here;
/// the others are real remediations that belong behind their own confirmation,
/// and offering them as one click would be the panel making a destructive choice
/// on somebody's behalf.
///
///   * Wi-Fi joining with the hardware MAC -> write `stable`: safe, reversible,
///     and it is the system default already.
///   * Bluetooth discoverable -> turn it off: safe, no effect on paired devices.
///   * A hidden saved network -> un-hide or forget: CONFIRM, deleting loses the
///     credential.
///   * DHCP hostname -> a per-connection rewrite: confirm-first, it is a saved
///     profile edit with no default to fall back on.
///   * LE privacy off -> edit `main.conf` and restart bluetoothd: CONFIRM, it
///     restarts a service and only partly helps.
fn one_click_safe(surface: Surface) -> bool {
    match surface {
        Surface::WifiMac | Surface::SavedMacPolicy | Surface::BluetoothDiscoverable => true,
        Surface::HiddenNetwork | Surface::DhcpHostname | Surface::BlerPrivacy => false,
    }
}

/// Compose the readout from what was measured.
///
/// Ordered worst-first - exposed, then unreadable, then protected - because the
/// list is read top-down and the thing somebody can act on belongs where they
/// look first. Within a group the surfaces keep their declaration order, so two
/// runs of the same machine produce the same list.
#[must_use]
pub fn compose(readings: &[(Surface, Posture)]) -> Vec<Line> {
    let rank = |p: Posture| match p {
        Posture::Exposed => 0,
        Posture::Unknown => 1,
        Posture::Protected => 2,
    };
    let mut lines: Vec<Line> = readings
        .iter()
        .map(|&(surface, posture)| Line {
            surface,
            posture,
            fixable: posture == Posture::Exposed && one_click_safe(surface),
        })
        .collect();
    lines.sort_by_key(|l| (rank(l.posture), l.surface));
    lines
}

/// Whether the readout as a whole warrants the shell's privacy badge.
///
/// Exposure only, and only where something is actually exposed: `Unknown` does
/// NOT light it. The badge is mounted while a warn condition holds and collapses
/// to nothing when clear (`privacy-sentinel-plan.md` §7), so lighting it for a
/// reading that failed would make it a permanent lamp on any machine with one
/// unreadable surface - which is the nagging the section rules out.
#[must_use]
pub fn warrants_badge(lines: &[Line]) -> bool {
    lines.iter().any(|l| l.posture == Posture::Exposed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all(p: Posture) -> Vec<(Surface, Posture)> {
        vec![
            (Surface::WifiMac, p),
            (Surface::SavedMacPolicy, p),
            (Surface::HiddenNetwork, p),
            (Surface::DhcpHostname, p),
            (Surface::BluetoothDiscoverable, p),
            (Surface::BlerPrivacy, p),
        ]
    }

    #[test]
    fn an_unreadable_surface_is_a_line_rather_than_a_silence() {
        let lines = compose(&all(Posture::Unknown));
        assert_eq!(lines.len(), 6);
        assert!(lines.iter().all(|l| l.posture == Posture::Unknown));
        // and none of them offers to fix a state nobody established
        assert!(lines.iter().all(|l| !l.fixable));
    }

    #[test]
    fn what_somebody_can_act_on_comes_first() {
        let lines = compose(&[
            (Surface::WifiMac, Posture::Protected),
            (Surface::BlerPrivacy, Posture::Unknown),
            (Surface::BluetoothDiscoverable, Posture::Exposed),
        ]);
        assert_eq!(lines[0].surface, Surface::BluetoothDiscoverable);
        assert_eq!(lines[1].surface, Surface::BlerPrivacy);
        assert_eq!(lines[2].surface, Surface::WifiMac);
    }

    #[test]
    fn only_the_remediations_the_plan_calls_safe_get_a_button() {
        let lines = compose(&all(Posture::Exposed));
        let fixable: Vec<Surface> = lines.iter().filter(|l| l.fixable).map(|l| l.surface).collect();
        assert_eq!(
            fixable,
            vec![
                Surface::WifiMac,
                Surface::SavedMacPolicy,
                Surface::BluetoothDiscoverable
            ]
        );
    }

    #[test]
    fn a_protected_surface_offers_no_fix() {
        assert!(compose(&all(Posture::Protected)).iter().all(|l| !l.fixable));
    }

    #[test]
    fn the_badge_lights_for_exposure_and_not_for_a_failed_reading() {
        assert!(warrants_badge(&compose(&[(
            Surface::BluetoothDiscoverable,
            Posture::Exposed
        )])));
        assert!(!warrants_badge(&compose(&all(Posture::Unknown))));
        assert!(!warrants_badge(&compose(&all(Posture::Protected))));
    }

    #[test]
    fn two_runs_of_one_machine_produce_the_same_list() {
        let readings = all(Posture::Exposed);
        assert_eq!(compose(&readings), compose(&readings));
    }
}

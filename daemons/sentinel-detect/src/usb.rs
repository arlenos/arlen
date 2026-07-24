//! SEN-2 USB BadUSB / HID-injection defense: the interface-class heuristic that
//! decides how much consent friction a newly-inserted, blocked USB device warrants.
//!
//! USBGuard runs in `InsertedDevicePolicy=block`, so a new device emits
//! `DevicePresenceChanged(Present, blocked)`. The daemon reads the device's USB
//! interface classes and routes an Allow/Deny decision to the ONE consent broker at
//! a friction rung chosen here. This is PURE (interface classes in, a rung out) so
//! the escalation rule is one auditable, testable place. USBGuard's own config keeps
//! the static `reject with-interface { 08:*:* 03:*:* }` backstop that fires even if
//! this daemon is down; this module is the brain that gives the broker the RIGHT
//! friction, which a blanket reject rule cannot.

/// USB HID interface class (keyboards, mice - the BadUSB injection surface).
pub const USB_CLASS_HID: u8 = 0x03;
/// USB Mass-Storage interface class (disks).
pub const USB_CLASS_MASS_STORAGE: u8 = 0x08;

/// The consent-broker friction rung a blocked USB device is routed to. These are two
/// rungs of the shared 6-rung consent ladder (`consent-grant-surface-plan.md`): a
/// normal new device asks at [`Rung::Tier2`]; the unambiguous BadUSB signature asks
/// at the higher-friction [`Rung::Tier3`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rung {
    /// Standard new-device consent (the default for any blocked insertion).
    Tier2,
    /// Elevated friction for the disk-plus-HID BadUSB signature.
    Tier3,
}

/// Whether a device presents BOTH a mass-storage and a HID interface - a disk that
/// is also a keyboard, which is almost never a legitimate single device and is the
/// classic BadUSB payload (a flash drive that also injects keystrokes).
pub fn is_disk_and_hid(interface_classes: &[u8]) -> bool {
    interface_classes.contains(&USB_CLASS_MASS_STORAGE)
        && interface_classes.contains(&USB_CLASS_HID)
}

/// The friction rung for a blocked USB device given its interface classes: the
/// disk-plus-HID BadUSB signature escalates to [`Rung::Tier3`]; every other blocked
/// device asks at the standard [`Rung::Tier2`]. (A device with no interface classes
/// - not yet enumerated - still asks at Tier2; it is never silently allowed.)
pub fn usb_consent_rung(interface_classes: &[u8]) -> Rung {
    if is_disk_and_hid(interface_classes) {
        Rung::Tier3
    } else {
        Rung::Tier2
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_disk_plus_keyboard_is_the_badusb_signature() {
        assert!(is_disk_and_hid(&[USB_CLASS_MASS_STORAGE, USB_CLASS_HID]));
        assert!(is_disk_and_hid(&[0x03, 0x08, 0x0E])); // order-independent, extra classes ok
        assert_eq!(usb_consent_rung(&[0x08, 0x03]), Rung::Tier3);
    }

    #[test]
    fn a_plain_new_device_asks_at_the_standard_rung() {
        // A lone keyboard, a lone flash drive, a webcam - normal insertions.
        assert_eq!(usb_consent_rung(&[USB_CLASS_HID]), Rung::Tier2);
        assert_eq!(usb_consent_rung(&[USB_CLASS_MASS_STORAGE]), Rung::Tier2);
        assert_eq!(usb_consent_rung(&[0x0E]), Rung::Tier2); // video (webcam)
        assert!(!is_disk_and_hid(&[USB_CLASS_HID]));
        assert!(!is_disk_and_hid(&[USB_CLASS_MASS_STORAGE]));
    }

    #[test]
    fn an_unenumerated_device_still_asks_never_silently_allows() {
        assert_eq!(usb_consent_rung(&[]), Rung::Tier2);
    }
}

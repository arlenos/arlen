//! SEN-3 recording-device confidence match-set: is a nearby BLE advertiser a
//! camera/recording wearable (smart glasses), and how sure are we?
//!
//! A Bluetooth SIG company id is per-VENDOR, not per-device, so a vendor id alone
//! cannot mean "glasses" (Meta's id is shared with Quest/Portal). The match-set is a
//! CONFIDENCE model: a broad vendor id alone is LOW, a narrow vendor id is MEDIUM, a
//! vendor id plus a class-specific name substring or a class-specific service UUID is
//! HIGH. This module is the pure matcher over a device-class table; the daemon reads
//! the advert (company id from the Manufacturer-Data first two bytes, little-endian;
//! the local name; the service UUIDs) and the versioned `device-classes.toml`, and
//! calls [`classify_device`]. The table is data (`bundled_device_classes` is the
//! shipped default), so a new class is a data edit, not a code change.

/// How sure a match is. Ordered so the daemon can keep the highest-confidence live
/// class (`None` < `Low` < `Medium` < `High`); the indicator never collapses `Low`
/// and `High` into the same signal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Confidence {
    /// No signal (e.g. a shared SoC id alone).
    None,
    /// A broad vendor id alone (surface only as a vague "a wearable", user-raisable).
    Low,
    /// A narrow-surface vendor id with no corroboration.
    Medium,
    /// A vendor id plus a class-specific name, or a class-specific service UUID.
    High,
}

/// One recording-device class in the match-set.
#[derive(Debug, Clone)]
pub struct DeviceClass {
    /// The concrete class shown when matched, e.g. "Meta smart glasses".
    pub label: String,
    /// The vendor's SIG company id, when the class is keyed on one.
    pub company_id: Option<u16>,
    /// A class-specific service UUID (lowercased) that alone confirms the class.
    pub service_uuid: Option<String>,
    /// Class-specific local-name substrings (lowercased) that corroborate the id.
    pub name_substrings: Vec<String>,
    /// Confidence for the company id alone (no name/UUID corroboration).
    pub id_alone: Confidence,
    /// Confidence for the id plus a name substring, or a service-UUID/name-only class.
    pub id_plus_name: Confidence,
    /// A disabled class never matches (present-but-off pending on-metal confirmation).
    pub enabled: bool,
}

/// A positive recording-device match: the concrete class and how sure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceMatch {
    /// The matched class label.
    pub label: String,
    /// The confidence of the match.
    pub confidence: Confidence,
}

/// Whether any of `name_substrings` occurs (case-insensitively) in `name`.
fn name_corroborates(name: Option<&str>, substrings: &[String]) -> bool {
    match name {
        Some(n) => {
            let lower = n.to_ascii_lowercase();
            substrings.iter().any(|s| lower.contains(s))
        }
        None => false,
    }
}

/// The confidence a single class assigns to one advertisement, or `None` when the
/// class does not match it at all.
fn class_confidence(
    class: &DeviceClass,
    company_id: Option<u16>,
    name: Option<&str>,
    service_uuids: &[String],
) -> Confidence {
    if !class.enabled {
        return Confidence::None;
    }
    // A class-specific service UUID confirms the class outright.
    if let Some(uuid) = &class.service_uuid {
        if service_uuids.iter().any(|u| u.eq_ignore_ascii_case(uuid)) {
            return class.id_plus_name;
        }
    }
    match class.company_id {
        Some(id) if company_id == Some(id) => {
            if name_corroborates(name, &class.name_substrings) {
                class.id_plus_name
            } else {
                class.id_alone
            }
        }
        // A UUID-less/id-less class matches only by its name substrings (e.g. a
        // platform keyed on its product name alone).
        None if name_corroborates(name, &class.name_substrings) => class.id_plus_name,
        _ => Confidence::None,
    }
}

/// Classify one BLE advertisement against the match-set, returning the
/// highest-confidence class match (or `None` when nothing rises above `None` - a
/// shared SoC id alone, or an unknown vendor). `company_id` is the parsed
/// Manufacturer-Data company id; `name` the local name; `service_uuids` the
/// advertised UUIDs (compared case-insensitively).
pub fn classify_device(
    company_id: Option<u16>,
    name: Option<&str>,
    service_uuids: &[String],
    classes: &[DeviceClass],
) -> Option<DeviceMatch> {
    classes
        .iter()
        .filter_map(|class| {
            let confidence = class_confidence(class, company_id, name, service_uuids);
            if confidence == Confidence::None {
                None
            } else {
                Some(DeviceMatch {
                    label: class.label.clone(),
                    confidence,
                })
            }
        })
        .max_by(|a, b| a.confidence.cmp(&b.confidence))
}

/// The shipped default match-set (the seed of the versioned `device-classes.toml`),
/// SIG-verified company ids. Luxottica `0x0D53` is present but DISABLED pending an
/// on-metal capture confirming a shipping Ray-Ban Meta advertises under it.
pub fn bundled_device_classes() -> Vec<DeviceClass> {
    let meta_names = || vec!["ray-ban".to_string(), "stories".to_string()];
    vec![
        DeviceClass {
            label: "Meta smart glasses".into(),
            company_id: Some(0x01AB),
            service_uuid: None,
            name_substrings: meta_names(),
            id_alone: Confidence::Low, // shared with Quest/Portal
            id_plus_name: Confidence::High,
            enabled: true,
        },
        DeviceClass {
            label: "Meta smart glasses".into(),
            company_id: Some(0x058E),
            service_uuid: None,
            name_substrings: meta_names(),
            id_alone: Confidence::Low,
            id_plus_name: Confidence::High,
            enabled: true,
        },
        DeviceClass {
            label: "Snap Spectacles".into(),
            company_id: Some(0x03C2),
            service_uuid: None,
            name_substrings: vec!["spectacles".into()],
            id_alone: Confidence::Medium, // narrow-surface vendor
            id_plus_name: Confidence::High,
            enabled: true,
        },
        DeviceClass {
            label: "audio SoC (Zhuhai Jieli)".into(),
            company_id: Some(0x05D6),
            service_uuid: None,
            name_substrings: vec![],
            id_alone: Confidence::None, // in countless earbuds/toys - never alone
            id_plus_name: Confidence::None,
            enabled: true,
        },
        DeviceClass {
            label: "HeyCyan camera glasses".into(),
            company_id: None,
            service_uuid: Some("7905fff0-b5ce-4e99-a40f-4b1e122d00d0".into()),
            name_substrings: vec!["heycyan".into()],
            id_alone: Confidence::None,
            id_plus_name: Confidence::High,
            enabled: true,
        },
        DeviceClass {
            label: "Ray-Ban Meta (Luxottica)".into(),
            company_id: Some(0x0D53),
            service_uuid: None,
            name_substrings: vec!["ray-ban".into()],
            id_alone: Confidence::Low,
            id_plus_name: Confidence::High,
            enabled: false, // disabled pending an on-metal sniff
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn classes() -> Vec<DeviceClass> {
        bundled_device_classes()
    }

    #[test]
    fn meta_id_alone_is_low_but_with_a_name_is_high() {
        let low = classify_device(Some(0x01AB), None, &[], &classes()).expect("meta");
        assert_eq!(low.confidence, Confidence::Low);
        assert_eq!(low.label, "Meta smart glasses");
        let high = classify_device(Some(0x01AB), Some("Ray-Ban Stories"), &[], &classes())
            .expect("meta+name");
        assert_eq!(high.confidence, Confidence::High);
    }

    #[test]
    fn snap_id_is_medium_and_upgrades_on_name() {
        let m = classify_device(Some(0x03C2), None, &[], &classes()).expect("snap");
        assert_eq!(m.confidence, Confidence::Medium);
        let h = classify_device(Some(0x03C2), Some("Spectacles 3"), &[], &classes()).expect("snap+name");
        assert_eq!(h.confidence, Confidence::High);
    }

    #[test]
    fn jieli_alone_is_never_a_signal() {
        assert!(classify_device(Some(0x05D6), None, &[], &classes()).is_none());
        assert!(classify_device(Some(0x05D6), Some("some earbuds"), &[], &classes()).is_none());
    }

    #[test]
    fn heycyan_matches_on_service_uuid_or_name() {
        let by_uuid = classify_device(
            None,
            None,
            &["7905FFF0-B5CE-4E99-A40F-4B1E122D00D0".into()],
            &classes(),
        )
        .expect("heycyan uuid");
        assert_eq!(by_uuid.confidence, Confidence::High);
        let by_name = classify_device(None, Some("HeyCyan Glass"), &[], &classes()).expect("heycyan name");
        assert_eq!(by_name.confidence, Confidence::High);
    }

    #[test]
    fn a_disabled_class_never_matches() {
        // Luxottica 0x0D53 is present but disabled -> no match even with its name.
        assert!(classify_device(Some(0x0D53), Some("Ray-Ban Meta"), &[], &classes()).is_none());
    }

    #[test]
    fn an_unknown_vendor_is_no_match() {
        assert!(classify_device(Some(0x0006), Some("Some Phone"), &[], &classes()).is_none());
    }

    #[test]
    fn the_highest_confidence_class_wins() {
        // An advert carrying both a low Meta id and a high HeyCyan UUID surfaces High.
        let m = classify_device(
            Some(0x01AB),
            None,
            &["7905fff0-b5ce-4e99-a40f-4b1e122d00d0".into()],
            &classes(),
        )
        .expect("match");
        assert_eq!(m.confidence, Confidence::High);
        assert_eq!(m.label, "HeyCyan camera glasses");
    }
}

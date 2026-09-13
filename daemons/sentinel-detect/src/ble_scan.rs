//! What to ask the controller to watch for, and how hard to listen
//! (`privacy-sentinel-plan.md` §4.3, §4.4).
//!
//! Two detectors want the same radio: SEN-4's finder-tags and the nearby
//! recording-device indicator. Both are answered by BlueZ's
//! `AdvertisementMonitor1` rather than by polling `StartDiscovery`, because on a
//! controller with MSFT or APCF offload the FILTERING RUNS ON THE CONTROLLER and
//! the host wakes only on a match. That is the difference between a laptop that
//! can watch all day and one whose battery says it cannot.
//!
//! This module is the part of that with no radio in it: which patterns to
//! register, and what a sensitivity setting means in dBm. Registering them, and
//! the `MonitorEvent` stream, is the daemon's.
//!
//! **A pattern is deliberately coarse.** It matches a company id or a service
//! UUID - the fact that a tag of some brand is nearby - and never a payload that
//! identifies one. The narrowing is [`crate::tracker`]'s, host-side, after the
//! advert arrives. Asking the controller for less than it could match is the
//! right direction here: a filter is a thing the hardware remembers.
//!
//! **Proximity is a sensitivity model, never metres.** RSSI cannot be honestly
//! converted to distance - the path-loss exponent swings between 2 and 4, TxPower
//! varies per device and firmware, and a body between you and the tag dominates
//! both. So the control has three stops and they are named for intent rather than
//! for range, and the high threshold sits above the low one with timeouts so a
//! device at the boundary does not flap.

/// Where in the advertisement data a pattern starts matching.
pub type StartPosition = u8;

/// An assigned advertising data type.
pub mod ad_type {
    /// 16-bit service UUIDs, incomplete list.
    pub const SERVICE_UUID16_INCOMPLETE: u8 = 0x02;
    /// 16-bit service UUIDs, complete list.
    pub const SERVICE_UUID16_COMPLETE: u8 = 0x03;
    /// Service data, 16-bit UUID.
    pub const SERVICE_DATA_UUID16: u8 = 0x16;
    /// Manufacturer-specific data, which begins with the company id.
    pub const MANUFACTURER_DATA: u8 = 0xFF;
}

/// One `or_patterns` entry: match `content` at `start_position` within the field
/// of type `ad_type`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pattern {
    /// Offset within the data field.
    pub start_position: StartPosition,
    /// The advertising data type to look in.
    pub ad_type: u8,
    /// The bytes to match.
    pub content: Vec<u8>,
}

impl Pattern {
    /// Match a company id at the start of manufacturer data. Bluetooth company
    /// ids are little-endian on the wire, which is the whole reason this is a
    /// constructor rather than two lines at each call site.
    pub fn company(id: u16) -> Self {
        Self {
            start_position: 0,
            ad_type: ad_type::MANUFACTURER_DATA,
            content: id.to_le_bytes().to_vec(),
        }
    }

    /// Match a 16-bit service UUID at the start of a service-data field, also
    /// little-endian.
    pub fn service_data(uuid16: u16) -> Self {
        Self {
            start_position: 0,
            ad_type: ad_type::SERVICE_DATA_UUID16,
            content: uuid16.to_le_bytes().to_vec(),
        }
    }
}

/// Every pattern the finder-tag detector needs, in a stable order.
///
/// One per brand [`crate::tracker`] can classify, and no more: a pattern that
/// matches something nothing downstream reads is a wake-up for nothing, which on
/// this path costs battery rather than correctness.
pub fn finder_tag_patterns() -> Vec<Pattern> {
    use crate::tracker::{APPLE_COMPANY_ID, DULT_UUID, GOOGLE_UUID, SAMSUNG_UUID, TILE_UUID};
    vec![
        Pattern::company(APPLE_COMPANY_ID),
        Pattern::service_data(SAMSUNG_UUID),
        Pattern::service_data(TILE_UUID),
        Pattern::service_data(GOOGLE_UUID),
        Pattern::service_data(DULT_UUID),
    ]
}

/// How hard to listen. Named for what the person wants, not for a distance
/// nobody can honestly promise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Sensitivity {
    /// Everything the radio can hear. More false company, fewer misses.
    Wide,
    /// The default.
    #[default]
    Balanced,
    /// Only what is close enough to be in the room with you.
    CloseOnly,
}

/// How many dB the release threshold sits above the catch threshold.
///
/// Not a taste number: a device sitting exactly at the boundary crosses it every
/// few seconds as a person shifts in a chair, and without a gap that is an
/// arrive/leave pair every time. The gap plus the timeouts below is what makes
/// "it is here" a statement rather than a flicker.
pub const HYSTERESIS_DB: i16 = 6;

/// Seconds a device must stay below the low threshold before it counts as gone.
pub const LOW_TIMEOUT_SECS: u16 = 30;
/// Seconds a device must stay above the high threshold before it counts as here.
pub const HIGH_TIMEOUT_SECS: u16 = 5;
/// Seconds the controller averages RSSI over.
pub const SAMPLING_PERIOD_SECS: u16 = 5;

/// The thresholds a monitor is registered with, in dBm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Thresholds {
    /// Below this, and for [`LOW_TIMEOUT_SECS`], the device is gone.
    pub rssi_low: i16,
    /// Above this, and for [`HIGH_TIMEOUT_SECS`], the device is here.
    pub rssi_high: i16,
    /// How long below `rssi_low` counts as gone.
    pub low_timeout_secs: u16,
    /// How long above `rssi_high` counts as here.
    pub high_timeout_secs: u16,
    /// The controller's averaging window.
    pub sampling_period_secs: u16,
}

/// The thresholds for one sensitivity stop.
pub fn thresholds(sensitivity: Sensitivity) -> Thresholds {
    let rssi_low = match sensitivity {
        Sensitivity::Wide => -90,
        Sensitivity::Balanced => -75,
        Sensitivity::CloseOnly => -60,
    };
    Thresholds {
        rssi_low,
        rssi_high: rssi_low + HYSTERESIS_DB,
        low_timeout_secs: LOW_TIMEOUT_SECS,
        high_timeout_secs: HIGH_TIMEOUT_SECS,
        sampling_period_secs: SAMPLING_PERIOD_SECS,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A company id goes on the wire little-endian, and Apple's 0x004C read the
    /// wrong way round is 0x4C00, which belongs to nobody. The constructor is
    /// here so that mistake is made once or not at all.
    #[test]
    fn a_company_pattern_is_little_endian() {
        let p = Pattern::company(0x004C);
        assert_eq!(p.content, vec![0x4C, 0x00]);
        assert_eq!(p.ad_type, ad_type::MANUFACTURER_DATA);
        assert_eq!(p.start_position, 0);
    }

    #[test]
    fn a_service_data_pattern_is_little_endian_too() {
        let p = Pattern::service_data(0xFD5A);
        assert_eq!(p.content, vec![0x5A, 0xFD]);
        assert_eq!(p.ad_type, ad_type::SERVICE_DATA_UUID16);
    }

    /// One per brand the classifier can name, and nothing else: a pattern with no
    /// reader downstream is a wake-up for nothing.
    #[test]
    fn there_is_one_pattern_per_classifiable_brand() {
        let patterns = finder_tag_patterns();
        assert_eq!(patterns.len(), 5);
        assert!(patterns.contains(&Pattern::company(crate::tracker::APPLE_COMPANY_ID)));
        assert!(patterns.contains(&Pattern::service_data(crate::tracker::DULT_UUID)));
        // Apple is the only brand on manufacturer data; the rest are service data.
        assert_eq!(
            patterns.iter().filter(|p| p.ad_type == ad_type::MANUFACTURER_DATA).count(),
            1
        );
    }

    #[test]
    fn the_release_threshold_sits_above_the_catch_threshold() {
        for s in [Sensitivity::Wide, Sensitivity::Balanced, Sensitivity::CloseOnly] {
            let t = thresholds(s);
            assert!(t.rssi_high > t.rssi_low, "{s:?} has no hysteresis");
            assert_eq!(t.rssi_high - t.rssi_low, HYSTERESIS_DB);
        }
    }

    /// Wide hears more than balanced, which hears more than close-only. The
    /// ordering is the whole meaning of the control.
    #[test]
    fn the_stops_are_ordered_the_way_they_are_named() {
        let wide = thresholds(Sensitivity::Wide).rssi_low;
        let balanced = thresholds(Sensitivity::Balanced).rssi_low;
        let close = thresholds(Sensitivity::CloseOnly).rssi_low;
        assert!(wide < balanced && balanced < close);
    }

    #[test]
    fn balanced_is_the_default() {
        assert_eq!(Sensitivity::default(), Sensitivity::Balanced);
        assert_eq!(thresholds(Sensitivity::default()).rssi_low, -75);
    }

    /// The gone-timeout is longer than the here-timeout on purpose: a tag is
    /// reported present quickly and dropped slowly, so a moment of shadowing does
    /// not read as the tag leaving.
    #[test]
    fn a_device_is_caught_faster_than_it_is_released() {
        let t = thresholds(Sensitivity::Balanced);
        assert!(t.low_timeout_secs > t.high_timeout_secs);
    }
}

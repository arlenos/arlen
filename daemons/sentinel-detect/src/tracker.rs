//! Classify a BLE advertisement as a known finder-tag (AirTag, Samsung SmartTag,
//! Tile, Google Find My Device, or the forward-looking DULT standard).
//!
//! SEN-4 (`tracker-sentinel-plan.md`): the unwanted-tracker sentinel watches for a
//! SEPARATED finder-tag that has been near the user across multiple locations (the
//! stalking signal). This module is the front of that detector: it turns one BLE
//! advertisement's manufacturer-data or service-data into a brand match plus the
//! separation state the advert reveals. It is PURE (bytes in, verdict out) so the
//! per-vendor formats are tested against fixtures without a Bluetooth radio; the
//! scanning, the movement/persistence model, the home-anchor and the notify decision
//! all live in the daemon on top of this.
//!
//! The formats follow the de-facto per-vendor advertisement layouts (research-
//! grounded against AirGuard). Near-owner adverts are dropped before they ever reach
//! the sighting store, so distinguishing separated from near-owner here is the
//! privacy-load-bearing step, not a nicety.

/// A known finder-tag ecosystem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackerBrand {
    /// Apple Find My (AirTag, 3rd-party Find My accessories like Chipolo ONE Spot).
    AppleFindMy,
    /// Samsung SmartTag / SmartTag+ / SmartTag2.
    SamsungSmartTag,
    /// Tile.
    Tile,
    /// Google Find My Device network (Chipolo, Pebblebee, Moto, eufy).
    GoogleFmdn,
    /// The DULT accessory-protocol standard (forward-looking convergence target).
    Dult,
}

/// What an advertisement reveals about whether the tag is with its owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Separation {
    /// The advert indicates the tag is SEPARATED from its owner: the relevant,
    /// sighting-worthy state (a nearby owner does not make a stalking signal).
    Separated,
    /// The advert indicates the owner is nearby: dropped before the sighting store.
    NearOwner,
    /// The brand matched but this single advert does not determine separation (e.g.
    /// Samsung's exact state code / Tile's owner-presence is inferred from
    /// persistence): the daemon's movement model decides.
    Unknown,
}

/// A classified advertisement: which ecosystem, and what it says about separation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrackerMatch {
    /// The finder-tag ecosystem this advert belongs to.
    pub brand: TrackerBrand,
    /// The separation state the advert reveals.
    pub separation: Separation,
}

/// Apple's company id in a BLE Manufacturer-Data field.
const APPLE_COMPANY_ID: u16 = 0x004C;
/// Apple Find My message type byte (offline finding).
const APPLE_FINDMY_TYPE: u8 = 0x12;
/// The Apple Find My offline (separated) frame's length byte: a 25-byte payload
/// carrying the public key. A near-owner AirTag does not advertise this frame.
const APPLE_OFFLINE_LEN: u8 = 0x19;

/// Samsung SmartTag Service-Data UUID.
const SAMSUNG_UUID: u16 = 0xFD5A;
/// Tile Service-Data UUID.
const TILE_UUID: u16 = 0xFEED;
/// Google Find My Device Network Service-Data UUID.
const GOOGLE_UUID: u16 = 0xFEAA;
/// DULT standard Service-Data UUID.
const DULT_UUID: u16 = 0xFCB2;

/// Classify a Manufacturer-Data field (a company id plus its payload bytes) as a
/// finder-tag. Only Apple advertises its finder-tag over Manufacturer-Data; the
/// others use Service-Data ([`classify_service_data`]). `None` when the company /
/// layout is not a known tag.
pub fn classify_manufacturer_data(company_id: u16, data: &[u8]) -> Option<TrackerMatch> {
    if company_id != APPLE_COMPANY_ID {
        return None;
    }
    // Apple Find My offline frame: [type=0x12, length, status, ...public key...].
    if data.first().copied() != Some(APPLE_FINDMY_TYPE) {
        return None;
    }
    // A received offline-finding frame IS a separated advert (a near-owner AirTag
    // does not advertise at all); the 0x19 length byte marks the full offline frame.
    let separation = if data.get(1).copied() == Some(APPLE_OFFLINE_LEN) {
        Separation::Separated
    } else {
        Separation::Unknown
    };
    Some(TrackerMatch {
        brand: TrackerBrand::AppleFindMy,
        separation,
    })
}

/// Classify a Service-Data field (a 16-bit service UUID plus its payload bytes) as a
/// finder-tag. Covers Samsung, Tile, Google Find My Device and the DULT standard.
/// `None` when the UUID / layout is not a known tag.
pub fn classify_service_data(uuid16: u16, data: &[u8]) -> Option<TrackerMatch> {
    match uuid16 {
        SAMSUNG_UUID => {
            // Samsung SmartTag: byte0 masked 0xF8 equals 0x10. The connection-state
            // (bits 0-2) distinguishes separated/overmature, but the exact code is
            // not pinned here, so the daemon's model decides: Unknown.
            let b0 = *data.first()?;
            if b0 & 0xF8 == 0x10 {
                Some(TrackerMatch {
                    brand: TrackerBrand::SamsungSmartTag,
                    separation: Separation::Unknown,
                })
            } else {
                None
            }
        }
        TILE_UUID => {
            // Tile: the frame opens with 0x02 0x00. Owner-presence is signalled only
            // weakly, so separation is inferred from persistence downstream: Unknown.
            if data.len() >= 2 && data[0] == 0x02 && data[1] == 0x00 {
                Some(TrackerMatch {
                    brand: TrackerBrand::Tile,
                    separation: Separation::Unknown,
                })
            } else {
                None
            }
        }
        GOOGLE_UUID => {
            // Google FMDN: byte0 bit0 == 1 is OVERMATURE_OFFLINE (>4 h since the
            // owner was seen) = the separated state; 0 is premature (not yet a signal).
            let b0 = *data.first()?;
            let separation = if b0 & 0x01 == 1 {
                Separation::Separated
            } else {
                Separation::Unknown
            };
            Some(TrackerMatch {
                brand: TrackerBrand::GoogleFmdn,
                separation,
            })
        }
        DULT_UUID => {
            // DULT: the near-owner bit is the LSB of byte 14 (1 = owner nearby).
            let near_owner_bit = data.get(14)?;
            let separation = if near_owner_bit & 0x01 == 0 {
                Separation::Separated
            } else {
                Separation::NearOwner
            };
            Some(TrackerMatch {
                brand: TrackerBrand::Dult,
                separation,
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apple_offline_frame_is_a_separated_airtag() {
        // [type, length=0x19, status, ...25 bytes total payload...]
        let mut data = vec![APPLE_FINDMY_TYPE, APPLE_OFFLINE_LEN, 0x00];
        data.extend(std::iter::repeat_n(0xABu8, 25));
        let m = classify_manufacturer_data(0x004C, &data).expect("apple match");
        assert_eq!(m.brand, TrackerBrand::AppleFindMy);
        assert_eq!(m.separation, Separation::Separated);
    }

    #[test]
    fn a_short_apple_frame_matches_the_brand_but_not_separation() {
        let data = [APPLE_FINDMY_TYPE, 0x02, 0x00];
        let m = classify_manufacturer_data(0x004C, &data).expect("apple match");
        assert_eq!(m.separation, Separation::Unknown);
    }

    #[test]
    fn a_non_apple_company_or_wrong_type_is_not_a_tag() {
        assert!(classify_manufacturer_data(0x0006, &[0x12, 0x19]).is_none());
        assert!(classify_manufacturer_data(0x004C, &[0x10, 0x19]).is_none());
        assert!(classify_manufacturer_data(0x004C, &[]).is_none());
    }

    #[test]
    fn samsung_smarttag_matches_the_masked_first_byte() {
        // byte0 & 0xF8 == 0x10 (e.g. 0x14).
        let m = classify_service_data(0xFD5A, &[0x14, 0, 0, 0]).expect("samsung");
        assert_eq!(m.brand, TrackerBrand::SamsungSmartTag);
        assert_eq!(m.separation, Separation::Unknown);
        // A first byte outside the mask is not a SmartTag.
        assert!(classify_service_data(0xFD5A, &[0x20]).is_none());
        assert!(classify_service_data(0xFD5A, &[]).is_none());
    }

    #[test]
    fn tile_matches_its_leading_bytes() {
        let m = classify_service_data(0xFEED, &[0x02, 0x00, 0x99]).expect("tile");
        assert_eq!(m.brand, TrackerBrand::Tile);
        assert!(classify_service_data(0xFEED, &[0x02, 0x01]).is_none());
    }

    #[test]
    fn google_fmdn_overmature_bit_is_separated() {
        let sep = classify_service_data(0xFEAA, &[0x01]).expect("google");
        assert_eq!(sep.brand, TrackerBrand::GoogleFmdn);
        assert_eq!(sep.separation, Separation::Separated);
        let prem = classify_service_data(0xFEAA, &[0x00]).expect("google");
        assert_eq!(prem.separation, Separation::Unknown);
    }

    #[test]
    fn dult_near_owner_bit_distinguishes_separation() {
        let mut sep = vec![0u8; 15];
        sep[14] = 0x00; // LSB 0 -> owner NOT nearby -> separated
        assert_eq!(
            classify_service_data(0xFCB2, &sep).unwrap().separation,
            Separation::Separated
        );
        let mut near = vec![0u8; 15];
        near[14] = 0x01; // LSB 1 -> owner nearby
        assert_eq!(
            classify_service_data(0xFCB2, &near).unwrap().separation,
            Separation::NearOwner
        );
        // Too short to carry the near-owner bit -> not classifiable.
        assert!(classify_service_data(0xFCB2, &[0u8; 10]).is_none());
    }

    #[test]
    fn an_unknown_service_uuid_is_not_a_tag() {
        assert!(classify_service_data(0x180F, &[0x01, 0x02]).is_none());
    }
}

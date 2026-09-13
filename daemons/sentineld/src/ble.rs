//! The shared BLE watch: register the monitor, turn what it finds into adverts
//! (`privacy-sentinel-plan.md` §4.3, `tracker-sentinel-plan.md` §6 job 3).
//!
//! [`arlen_sentinel_detect::ble_scan`] says WHAT to watch for and how hard to
//! listen; this registers that with BlueZ and turns a match into the
//! [`Advert`](crate::tracker_run::Advert) the detector chain reads. It is the seam
//! both undriven detectors were waiting on.
//!
//! **The controller does the filtering, when it can.** `AdvertisementMonitor1`
//! with `or_patterns` rather than `StartDiscovery` polling: measured on this
//! machine, `SupportedFeatures` reports `controller-patterns`, so the host sleeps
//! until something matches. That is what makes an always-on watch honest on a
//! laptop.
//!
//! **A match is not a finder-tag**, and the numbers say so rather loudly. Ten
//! seconds of the Apple company pattern here matched four devices, all of them
//! phones and earbuds advertising proximity-pairing with 8- and 20-byte payloads.
//! The offline-finding frame the classifier wants is 25 bytes behind a type byte
//! that none of them carried. So the controller's coarse match is the first of two
//! gates and [`arlen_sentinel_detect::tracker`] is the second, host-side - which is
//! the split §4.3 asks for and the reason the pattern is allowed to be broad.
//!
//! **The correlator is the payload, never the address.** The BLE address in a
//! `DeviceFound` is a resolvable private address that rotates on its own schedule,
//! so keying a tag by it would split one tag into many and merge none. What
//! identifies a tag inside its rotation window is the payload identifier, which is
//! what [`advert_from`] carries.

use std::collections::HashMap;

use arlen_sentinel_detect::ble_scan::{finder_tag_patterns, Thresholds};

use crate::tracker_run::Advert;

/// Turn the detector's pattern list into BlueZ's.
pub fn bluer_patterns() -> Vec<bluer::monitor::Pattern> {
    finder_tag_patterns()
        .into_iter()
        .map(|p| bluer::monitor::Pattern {
            data_type: p.ad_type,
            start_position: p.start_position,
            content: p.content,
        })
        .collect()
}

/// The monitor to register for the given listening width.
pub fn monitor_for(thresholds: Thresholds) -> bluer::monitor::Monitor {
    use std::time::Duration;
    bluer::monitor::Monitor {
        monitor_type: bluer::monitor::Type::OrPatterns,
        rssi_low_threshold: Some(thresholds.rssi_low),
        rssi_high_threshold: Some(thresholds.rssi_high),
        rssi_low_timeout: Some(Duration::from_secs(u64::from(thresholds.low_timeout_secs))),
        rssi_high_timeout: Some(Duration::from_secs(u64::from(thresholds.high_timeout_secs))),
        rssi_sampling_period: Some(bluer::monitor::RssiSamplingPeriod::Period(
            Duration::from_secs(u64::from(thresholds.sampling_period_secs)),
        )),
        patterns: Some(bluer_patterns()),
        ..Default::default()
    }
}

/// Build an advert from what BlueZ says about one matched device.
///
/// Pure, so the mapping is provable without a radio - which matters more here
/// than usual, because the shape of these two maps was read off a live adapter
/// rather than from documentation: manufacturer data is keyed by company id,
/// service data by full 128-bit UUID even for the 16-bit assignments.
///
/// `None` when nothing recognisable is carried. A device the controller matched
/// but that offers no payload is not an error and not a sighting; it is a device
/// whose advert said less than the filter promised.
pub fn advert_from(
    manufacturer: Option<&HashMap<u16, Vec<u8>>>,
    service: Option<&HashMap<uuid::Uuid, Vec<u8>>>,
) -> Option<Advert> {
    use arlen_sentinel_detect::tracker::APPLE_COMPANY_ID;

    // Apple first, because it is the only brand on manufacturer data and an
    // advert carrying both is Apple's.
    if let Some(payload) = manufacturer.and_then(|m| m.get(&APPLE_COMPANY_ID)) {
        return Some(Advert {
            correlator: payload.clone(),
            manufacturer: Some((APPLE_COMPANY_ID, payload.clone())),
            service: None,
        });
    }
    let service = service?;
    // The 16-bit assignments appear as their full base UUID, so the short id is
    // the third and fourth byte of the 128-bit form.
    for (uuid, payload) in service {
        let bytes = uuid.as_bytes();
        let short = u16::from_be_bytes([bytes[2], bytes[3]]);
        if is_finder_tag_service(short) {
            return Some(Advert {
                correlator: payload.clone(),
                manufacturer: None,
                service: Some((short, payload.clone())),
            });
        }
    }
    None
}

/// Whether a 16-bit service UUID is one of the finder-tag assignments.
fn is_finder_tag_service(short: u16) -> bool {
    use arlen_sentinel_detect::tracker::{DULT_UUID, GOOGLE_UUID, SAMSUNG_UUID, TILE_UUID};
    matches!(short, x if x == SAMSUNG_UUID || x == TILE_UUID || x == GOOGLE_UUID || x == DULT_UUID)
}

/// Watch for finder-tags until `stop` resolves, recording what is seen.
///
/// The whole SEN-4 chain in one place: register the monitor at the width the
/// person chose, read each match into an advert, place it with a coarse fix, keep
/// it in the sealed store, and apply the criteria.
///
/// **It refuses to start rather than watching for nothing.** No adapter, no
/// monitor manager, no store: each is a reason this detector cannot run, and a
/// task that sat in a loop finding nothing would look identical to one watching a
/// quiet room.
///
/// **A scan with no fix records nothing**, which `run_scan` enforces and this
/// relies on: a sighting with an invented place would corrupt the one signal the
/// criteria rest on.
pub async fn watch(
    store: &crate::sightings::SightingStore,
    sensitivity: arlen_sentinel_detect::ble_scan::Sensitivity,
    connection: &zbus::Connection,
    stop: impl std::future::Future<Output = ()>,
) -> Result<(), String> {
    use arlen_sentinel_detect::ble_scan::thresholds;
    use arlen_sentinel_detect::epoch::EpochTracker;
    use futures::StreamExt;

    let session = bluer::Session::new().await.map_err(|e| format!("no bluetooth session: {e}"))?;
    let adapter = session.default_adapter().await.map_err(|e| format!("no adapter: {e}"))?;
    let manager = adapter.monitor().await.map_err(|e| format!("no advertisement monitor: {e}"))?;
    let mut handle = manager
        .register(monitor_for(thresholds(sensitivity)))
        .await
        .map_err(|e| format!("the monitor was refused: {e}"))?;

    // Best-effort: a machine that cannot be placed still watches, and every
    // sighting it takes is simply not recorded (see `run_scan`). Saying so once
    // here beats a log line per advert.
    let feed = crate::location::LocationFeed::open(connection).await.ok();
    if feed.is_none() {
        tracing::info!("no coarse location, so tags are seen but not placed");
    }
    let anchor = store.anchor().unwrap_or_default();
    let epochs = EpochTracker::default();

    tokio::pin!(stop);
    loop {
        let event = tokio::select! {
            () = &mut stop => break,
            event = handle.next() => event,
        };
        let Some(bluer::monitor::MonitorEvent::DeviceFound(found)) = event else {
            // The stream ended, which means the monitor is gone; a watch that
            // silently stopped watching is the thing this must not become.
            break;
        };
        let Ok(device) = adapter.device(found.device) else { continue };
        let manufacturer = device.manufacturer_data().await.ok().flatten();
        let service = device.service_data().await.ok().flatten();
        let Some(advert) = advert_from(manufacturer.as_ref(), service.as_ref()) else { continue };

        let fix = match &feed {
            Some(feed) => feed.fix(connection).await.ok(),
            None => None,
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let mut once = crate::tracker_run::Once::new(advert);
        let outcome = crate::tracker_run::run_scan(&mut once, store, fix, &epochs, now, &anchor);
        for correlator in &outcome.alerts {
            tracing::warn!(tag = %hex(correlator), "a separated tag has followed this machine");
        }
        for correlator in &outcome.review {
            tracing::info!(tag = %hex(correlator), "a separated tag is worth a look");
        }
    }
    Ok(())
}

/// A correlator in the log, where the bytes themselves have no business being.
fn hex(correlator: &[u8]) -> String {
    correlator.iter().take(4).map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uuid16(short: u16) -> uuid::Uuid {
        // The Bluetooth base UUID with the 16-bit assignment in bytes 2 and 3.
        let [hi, lo] = short.to_be_bytes();
        uuid::Uuid::from_bytes([
            0x00, 0x00, hi, lo, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0x80, 0x5F, 0x9B, 0x34,
            0xFB,
        ])
    }

    #[test]
    fn the_registered_patterns_are_the_detectors_patterns() {
        let ours = finder_tag_patterns();
        let theirs = bluer_patterns();
        assert_eq!(ours.len(), theirs.len());
        for (a, b) in ours.iter().zip(theirs.iter()) {
            assert_eq!(a.ad_type, b.data_type);
            assert_eq!(a.content, b.content);
            assert_eq!(a.start_position, b.start_position);
        }
    }

    #[test]
    fn the_monitor_carries_the_thresholds_it_was_given() {
        use arlen_sentinel_detect::ble_scan::{thresholds, Sensitivity};
        let t = thresholds(Sensitivity::CloseOnly);
        let m = monitor_for(t);
        assert_eq!(m.rssi_low_threshold, Some(t.rssi_low));
        assert_eq!(m.rssi_high_threshold, Some(t.rssi_high));
        assert!(matches!(m.monitor_type, bluer::monitor::Type::OrPatterns));
    }

    /// The correlator is the payload, so two adverts from one tag agree and one
    /// tag behind two rotating addresses is still one tag.
    #[test]
    fn an_apple_advert_is_keyed_by_its_payload() {
        let mut md = HashMap::new();
        md.insert(0x004C_u16, vec![0x12, 0x19, 7, 7, 7]);
        let a = advert_from(Some(&md), None).expect("an Apple advert");
        assert_eq!(a.correlator, vec![0x12, 0x19, 7, 7, 7]);
        assert_eq!(a.manufacturer, Some((0x004C, vec![0x12, 0x19, 7, 7, 7])));
        assert!(a.service.is_none());
    }

    /// A 16-bit assignment arrives as its full base UUID, which is the detail a
    /// live adapter taught rather than the spec.
    #[test]
    fn a_service_advert_is_read_out_of_its_full_uuid() {
        let mut sd = HashMap::new();
        sd.insert(uuid16(0xFD5A), vec![1, 2, 3]);
        let a = advert_from(None, Some(&sd)).expect("a Samsung advert");
        assert_eq!(a.service, Some((0xFD5A, vec![1, 2, 3])));
        assert_eq!(a.correlator, vec![1, 2, 3]);
    }

    /// Everything the controller matched but that carries nothing we read is not
    /// a sighting and not an error.
    #[test]
    fn an_unrecognised_advert_is_no_advert() {
        assert!(advert_from(None, None).is_none());
        let mut md = HashMap::new();
        md.insert(0x0075_u16, vec![1, 2, 3]);
        assert!(advert_from(Some(&md), None).is_none());
        let mut sd = HashMap::new();
        sd.insert(uuid16(0x180F), vec![9]);
        assert!(advert_from(None, Some(&sd)).is_none());
    }
}

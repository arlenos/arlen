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

/// The monitor the nearby-recording indicator registers.
pub fn recording_monitor_for(
    thresholds: Thresholds,
    classes: &[arlen_sentinel_detect::recording::DeviceClass],
) -> bluer::monitor::Monitor {
    let mut monitor = monitor_for(thresholds);
    monitor.patterns = Some(
        arlen_sentinel_detect::ble_scan::recording_patterns(classes)
            .into_iter()
            .map(|p| bluer::monitor::Pattern {
                data_type: p.ad_type,
                start_position: p.start_position,
                content: p.content,
            })
            .collect(),
    );
    monitor
}

/// Classify one matched device against the recording match-set.
///
/// Pure, and separate from the finder-tag mapping because the two ask different
/// questions of the same advert: a tag is identified by its payload, a recording
/// device by its vendor plus what it calls itself. The name is what lifts a broad
/// vendor id from "a Meta wearable somewhere" to "Meta smart glasses", and it is
/// the one field the controller cannot filter on.
pub fn recording_match_from(
    manufacturer: Option<&HashMap<u16, Vec<u8>>>,
    service: Option<&HashMap<uuid::Uuid, Vec<u8>>>,
    name: Option<&str>,
    classes: &[arlen_sentinel_detect::recording::DeviceClass],
) -> Option<arlen_sentinel_detect::recording::DeviceMatch> {
    use arlen_sentinel_detect::recording::classify_device;
    let uuids: Vec<String> = service
        .map(|m| m.keys().map(|u| u.to_string()).collect())
        .unwrap_or_default();
    // One advert can carry several company ids; the classifier answers per id and
    // the strongest answer is the one shown, which is what "the indicator shows the
    // highest-confidence live class" means at this end.
    let ids: Vec<u16> = manufacturer.map(|m| m.keys().copied().collect()).unwrap_or_default();
    let mut best: Option<arlen_sentinel_detect::recording::DeviceMatch> = None;
    for id in ids.iter().map(|i| Some(*i)).chain(std::iter::once(None)) {
        if let Some(found) = classify_device(id, name, &uuids, classes) {
            if best.as_ref().is_none_or(|b| found.confidence > b.confidence) {
                best = Some(found);
            }
        }
    }
    best
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
    events: &impl os_sdk::event::EventEmitter,
    live: &crate::live::Live,
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

    // Asked for, not taken. A denial degrades this watch to seeing that a tag is
    // nearby without being able to say one is following you (§6, fail-soft), and
    // that is the same code path as having no fix: `run_scan` records nothing it
    // cannot place. So the feed is not even opened without consent - a capability
    // you were refused is not one to hold open in case.
    let consented = crate::consent::ask_for_location(&crate::consent::intake_socket_path()).await;
    let feed = match consented {
        crate::consent::LocationConsent::Granted => {
            crate::location::LocationFeed::open(connection).await.ok()
        }
        crate::consent::LocationConsent::Denied => None,
    };
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
        // What the page reports as "the tracker has location" is this, measured per
        // advert: whether the thing that places a sighting answered. A feed that
        // opened and then stopped answering is not location the tracker has.
        live.saw_location(fix.is_some());
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let mut once = crate::tracker_run::Once::new(advert);
        let outcome = crate::tracker_run::run_scan(&mut once, store, fix, &epochs, now, &anchor);
        for correlator in &outcome.alerts {
            tracing::warn!(tag = %hex(correlator), "a separated tag has followed this machine");
            publish_tag(events, store, correlator, "alert", now, live).await;
        }
        for correlator in &outcome.review {
            tracing::info!(tag = %hex(correlator), "a separated tag is worth a look");
            publish_tag(events, store, correlator, "review", now, live).await;
        }
    }
    Ok(())
}

/// Watch for nearby recording devices until `stop` resolves.
///
/// The sibling of [`watch`] over the same substrate, and separate from it on
/// purpose: the two detectors have their own switches and their own sensitivity,
/// so a person who wants one and not the other gets exactly that rather than a
/// shared monitor that stops when either is turned off.
///
/// **Nothing is written down.** A stranger's device near you is not your context,
/// which §4.3 states outright - so this reports the live class and keeps no history
/// of who passed by. The contrast with the tag watch beside it is the point: that
/// one keeps an encrypted, pruned log because the criteria need a span, and this
/// one needs only "is it here now".
pub async fn watch_recording(
    sensitivity: arlen_sentinel_detect::ble_scan::Sensitivity,
    classes: &[arlen_sentinel_detect::recording::DeviceClass],
    events: &impl os_sdk::event::EventEmitter,
    live: &crate::live::Live,
    stop: impl std::future::Future<Output = ()>,
) -> Result<(), String> {
    use arlen_sentinel_detect::ble_scan::thresholds;
    use futures::StreamExt;

    let session = bluer::Session::new().await.map_err(|e| format!("no bluetooth session: {e}"))?;
    let adapter = session.default_adapter().await.map_err(|e| format!("no adapter: {e}"))?;
    let manager = adapter.monitor().await.map_err(|e| format!("no advertisement monitor: {e}"))?;
    let mut handle = manager
        .register(recording_monitor_for(thresholds(sensitivity), classes))
        .await
        .map_err(|e| format!("the monitor was refused: {e}"))?;

    tokio::pin!(stop);
    loop {
        let event = tokio::select! {
            () = &mut stop => break,
            event = handle.next() => event,
        };
        let Some(bluer::monitor::MonitorEvent::DeviceFound(found)) = event else { break };
        let Ok(device) = adapter.device(found.device) else { continue };
        let manufacturer = device.manufacturer_data().await.ok().flatten();
        let service = device.service_data().await.ok().flatten();
        let name = device.name().await.ok().flatten();
        let Some(found) =
            recording_match_from(manufacturer.as_ref(), service.as_ref(), name.as_deref(), classes)
        else {
            continue;
        };
        // The label and how sure, never the address: the indicator says what kind
        // of thing is nearby, and which one it is would be a log of the people
        // around you.
        tracing::info!(class = %found.label, confidence = ?found.confidence, "a recording device is nearby");
        let payload = os_sdk::proto::SentinelRecordingNearbyPayload {
            class_label: found.label.clone(),
            confidence: confidence_word(found.confidence).to_string(),
        };
        live.saw_recording(&payload.class_label, &payload.confidence);
        publish(events, RECORDING_EVENT, payload).await;
    }
    Ok(())
}

/// The event a nearby recording device is published under.
pub const RECORDING_EVENT: &str = "sentinel.recording_device.nearby";
/// The event a suspected tracker is published under.
pub const TRACKER_EVENT: &str = "sentinel.tracker.suspected";

/// The confidence as the payload spells it.
fn confidence_word(c: arlen_sentinel_detect::recording::Confidence) -> &'static str {
    use arlen_sentinel_detect::recording::Confidence;
    match c {
        Confidence::None => "none",
        Confidence::Low => "low",
        Confidence::Medium => "medium",
        Confidence::High => "high",
    }
}

/// The brand as the payload spells it.
fn brand_word(b: arlen_sentinel_detect::tracker::TrackerBrand) -> &'static str {
    use arlen_sentinel_detect::tracker::TrackerBrand;
    match b {
        TrackerBrand::AppleFindMy => "apple-find-my",
        TrackerBrand::SamsungSmartTag => "samsung-smarttag",
        TrackerBrand::Tile => "tile",
        TrackerBrand::GoogleFmdn => "google-fmdn",
        TrackerBrand::Dult => "dult",
    }
}

/// Publish one payload, best-effort.
///
/// A bus that is down costs the indicator this reading and nothing else. These
/// are advisory events: the sentinel says them, it never acts on them, so a
/// failure to say one must not stop it watching.
async fn publish<M: prost::Message>(
    events: &impl os_sdk::event::EventEmitter,
    event_type: &str,
    payload: M,
) {
    if let Err(e) = events.emit(event_type, prost::Message::encode_to_vec(&payload)).await {
        tracing::debug!("{event_type} not published: {e}");
    }
}

/// Publish a tag verdict, with the counts that justify it and nothing that
/// identifies the tag.
///
/// The correlator does NOT travel. It is the tag's rotating identity and belongs
/// only in the sealed store; an event carrying it would put a passing stranger's
/// device id on a bus several consumers read, which is the proximity history this
/// detector exists to warn people about.
async fn publish_tag(
    events: &impl os_sdk::event::EventEmitter,
    store: &crate::sightings::SightingStore,
    correlator: &[u8],
    verdict: &str,
    now_secs: u64,
    live: &crate::live::Live,
) {
    let Ok(Some(tag)) = store.load(correlator) else { return };
    let observed = tag.observe(now_secs);
    let payload = os_sdk::proto::SentinelTrackerSuspectedPayload {
        brand: brand_word(tag.brand).to_string(),
        distinct_locations: observed.distinct_locations,
        distinct_epochs: observed.distinct_epochs,
        verdict: verdict.to_string(),
    };
    live.saw_tracker(payload.brand.as_str(), verdict);
    publish(events, TRACKER_EVENT, payload).await;
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
    fn a_meta_advert_without_a_name_is_only_a_wearable() {
        use arlen_sentinel_detect::recording::{bundled_device_classes, Confidence};
        let mut md = HashMap::new();
        md.insert(0x01AB_u16, vec![1, 2]);
        let m = recording_match_from(Some(&md), None, None, &bundled_device_classes())
            .expect("the vendor id alone is a match");
        assert_eq!(m.confidence, Confidence::Low);
    }

    /// The name is what the controller cannot filter on and what lifts the answer.
    #[test]
    fn the_name_lifts_a_meta_advert_to_the_concrete_class() {
        use arlen_sentinel_detect::recording::{bundled_device_classes, Confidence};
        let mut md = HashMap::new();
        md.insert(0x01AB_u16, vec![1, 2]);
        let m = recording_match_from(Some(&md), None, Some("Ray-Ban Meta"), &bundled_device_classes())
            .expect("a corroborated match");
        assert_eq!(m.confidence, Confidence::High);
        assert_eq!(m.label, "Meta smart glasses");
    }

    /// An advert carrying several vendor ids is shown as its strongest answer.
    #[test]
    fn the_strongest_answer_is_the_one_shown() {
        use arlen_sentinel_detect::recording::{bundled_device_classes, Confidence};
        let mut md = HashMap::new();
        md.insert(0x05D6_u16, vec![0]); // the SoC, never a signal
        md.insert(0x03C2_u16, vec![0]); // Snap, a narrow vendor
        let m = recording_match_from(Some(&md), None, None, &bundled_device_classes())
            .expect("the narrow vendor answers");
        assert_eq!(m.confidence, Confidence::Medium);
        assert_eq!(m.label, "Snap Spectacles");
    }

    #[test]
    fn an_ordinary_device_is_not_a_recording_device() {
        let mut md = HashMap::new();
        md.insert(0x004C_u16, vec![1, 2, 3]);
        assert!(recording_match_from(
            Some(&md),
            None,
            Some("AirPods"),
            &arlen_sentinel_detect::recording::bundled_device_classes()
        )
        .is_none());
    }

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

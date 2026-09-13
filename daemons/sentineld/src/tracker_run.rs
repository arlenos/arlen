//! The tracker sentinel's run loop: adverts in, a verdict out
//! (`tracker-sentinel-plan.md` §2).
//!
//! Everything SEN-4 needs is now built and this is the piece that holds it
//! together: [`crate::location`] says where the machine is, the classifier says
//! whether an advert is a separated finder-tag, [`crate::sightings`] keeps that tag
//! between sessions, and the criteria decide. Splitting it out of the daemon's
//! `main` is what makes it testable - the loop is driven through two seams, so the
//! whole chain can be exercised without a radio or a location service.
//!
//! **The BLE scan is the seam that is still open** (§6 job 3's substrate). An
//! [`AdvertSource`] hands over raw advertisement fields; the real one is a BlueZ
//! `AdvertisementMonitor`, which needs a live adapter and is the piece this waits
//! on. Everything above it is exercised here against a source that yields fixtures.
//!
//! **Near-owner adverts are dropped before the store, not after.** A tag saying its
//! owner is nearby is not a stalking signal, and writing it down anyway would mean
//! keeping location history about somebody's own keys. The drop is the first thing
//! that happens to a classified advert.
//!
//! **An unplaced sighting is still recorded.** A scan with no fix - GeoClue silent,
//! the machine somewhere it cannot be placed - still proves the tag was near you,
//! just not where. It goes in with the last known fix if there is one and is skipped
//! only when there has never been one, because a sighting at (0, 0) would be a place
//! in the Gulf of Guinea that the distinct-place count would believe.

use arlen_sentinel_detect::epoch::EpochTracker;
use arlen_sentinel_detect::home_anchor::HomeAnchor;
use arlen_sentinel_detect::movement::Fix;
use arlen_sentinel_detect::sighting::Sighting;
use arlen_sentinel_detect::tracker::{classify_manufacturer_data, classify_service_data, Separation, TrackerMatch};
use arlen_sentinel_detect::trigger::{verdict, Verdict};

use crate::sightings::SightingStore;

/// One BLE advertisement, as a scanner reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Advert {
    /// The payload identifier this tag is correlated by - never the MAC.
    pub correlator: Vec<u8>,
    /// Manufacturer-Data, if the advert carried it: company id and payload.
    pub manufacturer: Option<(u16, Vec<u8>)>,
    /// Service-Data, if the advert carried it: 16-bit UUID and payload.
    pub service: Option<(u16, Vec<u8>)>,
}

/// Where adverts come from. The real implementation is a BlueZ
/// `AdvertisementMonitor`; the tests use a list.
pub trait AdvertSource {
    /// The adverts seen in this scan.
    fn scan(&mut self) -> Vec<Advert>;
}

/// What one scan did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ScanOutcome {
    /// Adverts that were not finder-tags at all.
    pub ignored: usize,
    /// Finder-tag adverts dropped because the owner is nearby.
    pub near_owner: usize,
    /// Sightings written to the store.
    pub recorded: usize,
    /// Tags that met the alert bar in this scan.
    pub alerts: Vec<Vec<u8>>,
    /// Tags worth a person's eyes but not an interruption.
    pub review: Vec<Vec<u8>>,
}

/// Classify one advert. Manufacturer-Data first because Apple is the only brand
/// that uses it, and a single advert carrying both is Apple's.
fn classify(advert: &Advert) -> Option<TrackerMatch> {
    if let Some((company, data)) = &advert.manufacturer {
        if let Some(m) = classify_manufacturer_data(*company, data) {
            return Some(m);
        }
    }
    let (uuid, data) = advert.service.as_ref()?;
    classify_service_data(*uuid, data)
}

/// Run one scan: classify, drop the near-owner adverts, record the rest against
/// their tags, and decide.
///
/// `fix` is the machine's current coarse location, or `None` when it has none.
/// The awake-session comes from the [`EpochTracker`], which the daemon feeds with
/// resumes, fixes and access-point sets; two distinct ones is what makes a span
/// meaningful on a machine that suspends. It is read here rather than passed as a
/// number so a scan cannot be stamped with an epoch nothing opened.
///
/// Whether this scan happened at home is asked of the learned anchor rather than
/// passed in, so nobody upstream can answer it for a place the machine has never
/// slept at. An unlearned anchor answers no, which judges every tag as if away from
/// home - the direction that tells you about a tag rather than damping it.
pub fn run_scan(
    source: &mut dyn AdvertSource,
    store: &SightingStore,
    fix: Option<Fix>,
    epochs: &EpochTracker,
    now_secs: u64,
    home: &HomeAnchor,
) -> ScanOutcome {
    let mut out = ScanOutcome::default();
    for advert in source.scan() {
        let Some(m) = classify(&advert) else {
            out.ignored += 1;
            continue;
        };
        if m.separation == Separation::NearOwner {
            out.near_owner += 1;
            continue;
        }
        let Some(fix) = fix else {
            // Never placed: see the module doc. A sighting with an invented
            // location is worse than one that was not taken.
            continue;
        };
        let at_home = home.is_home(fix);
        let sighting = Sighting { seen_at_secs: now_secs, fix, epoch_id: epochs.current() };
        let Ok(mut tag) = store.record(&advert.correlator, m.brand, sighting) else {
            // A store that will not write is a detection this scan cannot make.
            // It is logged by the caller; the scan carries on with the others.
            continue;
        };
        out.recorded += 1;
        if at_home && !tag.seen_at_home {
            tag.seen_at_home = true;
            let _ = store.save(&tag);
        }
        match verdict(&tag.observe(now_secs), tag.seen_at_home, tag.allow_state) {
            Verdict::Alert => out.alerts.push(advert.correlator.clone()),
            Verdict::Review => out.review.push(advert.correlator.clone()),
            Verdict::Nothing => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An Apple offline (separated) finding frame, the shape `tracker.rs` matches.
    fn apple_separated() -> Advert {
        let mut payload = vec![0x12, 0x19];
        payload.extend_from_slice(&[0u8; 25]);
        Advert {
            correlator: b"apple-key-1".to_vec(),
            manufacturer: Some((0x004C, payload)),
            service: None,
        }
    }

    fn not_a_tag() -> Advert {
        Advert {
            correlator: b"headphones".to_vec(),
            manufacturer: Some((0x0075, vec![1, 2, 3])),
            service: None,
        }
    }

    struct Fixed(Vec<Advert>);
    impl AdvertSource for Fixed {
        fn scan(&mut self) -> Vec<Advert> {
            self.0.clone()
        }
    }

    fn store(dir: &std::path::Path) -> SightingStore {
        SightingStore::open_in(dir).expect("a fresh store opens")
    }

    #[test]
    fn a_non_tag_advert_is_ignored_and_nothing_is_written() {
        let tmp = tempfile::tempdir().unwrap();
        let s = store(tmp.path());
        let mut src = Fixed(vec![not_a_tag()]);
        let out = run_scan(&mut src, &s, Some(Fix { lat: 47.26, lon: 11.40 }), &EpochTracker::default(), 1_000, &HomeAnchor::default());
        assert_eq!(out.ignored, 1);
        assert_eq!(out.recorded, 0);
        assert!(s.all().unwrap().is_empty());
    }

    /// The whole chain: a tag seen across two places and two sessions alerts.
    #[test]
    fn a_tag_across_two_places_and_two_sessions_alerts() {
        let tmp = tempfile::tempdir().unwrap();
        let s = store(tmp.path());
        let mut src = Fixed(vec![apple_separated()]);
        let here = Fix { lat: 47.2692, lon: 11.4041 };
        let there = Fix { lat: 47.2692, lon: 11.4570 };

        let away = HomeAnchor::default();
        let mut epochs = EpochTracker::default();
        run_scan(&mut src, &s, Some(here), &epochs, 1_000, &away);
        run_scan(&mut src, &s, Some(here), &epochs, 1_600, &away);
        // The lid closed and opened again somewhere else.
        epochs.observe(arlen_sentinel_detect::epoch::Signal::Resumed);
        let out = run_scan(&mut src, &s, Some(there), &epochs, 4_700, &away);

        assert_eq!(out.alerts, vec![b"apple-key-1".to_vec()], "{out:?}");
        assert!(out.review.is_empty());
    }

    /// The home anchor downgrades the same evidence to something a person can look
    /// at, rather than an interruption.
    #[test]
    fn the_same_evidence_at_home_is_review_not_an_alert() {
        let tmp = tempfile::tempdir().unwrap();
        let s = store(tmp.path());
        let mut src = Fixed(vec![apple_separated()]);
        let here = Fix { lat: 47.2692, lon: 11.4041 };
        let there = Fix { lat: 47.2692, lon: 11.4570 };

        // Three nights at `here` make it the learned home.
        let mut home = HomeAnchor::default();
        for night in 1..=3 {
            home.record(arlen_sentinel_detect::home_anchor::NightFix { night, fix: here });
        }
        let mut epochs = EpochTracker::default();
        run_scan(&mut src, &s, Some(here), &epochs, 1_000, &home);
        run_scan(&mut src, &s, Some(here), &epochs, 1_600, &home);
        epochs.observe(arlen_sentinel_detect::epoch::Signal::Resumed);
        let out = run_scan(&mut src, &s, Some(there), &epochs, 4_700, &home);

        assert!(out.alerts.is_empty(), "{out:?}");
        assert_eq!(out.review, vec![b"apple-key-1".to_vec()]);
    }

    /// No fix means no place, and a sighting with an invented place is worse than
    /// one that was not taken.
    #[test]
    fn an_unplaced_scan_writes_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let s = store(tmp.path());
        let mut src = Fixed(vec![apple_separated()]);
        let out = run_scan(&mut src, &s, None, &EpochTracker::default(), 1_000, &HomeAnchor::default());
        assert_eq!(out.recorded, 0);
        assert!(s.all().unwrap().is_empty());
    }
}

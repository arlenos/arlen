//! Accumulate one tag's separated sightings into the summary the alert criteria read.
//!
//! SEN-4's middle. [`crate::tracker`] says a BLE advert is a separated finder-tag and
//! which brand; [`crate::movement`] turns fixes into places and distance;
//! [`crate::trigger`] decides. Between them something has to hold a tag's sightings
//! across the awake sessions and reduce them, and this is it: still PURE (sightings
//! in, a summary out) so the accumulation is provable on a laptop with no radio.
//!
//! **A tag is keyed by its payload correlator, never by its MAC.** The BLE MAC is a
//! resolvable private address that rotates; the payload identifier (Apple public-key
//! bytes, Samsung's 8-byte id, Google's EID) is stable inside one rotation window and
//! deliberately unchainable across it. That cap is the privacy feature, so evidence
//! has to accumulate INSIDE one window - which is why [`TrackedTag`] carries the
//! window it opened in and forgets a sighting that falls outside it, rather than
//! silently chaining two windows into one apparent long observation.
//!
//! **Near-owner adverts never arrive here.** A tag advertising that its owner is
//! nearby is not a stalking signal; it is dropped at classification, before the
//! store. [`TrackedTag::record`] takes a sighting that is already known separated.
//!
//! What is NOT here: the encrypted at-rest store under
//! `~/.local/state/arlen/tracker-sentinel/` (`tracker-sentinel-plan.md` §6 job 2),
//! the GeoClue2 fix feed (job 1), and the epoch boundaries themselves - a suspend, a
//! BSSID-set change - which are the daemon's. This module takes an epoch id as a
//! given and counts distinct ones.

use std::collections::BTreeSet;

use crate::movement::{count_distinct_locations, travelled_metres, Fix};
use crate::tracker::TrackerBrand;
use crate::trigger::TrackerObservation;

/// The longest a correlator is treated as one tag. Apple rotates its key daily and
/// the others are comparable, so a sighting older than this cannot be proven to be
/// the same physical tag - keeping it would manufacture a span nobody observed.
pub const MAX_WINDOW_SECS: u64 = 24 * 60 * 60;

/// How many sightings one tag retains. The criteria need counts, places, a span and
/// a distance, all of which the oldest sightings still contribute to, so the bound
/// exists to cap memory against a tag that adverts continuously for a day, not to
/// forget deliberately. At a continuous passive scan this is hours of adverts.
pub const MAX_SIGHTINGS: usize = 512;

/// What the person has said about this tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum AllowState {
    /// Nothing said. The only state that may raise a safety alert.
    #[default]
    Unknown,
    /// Recognised as travelling with a known companion (a partner's keys, a shared
    /// bag). Reviewable, never alerted on.
    KnownCompanion,
    /// The person said it is theirs.
    Mine,
}

/// One observation of a separated finder-tag in one scan.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Sighting {
    /// When it was seen, in unix seconds.
    pub seen_at_secs: u64,
    /// The coarse fix the daemon had at the time.
    pub fix: Fix,
    /// The awake-session this fell in. Suspend/resume and a BSSID-set change open a
    /// new one; two distinct epochs is what makes a span across a suspend meaningful.
    pub epoch_id: u64,
}

/// Every sighting of one physical tag inside its stable-identifier window.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TrackedTag {
    /// The payload identifier, never the MAC. Opaque bytes: each brand's own.
    pub correlator: Vec<u8>,
    /// Which ecosystem it belongs to.
    pub brand: TrackerBrand,
    /// When the current stable-identifier window opened, in unix seconds.
    pub window_opened_secs: u64,
    /// Seen at the home anchor at least once, which downgrades it: a tag that is
    /// where you live is a household or a neighbour's far more often than a stalker's.
    pub seen_at_home: bool,
    /// What the person has said about it.
    pub allow_state: AllowState,
    /// When this tag last raised an alert, in unix seconds.
    pub last_alert_secs: Option<u64>,
    /// Retained sightings, oldest first.
    sightings: Vec<Sighting>,
}

impl TrackedTag {
    /// Open a window for a newly seen correlator.
    pub fn new(correlator: Vec<u8>, brand: TrackerBrand, window_opened_secs: u64) -> Self {
        Self {
            correlator,
            brand,
            window_opened_secs,
            seen_at_home: false,
            allow_state: AllowState::Unknown,
            last_alert_secs: None,
            sightings: Vec::new(),
        }
    }

    /// Record one separated sighting.
    ///
    /// A sighting at or past the window's end does not extend the window, it OPENS a
    /// new one: the correlator may be a different physical tag by then, and carrying
    /// the old sightings over would claim a day-long observation nobody made. So the
    /// retained set is cleared and the window re-opens at this sighting.
    ///
    /// A sighting older than the window's start is ignored rather than inserted out
    /// of order - the only producer is a scan happening now, so an older one is a
    /// clock that went backwards, and the safe reading of that is to drop the reading
    /// rather than to rewrite the window around it.
    pub fn record(&mut self, sighting: Sighting) {
        if sighting.seen_at_secs < self.window_opened_secs {
            return;
        }
        if sighting.seen_at_secs - self.window_opened_secs >= MAX_WINDOW_SECS {
            self.sightings.clear();
            self.window_opened_secs = sighting.seen_at_secs;
        }
        if self.sightings.len() == MAX_SIGHTINGS {
            self.sightings.remove(0);
        }
        self.sightings.push(sighting);
    }

    /// How many sightings are retained.
    pub fn len(&self) -> usize {
        self.sightings.len()
    }

    /// Whether nothing has been recorded in this window.
    pub fn is_empty(&self) -> bool {
        self.sightings.is_empty()
    }

    /// Distinct awake-sessions this tag was seen in.
    ///
    /// Separate from the place count and not a substitute for it: a laptop can be
    /// carried between two places inside one session, and can sit in one place across
    /// several. The criteria want both.
    pub fn distinct_epochs(&self) -> u32 {
        self.sightings.iter().map(|s| s.epoch_id).collect::<BTreeSet<_>>().len() as u32
    }

    /// Reduce the retained sightings to the summary [`crate::trigger`] decides on.
    ///
    /// `now_secs` only fills the cooldown field; the span comes from the sightings
    /// themselves, so a summary taken twice without a new sighting is the same
    /// summary except for the cooldown ticking down.
    pub fn observe(&self, now_secs: u64) -> TrackerObservation {
        let fixes: Vec<Fix> = self.sightings.iter().map(|s| s.fix).collect();
        let span = match (self.sightings.first(), self.sightings.last()) {
            (Some(first), Some(last)) => last.seen_at_secs.saturating_sub(first.seen_at_secs),
            _ => 0,
        };
        TrackerObservation {
            sighting_count: self.sightings.len() as u32,
            distinct_locations: count_distinct_locations(&fixes),
            distinct_epochs: self.distinct_epochs(),
            observation_span_secs: span,
            travelled_metres: travelled_metres(&fixes),
            secs_since_last_alert: self.last_alert_secs.map(|at| now_secs.saturating_sub(at)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(secs: u64, lat: f64, lon: f64, epoch: u64) -> Sighting {
        Sighting { seen_at_secs: secs, fix: Fix { lat, lon }, epoch_id: epoch }
    }

    fn tag() -> TrackedTag {
        TrackedTag::new(vec![1, 2, 3], TrackerBrand::AppleFindMy, 1_000)
    }

    #[test]
    fn an_empty_tag_summarises_to_nothing() {
        let obs = tag().observe(2_000);
        assert_eq!(obs.sighting_count, 0);
        assert_eq!(obs.distinct_locations, 0);
        assert_eq!(obs.distinct_epochs, 0);
        assert_eq!(obs.observation_span_secs, 0);
        assert_eq!(obs.travelled_metres, 0.0);
        assert_eq!(obs.secs_since_last_alert, None);
    }

    /// The shape the criteria are written for: seen in two places, two sessions,
    /// over more than half an hour.
    #[test]
    fn sightings_across_two_places_summarise_as_the_criteria_read_them() {
        let mut t = tag();
        // Innsbruck, then about 4 km east, an hour later and after a suspend.
        t.record(at(1_000, 47.2692, 11.4041, 1));
        t.record(at(1_600, 47.2692, 11.4041, 1));
        t.record(at(4_700, 47.2692, 11.4570, 2));
        let obs = t.observe(5_000);
        assert_eq!(obs.sighting_count, 3);
        assert_eq!(obs.distinct_locations, 2);
        assert_eq!(obs.distinct_epochs, 2);
        assert_eq!(obs.observation_span_secs, 3_700);
        assert!(obs.travelled_metres > 400.0, "{}", obs.travelled_metres);
        assert_eq!(t.distinct_epochs(), 2);
    }

    /// A tag sitting in one spot is a neighbour, and the place count says so however
    /// many times it is seen.
    #[test]
    fn one_place_stays_one_place_however_often_it_is_seen() {
        let mut t = tag();
        for i in 0..20 {
            t.record(at(1_000 + i * 60, 47.2692, 11.4041, 1));
        }
        assert_eq!(t.observe(3_000).distinct_locations, 1);
    }

    /// Across a rotation the chaining is lost by design, so the window restarts
    /// rather than claiming a day-long observation.
    #[test]
    fn a_sighting_past_the_window_opens_a_new_one() {
        let mut t = tag();
        t.record(at(1_000, 47.2692, 11.4041, 1));
        t.record(at(1_500, 47.2692, 11.4570, 1));
        assert_eq!(t.len(), 2);

        t.record(at(1_000 + MAX_WINDOW_SECS, 47.2692, 11.4041, 9));
        assert_eq!(t.len(), 1, "the old window's sightings are gone");
        assert_eq!(t.window_opened_secs, 1_000 + MAX_WINDOW_SECS);
        assert_eq!(t.observe(1_000 + MAX_WINDOW_SECS).observation_span_secs, 0);
    }

    /// A clock that went backwards drops the reading rather than rewriting history.
    #[test]
    fn a_sighting_before_the_window_is_ignored() {
        let mut t = tag();
        t.record(at(500, 47.2692, 11.4041, 1));
        assert!(t.is_empty());
        assert_eq!(t.window_opened_secs, 1_000);
    }

    #[test]
    fn the_retained_set_is_bounded_and_keeps_the_newest() {
        let mut t = tag();
        for i in 0..(MAX_SIGHTINGS as u64 + 50) {
            t.record(at(1_000 + i, 47.2692, 11.4041, 1));
        }
        assert_eq!(t.len(), MAX_SIGHTINGS);
        let obs = t.observe(9_999);
        // The oldest 50 fell out, so the span is the bound, not the whole run.
        assert_eq!(obs.observation_span_secs, MAX_SIGHTINGS as u64 - 1);
    }

    #[test]
    fn the_cooldown_is_measured_from_the_last_alert() {
        let mut t = tag();
        t.record(at(1_000, 47.2692, 11.4041, 1));
        assert_eq!(t.observe(5_000).secs_since_last_alert, None);
        t.last_alert_secs = Some(2_000);
        assert_eq!(t.observe(5_000).secs_since_last_alert, Some(3_000));
    }
}

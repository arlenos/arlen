//! Decide whether a tracked finder-tag's observations meet the stalking-alert bar.
//!
//! SEN-4: after [`crate::tracker`] classifies SEPARATED finder-tag adverts, the
//! daemon aggregates them per device across its awake-session/movement model into a
//! [`TrackerObservation`] summary; this module applies the alert criteria to that
//! summary. It is PURE (a summary in, a yes/no out) so the thresholds are tested
//! without the geo/movement machinery. Keeping the decision here and off the daemon
//! means the load-bearing thresholds are one auditable place.
//!
//! Criteria (research-grounded against AirGuard + Apple TrackingAvoidance): a device
//! must have been SEEN across at least two distinct places (the load-bearing signal
//! that it is travelling WITH the user, not merely present in one location), seen
//! enough times, over a long enough window, having travelled far enough, and not
//! already alerted recently. Only separated adverts feed the count - a near-owner
//! advert is never a stalking signal and is dropped before it reaches here.

/// The load-bearing threshold: a tag must appear at >= 2 distinct places to signal
/// it is travelling with the user rather than fixed in one location.
pub const MIN_DISTINCT_LOCATIONS: u32 = 2;
/// A tag must be seen at least this many times (separated adverts only).
pub const MIN_SIGHTINGS: u32 = 3;
/// The first-to-last sighting window must span at least 30 minutes.
pub const MIN_OBSERVATION_SPAN_SECS: u64 = 30 * 60;
/// The tag must have travelled at least 400 m with the user.
pub const MIN_TRAVELLED_METRES: f64 = 400.0;
/// Do not re-alert for the same device within 7 hours.
pub const ALERT_COOLDOWN_SECS: u64 = 7 * 60 * 60;

/// A per-device summary of separated sightings, produced by the daemon's movement
/// model, that the alert criteria are applied to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrackerObservation {
    /// How many separated adverts from this device were seen.
    pub sighting_count: u32,
    /// Distinct coarse locations (GeoClue cell / BSSID-set) the device was seen at.
    pub distinct_locations: u32,
    /// Seconds between the first and last sighting.
    pub observation_span_secs: u64,
    /// Metres travelled with the user across the sightings.
    pub travelled_metres: f64,
    /// Seconds since the last alert for THIS device, or `None` if never alerted.
    pub secs_since_last_alert: Option<u64>,
}

impl TrackerObservation {
    /// Whether a fresh alert may fire: never alerted, or the cooldown has elapsed.
    fn cooldown_elapsed(&self) -> bool {
        match self.secs_since_last_alert {
            None => true,
            Some(elapsed) => elapsed >= ALERT_COOLDOWN_SECS,
        }
    }
}

/// Whether `obs` meets every stalking-alert criterion. All of: seen at >= 2 distinct
/// places (load-bearing), >= 3 times, over >= 30 min, having travelled >= 400 m, AND
/// outside the 7 h re-alert cooldown. `travelled_metres` is compared with `>=` on a
/// finite value; a non-finite distance (a bad GeoClue fix) never satisfies it.
pub fn should_alert(obs: &TrackerObservation) -> bool {
    obs.distinct_locations >= MIN_DISTINCT_LOCATIONS
        && obs.sighting_count >= MIN_SIGHTINGS
        && obs.observation_span_secs >= MIN_OBSERVATION_SPAN_SECS
        && obs.travelled_metres.is_finite()
        && obs.travelled_metres >= MIN_TRAVELLED_METRES
        && obs.cooldown_elapsed()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A summary that meets every criterion; tests knock one field below the bar.
    fn qualifying() -> TrackerObservation {
        TrackerObservation {
            sighting_count: 5,
            distinct_locations: 3,
            observation_span_secs: 45 * 60,
            travelled_metres: 900.0,
            secs_since_last_alert: None,
        }
    }

    #[test]
    fn a_fully_qualifying_observation_alerts() {
        assert!(should_alert(&qualifying()));
    }

    #[test]
    fn a_single_location_never_alerts() {
        let obs = TrackerObservation {
            distinct_locations: 1,
            ..qualifying()
        };
        assert!(!should_alert(&obs), "one place is not a travelling-with signal");
    }

    #[test]
    fn each_threshold_gate_holds() {
        assert!(!should_alert(&TrackerObservation { sighting_count: 2, ..qualifying() }));
        assert!(!should_alert(&TrackerObservation {
            observation_span_secs: 29 * 60,
            ..qualifying()
        }));
        assert!(!should_alert(&TrackerObservation { travelled_metres: 399.0, ..qualifying() }));
        // A non-finite distance (bad fix) never qualifies.
        assert!(!should_alert(&TrackerObservation {
            travelled_metres: f64::NAN,
            ..qualifying()
        }));
    }

    #[test]
    fn the_realert_cooldown_suppresses_a_recent_alert() {
        // Just alerted -> suppressed even though everything else qualifies.
        let recent = TrackerObservation {
            secs_since_last_alert: Some(ALERT_COOLDOWN_SECS - 1),
            ..qualifying()
        };
        assert!(!should_alert(&recent));
        // Past the cooldown -> alerts again.
        let stale = TrackerObservation {
            secs_since_last_alert: Some(ALERT_COOLDOWN_SECS),
            ..qualifying()
        };
        assert!(should_alert(&stale));
    }

    #[test]
    fn the_exact_thresholds_qualify() {
        let boundary = TrackerObservation {
            sighting_count: MIN_SIGHTINGS,
            distinct_locations: MIN_DISTINCT_LOCATIONS,
            observation_span_secs: MIN_OBSERVATION_SPAN_SECS,
            travelled_metres: MIN_TRAVELLED_METRES,
            secs_since_last_alert: None,
        };
        assert!(should_alert(&boundary), "the exact bar qualifies (>=, not >)");
    }
}

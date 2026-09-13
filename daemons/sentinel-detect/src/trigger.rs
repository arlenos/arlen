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
/// A tag must have been seen in at least this many distinct awake-sessions.
///
/// Separate from the place count and not a substitute for it. A laptop suspends,
/// and the plan redefines "duration" from continuous minutes to a span across
/// sessions precisely because a suspend gap shreds any continuous measure; a tag
/// seen in two sessions was there before and after you closed the lid.
pub const MIN_DISTINCT_EPOCHS: u32 = 2;
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
    /// Distinct awake-sessions the device was seen in.
    pub distinct_epochs: u32,
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
        && obs.distinct_epochs >= MIN_DISTINCT_EPOCHS
        && obs.sighting_count >= MIN_SIGHTINGS
        && obs.observation_span_secs >= MIN_OBSERVATION_SPAN_SECS
        && obs.travelled_metres.is_finite()
        && obs.travelled_metres >= MIN_TRAVELLED_METRES
        && obs.cooldown_elapsed()
}

/// What the sentinel does about one tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Every criterion met and nothing downgrades it: a safety notification.
    Alert,
    /// Worth a person's eyes but not an interruption. The pull-review surface.
    Review,
    /// Nothing. The overwhelming majority of adverts.
    Nothing,
}

/// Decide what to do about one tag: the criteria, plus the two things that
/// downgrade a tag no matter how well it scores.
///
/// **The home anchor downgrades rather than suppresses.** A tag seen where you live
/// is a household member's, a neighbour's or your own far more often than a
/// stalker's - but "far more often" is not "always", and silently dropping it would
/// make the one case that matters unreachable. So it goes to review, where a person
/// can look, instead of to a notification or to nothing.
///
/// **An allowlisted tag is never alerted on and is still reviewable.** Saying a tag
/// is yours or a companion's answers the question "should this interrupt me", not
/// "did it travel with me", and the second answer is the one somebody re-checks when
/// they change their mind about the first.
///
/// The middle state is the laptop form-factor's, not a hedge: a machine that spends
/// the day shut sees a genuine follower thinly, and the plan's answer to thin
/// evidence is a review surface rather than a lower bar.
pub fn verdict(obs: &TrackerObservation, seen_at_home: bool, allowlisted: bool) -> Verdict {
    if obs.distinct_locations < MIN_DISTINCT_LOCATIONS {
        // The load-bearing signal is absent: seen in one place, however often.
        return Verdict::Nothing;
    }
    if seen_at_home || allowlisted || !should_alert(obs) {
        return Verdict::Review;
    }
    Verdict::Alert
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A summary that meets every criterion; tests knock one field below the bar.
    fn qualifying() -> TrackerObservation {
        TrackerObservation {
            sighting_count: 5,
            distinct_epochs: 2,
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

    /// One place is the whole discriminator: three sightings at your desk is a
    /// neighbour's tag, not somebody following you.
    #[test]
    fn one_place_is_nothing_however_strong_the_rest() {
        let mut obs = qualifying();
        obs.distinct_locations = 1;
        obs.sighting_count = 50;
        assert_eq!(verdict(&obs, false, false), Verdict::Nothing);
    }

    /// Downgraded, not dropped: a person can still look.
    #[test]
    fn home_and_the_allowlist_downgrade_rather_than_suppress() {
        let obs = qualifying();
        assert_eq!(verdict(&obs, false, false), Verdict::Alert);
        assert_eq!(verdict(&obs, true, false), Verdict::Review);
        assert_eq!(verdict(&obs, false, true), Verdict::Review);
    }

    /// The laptop case: two places, but the evidence is thin. Review, not silence.
    #[test]
    fn thin_evidence_across_two_places_is_reviewable() {
        let mut obs = qualifying();
        obs.sighting_count = MIN_SIGHTINGS - 1;
        assert!(!should_alert(&obs));
        assert_eq!(verdict(&obs, false, false), Verdict::Review);
    }

    /// A tag seen twice in one session has not been shown to follow you between
    /// sessions, which is what the span is meant to prove on a machine that suspends.
    #[test]
    fn one_session_does_not_alert() {
        let mut obs = qualifying();
        obs.distinct_epochs = 1;
        assert!(!should_alert(&obs));
        assert_eq!(verdict(&obs, false, false), Verdict::Review);
    }

    #[test]
    fn the_exact_thresholds_qualify() {
        let boundary = TrackerObservation {
            sighting_count: MIN_SIGHTINGS,
            distinct_locations: MIN_DISTINCT_LOCATIONS,
            distinct_epochs: MIN_DISTINCT_EPOCHS,
            observation_span_secs: MIN_OBSERVATION_SPAN_SECS,
            travelled_metres: MIN_TRAVELLED_METRES,
            secs_since_last_alert: None,
        };
        assert!(should_alert(&boundary), "the exact bar qualifies (>=, not >)");
    }
}

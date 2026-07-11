//! Entity liveness / precision (agent-work-surfaces-plan.md, the coder edge).
//!
//! The briefing and prep-for-this surfaces must tell **live-and-important from
//! stale-and-noise** (the Rowboat "20 entities they had no idea what they were"
//! failure the plan names as the graveyard the whole briefing category falls
//! into). This is the pure scoring core: from an entity's temporal signals it
//! classifies liveness and gives a recency-weighted score in `[0, 1]`, so an
//! aggregation ranks what actually needs the human first and drops the noise.
//!
//! It is pure and deterministic (no clock of its own; `now` is passed in), so it
//! is fully unit-tested; the KG read supplies the signals. It fails toward STALE:
//! an entity with no activity signal is treated as noise, never surfaced as live.

/// Micros in one day, the unit the liveness windows are expressed in.
const MICROS_PER_DAY: i64 = 86_400 * 1_000_000;

/// Activity within this many days of `now` is LIVE.
const LIVE_WINDOW_DAYS: f64 = 7.0;
/// Activity within this many days (but past the live window) is DORMANT; beyond
/// it is STALE.
const DORMANT_WINDOW_DAYS: f64 = 30.0;
/// Recency half-life: the score halves for every this-many days since the last
/// activity, so a just-touched entity scores ~1 and an old one decays smoothly.
const RECENCY_HALF_LIFE_DAYS: f64 = 14.0;
/// Activity count at which the frequency bonus saturates (a heavily-touched entity
/// is more signal than a once-seen one, but the bonus is bounded so recency stays
/// the dominant term).
const ACTIVITY_SATURATION: f64 = 50.0;
/// How much of the score recency carries vs the activity-frequency bonus.
const RECENCY_WEIGHT: f64 = 0.7;
const ACTIVITY_WEIGHT: f64 = 0.3;

/// The temporal signals a KG node/entity carries for liveness. All optional so a
/// node missing a field degrades gracefully; times are unix micros.
#[derive(Debug, Clone, Copy, Default)]
pub struct LivenessSignals {
    /// The most recent activity on the entity (a `last_accessed` / `last_seen` /
    /// recent-mention stamp), unix micros. `None` = never observed active.
    pub last_activity_micros: Option<i64>,
    /// When the entity first entered the graph, unix micros. Used as the recency
    /// anchor when there is no explicit activity stamp.
    pub created_at_micros: Option<i64>,
    /// How many times the entity has been touched (`access_count` /
    /// `observe_count`), the frequency signal.
    pub activity_count: u64,
    /// A transaction-time close (`expired_at`): the system stopped believing the
    /// entity active (archived / retired). `Some` = retired, never surfaced live.
    pub expired_at_micros: Option<i64>,
}

/// The liveness verdict for an entity, coarsest-to-finest for a surface to bucket
/// on (a briefing shows LIVE first, may show DORMANT under a fold, hides the rest).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Liveness {
    /// Recent activity: this is what likely needs the human now.
    Live,
    /// Some history, but gone quiet: context, not a headline.
    Dormant,
    /// Old and low-signal: noise, kept out of the surface by default.
    Stale,
    /// Transaction-closed (archived / superseded): never surfaced as active.
    Retired,
}

impl Liveness {
    /// A stable lowercase tag for the wire / a surface to bucket on.
    pub fn as_str(self) -> &'static str {
        match self {
            Liveness::Live => "live",
            Liveness::Dormant => "dormant",
            Liveness::Stale => "stale",
            Liveness::Retired => "retired",
        }
    }
}

/// The recency anchor for an entity: the explicit activity stamp if present, else
/// the creation stamp (a just-created entity with no activity yet is still recent).
fn recency_anchor(signals: &LivenessSignals) -> Option<i64> {
    signals
        .last_activity_micros
        .or(signals.created_at_micros)
}

/// Age in days from the recency anchor to `now`. A future anchor (clock skew)
/// clamps to 0 so it never scores as "older than now".
fn age_days(anchor_micros: i64, now_micros: i64) -> f64 {
    let delta = (now_micros - anchor_micros).max(0);
    delta as f64 / MICROS_PER_DAY as f64
}

/// Classify an entity's liveness. A retired entity is `Retired` regardless of
/// recency; otherwise the age of the recency anchor buckets it, and an entity with
/// no temporal signal at all is `Stale` (fail toward noise, never surfaced live).
pub fn classify(signals: &LivenessSignals, now_micros: i64) -> Liveness {
    if signals.expired_at_micros.is_some() {
        return Liveness::Retired;
    }
    let Some(anchor) = recency_anchor(signals) else {
        return Liveness::Stale;
    };
    let days = age_days(anchor, now_micros);
    if days <= LIVE_WINDOW_DAYS {
        Liveness::Live
    } else if days <= DORMANT_WINDOW_DAYS {
        Liveness::Dormant
    } else {
        Liveness::Stale
    }
}

/// A recency-weighted liveness score in `[0, 1]`: recency (half-life decay) is the
/// dominant term, a bounded activity-frequency bonus differentiates within a tier.
/// A retired or signal-less entity scores 0, so an aggregation can rank + threshold
/// on one number. Deterministic for a given `(signals, now)`.
pub fn liveness_score(signals: &LivenessSignals, now_micros: i64) -> f64 {
    if signals.expired_at_micros.is_some() {
        return 0.0;
    }
    let Some(anchor) = recency_anchor(signals) else {
        return 0.0;
    };
    let days = age_days(anchor, now_micros);
    // Half-life decay: recency = 0.5 ^ (days / half_life), in (0, 1].
    let recency = 0.5_f64.powf(days / RECENCY_HALF_LIFE_DAYS);
    // Bounded frequency bonus in [0, 1].
    let activity = (signals.activity_count as f64 / ACTIVITY_SATURATION).min(1.0);
    (recency * RECENCY_WEIGHT + activity * ACTIVITY_WEIGHT).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: i64 = MICROS_PER_DAY;
    const NOW: i64 = 1_000_000 * 1_700_000_000; // a fixed "now" in micros

    fn at(days_ago: i64) -> Option<i64> {
        Some(NOW - days_ago * DAY)
    }

    #[test]
    fn a_recently_touched_entity_is_live_with_a_high_score() {
        let s = LivenessSignals {
            last_activity_micros: at(1),
            created_at_micros: at(400),
            activity_count: 12,
            expired_at_micros: None,
        };
        assert_eq!(classify(&s, NOW), Liveness::Live);
        assert!(liveness_score(&s, NOW) > 0.6, "recent + active must score high");
    }

    #[test]
    fn a_quiet_entity_is_dormant() {
        let s = LivenessSignals {
            last_activity_micros: at(20),
            activity_count: 3,
            ..Default::default()
        };
        assert_eq!(classify(&s, NOW), Liveness::Dormant);
    }

    #[test]
    fn an_old_low_signal_entity_is_stale_with_a_low_score() {
        let s = LivenessSignals {
            last_activity_micros: at(90),
            activity_count: 1,
            ..Default::default()
        };
        assert_eq!(classify(&s, NOW), Liveness::Stale);
        assert!(liveness_score(&s, NOW) < 0.1, "old + rare must score near zero");
    }

    #[test]
    fn a_retired_entity_is_never_live_and_scores_zero() {
        // Even with very recent activity, an expired (archived) entity is Retired.
        let s = LivenessSignals {
            last_activity_micros: at(0),
            activity_count: 99,
            expired_at_micros: at(1),
            ..Default::default()
        };
        assert_eq!(classify(&s, NOW), Liveness::Retired);
        assert_eq!(liveness_score(&s, NOW), 0.0);
    }

    #[test]
    fn no_signal_at_all_is_stale_not_live() {
        // The Rowboat failure: an entity with no temporal signal must never be
        // surfaced as live; it is noise until proven otherwise.
        let s = LivenessSignals::default();
        assert_eq!(classify(&s, NOW), Liveness::Stale);
        assert_eq!(liveness_score(&s, NOW), 0.0);
    }

    #[test]
    fn more_activity_beats_less_at_equal_recency() {
        let base = LivenessSignals {
            last_activity_micros: at(3),
            ..Default::default()
        };
        let busy = LivenessSignals { activity_count: 40, ..base };
        let quiet = LivenessSignals { activity_count: 2, ..base };
        assert!(
            liveness_score(&busy, NOW) > liveness_score(&quiet, NOW),
            "a more-touched entity outranks a rarely-touched one at the same recency"
        );
    }

    #[test]
    fn creation_stamp_anchors_when_no_activity_stamp() {
        // A just-created entity with no activity yet is still recent (live).
        let s = LivenessSignals {
            created_at_micros: at(2),
            ..Default::default()
        };
        assert_eq!(classify(&s, NOW), Liveness::Live);
    }

    #[test]
    fn a_future_anchor_clamps_to_now() {
        // Clock skew: an anchor in the future must not score as older-than-now.
        let s = LivenessSignals {
            last_activity_micros: Some(NOW + 5 * DAY),
            ..Default::default()
        };
        assert_eq!(classify(&s, NOW), Liveness::Live);
        assert!(liveness_score(&s, NOW) >= RECENCY_WEIGHT - 0.001);
    }

    #[test]
    fn the_score_is_always_within_the_unit_interval() {
        let s = LivenessSignals {
            last_activity_micros: at(0),
            created_at_micros: at(0),
            activity_count: u64::MAX,
            expired_at_micros: None,
        };
        let score = liveness_score(&s, NOW);
        assert!((0.0..=1.0).contains(&score), "score must stay in [0, 1], got {score}");
    }
}

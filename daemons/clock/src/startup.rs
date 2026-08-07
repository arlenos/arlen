//! Picking up where the daemon left off.
//!
//! Everything the clock knows is anchored to wall-clock instants, so coming back
//! is not a matter of restoring a countdown - it is asking, for each anchor,
//! whether its moment is still ahead, has just gone by, or went by so long ago
//! that acting on it would be wrong. That question is the same after a restart,
//! after a crash and after a night asleep, so there is one answer for all three
//! rather than a separate resume path per cause.
//!
//! **Anchors are re-derived, not trusted.** A stored `next_fire_at` was computed
//! against the rules and the timezone as they were; either may have changed
//! while nothing was running - the machine crossed a border, the clocks went
//! back, someone edited the state file. Recomputing costs nothing and removes a
//! class of "it rang an hour late and nobody knew why".

use crate::alarm::next_fire_at;
use crate::missed::{self, Missed};
use crate::state::ClockState;

/// What the daemon should do the moment it comes up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resumed {
    /// Alarms to ring once, late, because their moment passed while nothing was
    /// running and they asked to be told. Ids, in the order they appear.
    pub ring_late: Vec<String>,
    /// The moment to ask `Power1` to wake for, after the anchors were redone.
    pub wake_at: Option<i64>,
}

/// Bring a loaded state up to date.
///
/// Every alarm gets a verdict and then a fresh anchor, in that order - the
/// verdict is about the moment that *was* stored, and re-deriving first would
/// throw away the very thing being judged.
pub fn resume<Tz: chrono::TimeZone>(
    state: &mut ClockState,
    tz: &Tz,
    window_ms: i64,
    now_ms: i64,
) -> Resumed {
    let mut ring_late = Vec::new();
    for alarm in &mut state.alarms {
        if missed::verdict(alarm, now_ms, window_ms) == Missed::FireLate {
            ring_late.push(alarm.id.clone());
        }
        alarm.next_fire_at = next_fire_at(alarm, tz, now_ms);
    }
    Resumed {
        ring_late,
        wake_at: crate::wake::next_wake_at(state, now_ms),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::missed::LATE_WINDOW_MS;
    use crate::state::Alarm;
    use chrono::{FixedOffset, TimeZone};

    fn utc() -> FixedOffset {
        FixedOffset::east_opt(0).unwrap()
    }

    fn at(day: u32, h: u32, m: u32) -> i64 {
        utc()
            .with_ymd_and_hms(2026, 8, day, h, m, 0)
            .unwrap()
            .timestamp_millis()
    }

    fn alarm(id: &str, time: &str, due: Option<i64>, fire_late: bool) -> Alarm {
        Alarm {
            id: id.into(),
            time: time.into(),
            label: String::new(),
            days: vec![],
            enabled: true,
            fire_late,
            next_fire_at: due,
        }
    }

    fn state(alarms: Vec<Alarm>) -> ClockState {
        ClockState {
            alarms,
            ..ClockState::default()
        }
    }

    /// The overnight case, end to end: the moment passed, it rings once, and it
    /// is re-armed for tomorrow rather than left pointing at this morning.
    #[test]
    fn an_alarm_missed_overnight_rings_once_and_is_re_armed() {
        let mut s = state(vec![alarm("a", "07:00", Some(at(5, 7, 0)), true)]);
        let out = resume(&mut s, &utc(), LATE_WINDOW_MS, at(5, 9, 0));
        assert_eq!(out.ring_late, vec!["a".to_string()]);
        assert_eq!(s.alarms[0].next_fire_at, Some(at(6, 7, 0)));
        assert_eq!(out.wake_at, Some(at(6, 7, 0)));
    }

    #[test]
    fn an_alarm_still_ahead_is_left_where_it_is() {
        let mut s = state(vec![alarm("a", "07:00", Some(at(5, 7, 0)), true)]);
        let out = resume(&mut s, &utc(), LATE_WINDOW_MS, at(5, 6, 0));
        assert!(out.ring_late.is_empty());
        assert_eq!(s.alarms[0].next_fire_at, Some(at(5, 7, 0)));
    }

    /// The verdict is about the stored moment, so re-deriving first would erase
    /// what is being judged. This fails if the order is swapped.
    #[test]
    fn the_verdict_is_taken_before_the_anchor_is_redone() {
        let mut s = state(vec![alarm("a", "07:00", Some(at(5, 7, 0)), true)]);
        let out = resume(&mut s, &utc(), LATE_WINDOW_MS, at(5, 9, 0));
        assert_eq!(out.ring_late, vec!["a".to_string()]);
        // The fresh anchor is ahead, which is exactly what would have made the
        // verdict "pending" had it been computed first.
        assert!(s.alarms[0].next_fire_at.unwrap() > at(5, 9, 0));
    }

    /// Lost state must not become a ring: no anchor, nothing to be late for.
    #[test]
    fn a_state_with_no_anchors_rings_nothing_and_still_arms() {
        let mut s = state(vec![alarm("a", "07:00", None, true)]);
        let out = resume(&mut s, &utc(), LATE_WINDOW_MS, at(5, 9, 0));
        assert!(out.ring_late.is_empty());
        assert_eq!(s.alarms[0].next_fire_at, Some(at(6, 7, 0)));
    }

    #[test]
    fn a_week_of_downtime_re_arms_without_ringing() {
        let mut s = state(vec![alarm("a", "07:00", Some(at(5, 7, 0)), true)]);
        let out = resume(&mut s, &utc(), LATE_WINDOW_MS, at(12, 9, 0));
        assert!(out.ring_late.is_empty(), "far outside the late window");
        assert_eq!(s.alarms[0].next_fire_at, Some(at(13, 7, 0)));
    }

    #[test]
    fn a_disabled_alarm_neither_rings_nor_arms() {
        let mut a = alarm("a", "07:00", Some(at(5, 7, 0)), true);
        a.enabled = false;
        let mut s = state(vec![a]);
        let out = resume(&mut s, &utc(), LATE_WINDOW_MS, at(5, 9, 0));
        assert!(out.ring_late.is_empty());
        assert_eq!(s.alarms[0].next_fire_at, None);
        assert_eq!(out.wake_at, None);
    }

    /// Several late alarms are all reported, so the daemon decides what to do
    /// about a crowd rather than this silently picking one.
    #[test]
    fn every_late_alarm_is_named() {
        let mut s = state(vec![
            alarm("a", "07:00", Some(at(5, 7, 0)), true),
            alarm("b", "08:00", Some(at(5, 8, 0)), true),
            alarm("c", "08:30", Some(at(5, 8, 30)), false),
        ]);
        let out = resume(&mut s, &utc(), LATE_WINDOW_MS, at(5, 9, 0));
        assert_eq!(out.ring_late, vec!["a".to_string(), "b".to_string()]);
    }

    /// An anchor computed under different rules is replaced rather than
    /// honoured: the stored moment may predate an edit, a timezone change or a
    /// clock going back.
    #[test]
    fn a_stale_anchor_is_replaced_by_one_the_current_rules_produce() {
        // Stored as if it rings at 03:00, but the alarm says 07:00.
        let mut s = state(vec![alarm("a", "07:00", Some(at(6, 3, 0)), false)]);
        resume(&mut s, &utc(), LATE_WINDOW_MS, at(5, 9, 0));
        assert_eq!(s.alarms[0].next_fire_at, Some(at(6, 7, 0)));
    }
}

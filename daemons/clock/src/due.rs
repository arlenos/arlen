//! Moments arriving: what has come due, and what that does to the state.
//!
//! Every other module here answers "when"; this one answers "and then what".
//! Without it the anchors are correct and inert - a focus session whose phase
//! ran out sits in that phase for ever, and an alarm that rang keeps pointing at
//! the moment it already rang at, which makes it invisible to
//! [`crate::wake::next_wake_at`] (that ignores the past) so the machine is never
//! woken for it again. One alarm firing while the machine is awake would
//! otherwise silence that alarm for good.
//!
//! **A one-shot alarm disables itself.** The app labels an alarm with no
//! repeat days "Once" and [`crate::alarm::next_fire_at`] resolves an empty day
//! set to today-or-tomorrow, so re-deriving after it fires would arm it again
//! for tomorrow, and again the day after: "Once" would mean "every day, for
//! ever". Turning it off is what the label promises and what the toggle in the
//! app then shows.
//!
//! **Timers need nothing done to them.** A timer is its `ends_at`, so one that
//! has run out already reads as finished from the anchor alone. Removing it
//! would make it vanish at the exact moment it has something to say.

use crate::alarm::next_fire_at;
use crate::focus;
use crate::state::ClockState;

/// What arrived, in the order the state holds it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Due {
    /// Alarms whose moment came. Their ids, for whatever announces them.
    pub alarms: Vec<String>,
    /// Timers that ran out.
    pub timers: Vec<String>,
    /// Whether a focus phase ended, and what the session became: `Some(session)`
    /// for the phase after it, `None` when the session finished. Absent when no
    /// phase ended.
    pub focus: Option<Option<crate::state::FocusSession>>,
}

impl Due {
    /// Whether anything at all arrived, so a caller can skip the work of a
    /// moment that turned out to be nobody's.
    pub fn is_empty(&self) -> bool {
        self.alarms.is_empty() && self.timers.is_empty() && self.focus.is_none()
    }
}

/// Advance everything whose moment has arrived, and say what did.
///
/// `now_ms` is compared with `>=`, matching [`focus::phase_elapsed`]: the
/// instant an anchor names has arrived rather than being still ahead. The same
/// boundary everywhere is what stops a moment from being both due and not due
/// depending on which module is asked.
pub fn advance<Tz: chrono::TimeZone>(state: &mut ClockState, tz: &Tz, now_ms: i64) -> Due {
    let mut due = Due::default();

    for alarm in &mut state.alarms {
        let Some(at) = alarm.next_fire_at else {
            continue;
        };
        if at > now_ms {
            continue;
        }
        due.alarms.push(alarm.id.clone());
        if alarm.days.is_empty() {
            // "Once" - see the module note.
            alarm.enabled = false;
            alarm.next_fire_at = None;
        } else {
            alarm.next_fire_at = next_fire_at(alarm, tz, now_ms);
        }
    }

    for timer in &state.timers {
        if timer.ends_at.is_some_and(|at| at <= now_ms) {
            due.timers.push(timer.id.clone());
        }
    }

    if let Some(session) = &state.focus {
        if focus::phase_elapsed(session, now_ms) {
            let next = focus::advance(session, &state.focus_config, now_ms);
            due.focus = Some(next.clone());
            state.focus = next;
        }
    }

    due
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{Alarm, FocusConfig, FocusPhase, FocusSession, Timer};
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

    fn alarm(id: &str, time: &str, days: Vec<u8>, due: Option<i64>) -> Alarm {
        Alarm {
            id: id.into(),
            time: time.into(),
            label: String::new(),
            days,
            enabled: true,
            fire_late: true,
            next_fire_at: due,
        }
    }

    fn state(alarms: Vec<Alarm>) -> ClockState {
        ClockState {
            alarms,
            ..ClockState::default()
        }
    }

    /// The bug this module exists for: an alarm that fired while the machine was
    /// awake kept its old anchor, which `next_wake_at` ignores for being in the
    /// past, so the machine was never woken for that alarm again.
    #[test]
    fn a_repeating_alarm_that_fired_is_armed_for_its_next_day() {
        // Wednesday 5 August 2026; repeats Wednesday and Thursday.
        let mut s = state(vec![alarm("a", "07:00", vec![2, 3], Some(at(5, 7, 0)))]);
        let due = advance(&mut s, &utc(), at(5, 7, 0));
        assert_eq!(due.alarms, vec!["a".to_string()]);
        assert_eq!(s.alarms[0].next_fire_at, Some(at(6, 7, 0)));
        assert!(s.alarms[0].enabled, "a repeat is not spent by firing");
    }

    /// "Once" has to mean once. Re-deriving an empty day set would arm it for
    /// tomorrow, and the day after that, for ever.
    #[test]
    fn a_one_shot_alarm_turns_itself_off_after_it_fires() {
        let mut s = state(vec![alarm("a", "07:00", vec![], Some(at(5, 7, 0)))]);
        let due = advance(&mut s, &utc(), at(5, 7, 0));
        assert_eq!(due.alarms, vec!["a".to_string()]);
        assert!(!s.alarms[0].enabled);
        assert_eq!(s.alarms[0].next_fire_at, None);
    }

    #[test]
    fn an_alarm_still_ahead_is_left_alone() {
        let mut s = state(vec![alarm("a", "07:00", vec![], Some(at(5, 7, 0)))]);
        let due = advance(&mut s, &utc(), at(5, 6, 59));
        assert!(due.is_empty());
        assert_eq!(s.alarms[0].next_fire_at, Some(at(5, 7, 0)));
    }

    /// The instant an anchor names has arrived, not "is about to". The same
    /// boundary `focus::phase_elapsed` uses, so a moment cannot be due for one
    /// module and pending for another.
    #[test]
    fn the_named_instant_counts_as_arrived() {
        let mut s = state(vec![alarm("a", "07:00", vec![], Some(at(5, 7, 0)))]);
        assert_eq!(advance(&mut s, &utc(), at(5, 7, 0)).alarms.len(), 1);
    }

    #[test]
    fn a_disabled_alarm_has_no_anchor_and_so_never_arrives() {
        let mut a = alarm("a", "07:00", vec![], Some(at(5, 7, 0)));
        a.enabled = false;
        a.next_fire_at = None;
        let mut s = state(vec![a]);
        assert!(advance(&mut s, &utc(), at(5, 9, 0)).is_empty());
    }

    /// A timer is its anchor, so one that ran out reports itself and is left in
    /// place: removing it would make it vanish at the moment it has something to
    /// say.
    #[test]
    fn a_finished_timer_is_reported_and_kept() {
        let mut s = ClockState {
            timers: vec![Timer {
                id: "t".into(),
                duration_ms: 60_000,
                ends_at: Some(at(5, 7, 0)),
                remaining_ms: None,
                paused: false,
            }],
            ..ClockState::default()
        };
        let due = advance(&mut s, &utc(), at(5, 7, 0));
        assert_eq!(due.timers, vec!["t".to_string()]);
        assert_eq!(s.timers.len(), 1, "still there to be seen");
    }

    #[test]
    fn a_paused_timer_never_runs_out() {
        let mut s = ClockState {
            timers: vec![Timer {
                id: "t".into(),
                duration_ms: 60_000,
                ends_at: None,
                remaining_ms: Some(60_000),
                paused: true,
            }],
            ..ClockState::default()
        };
        assert!(advance(&mut s, &utc(), at(9, 9, 0)).is_empty());
    }

    /// Without this the session sticks in the phase that ran out and never
    /// reaches the break, which is the whole shape of a focus session.
    #[test]
    fn a_focus_phase_that_ran_out_becomes_the_next_one() {
        let mut s = ClockState {
            focus: Some(FocusSession {
                phase: FocusPhase::Focus,
                round: 1,
                rounds: 4,
                ends_at: at(5, 7, 0),
                held: vec![],
            }),
            focus_config: FocusConfig {
                focus_min: 25,
                break_min: 5,
                rounds: 4,
            },
            ..ClockState::default()
        };
        let due = advance(&mut s, &utc(), at(5, 7, 0));
        let session = s.focus.expect("still running, in its break");
        assert_eq!(session.phase, FocusPhase::Break);
        assert_eq!(session.round, 1);
        assert_eq!(due.focus, Some(Some(session)));
    }

    #[test]
    fn the_last_focus_phase_ends_the_session() {
        let mut s = ClockState {
            focus: Some(FocusSession {
                phase: FocusPhase::Focus,
                round: 4,
                rounds: 4,
                ends_at: at(5, 7, 0),
                held: vec![],
            }),
            ..ClockState::default()
        };
        let due = advance(&mut s, &utc(), at(5, 7, 0));
        assert_eq!(due.focus, Some(None));
        assert!(s.focus.is_none());
    }

    /// One pass advances one phase, so a daemon that was asleep for an hour does
    /// not silently run a whole focus session through in a single tick and
    /// report the end of something nobody sat through.
    #[test]
    fn a_long_gap_advances_one_phase_at_a_time() {
        let mut s = ClockState {
            focus: Some(FocusSession {
                phase: FocusPhase::Focus,
                round: 1,
                rounds: 4,
                ends_at: at(5, 7, 0),
                held: vec![],
            }),
            focus_config: FocusConfig {
                focus_min: 25,
                break_min: 5,
                rounds: 4,
            },
            ..ClockState::default()
        };
        advance(&mut s, &utc(), at(5, 9, 0));
        assert_eq!(s.focus.expect("running").round, 1, "one phase, not four");
    }

    /// Several things landing on the same instant all arrive, so a caller is
    /// never left to discover the second one on the next pass.
    #[test]
    fn everything_that_lands_on_one_instant_arrives_together() {
        let mut s = ClockState {
            alarms: vec![
                alarm("a", "07:00", vec![], Some(at(5, 7, 0))),
                alarm("b", "07:00", vec![], Some(at(5, 7, 0))),
            ],
            timers: vec![Timer {
                id: "t".into(),
                duration_ms: 1,
                ends_at: Some(at(5, 7, 0)),
                remaining_ms: None,
                paused: false,
            }],
            focus: Some(FocusSession {
                phase: FocusPhase::Focus,
                round: 4,
                rounds: 4,
                ends_at: at(5, 7, 0),
                held: vec![],
            }),
            ..ClockState::default()
        };
        let due = advance(&mut s, &utc(), at(5, 7, 0));
        assert_eq!(due.alarms.len(), 2);
        assert_eq!(due.timers.len(), 1);
        assert_eq!(due.focus, Some(None));
    }

    #[test]
    fn a_moment_that_is_nobodys_changes_nothing() {
        let mut s = state(vec![alarm("a", "07:00", vec![], Some(at(6, 7, 0)))]);
        let before = s.clone();
        assert!(advance(&mut s, &utc(), at(5, 9, 0)).is_empty());
        assert_eq!(s, before);
    }
}

//! Which moment the machine should be woken for.
//!
//! The clock daemon does not arm wakes itself. `org.arlen.Power1` owns suspend
//! policy and holds the one capability that lets a timer wake a sleeping
//! machine, and keeping it there is what stops the clock from becoming a second
//! place that decides when the machine sleeps. So this works out *when*, and the
//! power daemon is asked.
//!
//! **`ScheduleWake` has one slot: a second call replaces the first.** That is
//! the right shape - a machine has one next wake - but it means the caller has
//! to ask for the *earliest* moment rather than for each one as it is created.
//! Getting that backwards would arm the last alarm set rather than the next one
//! due, which fails in the most ordinary way there is: the evening alarm set
//! after the morning one silently cancels it.
//!
//! **Timers wake, the stopwatch does not.** A countdown is a promise about a
//! moment and the design gives it `CLOCK_BOOTTIME_ALARM` for exactly that
//! reason; a stopwatch has no moment to wake for - it measures the time
//! somebody was using the machine, and nobody is.

use crate::state::ClockState;

/// The next moment worth waking the machine for, as epoch milliseconds.
///
/// `None` when nothing is due - no armed alarm, no running timer, no focus
/// phase - in which case the daemon has no reason to hold a wake at all and
/// should not ask for one, so a machine with an empty clock sleeps like a
/// machine with no clock.
///
/// Anything already in the past is ignored rather than requested: the power
/// daemon refuses a wake in the past, and asking for one would turn a missed
/// alarm into an error at the moment the daemon is trying to recover from it.
pub fn next_wake_at(state: &ClockState, now_ms: i64) -> Option<i64> {
    let alarms = state
        .alarms
        .iter()
        .filter(|a| a.enabled)
        .filter_map(|a| a.next_fire_at);
    let timers = state.timers.iter().filter_map(|t| t.ends_at);
    let focus = state.focus.iter().map(|f| f.ends_at);

    alarms
        .chain(timers)
        .chain(focus)
        .filter(|at| *at > now_ms)
        .min()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{Alarm, FocusConfig, FocusPhase, FocusSession, Stopwatch, Timer};

    const HOUR: i64 = 60 * 60 * 1_000;

    fn alarm(id: &str, due: Option<i64>, enabled: bool) -> Alarm {
        Alarm {
            id: id.into(),
            time: "07:00".into(),
            label: String::new(),
            days: vec![],
            enabled,
            fire_late: false,
            next_fire_at: due,
        }
    }

    fn timer(ends_at: Option<i64>) -> Timer {
        Timer {
            id: "t".into(),
            duration_ms: HOUR,
            ends_at,
            remaining_ms: ends_at.is_none().then_some(HOUR),
            paused: ends_at.is_none(),
        }
    }

    fn state(alarms: Vec<Alarm>, timers: Vec<Timer>) -> ClockState {
        ClockState {
            wake_capable: true,
            alarms,
            timers,
            focus: None,
            focus_config: FocusConfig::default(),
            stopwatch: Stopwatch::default(),
            world: vec![],
        }
    }

    #[test]
    fn nothing_due_asks_for_no_wake() {
        assert_eq!(next_wake_at(&state(vec![], vec![]), HOUR), None);
    }

    /// The ordinary failure this exists to prevent: the evening alarm set after
    /// the morning one must not become the armed wake.
    #[test]
    fn the_earliest_moment_wins_regardless_of_the_order_they_were_set() {
        let s = state(
            vec![
                alarm("evening", Some(20 * HOUR), true),
                alarm("morning", Some(7 * HOUR), true),
            ],
            vec![],
        );
        assert_eq!(next_wake_at(&s, HOUR), Some(7 * HOUR));
    }

    #[test]
    fn a_disabled_alarm_is_not_a_reason_to_wake() {
        let s = state(
            vec![
                alarm("off", Some(7 * HOUR), false),
                alarm("on", Some(20 * HOUR), true),
            ],
            vec![],
        );
        assert_eq!(next_wake_at(&s, HOUR), Some(20 * HOUR));
    }

    /// A countdown is a promise about a moment, so it wakes the machine too.
    #[test]
    fn a_running_timer_is_a_reason_to_wake() {
        let s = state(
            vec![alarm("a", Some(20 * HOUR), true)],
            vec![timer(Some(2 * HOUR))],
        );
        assert_eq!(next_wake_at(&s, HOUR), Some(2 * HOUR));
    }

    /// A paused timer has no moment to wake for.
    #[test]
    fn a_paused_timer_is_not() {
        let s = state(vec![alarm("a", Some(20 * HOUR), true)], vec![timer(None)]);
        assert_eq!(next_wake_at(&s, HOUR), Some(20 * HOUR));
    }

    #[test]
    fn a_focus_phase_ending_is_a_reason_to_wake() {
        let mut s = state(vec![], vec![]);
        s.focus = Some(FocusSession {
            phase: FocusPhase::Focus,
            round: 1,
            rounds: 4,
            ends_at: 3 * HOUR,
            held: vec![],
        });
        assert_eq!(next_wake_at(&s, HOUR), Some(3 * HOUR));
    }

    /// A stopwatch measures the time somebody was using the machine, and
    /// nobody is.
    #[test]
    fn a_running_stopwatch_is_not_a_reason_to_wake() {
        let mut s = state(vec![], vec![]);
        s.stopwatch = Stopwatch {
            running: true,
            started_at: Some(0),
            accumulated_ms: 0,
            laps: vec![],
        };
        assert_eq!(next_wake_at(&s, HOUR), None);
    }

    /// A moment already gone is not asked for: the power daemon refuses a wake
    /// in the past, and a missed alarm should not surface as an error while the
    /// daemon is recovering from it.
    #[test]
    fn a_moment_already_past_is_not_requested() {
        let s = state(
            vec![
                alarm("missed", Some(2 * HOUR), true),
                alarm("next", Some(9 * HOUR), true),
            ],
            vec![],
        );
        assert_eq!(next_wake_at(&s, 5 * HOUR), Some(9 * HOUR));

        let only_missed = state(vec![alarm("missed", Some(2 * HOUR), true)], vec![]);
        assert_eq!(next_wake_at(&only_missed, 5 * HOUR), None);
    }

    /// The current instant has arrived rather than being ahead, so it is not
    /// something to be woken for - the same boundary the scheduler uses.
    #[test]
    fn the_present_instant_is_not_a_future_moment() {
        let s = state(vec![alarm("now", Some(5 * HOUR), true)], vec![]);
        assert_eq!(next_wake_at(&s, 5 * HOUR), None);
    }
}

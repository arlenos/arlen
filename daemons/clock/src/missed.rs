//! What to do about an alarm whose moment passed while nothing was running.
//!
//! A laptop shut at midnight and opened at nine has missed the seven o'clock
//! alarm. There are three answers and only one of them is right:
//!
//! - **Ring for every occurrence that was missed.** A week away and the machine
//!   greets you with seven alarms. Nobody wants this.
//! - **Say nothing.** The quiet answer, and the one that teaches a person their
//!   alarm cannot be relied on - the failure they discover is silence.
//! - **Ring once, late, if the alarm asked for it.** systemd's documented model
//!   for a missed timer, and the one this implements.
//!
//! **The stamp is the alarm's own `next_fire_at`**, which the daemon already
//! persists and the app already reads, rather than a second record of when it
//! last rang. That is not just economy: a separate stamp is exactly what goes
//! missing, and a lost stamp read as "it never fired" is how a catch-up rule
//! turns into the seven-alarm morning. With the anchor as the stamp, lost state
//! means no anchor, which means nothing to be late for.
//!
//! **And lateness is bounded.** systemd's model has no window because a service
//! that missed its window still wants to run; an alarm is a message to a person
//! at a moment, and the moment can pass. Ringing a fortnight later is not being
//! reliable, it is being wrong loudly.

use crate::state::Alarm;

/// How late an alarm may be and still ring.
///
/// Twelve hours: a machine shut overnight and opened in the morning still
/// rings, which is the case the whole feature exists for, and one opened after
/// a weekend away does not. The line has to be somewhere, and the honest place
/// is "would the person still want to hear it", which for a wall-clock alarm
/// stops being true within the same day.
pub const LATE_WINDOW_MS: i64 = 12 * 60 * 60 * 1_000;

/// What to do with an alarm at startup, or after a wake.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Missed {
    /// Its moment has not passed; leave it armed.
    Pending,
    /// Its moment passed, it asked to be told, and it is recent enough to
    /// matter. Ring once, then schedule the next one.
    FireLate,
    /// Its moment passed and it should not ring: either it did not ask to fire
    /// late, or too much time has gone by. Schedule the next one silently.
    Reschedule,
}

/// Decide what a missed alarm deserves.
///
/// `now_ms` is the wall clock. An alarm with no anchor is `Reschedule` rather
/// than `FireLate`: no anchor means the daemon does not know it was ever armed
/// for a moment that passed, and guessing yes is the failure mode that produces
/// alarms nobody set.
pub fn verdict(alarm: &Alarm, now_ms: i64, window_ms: i64) -> Missed {
    if !alarm.enabled {
        return Missed::Reschedule;
    }
    let Some(due) = alarm.next_fire_at else {
        return Missed::Reschedule;
    };
    if due > now_ms {
        return Missed::Pending;
    }
    if alarm.fire_late && now_ms - due <= window_ms {
        Missed::FireLate
    } else {
        Missed::Reschedule
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR: i64 = 60 * 60 * 1_000;

    fn alarm(due: Option<i64>, fire_late: bool) -> Alarm {
        Alarm {
            id: "a".into(),
            time: "07:00".into(),
            label: String::new(),
            days: vec![],
            enabled: true,
            fire_late,
            next_fire_at: due,
        }
    }

    #[test]
    fn an_alarm_still_ahead_is_left_alone() {
        assert_eq!(
            verdict(&alarm(Some(10 * HOUR), true), 9 * HOUR, LATE_WINDOW_MS),
            Missed::Pending
        );
    }

    /// The case the feature exists for: shut at midnight, opened at nine.
    #[test]
    fn an_alarm_missed_overnight_rings_once_when_it_asked_to() {
        assert_eq!(
            verdict(&alarm(Some(7 * HOUR), true), 9 * HOUR, LATE_WINDOW_MS),
            Missed::FireLate
        );
    }

    /// Opt-in: an alarm that did not ask for it is rescheduled in silence
    /// rather than surprising someone hours later.
    #[test]
    fn an_alarm_that_did_not_ask_to_fire_late_does_not() {
        assert_eq!(
            verdict(&alarm(Some(7 * HOUR), false), 9 * HOUR, LATE_WINDOW_MS),
            Missed::Reschedule
        );
    }

    /// Ringing a fortnight later is not reliability.
    #[test]
    fn an_alarm_missed_by_longer_than_the_window_stays_quiet() {
        let due = 7 * HOUR;
        assert_eq!(
            verdict(
                &alarm(Some(due), true),
                due + LATE_WINDOW_MS,
                LATE_WINDOW_MS
            ),
            Missed::FireLate,
            "exactly at the edge still counts"
        );
        assert_eq!(
            verdict(
                &alarm(Some(due), true),
                due + LATE_WINDOW_MS + 1,
                LATE_WINDOW_MS
            ),
            Missed::Reschedule
        );
    }

    /// **The failure mode the design names.** Lost state must not read as
    /// "never fired", or every past occurrence becomes a catch-up ring.
    #[test]
    fn an_alarm_with_no_anchor_does_not_invent_a_missed_ring() {
        assert_eq!(
            verdict(&alarm(None, true), 9 * HOUR, LATE_WINDOW_MS),
            Missed::Reschedule
        );
    }

    #[test]
    fn a_disabled_alarm_never_fires_late() {
        let mut a = alarm(Some(7 * HOUR), true);
        a.enabled = false;
        assert_eq!(verdict(&a, 9 * HOUR, LATE_WINDOW_MS), Missed::Reschedule);
    }

    /// One late ring, not one per missed occurrence: the verdict is about the
    /// single stored anchor, so a week of missed dailies is still one decision.
    #[test]
    fn a_week_of_missed_repeats_is_one_decision_not_seven() {
        let due = 7 * HOUR;
        let a = alarm(Some(due), true);
        // A week later the anchor is far outside the window, so the answer is
        // one silent reschedule rather than seven rings.
        assert_eq!(
            verdict(&a, due + 7 * 24 * HOUR, LATE_WINDOW_MS),
            Missed::Reschedule
        );
    }

    /// The moment itself has passed, by the same rule the scheduler uses.
    #[test]
    fn the_exact_due_instant_counts_as_due() {
        assert_eq!(
            verdict(&alarm(Some(7 * HOUR), true), 7 * HOUR, LATE_WINDOW_MS),
            Missed::FireLate
        );
    }
}

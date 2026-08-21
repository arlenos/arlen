// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

//! What to register with the clock, derived from what the files say.
//!
//! `calendar-app.md` section 4 draws the line this module sits on: the calendar
//! must NOT fire `VALARM` itself, "nor even by the calendar daemon's own
//! timers". It computes the next triggers for an expanded window and hands them
//! to `org.arlen.Clock1`, which owns arming, ringing eligibility and the
//! missed-alarm policy. The reason is not tidiness: `clock-app.md` records that
//! GNOME Clocks still cannot wake a suspended laptop, so a second timer path
//! inside the calendar would reproduce exactly that failure.
//!
//! Two rules the doc names, both load-bearing here:
//!
//! **Keyed by (UID, recurrence-id), re-derived on every store write.** A
//! reminder registered against an occurrence that is later moved or cancelled
//! must be findable and replaceable, which a free-floating timer is not. So this
//! is a pure function of the current events: call it again after any write and
//! register the difference.
//!
//! **A recurrence this machine cannot expand is reported, not guessed at.**
//! `rrule::expand` deliberately refuses the rules it does not model rather than
//! approximating them. Silently registering only the written occurrence's alarm
//! for a weekly meeting would leave somebody expecting a reminder every week and
//! getting one, once. So those UIDs come back in their own list and the surface
//! can say the series is not fully covered.

use chrono::{DateTime, Duration, NaiveDate, Utc};
use chrono_tz::Tz;

use crate::{rrule, when, CalTime, Event};

/// One alarm to arm: an instant, and the occurrence it belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Registration {
    /// The event's own identity.
    pub uid: String,
    /// Which occurrence this is for, as the date the occurrence starts on. This
    /// plus `uid` is the key section 4 requires: it survives a re-derivation, so
    /// a moved or cancelled occurrence drops its registration instead of leaving
    /// a timer nobody can find.
    pub recurrence_id: NaiveDate,
    /// When the alarm goes off.
    pub at: DateTime<Utc>,
    /// The event's title, carried so the clock has something to show without
    /// reading the calendar back.
    pub summary: String,
}

/// Everything derived for a window: what to arm, and what could not be worked
/// out.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Reminders {
    /// The alarms falling in the window, soonest first.
    pub due: Vec<Registration>,
    /// UIDs of repeating events whose rule this machine cannot expand, so their
    /// reminders cover only the occurrence the file writes. Reported rather than
    /// hidden: a person told nothing would assume every week was covered.
    pub unexpanded: Vec<String>,
}

/// Derive the registrations for `[from, to]`.
///
/// `local` resolves floating and all-day times, which are the reader's own clock
/// by definition. An alarm whose instant cannot be resolved - an unknown zone, a
/// wall-clock time its zone skips - is left out rather than moved to a nearby
/// one, on the same principle as the rest of this crate: not ringing is better
/// than ringing at a time nobody wrote.
#[must_use]
pub fn registrations(
    events: &[Event],
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    local: Tz,
) -> Reminders {
    let mut out = Reminders::default();
    // Occurrences are expanded over a wider span than the alarm window, because
    // an alarm can sit well before its event: a reminder a day ahead of a meeting
    // just outside the window is due INSIDE it. A day either side covers every
    // trigger anybody writes by hand without walking the rule further than it
    // needs.
    let span = Duration::days(1);
    let first = (from - span).date_naive();
    let last = (to + span).date_naive();

    for event in events {
        if event.alarms.is_empty() {
            continue;
        }
        let start_date = event.start.date();
        let dates = match event.rrule.as_deref() {
            None => vec![start_date],
            Some(rule) => match rrule::expand(rule, start_date, first, last) {
                Some(dates) => dates,
                None => {
                    // Refused, not empty: the machine does not model this rule.
                    out.unexpanded.push(event.uid.clone());
                    vec![start_date]
                }
            },
        };
        for date in dates {
            // The file's own corrections, honoured HERE as well as in the agenda.
            // The exclusion landed in `view.rs` first, and for a few hours the
            // week somebody called off was hidden from the list and still rang -
            // two surfaces disagreeing about the same file, which is worse than
            // either mistake alone.
            if event.exdates.contains(&date) {
                continue;
            }
            let start = on_date(&event.start, date);
            let end = event
                .end
                .as_ref()
                .map(|e| on_date(e, date + (e.date() - start_date)));
            let (Some(start_at), end_at) = (
                when::instant(&start, local),
                end.as_ref().and_then(|e| when::instant(e, local)),
            ) else {
                continue;
            };
            for at in when::alarm_times(event, start_at, end_at, local) {
                if at >= from && at <= to {
                    out.due.push(Registration {
                        uid: event.uid.clone(),
                        recurrence_id: date,
                        at,
                        summary: event.summary.clone(),
                    });
                }
            }
        }
    }
    out.due.sort_by(|a, b| (a.at, &a.uid).cmp(&(b.at, &b.uid)));
    out.unexpanded.sort();
    out.unexpanded.dedup();
    out
}

/// The same written time, moved to `date`, keeping its kind and zone.
///
/// The kind has to survive: an occurrence of a Tokyo-zoned event is still Tokyo
/// time, and flattening it to a local one would move the alarm by the offset.
fn on_date(t: &CalTime, date: NaiveDate) -> CalTime {
    match t {
        CalTime::Day(_) => CalTime::Day(date),
        CalTime::Floating(dt) => CalTime::Floating(date.and_time(dt.time())),
        CalTime::Utc(dt) => CalTime::Utc(date.and_time(dt.time())),
        CalTime::Zoned { at, tzid } => {
            CalTime::Zoned { at: date.and_time(at.time()), tzid: tzid.clone() }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_events;
    use chrono::TimeZone;

    fn at(y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, h, min, 0).unwrap()
    }

    const WEEKLY: &str = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:standup@x\r\n\
SUMMARY:Standup\r\nDTSTART:20260819T090000Z\r\nDTEND:20260819T091500Z\r\n\
RRULE:FREQ=WEEKLY;BYDAY=WE\r\nBEGIN:VALARM\r\nACTION:DISPLAY\r\nTRIGGER:-PT15M\r\n\
END:VALARM\r\nEND:VEVENT\r\nEND:VCALENDAR";

    #[test]
    fn every_occurrence_in_the_window_gets_its_own_registration() {
        let events = parse_events(WEEKLY).expect("parses");
        let r = registrations(&events, at(2026, 8, 19, 0, 0), at(2026, 9, 2, 23, 59), Tz::UTC);
        // Three Wednesdays, each with its own key and its own instant.
        assert_eq!(r.due.len(), 3);
        assert_eq!(r.due[0].at, at(2026, 8, 19, 8, 45));
        assert_eq!(r.due[1].at, at(2026, 8, 26, 8, 45));
        assert_eq!(r.due[2].at, at(2026, 9, 2, 8, 45));
        assert_eq!(
            r.due[1].recurrence_id,
            NaiveDate::from_ymd_opt(2026, 8, 26).unwrap(),
            "the key names the occurrence, so a later move can drop just this one"
        );
        assert!(r.unexpanded.is_empty());
    }

    #[test]
    fn the_week_somebody_called_off_does_not_ring() {
        // The agenda hides that week; if the alarm still fired, the two surfaces
        // would disagree about the same file, which is worse than either mistake
        // on its own.
        let ics = WEEKLY.replace(
            "RRULE:FREQ=WEEKLY;BYDAY=WE\r\n",
            "RRULE:FREQ=WEEKLY;BYDAY=WE\r\nEXDATE:20260826T090000Z\r\n",
        );
        let events = parse_events(&ics).expect("parses");
        let r = registrations(&events, at(2026, 8, 19, 0, 0), at(2026, 9, 2, 23, 59), Tz::UTC);
        assert_eq!(r.due.len(), 2);
        assert_eq!(r.due[0].at, at(2026, 8, 19, 8, 45));
        assert_eq!(r.due[1].at, at(2026, 9, 2, 8, 45), "and the series carries on after it");
    }

    #[test]
    fn an_alarm_outside_the_window_is_not_registered() {
        let events = parse_events(WEEKLY).expect("parses");
        // A window starting after the first alarm and ending before the second.
        let r = registrations(&events, at(2026, 8, 19, 9, 0), at(2026, 8, 20, 0, 0), Tz::UTC);
        assert!(r.due.is_empty());
    }

    #[test]
    fn an_alarm_before_the_window_but_for_an_event_inside_it_is_still_found() {
        // A day-ahead reminder for a meeting near the window's end. Expanding
        // only the window itself would miss the occurrence whose alarm is due.
        let ics = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:a\r\nSUMMARY:Trip\r\n\
DTSTART:20260821T090000Z\r\nBEGIN:VALARM\r\nTRIGGER:-P1D\r\nEND:VALARM\r\n\
END:VEVENT\r\nEND:VCALENDAR";
        let events = parse_events(ics).expect("parses");
        let r = registrations(&events, at(2026, 8, 20, 0, 0), at(2026, 8, 20, 23, 0), Tz::UTC);
        assert_eq!(r.due.len(), 1);
        assert_eq!(r.due[0].at, at(2026, 8, 20, 9, 0));
    }

    #[test]
    fn a_rule_this_machine_cannot_expand_is_named_rather_than_guessed_at() {
        // BYSETPOS is one `rrule::expand` refuses. Registering only the written
        // occurrence silently would leave somebody expecting a monthly reminder
        // and getting one, once.
        let ics = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:board@x\r\nSUMMARY:Board\r\n\
DTSTART:20260819T090000Z\r\nRRULE:FREQ=MONTHLY;BYDAY=WE;BYSETPOS=3\r\n\
BEGIN:VALARM\r\nTRIGGER:-PT15M\r\nEND:VALARM\r\nEND:VEVENT\r\nEND:VCALENDAR";
        let events = parse_events(ics).expect("parses");
        let r = registrations(&events, at(2026, 8, 1, 0, 0), at(2026, 12, 1, 0, 0), Tz::UTC);
        assert_eq!(r.unexpanded, vec!["board@x".to_string()]);
        assert_eq!(r.due.len(), 1, "only the occurrence the file writes is covered");
    }

    #[test]
    fn an_event_with_no_alarm_registers_nothing() {
        let ics = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:a\r\nDTSTART:20260819T090000Z\r\n\
END:VEVENT\r\nEND:VCALENDAR";
        let events = parse_events(ics).expect("parses");
        assert!(registrations(&events, at(2026, 8, 1, 0, 0), at(2026, 9, 1, 0, 0), Tz::UTC)
            .due
            .is_empty());
    }

    #[test]
    fn a_zoned_occurrence_keeps_its_zone_rather_than_the_readers() {
        // 09:00 in Vienna is 07:00 UTC in August, so a 15-minute warning is
        // 06:45 UTC. Flattening the occurrence to local time would move it.
        let ics = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:a\r\nSUMMARY:Wien\r\n\
DTSTART;TZID=Europe/Vienna:20260819T090000\r\nBEGIN:VALARM\r\nTRIGGER:-PT15M\r\n\
END:VALARM\r\nEND:VEVENT\r\nEND:VCALENDAR";
        let events = parse_events(ics).expect("parses");
        let r = registrations(&events, at(2026, 8, 19, 0, 0), at(2026, 8, 20, 0, 0), Tz::UTC);
        assert_eq!(r.due.len(), 1);
        assert_eq!(r.due[0].at, at(2026, 8, 19, 6, 45));
    }
}

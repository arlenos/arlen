// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Which meetings are about to start, said once each.
//!
//! `arlen-roadmap.md` records the agent behaviour `meeting-prep` as **dead**:
//! it triggers on `calendar.event.upcoming` and nothing has ever emitted that,
//! so it can never fire. `calendar-app.md` section 2 puts the event-bus
//! projection on this daemon, which makes this the missing producer - and the
//! reason the calendar is not merely another app but the source that lets an
//! already-built behaviour run at all.
//!
//! The whole difficulty is SAYING IT ONCE. The store is re-read on a timer, so
//! a meeting inside the lead window is due again at every pass; emitting each
//! time would wake the agent every minute for one meeting. So an occurrence is
//! remembered by (uid, occurrence date) once announced, which is the same key
//! the clock registrations use and survives the same things: a series keeps its
//! occurrences apart, and a moved one is a different key rather than a silent
//! duplicate.

use std::collections::BTreeSet;

use arlen_calendar_core::{view, when, Event};
use chrono::{DateTime, NaiveDate, Utc};
use chrono_tz::Tz;

/// A meeting about to start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Upcoming {
    /// The event's own identity.
    pub uid: String,
    /// Which occurrence, as the date it starts on.
    pub recurrence_id: NaiveDate,
    /// When it starts.
    pub at: DateTime<Utc>,
    /// Its title, as written.
    pub summary: String,
    /// Where, as written; empty when the file said nothing.
    pub location: String,
}

/// What has already been said, so it is not said again.
#[derive(Debug, Clone, Default)]
pub struct Announced(BTreeSet<(String, NaiveDate)>);

impl Announced {
    /// Whether this occurrence has been announced.
    #[must_use]
    pub fn contains(&self, u: &Upcoming) -> bool {
        self.0.contains(&(u.uid.clone(), u.recurrence_id))
    }

    /// Remember one.
    pub fn remember(&mut self, u: &Upcoming) {
        self.0.insert((u.uid.clone(), u.recurrence_id));
    }

    /// Forget occurrences whose day is behind `today`.
    ///
    /// Without this the set grows for as long as the daemon runs, and a daily
    /// meeting would add an entry a day for ever. Keyed on the day rather than
    /// the instant so a machine that slept through a meeting still drops it.
    pub fn forget_before(&mut self, today: NaiveDate) {
        self.0.retain(|(_, on)| *on >= today);
    }

    /// How many occurrences are remembered.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether nothing is remembered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// The occurrences starting in `(now, now + lead]`, soonest first.
///
/// Strictly ahead of `now`: a meeting that has already started is not upcoming,
/// and announcing it would prepare somebody for something they are already late
/// for. An occurrence whose instant cannot be resolved - a zone this machine
/// does not know, a wall-clock time its zone skips - is left out rather than
/// guessed at, on the same principle as everything else that reads these files.
#[must_use]
pub fn due(events: &[Event], now: DateTime<Utc>, lead: chrono::Duration, local: Tz) -> Vec<Upcoming> {
    let today = now.with_timezone(&local).date_naive();
    let mut out = Vec::new();
    for event in events {
        for date in view::occurrences(event, today) {
            let start = shift_to(&event.start, date);
            let Some(at) = when::instant(&start, local) else {
                continue;
            };
            let ahead = at - now;
            if ahead > chrono::Duration::zero() && ahead <= lead {
                out.push(Upcoming {
                    uid: event.uid.clone(),
                    recurrence_id: date,
                    at,
                    summary: event.summary.clone(),
                    location: event.location.clone(),
                });
            }
        }
    }
    out.sort_by(|a, b| (a.at, &a.uid).cmp(&(b.at, &b.uid)));
    out
}

/// The same written time, moved to `date`, keeping its kind and zone.
fn shift_to(t: &arlen_calendar_core::CalTime, date: NaiveDate) -> arlen_calendar_core::CalTime {
    use arlen_calendar_core::CalTime;
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
    use arlen_calendar_core::parse_events;
    use chrono::TimeZone;

    fn at(h: u32, m: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, 19, h, m, 0).unwrap()
    }

    const ONE: &str = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:standup@x\r\nSUMMARY:Standup\r\n\
LOCATION:Room 2\r\nDTSTART:20260819T090000Z\r\nEND:VEVENT\r\nEND:VCALENDAR";

    #[test]
    fn a_meeting_inside_the_window_is_due_and_one_outside_is_not() {
        let events = parse_events(ONE).expect("parses");
        let lead = chrono::Duration::minutes(30);
        let soon = due(&events, at(8, 45), lead, Tz::UTC);
        assert_eq!(soon.len(), 1);
        assert_eq!(soon[0].summary, "Standup");
        assert_eq!(soon[0].location, "Room 2");
        assert!(due(&events, at(8, 0), lead, Tz::UTC).is_empty(), "still an hour out");
    }

    #[test]
    fn a_meeting_that_has_started_is_not_upcoming() {
        let events = parse_events(ONE).expect("parses");
        // Announcing it would prepare somebody for something they are late for.
        assert!(due(&events, at(9, 0), chrono::Duration::minutes(30), Tz::UTC).is_empty());
        assert!(due(&events, at(9, 5), chrono::Duration::minutes(30), Tz::UTC).is_empty());
    }

    #[test]
    fn the_same_occurrence_is_only_said_once() {
        // The case this module exists for: the store is re-read on a timer, so
        // without a memory one meeting would wake the agent every pass.
        let events = parse_events(ONE).expect("parses");
        let lead = chrono::Duration::minutes(30);
        let mut said = Announced::default();
        let first = due(&events, at(8, 45), lead, Tz::UTC);
        let fresh: Vec<_> = first.iter().filter(|u| !said.contains(u)).collect();
        assert_eq!(fresh.len(), 1);
        for u in first {
            said.remember(&u);
        }
        let again = due(&events, at(8, 50), lead, Tz::UTC);
        assert_eq!(again.len(), 1, "still due");
        assert!(again.iter().all(|u| said.contains(u)), "and already said");
    }

    #[test]
    fn each_occurrence_of_a_series_is_its_own_announcement() {
        let ics = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:s@x\r\nSUMMARY:Standup\r\n\
DTSTART:20260819T090000Z\r\nRRULE:FREQ=DAILY\r\nEND:VEVENT\r\nEND:VCALENDAR";
        let events = parse_events(ics).expect("parses");
        let lead = chrono::Duration::minutes(30);
        let mut said = Announced::default();
        for u in due(&events, at(8, 45), lead, Tz::UTC) {
            said.remember(&u);
        }
        // Tomorrow's occurrence is a different key, so it is announced on its
        // own day rather than swallowed by yesterday's.
        let tomorrow = Utc.with_ymd_and_hms(2026, 8, 20, 8, 45, 0).unwrap();
        let next = due(&events, tomorrow, lead, Tz::UTC);
        assert_eq!(next.len(), 1);
        assert!(!said.contains(&next[0]));
    }

    #[test]
    fn the_memory_does_not_grow_for_ever() {
        let events = parse_events(ONE).expect("parses");
        let mut said = Announced::default();
        for u in due(&events, at(8, 45), chrono::Duration::minutes(30), Tz::UTC) {
            said.remember(&u);
        }
        assert_eq!(said.len(), 1);
        said.forget_before(NaiveDate::from_ymd_opt(2026, 8, 20).unwrap());
        assert!(said.is_empty(), "yesterday's meetings are not coming back");
    }
}

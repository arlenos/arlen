// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

//! An agenda: events flattened into the rows a surface draws.
//!
//! Here rather than in the app because two things now build it. The app builds
//! one for the file it was opened on, and the calendar daemon builds one for the
//! whole store; a surface that got a different shape depending on which asked
//! would be two calendars wearing one face.
//!
//! What it deliberately does NOT do is resolve zones. The rows carry the time
//! the file wrote plus WHICH KIND of time it is, and the surface says so - a
//! reader in Vienna seeing a Tokyo meeting is told it is Tokyo time rather than
//! shown a number that silently means something else. Ordering events written in
//! different zones against each other needs the zone database and is its own
//! step; doing it by string comparison would be a guess dressed as an answer.

use chrono::{NaiveDate, Weekday};
use serde::{Deserialize, Serialize};

use crate::{rrule, CalTime, Event};

/// How far the agenda draws GENERATED occurrences, either side of today.
///
/// A written event is a fact in a file and is shown whenever it falls; a repeat
/// is generated, and generation without bounds is infinite - "every Monday for
/// ever" has no last row to draw. Backwards as well as forwards, because an
/// agenda that hid this Monday's standup on Tuesday would be answering a
/// question nobody asked.
///
/// The numbers are a choice rather than a law, which is why they are named: far
/// enough that somebody planning a quarter sees their meetings, near enough that
/// a daily rule is a few hundred rows.
pub const REPEAT_BACK_DAYS: i64 = 30;
/// See [`REPEAT_BACK_DAYS`].
pub const REPEAT_AHEAD_DAYS: i64 = 120;

/// One event, flattened into a row.
///
/// The times are strings in the file's own terms plus the KIND of time they are,
/// because the three forms mean different things and the surface has to be able
/// to say which one it is showing. Collapsing them here would undo the care the
/// parser takes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgendaEvent {
    /// The event's own identity.
    pub uid: String,
    /// Its title, as written.
    pub summary: String,
    /// Where, as written; empty when the file said nothing.
    pub location: String,
    /// `YYYY-MM-DD` in the event's own terms.
    pub date: String,
    /// `HH:MM`, or absent for an all-day event.
    pub time: Option<String>,
    /// `HH:MM` of the end, when the file gave one.
    pub end_time: Option<String>,
    /// `day`, `floating`, `utc` or `zoned`.
    pub kind: String,
    /// The zone name for a `zoned` time, so a surface can show it rather than
    /// pretending the reader is in it.
    pub tzid: Option<String>,
    /// True when the event carries an RRULE.
    pub repeats: bool,
    /// How often it repeats, when the rule is one this understands: `daily`,
    /// `weekly`, `monthly` or `yearly`. Absent when the event does not repeat or
    /// when its rule is refused.
    ///
    /// DATA, not a sentence, for the same reason mail's divergence notice is:
    /// "Repeats" alone told a reader nothing they could act on - a standup every
    /// weekday and a birthday every year both said it - and the sentence that
    /// makes it useful has to be written in their language, not in Rust.
    pub every: Option<String>,
    /// The `INTERVAL` of that rule: every 1 week, every 2 weeks. 1 unless the
    /// file says otherwise.
    pub every_n: u32,
    /// The weekdays a weekly rule fires on, as `mon`..`sun`, when it names any.
    pub on_days: Vec<String>,
    /// True when THIS row is one the calendar worked out from the rule. False on
    /// a repeating event whose rule `rrule` refuses - that row is the one date
    /// the file names, and the surface has to say so rather than implying the
    /// series is drawn.
    pub expanded: bool,
}

/// One read of a calendar source, and what it could and could not read.
///
/// The counts are separate on purpose. No files and no events are different
/// states - nothing has been put here, versus what is here holds nothing - and a
/// surface that draws both as empty space tells a first-time reader their
/// calendar is broken. A file that could not be read is counted rather than
/// quietly missing, because an agenda short one file with no sign of it is worse
/// than one that says so.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Agenda {
    /// The rows to draw.
    pub events: Vec<AgendaEvent>,
    /// What was read: the calendar directory, or the single file the app was
    /// opened on. Carried so the surface can NAME it when there is nothing to
    /// show - telling somebody to put files "somewhere" is not an instruction.
    pub directory: String,
    /// Whether that path is there at all.
    pub directory_exists: bool,
    /// How many calendar files were found.
    pub files: usize,
    /// How many could not be read or parsed.
    pub unreadable: usize,
    /// Whether the calendar service answered.
    ///
    /// False means the app read the files itself, which shows the same agenda
    /// but arms no reminders - the daemon is what registers them with the clock.
    /// Those are two different facts and the surface has to be able to say the
    /// second without hiding the first.
    pub service_running: bool,
}

/// Which of the three written forms a time is, and its zone when it names one.
#[must_use]
pub fn kind_of(t: &CalTime) -> (&'static str, Option<String>) {
    match t {
        CalTime::Day(_) => ("day", None),
        CalTime::Floating(_) => ("floating", None),
        CalTime::Utc(_) => ("utc", None),
        CalTime::Zoned { tzid, .. } => ("zoned", Some(tzid.clone())),
    }
}

/// Every date this event actually falls on, inside the window around `today`.
///
/// A non-repeating event is its own single date. A repeat this machine can work
/// out becomes one date per occurrence. A repeat it CANNOT work out - the rules
/// `rrule` refuses - stays a single date that still says it repeats: better a
/// row that admits it does not know than rows on days nobody agreed to.
#[must_use]
pub fn occurrences(e: &Event, today: NaiveDate) -> Vec<NaiveDate> {
    let start = e.start.date();
    let Some(rule) = e.rrule.as_deref() else {
        return vec![start];
    };
    let from = today - chrono::Duration::days(REPEAT_BACK_DAYS);
    let to = today + chrono::Duration::days(REPEAT_AHEAD_DAYS);
    let dates = match rrule::expand(rule, start, from, to) {
        Some(dates) if !dates.is_empty() => dates,
        // Refused, or a series that has ended before the window: the event is
        // still real and still says it repeats.
        _ => vec![start],
    };
    // The file's own corrections. An EXDATE is the calendar saying "not that
    // week", and an agenda that draws it anyway is showing a meeting somebody
    // cancelled. Applied to the refused case too: the one date shown there is
    // the date the file names, and the same file may have called it off.
    dates.into_iter().filter(|d| !e.exdates.contains(d)).collect()
}

/// The stable key for a frequency, for a surface to translate.
fn freq_key(f: rrule::Freq) -> &'static str {
    match f {
        rrule::Freq::Daily => "daily",
        rrule::Freq::Weekly => "weekly",
        rrule::Freq::Monthly => "monthly",
        rrule::Freq::Yearly => "yearly",
    }
}

/// The stable key for a weekday. Not a name: a name is the surface's job and
/// depends on the reader's language.
fn weekday_key(d: Weekday) -> &'static str {
    match d {
        Weekday::Mon => "mon",
        Weekday::Tue => "tue",
        Weekday::Wed => "wed",
        Weekday::Thu => "thu",
        Weekday::Fri => "fri",
        Weekday::Sat => "sat",
        Weekday::Sun => "sun",
    }
}

/// One event as the row for `on`.
#[must_use]
pub fn flatten(e: &Event, on: NaiveDate, expanded: bool) -> AgendaEvent {
    let (kind, tzid) = kind_of(&e.start);
    // Parsed here rather than carried from `rows`, because a row is also built
    // for an event whose rule `rrule` refuses - and a refused rule must leave
    // these fields empty rather than guessed at, which `parse_rule` does by
    // returning `None`.
    let rule = e.rrule.as_deref().and_then(rrule::parse_rule);
    AgendaEvent {
        uid: e.uid.clone(),
        summary: e.summary.clone(),
        location: e.location.clone(),
        date: on.format("%Y-%m-%d").to_string(),
        time: e.start.time().map(|t| t.format("%H:%M").to_string()),
        end_time: e
            .end
            .as_ref()
            .and_then(|t| t.time())
            .map(|t| t.format("%H:%M").to_string()),
        kind: kind.to_string(),
        tzid,
        repeats: e.repeats(),
        every: rule.as_ref().map(|r| freq_key(r.freq).to_string()),
        every_n: rule.as_ref().map_or(1, |r| r.interval),
        on_days: rule
            .as_ref()
            .map(|r| r.by_day.iter().map(|d| weekday_key(*d).to_string()).collect())
            .unwrap_or_default(),
        expanded,
    }
}

/// Day, then time of day, then title: the order an agenda is read in.
pub fn sort_events(events: &mut [AgendaEvent]) {
    events.sort_by(|a, b| (&a.date, &a.time, &a.summary).cmp(&(&b.date, &b.time, &b.summary)));
}

/// Every event as its rows, sorted.
#[must_use]
pub fn rows(events: &[Event], today: NaiveDate) -> Vec<AgendaEvent> {
    let mut out = Vec::new();
    for e in events {
        let dates = occurrences(e, today);
        // One date back from a repeating event means the rule was refused or the
        // series is over, and the row must not claim to be a worked-out one.
        let expanded = e.repeats() && dates.len() > 1;
        out.extend(dates.into_iter().map(|on| flatten(e, on, expanded)));
    }
    sort_events(&mut out);
    out
}

#[cfg(test)]
mod tests {

    /// The week somebody called off is not on the agenda.
    #[test]
    fn an_excluded_date_is_left_out_of_the_series() {
        let ics = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:s@x\r\nSUMMARY:Standup\r\n\
DTSTART:20260819T090000Z\r\nRRULE:FREQ=WEEKLY;COUNT=4\r\nEXDATE:20260826T090000Z\r\n\
END:VEVENT\r\nEND:VCALENDAR";
        let events = parse_events(ics).expect("parses");
        let rows = rows(&events, day(2026, 8, 19));
        let dates: Vec<&str> = rows.iter().map(|r| r.date.as_str()).collect();
        assert!(dates.contains(&"2026-08-19"), "the first is still there");
        assert!(!dates.contains(&"2026-08-26"), "the excluded week is not");
        assert!(dates.contains(&"2026-09-02"), "and the series carries on after it");
    }

    /// Several dates on one line, which is how a file usually writes them.
    #[test]
    fn a_list_of_excluded_dates_is_read_whole() {
        let ics = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:s@x\r\nSUMMARY:Standup\r\n\
DTSTART;VALUE=DATE:20260819\r\nRRULE:FREQ=DAILY;COUNT=4\r\n\
EXDATE;VALUE=DATE:20260820,20260821\r\nEND:VEVENT\r\nEND:VCALENDAR";
        let rows = rows(&parse_events(ics).expect("parses"), day(2026, 8, 19));
        let dates: Vec<&str> = rows.iter().map(|r| r.date.as_str()).collect();
        assert_eq!(dates, vec!["2026-08-19", "2026-08-22"]);
    }

    /// A weekday standup says which days, not merely that it repeats.
    #[test]
    fn a_weekly_rule_names_its_days() {
        let ics = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:s@x\r\nSUMMARY:Standup\r\n\
DTSTART;TZID=Europe/Vienna:20260819T090000\r\nRRULE:FREQ=WEEKLY;BYDAY=MO,WE\r\n\
END:VEVENT\r\nEND:VCALENDAR";
        let events = parse_events(ics).expect("parses");
        let rows = rows(&events, day(2026, 8, 19));
        assert_eq!(rows[0].every.as_deref(), Some("weekly"));
        assert_eq!(rows[0].every_n, 1);
        assert_eq!(rows[0].on_days, vec!["mon".to_string(), "wed".to_string()]);
    }

    /// An interval survives to the surface, because "every 2 weeks" and "weekly"
    /// are different facts about somebody's month.
    #[test]
    fn an_interval_reaches_the_row() {
        let ics = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:r@x\r\nSUMMARY:Retro\r\n\
DTSTART:20260819T090000Z\r\nRRULE:FREQ=WEEKLY;INTERVAL=2\r\nEND:VEVENT\r\n\
END:VCALENDAR";
        let events = parse_events(ics).expect("parses");
        let rows = rows(&events, day(2026, 8, 19));
        assert_eq!(rows[0].every_n, 2);
        assert!(rows[0].on_days.is_empty(), "the rule names no days");
    }

    /// A rule this cannot work out leaves the fields EMPTY rather than guessed.
    #[test]
    fn a_refused_rule_says_nothing_about_how_often() {
        let ics = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:o@x\r\nSUMMARY:Board\r\n\
DTSTART:20260819T090000Z\r\nRRULE:FREQ=MONTHLY;BYDAY=WE;BYSETPOS=3\r\nEND:VEVENT\r\n\
END:VCALENDAR";
        let events = parse_events(ics).expect("parses");
        let rows = rows(&events, day(2026, 8, 19));
        assert!(rows[0].repeats, "it does repeat");
        assert_eq!(rows[0].every, None, "but how often is not known");
    }

    use super::*;
    use crate::parse_events;

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    const WEEKLY: &str = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:s@x\r\nSUMMARY:Standup\r\n\
DTSTART;TZID=Europe/Vienna:20260819T090000\r\nRRULE:FREQ=WEEKLY;BYDAY=WE\r\n\
END:VEVENT\r\nEND:VCALENDAR";

    #[test]
    fn a_repeating_event_becomes_one_row_per_occurrence_and_says_so() {
        let events = parse_events(WEEKLY).expect("parses");
        let rows = rows(&events, day(2026, 8, 19));
        assert!(rows.len() > 4, "a weekly rule fills the window");
        assert!(rows.iter().all(|r| r.repeats && r.expanded));
        assert_eq!(rows[0].date, "2026-08-19");
        assert_eq!(rows[0].time.as_deref(), Some("09:00"));
        // The zone travels with the row rather than being resolved away.
        assert_eq!(rows[0].kind, "zoned");
        assert_eq!(rows[0].tzid.as_deref(), Some("Europe/Vienna"));
    }

    #[test]
    fn a_rule_this_machine_refuses_stays_one_row_that_does_not_claim_to_be_drawn() {
        let ics = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:b@x\r\nSUMMARY:Board\r\n\
DTSTART:20260819T090000Z\r\nRRULE:FREQ=MONTHLY;BYDAY=WE;BYSETPOS=3\r\nEND:VEVENT\r\n\
END:VCALENDAR";
        let events = parse_events(ics).expect("parses");
        let rows = rows(&events, day(2026, 8, 19));
        assert_eq!(rows.len(), 1);
        assert!(rows[0].repeats, "it does repeat, and the surface says so");
        assert!(!rows[0].expanded, "but this row is the date the file names");
    }

    #[test]
    fn an_all_day_entry_has_no_time_rather_than_midnight() {
        let ics = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:h@x\r\nSUMMARY:Holiday\r\n\
DTSTART;VALUE=DATE:20260820\r\nEND:VEVENT\r\nEND:VCALENDAR";
        let rows = rows(&parse_events(ics).expect("parses"), day(2026, 8, 19));
        assert_eq!(rows[0].time, None);
        assert_eq!(rows[0].kind, "day");
    }

    #[test]
    fn rows_are_read_in_the_order_an_agenda_is_read() {
        let ics = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:b\r\nSUMMARY:Later\r\n\
DTSTART:20260819T140000Z\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nUID:a\r\nSUMMARY:Earlier\r\n\
DTSTART:20260819T090000Z\r\nEND:VEVENT\r\nEND:VCALENDAR";
        let rows = rows(&parse_events(ics).expect("parses"), day(2026, 8, 19));
        assert_eq!(rows[0].summary, "Earlier");
        assert_eq!(rows[1].summary, "Later");
    }
}

//! Reading iCalendar, without deciding what time means.
//!
//! RFC 5545 is the interop floor and there is no way around it
//! (`calendar-app.md` §1). This reads the part of it a calendar needs to SHOW
//! you something: the events in a `.ics` file, with their times kept in the form
//! the file wrote them.
//!
//! **The rule that shapes everything here: never normalise to UTC.** iCalendar
//! has three date-time forms - floating local time, UTC, and a zoned time
//! carrying a TZID - and they mean different things. "09:00 daily standup" as a
//! floating time follows you when you fly; as `Europe/Vienna` it does not; as
//! UTC it is the same instant everywhere. Collapsing the three at parse time
//! picks one of those answers silently and the original is then unrecoverable,
//! so [`CalTime`] keeps them apart and the resolution to an instant happens where
//! it is needed, on purpose, with the zone named.
//!
//! What this deliberately does NOT do is expand recurrence. `RRULE` is kept
//! verbatim and flagged, because expanding it correctly is its own body of work
//! (§1's "recurrence hell") and a calendar that silently showed only the first
//! occurrence of a weekly meeting would be worse than one that says the event
//! repeats and it cannot yet say when.

pub mod rrule;

use chrono::{NaiveDate, NaiveDateTime, NaiveTime};

/// A date-time as the file wrote it.
///
/// Not one type with a flag: a flag invites `if utc { ... }` and a default
/// branch that quietly treats floating as local, which is the bug this models
/// away.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CalTime {
    /// A whole day, from `VALUE=DATE`. Has no time of day at all, so giving it
    /// midnight would be an invention.
    Day(NaiveDate),
    /// Wall-clock time with no zone: it means the same reading of a clock
    /// wherever the reader is.
    Floating(NaiveDateTime),
    /// An absolute instant, written with a trailing `Z`.
    Utc(NaiveDateTime),
    /// Wall-clock time in a named zone. The name is kept as written; resolving
    /// it against a zone database is the caller's step and not the parser's.
    Zoned { at: NaiveDateTime, tzid: String },
}

impl CalTime {
    /// The wall-clock date this falls on, in its own terms.
    ///
    /// Enough to group an agenda by day WITHOUT resolving zones, which is the
    /// honest thing to show before a zone database is wired in: an event written
    /// in Tokyo time is listed on the date Tokyo would call it.
    pub fn date(&self) -> NaiveDate {
        match self {
            Self::Day(d) => *d,
            Self::Floating(dt) | Self::Utc(dt) | Self::Zoned { at: dt, .. } => dt.date(),
        }
    }

    /// The time of day, or `None` for an all-day entry.
    pub fn time(&self) -> Option<NaiveTime> {
        match self {
            Self::Day(_) => None,
            Self::Floating(dt) | Self::Utc(dt) | Self::Zoned { at: dt, .. } => Some(dt.time()),
        }
    }
}

/// One VEVENT, as far as this reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    /// The file's own identity for this event. Empty when the file omitted it,
    /// which is malformed but common; the caller decides whether to care.
    pub uid: String,
    pub summary: String,
    pub start: CalTime,
    /// The end, when the file gave one. `DTEND` and `DURATION` are mutually
    /// exclusive in the specification, so both land here.
    pub end: Option<CalTime>,
    pub location: String,
    /// The `RRULE` line verbatim, when the event repeats. NOT expanded: see the
    /// module note.
    pub rrule: Option<String>,
    /// The event's `VALARM` blocks, in file order.
    ///
    /// Parsed, never fired. `calendar-app.md` section 4 is explicit that neither
    /// this app nor a calendar daemon may own the timer: the trigger is computed
    /// here and registered with `org.arlen.Clock1`, which is the only component
    /// that can wake a suspended machine. A second timer path inside the
    /// calendar would reproduce the incumbent failure the doc cites.
    pub alarms: Vec<Alarm>,
}

/// Which end of the event a relative trigger counts from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Related {
    /// The default: relative to `DTSTART`.
    Start,
    /// `RELATED=END`: relative to the event's end.
    End,
}

/// When an alarm goes off.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Trigger {
    /// A duration from one end of the event. Negative is before it, which is
    /// what almost every real alarm is.
    Relative {
        /// Seconds, signed.
        seconds: i64,
        /// Which end it counts from.
        related: Related,
    },
    /// A fixed instant the file states outright.
    Absolute(CalTime),
}

/// One `VALARM`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Alarm {
    /// When it goes off.
    pub trigger: Trigger,
    /// The `ACTION` verbatim (`DISPLAY`, `AUDIO`, `EMAIL`), uppercased, when the
    /// file gave one. Carried rather than interpreted: presentation belongs to
    /// the notification daemon, and an alarm with no action is still an alarm.
    pub action: Option<String>,
}

impl Event {
    /// Does this event repeat?
    pub fn repeats(&self) -> bool {
        self.rrule.is_some()
    }
}

/// What can go wrong reading a calendar file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IcsError {
    /// No `BEGIN:VCALENDAR` line: this is not an iCalendar file.
    NotCalendar,
}

impl std::fmt::Display for IcsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotCalendar => write!(f, "this file is not an iCalendar file"),
        }
    }
}

impl std::error::Error for IcsError {}

/// Undo the line folding RFC 5545 §3.1 requires.
///
/// A long line is broken with CRLF and continued by a line starting with a space
/// or a tab, and the whitespace is NOT part of the value. Readers that skip this
/// see a `SUMMARY` cut in half and a second line that parses as an unknown
/// property, which loses the tail of every long title silently.
fn unfold(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for raw in text.split('\n') {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        match line.strip_prefix([' ', '\t']) {
            Some(rest) if !out.is_empty() => out.last_mut().expect("checked").push_str(rest),
            _ => out.push(line.to_string()),
        }
    }
    out
}

/// A property line split into its name, parameters and value.
struct Property {
    name: String,
    params: Vec<(String, String)>,
    value: String,
}

impl Property {
    fn param(&self, key: &str) -> Option<&str> {
        self.params
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v.as_str())
    }
}

/// Split `NAME;PARAM=value:the value`.
///
/// The colon that ends the name-and-parameters part can also appear INSIDE a
/// quoted parameter value (`ALTREP="http://x:80/y"`), so the split tracks quotes
/// rather than taking the first colon.
fn parse_property(line: &str) -> Option<Property> {
    let mut in_quotes = false;
    let mut split_at = None;
    for (i, c) in line.char_indices() {
        match c {
            '"' => in_quotes = !in_quotes,
            ':' if !in_quotes => {
                split_at = Some(i);
                break;
            }
            _ => {}
        }
    }
    let at = split_at?;
    let (head, value) = line.split_at(at);
    let value = &value[1..];

    let mut parts = head.split(';');
    let name = parts.next()?.trim().to_uppercase();
    let params = parts
        .filter_map(|p| {
            let (k, v) = p.split_once('=')?;
            Some((k.trim().to_uppercase(), v.trim().trim_matches('"').to_string()))
        })
        .collect();
    Some(Property {
        name,
        params,
        value: value.to_string(),
    })
}

/// Undo TEXT escaping (RFC 5545 §3.3.11).
///
/// `\n` is a real line break, and the comma and semicolon are escaped because
/// they separate values. A reader that skips this shows `Lunch\, then talk`.
fn unescape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n' | 'N') => out.push('\n'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

/// Read a DATE or DATE-TIME value into the form the file wrote it.
fn parse_time(prop: &Property) -> Option<CalTime> {
    let v = prop.value.trim();
    if prop.param("VALUE").is_some_and(|p| p.eq_ignore_ascii_case("DATE")) || v.len() == 8 {
        return NaiveDate::parse_from_str(v, "%Y%m%d").ok().map(CalTime::Day);
    }
    if let Some(stripped) = v.strip_suffix('Z') {
        return NaiveDateTime::parse_from_str(stripped, "%Y%m%dT%H%M%S")
            .ok()
            .map(CalTime::Utc);
    }
    let at = NaiveDateTime::parse_from_str(v, "%Y%m%dT%H%M%S").ok()?;
    match prop.param("TZID") {
        Some(tzid) if !tzid.is_empty() => Some(CalTime::Zoned {
            at,
            tzid: tzid.to_string(),
        }),
        _ => Some(CalTime::Floating(at)),
    }
}

/// Read an ISO 8601 duration as RFC 5545 uses it (`PT1H30M`, `P2D`, `-PT15M`).
///
/// Returns seconds. Weeks are `P3W` and are exclusive of the rest by the grammar,
/// but accepting them alongside costs nothing and refusing would drop a real
/// value.
/// A `TRIGGER` value: a signed duration from one end of the event, or a fixed
/// instant.
///
/// The default when nothing says otherwise is a duration from `DTSTART`, which
/// is what RFC 5545 states and what every alarm anybody writes by hand means.
/// A `VALUE=DATE-TIME` trigger is an instant instead, and one that parses as
/// neither yields `None` so the alarm is dropped rather than fired at a guess.
fn parse_trigger(prop: &Property) -> Option<Trigger> {
    let absolute = prop
        .param("VALUE")
        .is_some_and(|v| v.eq_ignore_ascii_case("DATE-TIME") || v.eq_ignore_ascii_case("DATE"));
    if absolute {
        return parse_time(prop).map(Trigger::Absolute);
    }
    let related = match prop.param("RELATED") {
        Some(v) if v.eq_ignore_ascii_case("END") => Related::End,
        _ => Related::Start,
    };
    parse_duration(&prop.value).map(|seconds| Trigger::Relative { seconds, related })
}

fn parse_duration(value: &str) -> Option<i64> {
    let v = value.trim();
    let (sign, v) = match v.strip_prefix('-') {
        Some(rest) => (-1i64, rest),
        None => (1i64, v.strip_prefix('+').unwrap_or(v)),
    };
    let v = v.strip_prefix('P')?;
    let (date_part, time_part) = match v.split_once('T') {
        Some((d, t)) => (d, Some(t)),
        None => (v, None),
    };

    let mut total = 0i64;
    let mut digits = String::new();
    for c in date_part.chars() {
        if c.is_ascii_digit() {
            digits.push(c);
            continue;
        }
        let n: i64 = digits.parse().ok()?;
        digits.clear();
        total += match c {
            'W' => n * 7 * 86_400,
            'D' => n * 86_400,
            _ => return None,
        };
    }
    if !digits.is_empty() {
        return None; // a number with no unit after it
    }
    if let Some(time) = time_part {
        for c in time.chars() {
            if c.is_ascii_digit() {
                digits.push(c);
                continue;
            }
            let n: i64 = digits.parse().ok()?;
            digits.clear();
            total += match c {
                'H' => n * 3_600,
                'M' => n * 60,
                'S' => n,
                _ => return None,
            };
        }
        if !digits.is_empty() {
            return None;
        }
    }
    Some(sign * total)
}

/// Add seconds to a time, keeping its form.
fn shift(start: &CalTime, seconds: i64) -> CalTime {
    let delta = chrono::TimeDelta::try_seconds(seconds).unwrap_or_default();
    match start {
        CalTime::Day(d) => CalTime::Day(*d + delta),
        CalTime::Floating(dt) => CalTime::Floating(*dt + delta),
        CalTime::Utc(dt) => CalTime::Utc(*dt + delta),
        CalTime::Zoned { at, tzid } => CalTime::Zoned {
            at: *at + delta,
            tzid: tzid.clone(),
        },
    }
}

/// Every VEVENT in one calendar file.
///
/// Unknown components (VTODO, VJOURNAL, VTIMEZONE) are skipped rather than
/// refused: a file holding a timezone definition beside its events is the normal
/// case, and refusing the file would lose the events.
pub fn parse_events(text: &str) -> Result<Vec<Event>, IcsError> {
    let lines = unfold(text);
    if !lines
        .iter()
        .any(|l| l.trim().eq_ignore_ascii_case("BEGIN:VCALENDAR"))
    {
        return Err(IcsError::NotCalendar);
    }

    let mut events = Vec::new();
    let mut current: Option<(Option<CalTime>, Option<CalTime>, Option<i64>, Event)> = None;
    // A VALARM sits INSIDE a VEVENT and carries properties with the same names.
    // Its `DURATION` is the repeat interval of the alarm, not the length of the
    // meeting, so reading it as the event's would move the end of an event that
    // merely has a reminder attached. Tracked as its own state rather than
    // ignored for that reason.
    let mut alarm: Option<(Option<Trigger>, Option<String>)> = None;
    for line in lines {
        let trimmed = line.trim();
        if trimmed.eq_ignore_ascii_case("BEGIN:VALARM") {
            alarm = Some((None, None));
            continue;
        }
        if trimmed.eq_ignore_ascii_case("END:VALARM") {
            // An alarm with no TRIGGER has no time to go off at, and inventing
            // one would ring at a moment nobody wrote.
            if let (Some((Some(trigger), action)), Some((_, _, _, ev))) =
                (alarm.take(), current.as_mut())
            {
                ev.alarms.push(Alarm { trigger, action });
            }
            continue;
        }
        if let Some((trigger, action)) = alarm.as_mut() {
            if let Some(prop) = parse_property(trimmed) {
                match prop.name.as_str() {
                    "TRIGGER" => *trigger = parse_trigger(&prop),
                    "ACTION" => *action = Some(prop.value.trim().to_ascii_uppercase()),
                    _ => {}
                }
            }
            continue;
        }
        if trimmed.eq_ignore_ascii_case("BEGIN:VEVENT") {
            current = Some((
                None,
                None,
                None,
                Event {
                    uid: String::new(),
                    summary: String::new(),
                    start: CalTime::Day(NaiveDate::default()),
                    end: None,
                    location: String::new(),
                    rrule: None,
                    alarms: Vec::new(),
                },
            ));
            continue;
        }
        if trimmed.eq_ignore_ascii_case("END:VEVENT") {
            if let Some((start, end, duration, mut ev)) = current.take() {
                // An event with no start is not an event. Dropped rather than
                // shown at the epoch, which would put it at the top of every
                // agenda forever.
                let Some(start) = start else { continue };
                ev.start = start;
                ev.end = match (end, duration) {
                    (Some(e), _) => Some(e),
                    (None, Some(secs)) => Some(shift(&ev.start, secs)),
                    (None, None) => None,
                };
                events.push(ev);
            }
            continue;
        }
        let Some((start, end, duration, ev)) = current.as_mut() else {
            continue; // outside a VEVENT
        };
        let Some(prop) = parse_property(trimmed) else {
            continue;
        };
        match prop.name.as_str() {
            "UID" => ev.uid = unescape(&prop.value),
            "SUMMARY" => ev.summary = unescape(&prop.value),
            "LOCATION" => ev.location = unescape(&prop.value),
            "RRULE" => ev.rrule = Some(prop.value.trim().to_string()),
            "DTSTART" => *start = parse_time(&prop),
            "DTEND" => *end = parse_time(&prop),
            "DURATION" => *duration = parse_duration(&prop.value),
            _ => {}
        }
    }
    Ok(events)
}

#[cfg(test)]
mod tests {
    use super::*;

    const WITH_ALARM: &str = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:a@x\r\n\
SUMMARY:Standup\r\nDTSTART:20260819T090000Z\r\nDTEND:20260819T091500Z\r\n\
BEGIN:VALARM\r\nACTION:DISPLAY\r\nTRIGGER:-PT15M\r\nDURATION:PT5M\r\nREPEAT:3\r\n\
END:VALARM\r\nEND:VEVENT\r\nEND:VCALENDAR";

    #[test]
    fn an_alarms_own_duration_is_not_the_events_length() {
        // The trap this parser walked into before VALARM had its own state: a
        // VALARM's DURATION is how often it repeats, and reading it as the
        // event's would end a 15-minute standup after 5.
        let ev = &parse_events(WITH_ALARM).expect("parses")[0];
        assert_eq!(ev.end, Some(CalTime::Utc(
            chrono::NaiveDate::from_ymd_opt(2026, 8, 19).unwrap().and_hms_opt(9, 15, 0).unwrap()
        )));
        assert_eq!(ev.alarms.len(), 1);
        assert_eq!(ev.alarms[0].action.as_deref(), Some("DISPLAY"));
        assert_eq!(
            ev.alarms[0].trigger,
            Trigger::Relative { seconds: -900, related: Related::Start }
        );
    }

    #[test]
    fn a_trigger_says_which_end_it_counts_from_and_when_it_is_a_fixed_time() {
        let ics = |t: &str| format!("BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:a\r\n\
DTSTART:20260819T090000Z\r\nDTEND:20260819T100000Z\r\nBEGIN:VALARM\r\n{t}\r\n\
END:VALARM\r\nEND:VEVENT\r\nEND:VCALENDAR");
        let one = |t: &str| parse_events(&ics(t)).expect("parses")[0].alarms.first().cloned();
        assert_eq!(
            one("TRIGGER;RELATED=END:PT5M").map(|a| a.trigger),
            Some(Trigger::Relative { seconds: 300, related: Related::End })
        );
        assert!(matches!(
            one("TRIGGER;VALUE=DATE-TIME:20260819T083000Z").map(|a| a.trigger),
            Some(Trigger::Absolute(CalTime::Utc(_)))
        ));
        // No TRIGGER is no time to ring at, so the alarm is dropped rather than
        // given one.
        assert_eq!(one("ACTION:DISPLAY"), None);
    }

    #[test]
    fn an_alarm_is_timed_against_the_occurrence_it_belongs_to() {
        use chrono::{TimeZone, Utc};
        let ev = &parse_events(WITH_ALARM).expect("parses")[0];
        // A LATER occurrence of the same event: the alarm follows it rather than
        // staying on the one the file wrote.
        let start = Utc.with_ymd_and_hms(2026, 8, 26, 9, 0, 0).unwrap();
        let times = when::alarm_times(ev, start, Some(start), chrono_tz::Tz::UTC);
        assert_eq!(times, vec![Utc.with_ymd_and_hms(2026, 8, 26, 8, 45, 0).unwrap()]);
    }

    #[test]
    fn an_end_relative_alarm_on_an_event_with_no_end_is_dropped() {
        use chrono::{TimeZone, Utc};
        let ics = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:a\r\nDTSTART:20260819T090000Z\r\n\
BEGIN:VALARM\r\nTRIGGER;RELATED=END:PT5M\r\nEND:VALARM\r\nEND:VEVENT\r\nEND:VCALENDAR";
        let ev = &parse_events(ics).expect("parses")[0];
        let start = Utc.with_ymd_and_hms(2026, 8, 19, 9, 0, 0).unwrap();
        // Folding it onto the start would ring at a different time than written.
        assert!(when::alarm_times(ev, start, None, chrono_tz::Tz::UTC).is_empty());
    }


    const SAMPLE: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:a@arlen\r\nSUMMARY:Standup\r\nDTSTART;TZID=Europe/Vienna:20260819T090000\r\nDTEND;TZID=Europe/Vienna:20260819T091500\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

    #[test]
    fn an_event_comes_back_with_its_zone_kept() {
        let events = parse_events(SAMPLE).expect("a calendar");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].summary, "Standup");
        assert_eq!(
            events[0].start,
            CalTime::Zoned {
                at: NaiveDate::from_ymd_opt(2026, 8, 19)
                    .unwrap()
                    .and_hms_opt(9, 0, 0)
                    .unwrap(),
                tzid: "Europe/Vienna".into()
            }
        );
    }

    /// The three forms must stay apart. A parser that returned one type here
    /// would have decided, at read time, that a floating standup follows you
    /// across a timezone or that it does not - and thrown away the evidence.
    #[test]
    fn the_three_date_time_forms_are_distinguished() {
        let ics = "BEGIN:VCALENDAR\nBEGIN:VEVENT\nDTSTART:20260819T090000\nEND:VEVENT\n\
                   BEGIN:VEVENT\nDTSTART:20260819T090000Z\nEND:VEVENT\n\
                   BEGIN:VEVENT\nDTSTART;VALUE=DATE:20260819\nEND:VEVENT\nEND:VCALENDAR\n";
        let events = parse_events(ics).expect("a calendar");
        assert!(matches!(events[0].start, CalTime::Floating(_)));
        assert!(matches!(events[1].start, CalTime::Utc(_)));
        assert!(matches!(events[2].start, CalTime::Day(_)));
    }

    /// Folding is not cosmetic: skipping it cuts the tail off every long title.
    #[test]
    fn a_folded_line_is_rejoined_without_its_leading_space() {
        let ics = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nDTSTART:20260819T090000Z\r\n\
                   SUMMARY:A title long enough that a writer would\r\n  fold it here\r\n\
                   END:VEVENT\r\nEND:VCALENDAR\r\n";
        let events = parse_events(ics).expect("a calendar");
        assert_eq!(events[0].summary, "A title long enough that a writer would fold it here");
    }

    #[test]
    fn escaped_text_comes_back_unescaped() {
        let ics = "BEGIN:VCALENDAR\nBEGIN:VEVENT\nDTSTART:20260819T090000Z\n\
                   SUMMARY:Lunch\\, then a talk\\nsecond line\nEND:VEVENT\nEND:VCALENDAR\n";
        let events = parse_events(ics).expect("a calendar");
        assert_eq!(events[0].summary, "Lunch, then a talk\nsecond line");
    }

    #[test]
    fn a_duration_becomes_an_end_in_the_same_form_as_the_start() {
        let ics = "BEGIN:VCALENDAR\nBEGIN:VEVENT\nDTSTART;TZID=Europe/Vienna:20260819T090000\n\
                   DURATION:PT1H30M\nEND:VEVENT\nEND:VCALENDAR\n";
        let events = parse_events(ics).expect("a calendar");
        assert_eq!(
            events[0].end,
            Some(CalTime::Zoned {
                at: NaiveDate::from_ymd_opt(2026, 8, 19)
                    .unwrap()
                    .and_hms_opt(10, 30, 0)
                    .unwrap(),
                tzid: "Europe/Vienna".into()
            })
        );
    }

    #[test]
    fn durations_parse_across_their_units() {
        assert_eq!(parse_duration("PT15M"), Some(900));
        assert_eq!(parse_duration("P1D"), Some(86_400));
        assert_eq!(parse_duration("P1W"), Some(604_800));
        assert_eq!(parse_duration("PT1H30M10S"), Some(5_410));
        assert_eq!(parse_duration("-PT15M"), Some(-900));
        assert_eq!(parse_duration("PT15"), None, "a number with no unit is not a duration");
        assert_eq!(parse_duration("1H"), None, "no P prefix");
    }

    /// Repeating events are FLAGGED, not expanded, and the rule is kept verbatim
    /// so whoever expands it later has the original rather than our reading of it.
    #[test]
    fn a_repeating_event_is_flagged_and_its_rule_kept() {
        let ics = "BEGIN:VCALENDAR\nBEGIN:VEVENT\nDTSTART:20260819T090000Z\n\
                   RRULE:FREQ=WEEKLY;BYDAY=MO,WE\nEND:VEVENT\nEND:VCALENDAR\n";
        let events = parse_events(ics).expect("a calendar");
        assert!(events[0].repeats());
        assert_eq!(events[0].rrule.as_deref(), Some("FREQ=WEEKLY;BYDAY=MO,WE"));
    }

    /// A colon inside a quoted parameter must not end the parameter section.
    #[test]
    fn a_quoted_parameter_containing_a_colon_does_not_split_the_line() {
        let p = parse_property(r#"DTSTART;ALTREP="http://example.com:80/x";TZID=Europe/Vienna:20260819T090000"#)
            .expect("a property");
        assert_eq!(p.name, "DTSTART");
        assert_eq!(p.value, "20260819T090000");
        assert_eq!(p.param("TZID"), Some("Europe/Vienna"));
    }

    #[test]
    fn a_file_that_is_not_a_calendar_is_refused() {
        assert_eq!(parse_events("hello\n"), Err(IcsError::NotCalendar));
    }

    /// An event with no start is dropped rather than placed at the epoch, where
    /// it would sit at the top of every agenda for ever.
    #[test]
    fn an_event_without_a_start_is_dropped() {
        let ics = "BEGIN:VCALENDAR\nBEGIN:VEVENT\nSUMMARY:No time\nEND:VEVENT\nEND:VCALENDAR\n";
        assert!(parse_events(ics).expect("a calendar").is_empty());
    }

    /// Other components are skipped, not fatal: a VTIMEZONE block beside the
    /// events is the normal shape of a real file.
    #[test]
    fn other_components_do_not_swallow_the_events() {
        let ics = "BEGIN:VCALENDAR\nBEGIN:VTIMEZONE\nTZID:Europe/Vienna\nBEGIN:STANDARD\n\
                   DTSTART:19701025T030000\nEND:STANDARD\nEND:VTIMEZONE\n\
                   BEGIN:VEVENT\nUID:x\nDTSTART:20260819T090000Z\nSUMMARY:Real\nEND:VEVENT\nEND:VCALENDAR\n";
        let events = parse_events(ics).expect("a calendar");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].summary, "Real");
    }
}

/// Turning a written time into an instant, which is where the three forms stop
/// being interchangeable.
///
/// Reading an agenda needs no instants: an event is listed on the date it writes
/// for itself. Asking "what starts in the next fifteen minutes" needs them, and
/// each form answers differently:
///
///   * **UTC** already is one.
///   * **Zoned** resolves through the zone database. An unknown TZID has no
///     answer, and guessing the reader's zone for it would move a meeting by
///     hours - so it is refused rather than approximated.
///   * **Floating** means the reader's own clock, by definition, so it resolves
///     against the zone passed in. That IS the definition, not a fallback.
///   * **All-day** starts at local midnight in that same zone. The alternative,
///     treating it as an instant at UTC midnight, moves a public holiday into
///     the previous evening for anyone west of London.
///
/// A zone whose rules skip the wall-clock time (the hour that does not exist on
/// a spring-forward night) has no instant either; a repeated hour resolves to
/// the earlier of the two, which is the reading a person would give it.
pub mod when {
    use super::{CalTime, Event, Related, Trigger};
    use chrono::{DateTime, LocalResult, TimeZone, Utc};
    use chrono_tz::Tz;

    /// The instant a written time falls at, in `local` where the time does not
    /// name its own zone. `None` when the file names a zone this machine does not
    /// know, or a wall-clock time its zone skips.
    pub fn instant(t: &CalTime, local: Tz) -> Option<DateTime<Utc>> {
        let (naive, zone) = match t {
            CalTime::Utc(dt) => return Some(Utc.from_utc_datetime(dt)),
            CalTime::Floating(dt) => (*dt, local),
            CalTime::Day(d) => (d.and_hms_opt(0, 0, 0)?, local),
            CalTime::Zoned { at, tzid } => (*at, tzid.parse::<Tz>().ok()?),
        };
        match zone.from_local_datetime(&naive) {
            // The earlier of a repeated hour: the reading a person gives a clock
            // they are looking at.
            LocalResult::Single(dt) | LocalResult::Ambiguous(dt, _) => Some(dt.with_timezone(&Utc)),
            // The hour that does not exist. There is no instant to return and
            // inventing one would fire an alarm at a time nobody wrote.
            LocalResult::None => None,
        }
    }

    /// When each of an event's alarms goes off, for ONE occurrence.
    ///
    /// `start` and `end` are that occurrence's instants, so a repeating event is
    /// fed one occurrence at a time and its alarms come back keyed to it. This is
    /// what `calendar-app.md` section 4 requires of the registration: keyed by
    /// (UID, recurrence-id) and re-derived on every write, never a free-floating
    /// timer that outlives the occurrence it was made for.
    ///
    /// An alarm relative to the END of an event with no end is DROPPED rather
    /// than folded onto the start: those are different times, and an alarm that
    /// rings at the wrong one is worse than one that does not ring.
    pub fn alarm_times(
        event: &Event,
        start: DateTime<Utc>,
        end: Option<DateTime<Utc>>,
        local: Tz,
    ) -> Vec<DateTime<Utc>> {
        event
            .alarms
            .iter()
            .filter_map(|a| match &a.trigger {
                Trigger::Relative { seconds, related } => {
                    let anchor = match related {
                        Related::Start => Some(start),
                        Related::End => end,
                    }?;
                    anchor.checked_add_signed(chrono::Duration::seconds(*seconds))
                }
                Trigger::Absolute(t) => instant(t, local),
            })
            .collect()
    }

    /// The events starting within `lead` seconds after `now`, soonest first.
    ///
    /// Strictly ahead: an event that has already started is not upcoming, and
    /// announcing it would be an alarm for a meeting the person is late for
    /// rather than one they can still walk to. Events whose instant cannot be
    /// resolved are left out - a reminder at a guessed time is worse than none.
    ///
    /// Recurrence is NOT expanded here either, so a weekly meeting is upcoming
    /// only on the day the file writes it. Whoever expands `RRULE` feeds the
    /// expansion in and this needs no change.
    pub fn upcoming<'a>(
        events: &'a [Event],
        now: DateTime<Utc>,
        local: Tz,
        lead_seconds: i64,
    ) -> Vec<(&'a Event, DateTime<Utc>)> {
        let mut out: Vec<(&Event, DateTime<Utc>)> = events
            .iter()
            .filter_map(|e| instant(&e.start, local).map(|i| (e, i)))
            .filter(|(_, i)| {
                let ahead = (*i - now).num_seconds();
                ahead > 0 && ahead <= lead_seconds
            })
            .collect();
        out.sort_by_key(|(_, i)| *i);
        out
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::parse_events;
        use chrono::NaiveDate;

        const VIENNA: &str = "Europe/Vienna";

        fn at(y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<Utc> {
            Utc.from_utc_datetime(
                &NaiveDate::from_ymd_opt(y, m, d).unwrap().and_hms_opt(h, min, 0).unwrap(),
            )
        }

        /// August in Vienna is UTC+2, so 09:00 there is 07:00 UTC. A resolver
        /// that ignored the zone would be two hours out, which is the whole
        /// reason the parser keeps the zone.
        #[test]
        fn a_zoned_time_resolves_through_its_own_zone() {
            let t = CalTime::Zoned {
                at: NaiveDate::from_ymd_opt(2026, 8, 19).unwrap().and_hms_opt(9, 0, 0).unwrap(),
                tzid: VIENNA.into(),
            };
            assert_eq!(instant(&t, VIENNA.parse().unwrap()), Some(at(2026, 8, 19, 7, 0)));
        }

        /// A zone this machine has never heard of is refused. Falling back to the
        /// reader's zone would move the meeting by however far apart they are.
        #[test]
        fn an_unknown_zone_has_no_instant() {
            let t = CalTime::Zoned {
                at: NaiveDate::from_ymd_opt(2026, 8, 19).unwrap().and_hms_opt(9, 0, 0).unwrap(),
                tzid: "Mars/Olympus".into(),
            };
            assert_eq!(instant(&t, VIENNA.parse().unwrap()), None);
        }

        /// Floating means the reader's clock. This is the definition, so the same
        /// file gives a different instant to a reader in another zone - which is
        /// exactly what "09:00 wherever you are" means.
        #[test]
        fn a_floating_time_follows_the_reader() {
            let t = CalTime::Floating(
                NaiveDate::from_ymd_opt(2026, 8, 19).unwrap().and_hms_opt(9, 0, 0).unwrap(),
            );
            assert_eq!(instant(&t, VIENNA.parse().unwrap()), Some(at(2026, 8, 19, 7, 0)));
            assert_eq!(
                instant(&t, "Asia/Tokyo".parse().unwrap()),
                Some(at(2026, 8, 19, 0, 0)),
                "Tokyo is UTC+9, so their 09:00 is midnight UTC"
            );
        }

        /// An all-day entry starts at local midnight. Treating it as UTC midnight
        /// would put a holiday in the previous evening for anyone west of London.
        #[test]
        fn an_all_day_entry_starts_at_local_midnight() {
            let t = CalTime::Day(NaiveDate::from_ymd_opt(2026, 8, 20).unwrap());
            assert_eq!(
                instant(&t, VIENNA.parse().unwrap()),
                Some(at(2026, 8, 19, 22, 0)),
                "midnight in Vienna is 22:00 UTC the day before"
            );
        }

        /// The hour that does not exist on a spring-forward night. Vienna moves
        /// 02:00 to 03:00 on 29 March 2026, so 02:30 was never on any clock.
        #[test]
        fn a_wall_clock_time_its_zone_skips_has_no_instant() {
            let t = CalTime::Floating(
                NaiveDate::from_ymd_opt(2026, 3, 29).unwrap().and_hms_opt(2, 30, 0).unwrap(),
            );
            assert_eq!(instant(&t, VIENNA.parse().unwrap()), None);
        }

        #[test]
        fn upcoming_is_the_window_ahead_and_nothing_behind_it() {
            let ics = "BEGIN:VCALENDAR\n\
                       BEGIN:VEVENT\nUID:past\nSUMMARY:Started\nDTSTART:20260819T065500Z\nEND:VEVENT\n\
                       BEGIN:VEVENT\nUID:soon\nSUMMARY:Standup\nDTSTART:20260819T070500Z\nEND:VEVENT\n\
                       BEGIN:VEVENT\nUID:later\nSUMMARY:Review\nDTSTART:20260819T090000Z\nEND:VEVENT\n\
                       END:VCALENDAR\n";
            let events = parse_events(ics).expect("a calendar");
            let now = at(2026, 8, 19, 7, 0);
            let soon = upcoming(&events, now, VIENNA.parse().unwrap(), 15 * 60);
            assert_eq!(soon.len(), 1, "one event is inside the fifteen minutes");
            assert_eq!(soon[0].0.uid, "soon");
        }

        /// Soonest first, so whoever announces them does not have to sort.
        #[test]
        fn upcoming_comes_back_in_time_order() {
            let ics = "BEGIN:VCALENDAR\n\
                       BEGIN:VEVENT\nUID:b\nDTSTART:20260819T075000Z\nEND:VEVENT\n\
                       BEGIN:VEVENT\nUID:a\nDTSTART:20260819T071000Z\nEND:VEVENT\n\
                       END:VCALENDAR\n";
            let events = parse_events(ics).expect("a calendar");
            let soon = upcoming(&events, at(2026, 8, 19, 7, 0), VIENNA.parse().unwrap(), 3600);
            assert_eq!(
                soon.iter().map(|(e, _)| e.uid.as_str()).collect::<Vec<_>>(),
                vec!["a", "b"]
            );
        }

        /// An event whose zone cannot be resolved is left out rather than
        /// announced at a guessed time.
        #[test]
        fn an_unresolvable_event_is_left_out_of_the_window() {
            let ics = "BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:x\n\
                       DTSTART;TZID=Mars/Olympus:20260819T071000\nEND:VEVENT\nEND:VCALENDAR\n";
            let events = parse_events(ics).expect("a calendar");
            assert!(upcoming(&events, at(2026, 8, 19, 7, 0), VIENNA.parse().unwrap(), 3600).is_empty());
        }
    }
}

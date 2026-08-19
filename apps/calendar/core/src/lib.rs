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
    for line in lines {
        let trimmed = line.trim();
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

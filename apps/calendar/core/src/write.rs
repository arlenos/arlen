//! Writing one event back out as iCalendar text.
//!
//! The reader in [`crate::parse_events`] is the specification this has to
//! satisfy: whatever is written here is read back by that, in this app and in
//! whatever else opens the file. So every test below writes an event, parses it
//! with the real reader, and compares - a writer checked only against its own
//! expectations agrees with itself and with nothing else.
//!
//! WHAT THIS DOES NOT DO. It writes ONE event into its own file. Editing an
//! event inside a file that holds many is a different operation with a different
//! risk (a rewrite loses whatever the reader did not model), and this app does
//! not do it yet.

use chrono::{NaiveDate, NaiveTime};

/// The event to write, in the terms the surface collects it.
#[derive(Debug, Clone)]
pub struct NewEvent {
    pub uid: String,
    pub summary: String,
    pub date: NaiveDate,
    /// `None` for a whole day.
    pub start: Option<NaiveTime>,
    pub end: Option<NaiveTime>,
    pub location: String,
    /// An `RRULE` value without the property name, e.g. `FREQ=WEEKLY;BYDAY=MO`.
    pub rrule: Option<String>,
    /// When the file was written, for `DTSTAMP`.
    pub stamp: chrono::NaiveDateTime,
}

/// Escape a text value: the four characters iCalendar gives meaning to.
///
/// A summary with a comma in it - "Lunch, then the dentist" - splits into two
/// values without this, and the reader hands back half a title. The backslash
/// goes FIRST, or escaping the others would escape the escapes.
fn escape(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace(';', "\\;")
        .replace(',', "\\,")
        .replace('\n', "\\n")
        .replace('\r', "")
}

/// Fold a content line to 75 octets, continuing with a leading space.
///
/// Folding is by OCTET and a UTF-8 character must not be split across the break,
/// so this counts bytes and only breaks on a character boundary. A German
/// summary is the ordinary case where a naive byte split writes an invalid file.
fn fold(line: &str) -> String {
    const LIMIT: usize = 75;
    if line.len() <= LIMIT {
        return line.to_string();
    }
    let mut out = String::new();
    let mut start = 0;
    let mut budget = LIMIT;
    while start < line.len() {
        let mut end = (start + budget).min(line.len());
        while end > start && !line.is_char_boundary(end) {
            end -= 1;
        }
        if end == start {
            // A single character wider than the budget: emit it whole rather
            // than loop forever. An unfolded long line is legible; a truncated
            // one is not.
            end = line[start..]
                .char_indices()
                .nth(1)
                .map_or(line.len(), |(i, _)| start + i);
        }
        if start > 0 {
            out.push_str("\r\n ");
        }
        out.push_str(&line[start..end]);
        start = end;
        // The continuation's leading space counts against the next line.
        budget = LIMIT - 1;
    }
    out
}

/// One `VEVENT` in its own `VCALENDAR`, as the reader expects to find it.
pub fn vcalendar(event: &NewEvent) -> String {
    let mut lines: Vec<String> = vec![
        "BEGIN:VCALENDAR".into(),
        "VERSION:2.0".into(),
        // Who wrote the file, per RFC 5545 - a reader that has to explain a
        // malformed event can say where it came from.
        "PRODID:-//Arlen//Calendar//EN".into(),
        "BEGIN:VEVENT".into(),
        format!("UID:{}", escape(&event.uid)),
        format!("DTSTAMP:{}", event.stamp.format("%Y%m%dT%H%M%SZ")),
    ];
    match event.start {
        None => {
            // A whole day has no time of day, and `VALUE=DATE` is how the file
            // says so. Writing midnight instead would invent one.
            lines.push(format!(
                "DTSTART;VALUE=DATE:{}",
                event.date.format("%Y%m%d")
            ));
        }
        Some(t) => {
            lines.push(format!(
                "DTSTART:{}T{}",
                event.date.format("%Y%m%d"),
                t.format("%H%M%S")
            ));
            if let Some(e) = event.end {
                lines.push(format!(
                    "DTEND:{}T{}",
                    event.date.format("%Y%m%d"),
                    e.format("%H%M%S")
                ));
            }
        }
    }
    lines.push(format!("SUMMARY:{}", escape(&event.summary)));
    if !event.location.is_empty() {
        lines.push(format!("LOCATION:{}", escape(&event.location)));
    }
    if let Some(rule) = &event.rrule {
        // NOT escaped: an RRULE is a structured value whose semicolons and
        // commas are its syntax, and escaping them would write a rule that
        // means nothing.
        lines.push(format!("RRULE:{rule}"));
    }
    lines.push("END:VEVENT".into());
    lines.push("END:VCALENDAR".into());
    let folded: Vec<String> = lines.iter().map(|l| fold(l)).collect();
    format!("{}\r\n", folded.join("\r\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_colour_is_replaced_or_inserted_and_nothing_else_moves() {
        let with_event = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nX-WR-CALNAME:Arbeit\r\n\
                          BEGIN:VEVENT\r\nUID:a\r\nCOLOR:red\r\nEND:VEVENT\r\n\
                          END:VCALENDAR\r\n";
        let out = set_calendar_color(with_event, "turquoise").unwrap();
        // Inserted before the first component, and the EVENT's own colour is
        // untouched - it is not the calendar's.
        assert!(out.contains("COLOR:turquoise\r\nBEGIN:VEVENT"), "{out}");
        assert!(
            out.contains("UID:a\r\nCOLOR:red"),
            "the event kept its own: {out}"
        );
        assert!(
            out.contains("X-WR-CALNAME:Arbeit"),
            "the rest of the file survives"
        );

        // A second write replaces rather than stacking.
        let again = set_calendar_color(&out, "orange").unwrap();
        assert_eq!(again.matches("COLOR:orange").count(), 1);
        assert_eq!(again.matches("COLOR:turquoise").count(), 0);
        assert!(
            again.contains("UID:a\r\nCOLOR:red"),
            "still the event's own"
        );

        // A calendar with nothing in it yet.
        let empty = "BEGIN:VCALENDAR\nVERSION:2.0\nEND:VCALENDAR\n";
        let out = set_calendar_color(empty, "blue").unwrap();
        assert!(out.contains("COLOR:blue\nEND:VCALENDAR"), "{out}");

        // Not a calendar, and a value that would forge a line: neither is written.
        assert_eq!(set_calendar_color("hello", "blue"), None);
        assert_eq!(set_calendar_color(empty, "blue\r\nSUMMARY:x"), None);
        assert_eq!(set_calendar_color(empty, "  "), None);
    }
    use crate::{parse_events, CalTime};

    fn at(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn stamp() -> chrono::NaiveDateTime {
        at(2026, 8, 24).and_hms_opt(9, 0, 0).unwrap()
    }

    fn draft(summary: &str) -> NewEvent {
        NewEvent {
            uid: "one@arlen".into(),
            summary: summary.into(),
            date: at(2026, 8, 25),
            start: NaiveTime::from_hms_opt(10, 0, 0),
            end: NaiveTime::from_hms_opt(11, 30, 0),
            location: String::new(),
            rrule: None,
            stamp: stamp(),
        }
    }

    #[test]
    fn what_is_written_is_what_the_reader_reads_back() {
        let text = vcalendar(&draft("Design review"));
        let events = parse_events(&text).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].uid, "one@arlen");
        assert_eq!(events[0].summary, "Design review");
        assert_eq!(
            events[0].start,
            CalTime::Floating(at(2026, 8, 25).and_hms_opt(10, 0, 0).unwrap())
        );
    }

    #[test]
    fn a_comma_in_a_title_stays_one_title() {
        // Without escaping the reader gets "Lunch" and loses the rest.
        let events = parse_events(&vcalendar(&draft("Lunch, then the dentist"))).unwrap();
        assert_eq!(events[0].summary, "Lunch, then the dentist");
    }

    #[test]
    fn a_semicolon_and_a_backslash_survive_too() {
        let events = parse_events(&vcalendar(&draft(r"Backup; then C:\work"))).unwrap();
        assert_eq!(events[0].summary, r"Backup; then C:\work");
    }

    #[test]
    fn a_whole_day_has_no_time_of_day() {
        let mut d = draft("Public holiday");
        d.start = None;
        d.end = None;
        let events = parse_events(&vcalendar(&d)).unwrap();
        assert_eq!(events[0].start, CalTime::Day(at(2026, 8, 25)));
    }

    #[test]
    fn a_long_german_summary_folds_without_splitting_a_character() {
        let long = "Besprechung über die Größe der Änderungen an der Benutzeroberfläche und was daraus folgt";
        let text = vcalendar(&draft(long));
        for line in text.split("\r\n") {
            assert!(
                line.len() <= 75,
                "unfolded line of {} octets: {line}",
                line.len()
            );
        }
        // And it still reads back whole, which is what folding is for.
        assert_eq!(parse_events(&text).unwrap()[0].summary, long);
    }

    #[test]
    fn a_repeat_rule_is_written_as_syntax_rather_than_text() {
        let mut d = draft("Standup");
        d.rrule = Some("FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR".into());
        let events = parse_events(&vcalendar(&d)).unwrap();
        assert_eq!(
            events[0].rrule.as_deref(),
            Some("FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR"),
            "escaping an RRULE writes a rule that means nothing"
        );
    }

    #[test]
    fn a_location_is_written_only_when_there_is_one() {
        assert!(!vcalendar(&draft("x")).contains("LOCATION"));
        let mut d = draft("x");
        d.location = "Studio".into();
        assert_eq!(parse_events(&vcalendar(&d)).unwrap()[0].location, "Studio");
    }
}

/// Set a calendar's own `COLOR`, leaving everything else where it was.
///
/// LINE SURGERY, not a re-serialise. The file is somebody's calendar and may
/// carry properties, comments in the form of X- lines, and events this crate does
/// not model; writing back a parsed-and-regenerated file would quietly drop all of
/// it. So this replaces the one line if it is there and inserts it if it is not,
/// and touches nothing else.
///
/// The insert goes after `BEGIN:VCALENDAR` rather than at the end of the header:
/// it must land before the first component, and that is the only position that is
/// certainly before one.
///
/// Returns `None` when the text is not a calendar - no `BEGIN:VCALENDAR` line at
/// all - because writing a colour into a file that is not one would make it
/// neither.
pub fn set_calendar_color(text: &str, color: &str) -> Option<String> {
    // The value travels into a line-based format, so a newline in it would forge
    // a property. Refused rather than escaped: ICS has no escape for this
    // position, and a colour with a line break in it is not a colour.
    if color.contains(['\r', '\n']) || color.trim().is_empty() {
        return None;
    }
    let ending = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let mut out: Vec<String> = Vec::new();
    // Two states, kept apart because conflating them was the first bug here: the
    // calendar's own header is what lies BETWEEN `BEGIN:VCALENDAR` and the first
    // component, and only a COLOR in there is the calendar's.
    let mut in_header = false;
    let mut seen_calendar = false;
    let mut wrote = false;
    for raw in text.split('\n') {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        let upper = line.to_ascii_uppercase();

        if in_header && (upper.starts_with("COLOR:") || upper.starts_with("COLOR;")) {
            out.push(format!("COLOR:{color}"));
            wrote = true;
            continue;
        }
        // A component starts: the header is over, and if nothing was replaced the
        // colour goes in just above it.
        if in_header && upper.starts_with("BEGIN:V") {
            if !wrote {
                out.push(format!("COLOR:{color}"));
                wrote = true;
            }
            in_header = false;
        }
        out.push(line.to_string());
        if upper.starts_with("BEGIN:VCALENDAR") {
            in_header = true;
            seen_calendar = true;
        }
    }
    if !seen_calendar {
        return None;
    }
    if !wrote {
        // A calendar with no components at all: the colour goes before END.
        let at = out
            .iter()
            .position(|l| l.to_ascii_uppercase().starts_with("END:VCALENDAR"))
            .unwrap_or(out.len());
        out.insert(at, format!("COLOR:{color}"));
    }
    Some(out.join(ending))
}

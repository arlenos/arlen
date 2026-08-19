//! The calendar's host: read the calendar files on this machine, and say what is
//! in them.
//!
//! `calendar-app.md` is a large plan and this is its floor: an agenda over local
//! `.ics` files, which is the shape every calendar has underneath its sync. It
//! reads and never writes. There is no CalDAV here, no invitation handling and no
//! recurrence expansion - those are named in the plan and each is its own body of
//! work, and a calendar that shows you your own files honestly is worth more than
//! one that pretends to sync.
//!
//! WHY THIS EXISTS AT ALL, beyond being an app somebody wants: `arlen-roadmap.md`
//! records that the agent behaviour `meeting-prep` is dead because no calendar
//! source exists. This is that source.

use std::path::PathBuf;

use arlen_calendar_core as ics;
use serde::Serialize;

/// Where a calendar file is looked for.
///
/// `$XDG_DATA_HOME/arlen/calendars`, falling back to `~/.local/share`. One
/// directory, no configuration yet: a setting for it belongs with the sync work,
/// where there is more than one kind of source to point at.
fn calendar_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))?;
    Some(base.join("arlen/calendars"))
}

/// One event, flattened for the frontend.
///
/// The times are strings in the file's own terms plus the KIND of time they are,
/// because the three forms mean different things and the surface has to be able
/// to say which one it is showing. Collapsing them here would undo the care the
/// parser takes.
#[derive(Debug, Clone, Serialize)]
pub struct AgendaEvent {
    pub uid: String,
    pub summary: String,
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
    /// True when the event carries an RRULE. NOT expanded: the surface says so.
    pub repeats: bool,
}

/// What one read of the calendar directory found.
#[derive(Debug, Clone, Serialize)]
pub struct Agenda {
    pub events: Vec<AgendaEvent>,
    /// The directory that was read, so the surface can name it when it is empty
    /// rather than telling the reader to put files "somewhere".
    pub directory: String,
    /// Does that directory exist? Absent and empty are different states and the
    /// surface says different things about them.
    pub directory_exists: bool,
    /// Files that could not be read or parsed. Counted rather than hidden: an
    /// agenda quietly missing a file is worse than one that says a file is
    /// missing from it.
    pub unreadable: usize,
}

fn kind_of(t: &ics::CalTime) -> (&'static str, Option<String>) {
    match t {
        ics::CalTime::Day(_) => ("day", None),
        ics::CalTime::Floating(_) => ("floating", None),
        ics::CalTime::Utc(_) => ("utc", None),
        ics::CalTime::Zoned { tzid, .. } => ("zoned", Some(tzid.clone())),
    }
}

fn flatten(e: &ics::Event) -> AgendaEvent {
    let (kind, tzid) = kind_of(&e.start);
    AgendaEvent {
        uid: e.uid.clone(),
        summary: e.summary.clone(),
        location: e.location.clone(),
        date: e.start.date().format("%Y-%m-%d").to_string(),
        time: e.start.time().map(|t| t.format("%H:%M").to_string()),
        end_time: e
            .end
            .as_ref()
            .and_then(|t| t.time())
            .map(|t| t.format("%H:%M").to_string()),
        kind: kind.to_string(),
        tzid,
        repeats: e.repeats(),
    }
}

/// Read every `.ics` file in the calendar directory.
///
/// Sorted by the date and time each event writes for itself, which groups an
/// agenda correctly for a reader in one place without resolving zones. Ordering
/// events written in different zones against each other needs a zone database
/// and is a separate step; doing it by string comparison would be a guess
/// dressed as an answer.
#[tauri::command]
fn calendar_agenda() -> Result<Agenda, String> {
    let Some(dir) = calendar_dir() else {
        return Err("this machine has no home directory to read calendars from".into());
    };
    let mut agenda = Agenda {
        events: Vec::new(),
        directory: dir.display().to_string(),
        directory_exists: dir.is_dir(),
        unreadable: 0,
    };
    if !agenda.directory_exists {
        return Ok(agenda);
    }

    let entries = std::fs::read_dir(&dir).map_err(|e| format!("could not read {}: {e}", dir.display()))?;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.extension().is_some_and(|e| e.eq_ignore_ascii_case("ics")) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            agenda.unreadable += 1;
            continue;
        };
        match ics::parse_events(&text) {
            Ok(events) => agenda.events.extend(events.iter().map(flatten)),
            Err(_) => agenda.unreadable += 1,
        }
    }
    agenda
        .events
        .sort_by(|a, b| (&a.date, &a.time, &a.summary).cmp(&(&b.date, &b.time, &b.summary)));
    Ok(agenda)
}

/// Start the calendar window.
pub fn run() {
    // Not the bare `init()`: it defaults to `error`, which makes an app mute in
    // the journal exactly when someone is trying to find out why it is empty.
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("warn,arlen_calendar_lib=info"),
    )
    .init();
    tauri::Builder::default()
        .plugin(tauri_plugin_arlen_shell::init())
        .invoke_handler(tauri::generate_handler![calendar_agenda])
        .run(tauri::generate_context!())
        .expect("error while running arlen-calendar");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The flattening must not lose which KIND of time an event carries - that
    /// is the whole point of the parser keeping them apart, and a surface that
    /// cannot tell a zoned time from a floating one will print one as the other.
    #[test]
    fn a_zoned_event_keeps_its_zone_through_the_flattening() {
        let events = ics::parse_events(
            "BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:x\nSUMMARY:Standup\n\
             DTSTART;TZID=Europe/Vienna:20260819T090000\nEND:VEVENT\nEND:VCALENDAR\n",
        )
        .expect("a calendar");
        let flat = flatten(&events[0]);
        assert_eq!(flat.kind, "zoned");
        assert_eq!(flat.tzid.as_deref(), Some("Europe/Vienna"));
        assert_eq!(flat.date, "2026-08-19");
        assert_eq!(flat.time.as_deref(), Some("09:00"));
    }

    /// An all-day event has no time of day, and inventing midnight for it would
    /// put it in the agenda ahead of a 00:30 event that really is at 00:30.
    #[test]
    fn an_all_day_event_carries_no_time() {
        let events = ics::parse_events(
            "BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:x\nSUMMARY:Holiday\n\
             DTSTART;VALUE=DATE:20260819\nEND:VEVENT\nEND:VCALENDAR\n",
        )
        .expect("a calendar");
        let flat = flatten(&events[0]);
        assert_eq!(flat.kind, "day");
        assert_eq!(flat.time, None);
    }
}

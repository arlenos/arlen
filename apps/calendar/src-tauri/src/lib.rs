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
use std::time::Duration;

use arlen_calendar_core as ics;
use serde::Serialize;
use tauri::Emitter;

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
    /// True when the event carries an RRULE.
    pub repeats: bool,
    /// True when THIS row is one the calendar worked out from the rule. False on
    /// a repeating event whose rule `rrule` refuses - that row is the one date
    /// the file names, and the surface has to say so rather than implying the
    /// series is drawn.
    pub expanded: bool,
}

/// What one read of the calendar directory found.
#[derive(Debug, Clone, Serialize)]
pub struct Agenda {
    pub events: Vec<AgendaEvent>,
    /// The directory that was read, so the surface can name it when it is empty
    /// rather than telling the reader to put files "somewhere".
    pub directory: String,
    /// Does that directory exist? Nearly always true, because opening the app
    /// creates it - the watcher cannot watch a path that is not there, and a
    /// calendar that names a directory it did not make is asking the reader to
    /// do its typing.
    pub directory_exists: bool,
    /// How many `.ics` files were found. No files and no events are different
    /// states: the first means nothing has been put here, the second means what
    /// is here holds nothing. Both name the directory, because "put files
    /// somewhere" is not an instruction.
    pub files: usize,
    /// Files that could not be read or parsed. Counted rather than hidden: an
    /// agenda quietly missing a file is worse than one that says a file is
    /// missing from it.
    pub unreadable: usize,
}

/// How far the agenda draws GENERATED occurrences, either side of today.
///
/// A written event is a fact in a file and is shown whenever it falls; a
/// repeat is generated, and generation without bounds is infinite - "every
/// Monday for ever" has no last row to draw. Backwards as well as forwards
/// because an agenda that hid this Monday's standup on Tuesday would be
/// answering a question nobody asked.
///
/// The numbers are a choice rather than a law, which is why they are here with
/// a name instead of inline: far enough that a person planning a quarter sees
/// their meetings, near enough that a daily rule is a few hundred rows.
const REPEAT_BACK_DAYS: i64 = 30;
const REPEAT_AHEAD_DAYS: i64 = 120;

/// Every date this event actually falls on, inside the window.
///
/// A non-repeating event is its own single date. A repeat this machine can work
/// out becomes one row per occurrence. A repeat it CANNOT work out - the rules
/// `rrule` refuses - stays a single row that still says it repeats, which is
/// what the surface said before any of this existed: better a row that admits
/// it does not know than rows on days nobody agreed to.
fn occurrences(e: &ics::Event, today: chrono::NaiveDate) -> Vec<chrono::NaiveDate> {
    let start = e.start.date();
    let Some(rule) = e.rrule.as_deref() else {
        return vec![start];
    };
    let from = today - chrono::Duration::days(REPEAT_BACK_DAYS);
    let to = today + chrono::Duration::days(REPEAT_AHEAD_DAYS);
    match ics::rrule::expand(rule, start, from, to) {
        Some(dates) if !dates.is_empty() => dates,
        // Refused, or a series that has ended before the window: the event is
        // still real and still says it repeats.
        _ => vec![start],
    }
}

fn kind_of(t: &ics::CalTime) -> (&'static str, Option<String>) {
    match t {
        ics::CalTime::Day(_) => ("day", None),
        ics::CalTime::Floating(_) => ("floating", None),
        ics::CalTime::Utc(_) => ("utc", None),
        ics::CalTime::Zoned { tzid, .. } => ("zoned", Some(tzid.clone())),
    }
}

fn flatten(e: &ics::Event, on: chrono::NaiveDate, expanded: bool) -> AgendaEvent {
    let (kind, tzid) = kind_of(&e.start);
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
        expanded,
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
        files: 0,
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
        agenda.files += 1;
        let Ok(text) = std::fs::read_to_string(&path) else {
            agenda.unreadable += 1;
            continue;
        };
        match ics::parse_events(&text) {
            Ok(events) => {
                let today = chrono::Local::now().date_naive();
                for e in &events {
                    let dates = occurrences(e, today);
                    // One date back from a repeating event means the rule was
                    // refused, not that the series has one occurrence.
                    let expanded = e.repeats() && dates.len() > 1;
                    agenda
                        .events
                        .extend(dates.into_iter().map(|on| flatten(e, on, expanded)));
                }
            }
            Err(_) => agenda.unreadable += 1,
        }
    }
    agenda
        .events
        .sort_by(|a, b| (&a.date, &a.time, &a.summary).cmp(&(&b.date, &b.time, &b.summary)));
    Ok(agenda)
}

/// Tell the window when a calendar file changes.
///
/// Without this the agenda is whatever the directory held at the moment the
/// window opened. Someone who edits an `.ics`, or whose sync writes one, sees
/// the old day until they restart the app - and a calendar showing yesterday's
/// answer with no sign that it is stale is the quiet kind of wrong this system
/// is meant not to do.
///
/// Failure is logged and left: a machine without inotify watches gets an agenda
/// that is correct when opened, which is worse than live and much better than
/// nothing.
fn spawn_calendar_watcher(app: tauri::AppHandle) {
    use notify::{EventKind, RecursiveMode, Watcher};

    let Some(dir) = calendar_dir() else { return };
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    std::thread::spawn(move || {
        let last = std::sync::Mutex::new(std::time::Instant::now() - Duration::from_secs(1));
        let mut watcher = match notify::recommended_watcher(move |ev: Result<notify::Event, _>| {
            let Ok(ev) = ev else { return };
            if !matches!(
                ev.kind,
                EventKind::Modify(_) | EventKind::Create(_) | EventKind::Remove(_)
            ) {
                return;
            }
            if !ev.paths.iter().any(|p| {
                p.extension().is_some_and(|e| e.eq_ignore_ascii_case("ics"))
            }) {
                return;
            }
            // An atomic write is a burst of events for one change, and a sync
            // rewriting a whole directory is a burst of bursts.
            {
                let mut l = last.lock().expect("watcher mutex");
                if l.elapsed() < Duration::from_millis(200) {
                    return;
                }
                *l = std::time::Instant::now();
            }
            // Let the writer finish: a rename lands before its contents on some
            // filesystems, and re-reading too eagerly reads the half of it.
            std::thread::sleep(Duration::from_millis(50));
            if let Err(e) = app.emit("arlen://calendar-changed", ()) {
                log::warn!("calendar watcher: emit failed: {e}");
            }
        }) {
            Ok(w) => w,
            Err(e) => {
                log::warn!("calendar watcher: could not start: {e}");
                return;
            }
        };
        if let Err(e) = watcher.watch(&dir, RecursiveMode::NonRecursive) {
            log::warn!("calendar watcher: could not watch {}: {e}", dir.display());
            return;
        }
        loop {
            std::thread::sleep(Duration::from_secs(3600));
        }
    });
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
        .setup(|app| {
            spawn_calendar_watcher(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![calendar_agenda])
        .run(tauri::generate_context!())
        .expect("error while running arlen-calendar");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A weekly meeting must appear on its days, not once on the day the file
    /// happens to name. This is the whole reason the expansion exists.
    #[test]
    fn a_weekly_event_draws_one_row_per_occurrence_in_the_window() {
        let events = ics::parse_events(
            "BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:standup\nSUMMARY:Standup\n\
             DTSTART:20260819T070000Z\nRRULE:FREQ=WEEKLY;BYDAY=WE\nEND:VEVENT\nEND:VCALENDAR\n",
        )
        .expect("a calendar");
        let today = chrono::NaiveDate::from_ymd_opt(2026, 8, 19).unwrap();
        let dates = occurrences(&events[0], today);
        assert!(dates.len() > 4, "a weekly rule fills the window: {}", dates.len());
        assert!(dates.iter().all(|d| chrono::Datelike::weekday(d) == chrono::Weekday::Wed));
    }

    /// A rule the expander refuses stays ONE row that still says it repeats -
    /// the state the surface handled before expansion existed. Rows on guessed
    /// days would be worse than a row that admits it does not know.
    #[test]
    fn a_refused_rule_stays_a_single_row() {
        let events = ics::parse_events(
            "BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:board\nSUMMARY:Board\n\
             DTSTART:20260803T090000Z\nRRULE:FREQ=MONTHLY;BYDAY=1MO\nEND:VEVENT\nEND:VCALENDAR\n",
        )
        .expect("a calendar");
        let today = chrono::NaiveDate::from_ymd_opt(2026, 8, 19).unwrap();
        assert_eq!(occurrences(&events[0], today).len(), 1);
        assert!(events[0].repeats(), "and it still says it repeats");
    }

    /// A one-off is its own single date, whatever the window is.
    #[test]
    fn a_single_event_is_untouched_by_the_window() {
        let events = ics::parse_events(
            "BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:x\nDTSTART:20200101T090000Z\nEND:VEVENT\nEND:VCALENDAR\n",
        )
        .expect("a calendar");
        let today = chrono::NaiveDate::from_ymd_opt(2026, 8, 19).unwrap();
        assert_eq!(
            occurrences(&events[0], today),
            vec![chrono::NaiveDate::from_ymd_opt(2020, 1, 1).unwrap()],
            "a written date is a fact in the file, not something generated"
        );
    }

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
        let flat = flatten(&events[0], events[0].start.date(), false);
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
        let flat = flatten(&events[0], events[0].start.date(), false);
        assert_eq!(flat.kind, "day");
        assert_eq!(flat.time, None);
    }
}

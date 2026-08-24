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
use ics::view::{rows, Agenda};
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

/// The `.ics` the calendar was opened on, when it was opened on one.
///
/// `arlen-calendar <file>`, or the desktop entry's `%f` when a person
/// double-clicks a calendar file in the file manager. `None` when launched bare,
/// which is the ordinary case and means "read the directory".
struct LaunchFile(Option<String>);

/// The file the app was launched with, for the page to ask about on mount.
#[tauri::command]
fn launch_file(state: tauri::State<'_, LaunchFile>) -> Option<String> {
    state.0.clone()
}

/// One file's events, for the launched-on-a-file case.
///
/// `directory` carries the FILE's own path here rather than a directory, because
/// that is what the surface has to name when the file turns out to hold nothing:
/// "no events in <this file>" is the honest sentence, and naming the calendar
/// folder instead would point at something the person did not open.
fn agenda_of_file(path: &std::path::Path) -> Agenda {
    let mut agenda = Agenda {
        events: Vec::new(),
        directory: path.display().to_string(),
        directory_exists: path.is_file(),
        files: 0,
        unreadable: 0,
        // Set by whoever answered. The file case never involves the service, and
        // the directory case fills it in from whether the daemon replied.
        service_running: false,
    };
    let Ok(text) = std::fs::read_to_string(path) else {
        agenda.unreadable = 1;
        return agenda;
    };
    agenda.files = 1;
    match ics::parse_events(&text) {
        Ok(events) => {
            agenda.events = rows(&events, chrono::Local::now().date_naive());
        }
        Err(_) => agenda.unreadable = 1,
    }
    agenda
}

/// Read every `.ics` file in the calendar directory.
///
/// Sorted by the date and time each event writes for itself, which groups an
/// agenda correctly for a reader in one place without resolving zones. Ordering
/// events written in different zones against each other needs a zone database
/// and is a separate step; doing it by string comparison would be a guess
/// dressed as an answer.
/// The agenda as the calendar service holds it, or `None` if it did not answer.
///
/// Asked FIRST, because the daemon is the thing that also arms the reminders:
/// when it answers, the agenda on screen and the alarms on the clock came from
/// one read of one set of files. When it does not, the app reads the files
/// itself - the same rows, from the same code - and says so, because showing
/// somebody their calendar while nothing is arming their reminders is the quiet
/// kind of wrong this app was written against.
async fn agenda_from_service() -> Option<Agenda> {
    const NAME: &str = "org.arlen.Calendar1";
    let connection = zbus::Connection::session().await.ok()?;
    let proxy = zbus::Proxy::new(&connection, NAME, "/org/arlen/Calendar1", NAME)
        .await
        .ok()?;
    let json: String = proxy.call("Agenda", &()).await.ok()?;
    serde_json::from_str(&json).ok()
}

/// Why keeping a calendar did not happen, as a word rather than a sentence.
///
/// All five were English sentences built here and rendered with no catalogue
/// around them at all, so a German reader met them verbatim. They are ordinary
/// outcomes of pressing Keep - the file is gone, the name is taken - and the
/// window is the only place that knows the reader's language.
#[derive(serde::Serialize)]
#[serde(rename_all = "kebab-case", tag = "problem")]
enum ImportProblem {
    /// The path handed over does not name a file at all.
    NotAFile,
    /// Nowhere to keep calendars: no home directory for this session.
    NoHome,
    /// The calendar directory could not be created. `why` is what the
    /// filesystem said.
    CannotMakeDir { why: String },
    /// A calendar of that name is already kept, and keeping would overwrite it.
    AlreadyKept { name: String },
    /// The copy itself failed.
    CopyFailed { why: String },
}

/// What happened when a person asked to keep an opened calendar.
#[derive(serde::Serialize)]
struct ImportResult {
    /// Where it now lives, when it was copied.
    path: Option<String>,
    /// Why it was not, named rather than written out.
    problem: Option<ImportProblem>,
}

/// Why an agenda could not be read, as a word.
#[derive(serde::Serialize)]
#[serde(rename_all = "kebab-case", tag = "problem")]
enum AgendaProblem {
    /// No home directory, so there is no calendar folder to read.
    NoHome,
    /// The calendar folder itself would not be read.
    Unreadable { why: String },
}

/// Copy an opened `.ics` into the calendar directory, so it is still there
/// tomorrow and the reminder daemon can see it.
///
/// WHY THIS IS AN ACTION AND NOT AUTOMATIC. Opening a file reads it in place, on
/// purpose: somebody who double-clicks one invitation wants to see that
/// invitation, and quietly merging it into their calendar would be an edit they
/// did not ask for. But without SOME way in, the directory is empty on every
/// machine, the agenda is empty for everyone, and the reminder daemon watches a
/// folder that never gains a file - which is why neither shipped. This is the
/// smallest way in that keeps the choice with the person: they opened it, they
/// can see what is in it, and then they say keep it.
///
/// Refuses to overwrite. A calendar already at that name is somebody's data, and
/// silently replacing it is exactly the edit this design is avoiding.
#[tauri::command]
async fn calendar_import(path: String) -> ImportResult {
    let src = std::path::PathBuf::from(&path);
    let Some(name) = src.file_name() else {
        return ImportResult { path: None, problem: Some(ImportProblem::NotAFile) };
    };
    let Some(dir) = calendar_dir() else {
        return ImportResult { path: None, problem: Some(ImportProblem::NoHome) };
    };
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return ImportResult {
            path: None,
            problem: Some(ImportProblem::CannotMakeDir { why: e.to_string() }),
        };
    }
    let dest = dir.join(name);
    if dest.exists() {
        return ImportResult {
            path: None,
            problem: Some(ImportProblem::AlreadyKept {
                name: name.to_string_lossy().into_owned(),
            }),
        };
    }
    match std::fs::copy(&src, &dest) {
        Ok(_) => ImportResult { path: Some(dest.display().to_string()), problem: None },
        Err(e) => ImportResult {
            path: None,
            problem: Some(ImportProblem::CopyFailed { why: e.to_string() }),
        },
    }
}

/// Why an event was not created, as a word rather than a sentence.
#[derive(serde::Serialize)]
#[serde(rename_all = "kebab-case", tag = "problem")]
enum CreateProblem {
    /// Nowhere to keep calendars: no home directory for this session.
    NoHome,
    /// The calendar directory could not be made. `why` is the filesystem's.
    CannotMakeDir { why: String },
    /// The date or a time did not read as one.
    BadDate,
    /// The write itself failed.
    NotWritten { why: String },
}

/// The event as the form collects it.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct EventDraft {
    summary: String,
    date: String,
    all_day: bool,
    time: Option<String>,
    end_time: Option<String>,
    location: String,
    repeat: String,
    on_days: Vec<String>,
}

/// Write one event into the calendar directory, as its own file.
///
/// ITS OWN FILE, not an edit of an existing one. Rewriting a file that holds
/// several events would put whatever this app does not model at risk - and it
/// models a subset on purpose. A new file is additive, and the watcher picks it
/// up the same way it picks up one that was copied in.
///
/// # Errors
/// When there is no home, the directory cannot be made, the date does not read,
/// or the write fails.
#[tauri::command]
async fn calendar_create_event(draft: EventDraft) -> Result<(), CreateProblem> {
    use arlen_calendar_core::write::{vcalendar, NewEvent};

    let dir = calendar_dir().ok_or(CreateProblem::NoHome)?;
    std::fs::create_dir_all(&dir)
        .map_err(|e| CreateProblem::CannotMakeDir { why: e.to_string() })?;

    let date = chrono::NaiveDate::parse_from_str(&draft.date, "%Y-%m-%d")
        .map_err(|_| CreateProblem::BadDate)?;
    let parse_time = |t: &Option<String>| -> Result<Option<chrono::NaiveTime>, CreateProblem> {
        match t.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            None => Ok(None),
            Some(s) => chrono::NaiveTime::parse_from_str(s, "%H:%M")
                .map(Some)
                .map_err(|_| CreateProblem::BadDate),
        }
    };
    let (start, end) = if draft.all_day {
        (None, None)
    } else {
        (parse_time(&draft.time)?, parse_time(&draft.end_time)?)
    };

    let event = NewEvent {
        // Ours and unique: the file's identity, which every reader keys on.
        uid: format!("{}@arlen", uuid::Uuid::now_v7()),
        summary: draft.summary,
        date,
        start,
        end,
        location: draft.location,
        rrule: rrule_of(&draft.repeat, &draft.on_days),
        stamp: chrono::Utc::now().naive_utc(),
    };

    // The filename carries the date and the uid's first field, so a directory
    // listing is readable and two events on one day cannot collide.
    let short = event.uid.split('-').next().unwrap_or("event").to_string();
    let target = dir.join(format!("{}-{short}.ics", date.format("%Y%m%d")));
    std::fs::write(&target, vcalendar(&event))
        .map_err(|e| CreateProblem::NotWritten { why: e.to_string() })
}

/// The `RRULE` value for a repeat the form offers, or `None` for a single event.
///
/// A weekly repeat with no day chosen is not weekly-on-nothing: it repeats on
/// the day the event is on, which is what leaving `BYDAY` out means.
fn rrule_of(repeat: &str, on_days: &[String]) -> Option<String> {
    match repeat {
        "daily" => Some("FREQ=DAILY".into()),
        "weekly" => {
            let days: Vec<String> = on_days
                .iter()
                .filter_map(|d| match d.to_ascii_lowercase().as_str() {
                    "mon" => Some("MO"),
                    "tue" => Some("TU"),
                    "wed" => Some("WE"),
                    "thu" => Some("TH"),
                    "fri" => Some("FR"),
                    "sat" => Some("SA"),
                    "sun" => Some("SU"),
                    _ => None,
                })
                .map(str::to_string)
                .collect();
            Some(if days.is_empty() {
                "FREQ=WEEKLY".into()
            } else {
                format!("FREQ=WEEKLY;BYDAY={}", days.join(","))
            })
        }
        _ => None,
    }
}

#[tauri::command]
async fn calendar_agenda(file: Option<String>) -> Result<Agenda, AgendaProblem> {
    // Opened ON a file: that file is the whole agenda. Reading the directory too
    // would answer a question the person did not ask - they double-clicked one
    // calendar, and mixing it with everything else would bury it.
    if let Some(path) = file.filter(|p| !p.is_empty()) {
        // Not through the service: it holds the calendar directory, and a file
        // somebody opened may be anywhere. Asking it for one would either return
        // the wrong agenda or nothing at all.
        // Never an error: a file that will not read or parse is reported IN the
        // agenda as `unreadable`, which is what the window shows.
        return Ok(agenda_of_file(std::path::Path::new(&path)));
    }
    if let Some(agenda) = agenda_from_service().await {
        return Ok(agenda);
    }
    let Some(dir) = calendar_dir() else {
        return Err(AgendaProblem::NoHome);
    };
    let mut agenda = Agenda {
        events: Vec::new(),
        directory: dir.display().to_string(),
        directory_exists: dir.is_dir(),
        files: 0,
        unreadable: 0,
        // Set by whoever answered. The file case never involves the service, and
        // the directory case fills it in from whether the daemon replied.
        service_running: false,
    };
    let mut parsed: Vec<ics::Event> = Vec::new();
    if !agenda.directory_exists {
        return Ok(agenda);
    }

    let entries = std::fs::read_dir(&dir).map_err(|e| AgendaProblem::Unreadable {
        why: e.to_string(),
    })?;
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
            Ok(events) => parsed.extend(events),
            Err(_) => agenda.unreadable += 1,
        }
    }
    // Sorted across every file at once, not per file: an agenda is one list, and
    // sorting each file's rows separately would leave the day's meetings grouped
    // by which file happened to hold them.
    agenda.events = rows(&parsed, chrono::Local::now().date_naive());
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
        .manage(LaunchFile(
            std::env::args().skip(1).find(|a| !a.starts_with('-')),
        ))
        .invoke_handler(tauri::generate_handler![
            calendar_agenda,
            launch_file,
            calendar_import,
            calendar_create_event
        ])
        .run(tauri::generate_context!())
        .expect("error while running arlen-calendar");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The rows themselves are `arlen_calendar_core::view`'s and tested there,
    /// where the daemon reads them from too. What is left here is what this host
    /// alone decides: that a file it was opened on is the whole agenda, and that
    /// the surface can name what it read when there is nothing in it.
    #[test]
    fn opened_on_a_file_the_agenda_is_that_file_and_names_it() {
        let dir = std::env::temp_dir().join(format!("arlen-cal-host-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("one.ics");
        std::fs::write(
            &path,
            "BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:only\nSUMMARY:The only one\n\
             DTSTART:20260819T090000Z\nEND:VEVENT\nEND:VCALENDAR\n",
        )
        .expect("write");

        let agenda = agenda_of_file(&path);
        assert_eq!(agenda.files, 1);
        assert_eq!(agenda.unreadable, 0);
        assert_eq!(agenda.events.len(), 1);
        assert_eq!(agenda.events[0].summary, "The only one");
        // The FILE, not the calendar folder: "no events in <this file>" is the
        // honest sentence when the thing somebody opened turns out to be empty.
        assert_eq!(agenda.directory, path.display().to_string());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_file_that_is_not_a_calendar_is_counted_rather_than_erroring() {
        let dir = std::env::temp_dir().join(format!("arlen-cal-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("notes.ics");
        std::fs::write(&path, "just some text").expect("write");

        let agenda = agenda_of_file(&path);
        assert_eq!(agenda.unreadable, 1);
        assert!(agenda.events.is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_repeat_becomes_a_rule_the_reader_understands() {
        assert_eq!(rrule_of("none", &[]), None);
        assert_eq!(rrule_of("daily", &[]), Some("FREQ=DAILY".into()));
        assert_eq!(
            rrule_of("weekly", &["mon".into(), "fri".into()]),
            Some("FREQ=WEEKLY;BYDAY=MO,FR".into())
        );
        // Weekly with no day chosen repeats on the event's own day, which is
        // what leaving BYDAY out means - not weekly-on-nothing.
        assert_eq!(rrule_of("weekly", &[]), Some("FREQ=WEEKLY".into()));
        // A day name nobody writes is dropped rather than passed through into a
        // rule the reader would then fail on.
        assert_eq!(rrule_of("weekly", &["montag".into()]), Some("FREQ=WEEKLY".into()));
    }
}

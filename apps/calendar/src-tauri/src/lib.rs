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
fn agenda_of_file(path: &std::path::Path) -> Result<Agenda, String> {
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
        return Ok(agenda);
    };
    agenda.files = 1;
    match ics::parse_events(&text) {
        Ok(events) => {
            agenda.events = rows(&events, chrono::Local::now().date_naive());
        }
        Err(_) => agenda.unreadable = 1,
    }
    Ok(agenda)
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

#[tauri::command]
async fn calendar_agenda(file: Option<String>) -> Result<Agenda, String> {
    // Opened ON a file: that file is the whole agenda. Reading the directory too
    // would answer a question the person did not ask - they double-clicked one
    // calendar, and mixing it with everything else would bury it.
    if let Some(path) = file.filter(|p| !p.is_empty()) {
        // Not through the service: it holds the calendar directory, and a file
        // somebody opened may be anywhere. Asking it for one would either return
        // the wrong agenda or nothing at all.
        return agenda_of_file(std::path::Path::new(&path));
    }
    if let Some(agenda) = agenda_from_service().await {
        return Ok(agenda);
    }
    let Some(dir) = calendar_dir() else {
        return Err("this machine has no home directory to read calendars from".into());
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
        .invoke_handler(tauri::generate_handler![calendar_agenda, launch_file])
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

        let agenda = agenda_of_file(&path).expect("reads");
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

        let agenda = agenda_of_file(&path).expect("reads");
        assert_eq!(agenda.unreadable, 1);
        assert!(agenda.events.is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }
}

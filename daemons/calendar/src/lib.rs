// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

//! The calendar daemon's store: what the files on disk currently say.
//!
//! `calendar-app.md` section 2 puts this here rather than in the app, and gives
//! the reason in one line: **anything else means reminders die when the window
//! closes.** The app is a view that can be shut; a reminder for a meeting at
//! four is not.
//!
//! Stored iCalendar-native, per the same section: the original text is what
//! every peer speaks, and a lossy normalisation would destroy the `VTIMEZONE`
//! and `X-` properties a round-trip needs. So the daemon reads files and parses
//! them; it does not keep a second representation that could disagree.
//!
//! Everything decided here is decided in `arlen-calendar-core` and tested there.
//! What this adds is the part that needs a filesystem: which directory, which
//! files, and what to say when one of them cannot be read.

use std::path::{Path, PathBuf};

use arlen_calendar_core as ics;

/// Where calendars live, under the user's data directory.
///
/// One place, named in full wherever the absence of a file is reported: telling
/// somebody to put files "somewhere" is not an instruction.
#[must_use]
pub fn calendar_dir() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join("arlen/calendars"))
}

/// What the directory currently holds.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Store {
    /// Every event across every readable file.
    pub events: Vec<ics::Event>,
    /// How many `.ics` files were found.
    ///
    /// Kept apart from the event count because no files and no events are
    /// different states: nothing has been put here, versus what is here holds
    /// nothing. A surface that renders both the same way tells a first-time
    /// reader their calendar is broken.
    pub files: usize,
    /// How many could not be read or parsed.
    ///
    /// Counted out loud rather than quietly missing from the list. A calendar
    /// showing four of your five files with no sign of the fifth is worse than
    /// one that says so.
    pub unreadable: usize,
}

/// Read every calendar in `dir`.
///
/// A file that cannot be read or does not parse is counted, not fatal: one bad
/// file must not take the other four with it. The directory not existing at all
/// is an empty store rather than an error, because that is what a machine that
/// has never had a calendar looks like.
#[must_use]
pub fn read_dir(dir: &Path) -> Store {
    let mut store = Store::default();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return store;
    };
    // Sorted, so two reads of an unchanged directory agree. `read_dir` yields
    // whatever order the filesystem holds, and an agenda that reshuffles itself
    // between reads looks like data changing when nothing did.
    let mut paths: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("ics")))
        .collect();
    paths.sort();

    for path in paths {
        store.files += 1;
        match std::fs::read_to_string(&path).ok().map(|t| ics::parse_events(&t)) {
            Some(Ok(events)) => store.events.extend(events),
            _ => store.unreadable += 1,
        }
    }
    store
}

#[cfg(test)]
mod tests {
    use super::*;

    const ONE: &str = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:a\r\nSUMMARY:One\r\n\
DTSTART:20260819T090000Z\r\nEND:VEVENT\r\nEND:VCALENDAR";

    #[test]
    fn a_directory_that_is_not_there_is_empty_rather_than_an_error() {
        let store = read_dir(Path::new("/nonexistent/arlen/calendars"));
        assert_eq!(store, Store::default());
    }

    #[test]
    fn one_unreadable_file_does_not_take_the_others_with_it() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("good.ics"), ONE).expect("write");
        std::fs::write(dir.path().join("bad.ics"), "this is not a calendar").expect("write");
        // Not an .ics at all: not a calendar file, so not counted as one either.
        std::fs::write(dir.path().join("notes.txt"), "hello").expect("write");

        let store = read_dir(dir.path());
        assert_eq!(store.files, 2);
        assert_eq!(store.unreadable, 1);
        assert_eq!(store.events.len(), 1, "the good file still came through");
        assert_eq!(store.events[0].summary, "One");
    }

    #[test]
    fn an_empty_directory_is_files_none_rather_than_events_none() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = read_dir(dir.path());
        assert_eq!(store.files, 0);
        assert!(store.events.is_empty());
        assert_eq!(store.unreadable, 0);
    }

    #[test]
    fn two_reads_of_an_unchanged_directory_agree() {
        let dir = tempfile::tempdir().expect("tempdir");
        for name in ["b.ics", "a.ics", "c.ics"] {
            std::fs::write(dir.path().join(name), ONE).expect("write");
        }
        assert_eq!(read_dir(dir.path()), read_dir(dir.path()));
    }
}

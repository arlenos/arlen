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

/// Turning derived reminders into clock registrations, and back out again.
///
/// The clock owns arming; this owns deciding WHICH registrations should exist.
/// `calendar-app.md` section 4 requires them re-derived on every store write, so
/// this is a plan against what the clock currently holds rather than a stream of
/// one-way calls: an occurrence that moved must lose its old registration, and
/// nothing else may be disturbed.
pub mod registry {
    use arlen_calendar_core::reminders::Registration;
    use chrono::NaiveDate;

    /// The mark on every alarm this daemon owns.
    ///
    /// A user alarm carries no payload at all, which is what makes the rule
    /// below safe: the calendar can only ever remove its own.
    pub const SOURCE: &str = "calendar";

    /// One alarm to register.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Desired {
        /// The clock-side handle, derived so the same occurrence always lands on
        /// the same alarm rather than a second copy of it.
        pub id: String,
        /// The day it rings on.
        pub on_date: NaiveDate,
        /// `HH:MM`, the shape the clock stores.
        pub time: String,
        /// What to show: the event's own title.
        pub label: String,
        /// The identification the clock hands back, as JSON.
        pub payload: String,
    }

    /// What to tell the clock.
    #[derive(Debug, Clone, Default, PartialEq, Eq)]
    pub struct Plan {
        /// Alarms to set. Setting an id that exists replaces it, so this is the
        /// same call for a new registration and a moved one.
        pub set: Vec<Desired>,
        /// Ids to delete: registrations this daemon made that no longer follow
        /// from the files.
        pub delete: Vec<String>,
    }

    /// A stable clock-side id for one occurrence.
    ///
    /// Derived rather than minted, so re-deriving after a write replaces the
    /// registration instead of adding a second one beside it.
    #[must_use]
    pub fn alarm_id(uid: &str, on: NaiveDate) -> String {
        format!("{SOURCE}:{on}:{uid}")
    }

    /// The payload the clock carries back, as JSON.
    #[must_use]
    pub fn payload(uid: &str, on: NaiveDate) -> String {
        // Hand-built rather than through serde: the two fields are known and
        // escaping the uid is the only care needed, which `to_string` on a
        // string gives.
        format!(
            "{{\"source\":\"{SOURCE}\",\"uid\":{},\"on\":\"{on}\"}}",
            serde_json_string(uid)
        )
    }

    /// A JSON string literal for `s`.
    fn serde_json_string(s: &str) -> String {
        let mut out = String::with_capacity(s.len() + 2);
        out.push('"');
        for c in s.chars() {
            match c {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
                c => out.push(c),
            }
        }
        out.push('"');
        out
    }

    /// What the clock should hold, given what it holds now.
    ///
    /// `existing` is every alarm the clock has, as `(id, payload)`. The rule that
    /// makes this safe to run unattended: **an alarm with no payload is
    /// somebody's own and is never touched.** Only registrations bearing this
    /// daemon's source are candidates for removal, so a user alarm cannot be
    /// deleted by a calendar file changing.
    #[must_use]
    pub fn plan(existing: &[(String, Option<String>)], desired: &[Desired]) -> Plan {
        let wanted: Vec<&str> = desired.iter().map(|d| d.id.as_str()).collect();
        let delete = existing
            .iter()
            .filter(|(_, payload)| payload.as_deref().is_some_and(is_ours))
            .filter(|(id, _)| !wanted.contains(&id.as_str()))
            .map(|(id, _)| id.clone())
            .collect();
        Plan { set: desired.to_vec(), delete }
    }

    /// Whether a payload is one of ours.
    ///
    /// A substring check on the source field rather than a parse: this decides
    /// only whether the calendar may DELETE an alarm, and being wrong in the
    /// permissive direction on a malformed payload would let it remove something
    /// it did not create.
    fn is_ours(payload: &str) -> bool {
        payload.contains(&format!("\"source\":\"{SOURCE}\""))
    }

    /// The registrations, in the shape the clock takes.
    #[must_use]
    pub fn desired_from(registrations: &[Registration], local: chrono_tz::Tz) -> Vec<Desired> {
        registrations
            .iter()
            .map(|r| {
                let local_at = r.at.with_timezone(&local);
                Desired {
                    id: alarm_id(&r.uid, r.recurrence_id),
                    // The DAY the alarm rings on, which is the alarm's own day
                    // and not the occurrence's: a reminder the evening before a
                    // morning meeting belongs to the evening.
                    on_date: local_at.date_naive(),
                    time: local_at.format("%H:%M").to_string(),
                    label: r.summary.clone(),
                    payload: payload(&r.uid, r.recurrence_id),
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod registry_tests {
    use super::registry::*;
    use arlen_calendar_core::reminders::Registration;
    use chrono::{NaiveDate, TimeZone, Utc};

    fn day(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 8, d).unwrap()
    }

    #[test]
    fn a_user_alarm_is_never_touched() {
        // The rule that makes this safe to run every time a file changes. The
        // clock holds one alarm somebody set and one this daemon registered for
        // an occurrence that has since gone.
        let existing = vec![
            ("morning".into(), None),
            (alarm_id("gone@x", day(20)), Some(payload("gone@x", day(20)))),
        ];
        let plan = plan(&existing, &[]);
        assert_eq!(plan.delete, vec![alarm_id("gone@x", day(20))]);
    }

    #[test]
    fn a_registration_that_still_follows_is_kept_rather_than_deleted() {
        let keep = Desired {
            id: alarm_id("standup@x", day(26)),
            on_date: day(26),
            time: "08:45".into(),
            label: "Standup".into(),
            payload: payload("standup@x", day(26)),
        };
        let existing = vec![(keep.id.clone(), Some(keep.payload.clone()))];
        let plan = plan(&existing, std::slice::from_ref(&keep));
        assert!(plan.delete.is_empty());
        assert_eq!(plan.set, vec![keep]);
    }

    #[test]
    fn an_alarm_with_a_payload_this_daemon_did_not_write_is_left_alone() {
        // Another component's registration. Deleting it because it is not in
        // this calendar's list would be the calendar reaching outside itself.
        let existing = vec![("t".into(), Some(r#"{"source":"timer","id":"7"}"#.into()))];
        assert!(plan(&existing, &[]).delete.is_empty());
    }

    #[test]
    fn a_uid_with_a_quote_in_it_does_not_break_the_payload() {
        let p = payload(r#"od"d@x"#, day(26));
        let parsed: serde_json::Value = serde_json::from_str(&p).expect("valid JSON");
        assert_eq!(parsed["uid"], r#"od"d@x"#);
        assert_eq!(parsed["source"], "calendar");
    }

    #[test]
    fn the_alarm_lands_on_the_day_it_rings_not_the_day_of_the_meeting() {
        // A reminder the evening before a morning meeting belongs to the
        // evening: registering it on the meeting's day would arm it twelve
        // hours late.
        let r = Registration {
            uid: "trip@x".into(),
            recurrence_id: day(21),
            at: Utc.with_ymd_and_hms(2026, 8, 20, 19, 0, 0).unwrap(),
            summary: "Trip".into(),
        };
        let d = &desired_from(&[r], chrono_tz::Tz::UTC)[0];
        assert_eq!(d.on_date, day(20));
        assert_eq!(d.time, "19:00");
        // The key still names the OCCURRENCE, so the registration follows the
        // meeting rather than the evening.
        assert_eq!(d.id, alarm_id("trip@x", day(21)));
    }
}

// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Who may do what to the alarms.
//!
//! The clock app drives everything, which is the case this daemon was written
//! for. But `calendar-app.md` section 4 hands the calendar a second job here:
//! it registers a reminder for each occurrence and re-derives them on every
//! write, so it must be able to set and remove alarms without a person doing it.
//!
//! Admitting it to the whole interface would be the easy answer and the wrong
//! one. Registering a reminder is not the same authority as deleting the alarm
//! somebody set to catch a flight, and the calendar re-derives unattended: a bug
//! in it would be a bug that silently removes your alarms. So a registrant
//! reaches ONLY the alarms carrying its own payload, enforced here rather than
//! left to the registrant's own discipline. The rule holds even if the calendar
//! is wrong about which alarms are its own.
//!
//! A user alarm carries no payload at all, so it is outside every registrant's
//! reach by construction rather than by a list that could go stale.

/// The clock app itself: the caller this daemon exists for.
pub const CLOCK_APP: &str = "clock";

/// Components allowed to register alarms, and the payload source each owns.
///
/// One entry per component that has a reason to arm something in the future
/// without a person present. `calendard` is the calendar daemon, resolved from
/// `/usr/lib/arlen/libexec/arlen-calendard`.
pub const REGISTRARS: &[(&str, &str)] = &[("calendard", "calendar")];

/// How far a caller reaches into the alarm list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    /// Every alarm: the clock app, and in a debug build its build-tree twin.
    Full,
    /// Only alarms carrying this payload source.
    OnlyOwn(&'static str),
    /// Nothing.
    None,
}

/// What `app_id` may reach.
#[must_use]
pub fn reach_of(app_id: &str) -> Reach {
    if app_id == CLOCK_APP {
        return Reach::Full;
    }
    #[cfg(debug_assertions)]
    if app_id == "dev.arlen-clock-app" {
        return Reach::Full;
    }
    for (id, source) in REGISTRARS {
        if app_id == *id {
            return Reach::OnlyOwn(source);
        }
        // The same component run from a build tree, debug only, so `just dev`
        // and the screenshot harness reach a daemon a release build refuses.
        #[cfg(debug_assertions)]
        if app_id == format!("dev.arlen-{id}") {
            return Reach::OnlyOwn(source);
        }
    }
    Reach::None
}

/// Whether `reach` covers an alarm carrying `payload`.
///
/// `None` payload means a person set it, which no registrant may touch. A
/// payload that does not name the registrant's own source is somebody else's
/// registration, equally out of reach: the calendar deleting a timer's alarm
/// because it is not in the calendar's list would be the same failure one step
/// over.
#[must_use]
pub fn may_touch(reach: Reach, payload: Option<&str>) -> bool {
    match reach {
        Reach::Full => true,
        Reach::None => false,
        Reach::OnlyOwn(source) => {
            payload.is_some_and(|p| p.contains(&format!("\"source\":\"{source}\"")))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CAL: &str = r#"{"source":"calendar","uid":"a@x","on":"2026-08-26"}"#;

    #[test]
    fn the_clock_app_reaches_everything_and_a_stranger_reaches_nothing() {
        assert_eq!(reach_of(CLOCK_APP), Reach::Full);
        assert_eq!(reach_of("files"), Reach::None);
        assert!(may_touch(Reach::Full, None));
        assert!(!may_touch(Reach::None, Some(CAL)));
    }

    #[test]
    fn a_registrant_cannot_touch_an_alarm_a_person_set() {
        // The case this module exists for. The calendar re-derives unattended,
        // so a bug in it must not be able to remove the alarm somebody set to
        // catch a flight.
        let reach = reach_of("calendard");
        assert_eq!(reach, Reach::OnlyOwn("calendar"));
        assert!(!may_touch(reach, None));
        assert!(may_touch(reach, Some(CAL)));
    }

    #[test]
    fn a_registrant_cannot_touch_another_registrants_alarm() {
        let reach = reach_of("calendard");
        assert!(!may_touch(reach, Some(r#"{"source":"timer","id":"7"}"#)));
    }
}

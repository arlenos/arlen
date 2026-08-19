//! Alarms, and the one piece of arithmetic they turn on: when does this ring next.
//!
//! Everything the app shows about an alarm derives from `next_fire_at`, so this
//! is the function the surface rests on. It is also the one with the awkward
//! cases, which is why it is here and pure rather than inline in a timer loop:
//! a week that wraps, a time that has already passed today, and the two days a
//! year when a local wall-clock time either does not exist or exists twice.

use chrono::{Datelike, Duration, LocalResult, NaiveTime, TimeZone, Timelike};
use serde::{Deserialize, Serialize};

/// One alarm, in the shape the daemon serves and the app renders.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Alarm {
    /// Stable id, the handle every command names.
    pub id: String,
    /// `HH:MM`, the canonical wall-clock form. Wall clock and not an instant,
    /// because "07:00" means seven in the morning wherever and whenever the
    /// machine is, including the morning after the clocks changed.
    pub time: String,
    /// What the user called it.
    pub label: String,
    /// Days it repeats on, `0..=6` with **0 = Monday** (the kit's DaysPicker
    /// convention, which the app is already built to). Empty means one-shot.
    pub days: Vec<u8>,
    /// The day this rings on, for an alarm that belongs to a date rather than to
    /// a time of day.
    ///
    /// A plain one-shot means "the next 07:00", which is what somebody setting a
    /// morning alarm means. A calendar reminder does not: `calendar-app.md`
    /// section 4 has the calendar register the trigger for one OCCURRENCE, and
    /// 08:45 on the 26th is not the same claim as the next 08:45. Without this
    /// the registration lands a day away from the meeting it is for.
    ///
    /// Serde-default, so an alarm stored before this existed still reads.
    #[serde(default)]
    pub on_date: Option<chrono::NaiveDate>,
    /// Whatever the registrant needs handed back when this rings.
    ///
    /// Opaque here on purpose: the clock owns arming and ringing, and knowing
    /// that a string is an event UID plus a recurrence-id would make it own the
    /// calendar's model too. Carried so a re-derivation can recognise its own
    /// registrations and replace them, which section 4 requires and a
    /// free-floating timer cannot offer.
    ///
    /// It is also the mark of an alarm somebody's software set rather than a
    /// person: a user alarm never carries one. A surface listing alarms has to
    /// respect that, because a calendar reminder shown as an editable alarm
    /// invites an edit the next derivation silently undoes.
    #[serde(default)]
    pub payload: Option<String>,
    /// Whether it is armed.
    pub enabled: bool,
    /// Opt-in fire-late-once: after downtime, ring once rather than dropping
    /// silently or ringing for every occurrence that was missed.
    pub fire_late: bool,
    /// Epoch milliseconds of the next ring, computed by the daemon. `None` when
    /// disabled - an anchor, never a countdown, so a view that renders it late
    /// is still right.
    pub next_fire_at: Option<i64>,
}

/// Parse `HH:MM` into a time of day, or `None` if it is not one.
///
/// Deliberately strict: `7:00`, `07:00:00` and `24:00` are all refused rather
/// than coerced, because an alarm stored in a shape nobody agreed on is one
/// that rings at a time nobody chose.
pub fn parse_hhmm(text: &str) -> Option<NaiveTime> {
    let (h, m) = text.split_once(':')?;
    if h.len() != 2 || m.len() != 2 {
        return None;
    }
    let h: u32 = h.parse().ok()?;
    let m: u32 = m.parse().ok()?;
    NaiveTime::from_hms_opt(h, m, 0)
}

/// When this alarm next rings, as epoch milliseconds in `tz`.
///
/// `None` when it is disabled or its time is unparseable. Strictly after `now`:
/// an alarm set for the current minute rings tomorrow, not immediately, because
/// the alternative is a alarm that fires the moment you finish creating it.
///
/// **The clocks-change cases are handled rather than left to the library's
/// default**, because both produce a wrong-looking alarm on a real morning:
///
/// - A time that does not exist (spring forward skips 02:30) rings at the
///   moment the clock jumps past it, so an alarm set inside the lost hour still
///   goes off that morning rather than silently skipping a day.
/// - A time that exists twice (autumn back repeats 02:30) rings at the first
///   one. Ringing once is the point; the earlier is the one the user meant when
///   they set it the night before.
pub fn next_fire_at<Tz: TimeZone>(alarm: &Alarm, tz: &Tz, now_ms: i64) -> Option<i64> {
    if !alarm.enabled {
        return None;
    }
    let at = parse_hhmm(&alarm.time)?;
    let now = tz.timestamp_millis_opt(now_ms).single()?;
    let today = now.date_naive();

    if let Some(date) = alarm.on_date {
        // A repeat set AND a date are two different claims about when this
        // rings, and there is no reading that honours both. Refused rather than
        // resolved to one of them silently.
        if !alarm.days.is_empty() {
            return None;
        }
        let ms = resolve_local(tz, date, at)?.timestamp_millis();
        // A dated alarm whose moment has passed is over. Rolling it to the next
        // matching wall-clock time would ring for a meeting that already
        // happened, on a day nobody asked about.
        return (ms > now_ms).then_some(ms);
    }

    // A week ahead is enough for any repeat set, and one extra day covers a
    // one-shot whose time has already passed today.
    for ahead in 0..=7 {
        let day = today + Duration::days(ahead);
        if !alarm.days.is_empty() {
            // `weekday().num_days_from_monday()` is 0 = Monday, the same
            // convention the app sends, so the two never need translating.
            let dow = u8::try_from(day.weekday().num_days_from_monday()).ok()?;
            if !alarm.days.contains(&dow) {
                continue;
            }
        } else if ahead > 1 {
            // A one-shot only ever means today or tomorrow.
            break;
        }

        let Some(candidate) = resolve_local(tz, day, at) else {
            continue;
        };
        let ms = candidate.timestamp_millis();
        if ms > now_ms {
            return Some(ms);
        }
    }
    None
}

/// A local wall-clock time as an instant, deciding the two ambiguous cases.
fn resolve_local<Tz: TimeZone>(
    tz: &Tz,
    day: chrono::NaiveDate,
    at: NaiveTime,
) -> Option<chrono::DateTime<Tz>> {
    match tz.from_local_datetime(&day.and_time(at)) {
        // The ordinary case.
        LocalResult::Single(dt) => Some(dt),
        // Repeated: the first of the two.
        LocalResult::Ambiguous(first, _) => Some(first),
        // Skipped: walk forward a minute at a time to the moment the clock
        // reaches the far side of the gap. Bounded by the largest jump anyone
        // actually uses (an hour), so a broken zone cannot spin here.
        LocalResult::None => {
            let mut probe = at;
            for _ in 0..60 {
                probe += Duration::minutes(1);
                if probe.hour() == 0 && probe.minute() == 0 {
                    return None;
                }
                if let LocalResult::Single(dt) = tz.from_local_datetime(&day.and_time(probe)) {
                    return Some(dt);
                }
            }
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{FixedOffset, TimeZone};
    use chrono_tz::Tz;

    /// A fixed zone with no clock changes, for the cases that are not about them.
    fn utc() -> FixedOffset {
        FixedOffset::east_opt(0).unwrap()
    }

    fn at(tz: &FixedOffset, y: i32, m: u32, d: u32, h: u32, min: u32) -> i64 {
        tz.with_ymd_and_hms(y, m, d, h, min, 0)
            .unwrap()
            .timestamp_millis()
    }

    fn alarm(time: &str, days: &[u8]) -> Alarm {
        Alarm {
            id: "a".into(),
            time: time.into(),
            label: String::new(),
            days: days.to_vec(),
            enabled: true,
            fire_late: false,
            on_date: None,
            payload: None,
            next_fire_at: None,
        }
    }

    #[test]
    fn a_dated_alarm_rings_on_its_own_day_rather_than_the_next_matching_time() {
        let tz = utc();
        // Wednesday 2026-08-19, 09:00. A reminder for the 26th at 08:45 is a
        // week out; a plain one-shot at 08:45 would ring TOMORROW, which is the
        // whole reason a date exists.
        let now = at(&tz, 2026, 8, 19, 9, 0);
        let mut a = alarm("08:45", &[]);
        assert_eq!(next_fire_at(&a, &tz, now), Some(at(&tz, 2026, 8, 20, 8, 45)));
        a.on_date = Some(chrono::NaiveDate::from_ymd_opt(2026, 8, 26).unwrap());
        assert_eq!(next_fire_at(&a, &tz, now), Some(at(&tz, 2026, 8, 26, 8, 45)));
    }

    #[test]
    fn a_dated_alarm_whose_day_has_passed_is_over() {
        let tz = utc();
        let now = at(&tz, 2026, 8, 19, 9, 0);
        let mut a = alarm("08:45", &[]);
        a.on_date = Some(chrono::NaiveDate::from_ymd_opt(2026, 8, 19).unwrap());
        // Rolling it forward would ring for a meeting that already happened.
        assert_eq!(next_fire_at(&a, &tz, now), None);
    }

    #[test]
    fn a_date_and_a_repeat_set_together_are_refused_rather_than_resolved() {
        let tz = utc();
        let now = at(&tz, 2026, 8, 19, 9, 0);
        let mut a = alarm("08:45", &[0, 1, 2]);
        a.on_date = Some(chrono::NaiveDate::from_ymd_opt(2026, 8, 26).unwrap());
        // Two different claims about when this rings; picking one silently would
        // arm an alarm nobody described.
        assert_eq!(next_fire_at(&a, &tz, now), None);
    }

    #[test]
    fn an_alarm_stored_before_dates_existed_still_reads() {
        let stored = r#"{"id":"a","time":"07:00","label":"","days":[],"enabled":true,
            "fire_late":false,"next_fire_at":null}"#;
        let a: Alarm = serde_json::from_str(stored).expect("old shape parses");
        assert_eq!(a.on_date, None);
        assert_eq!(a.payload, None);
    }

    #[test]
    fn a_one_shot_later_today_rings_today() {
        let tz = utc();
        // Wednesday 2026-08-05, 06:00. An alarm at 07:00 is three hours off.
        let now = at(&tz, 2026, 8, 5, 6, 0);
        assert_eq!(
            next_fire_at(&alarm("07:00", &[]), &tz, now),
            Some(at(&tz, 2026, 8, 5, 7, 0))
        );
    }

    #[test]
    fn a_one_shot_already_past_rings_tomorrow() {
        let tz = utc();
        let now = at(&tz, 2026, 8, 5, 9, 0);
        assert_eq!(
            next_fire_at(&alarm("07:00", &[]), &tz, now),
            Some(at(&tz, 2026, 8, 6, 7, 0))
        );
    }

    /// An alarm set for the current minute belongs to tomorrow, or creating one
    /// would set it off.
    #[test]
    fn the_current_minute_is_not_the_next_ring() {
        let tz = utc();
        let now = at(&tz, 2026, 8, 5, 7, 0);
        assert_eq!(
            next_fire_at(&alarm("07:00", &[]), &tz, now),
            Some(at(&tz, 2026, 8, 6, 7, 0))
        );
    }

    /// 0 = Monday, the convention the app sends. A weekday alarm on a Friday
    /// evening next rings on Monday, not on Saturday.
    #[test]
    fn a_repeat_set_skips_to_its_next_day_and_wraps_the_week() {
        let tz = utc();
        // Friday 2026-08-07, 20:00; the set is Mon..Fri.
        let now = at(&tz, 2026, 8, 7, 20, 0);
        assert_eq!(
            next_fire_at(&alarm("07:00", &[0, 1, 2, 3, 4]), &tz, now),
            Some(at(&tz, 2026, 8, 10, 7, 0))
        );
    }

    #[test]
    fn a_repeat_on_today_still_ahead_rings_today() {
        let tz = utc();
        // Wednesday = 2 in the Monday-first convention.
        let now = at(&tz, 2026, 8, 5, 6, 0);
        assert_eq!(
            next_fire_at(&alarm("07:00", &[2]), &tz, now),
            Some(at(&tz, 2026, 8, 5, 7, 0))
        );
    }

    /// A single-day repeat that has passed waits a whole week.
    #[test]
    fn a_repeat_on_today_already_past_waits_a_week() {
        let tz = utc();
        let now = at(&tz, 2026, 8, 5, 9, 0);
        assert_eq!(
            next_fire_at(&alarm("07:00", &[2]), &tz, now),
            Some(at(&tz, 2026, 8, 12, 7, 0))
        );
    }

    #[test]
    fn a_disabled_alarm_has_no_next_ring() {
        let tz = utc();
        let mut a = alarm("07:00", &[]);
        a.enabled = false;
        assert_eq!(next_fire_at(&a, &tz, at(&tz, 2026, 8, 5, 6, 0)), None);
    }

    /// Wall clock, not an instant: the morning after the clocks go forward,
    /// 07:00 is still 07:00 and the alarm is an hour closer in real time.
    #[test]
    fn an_alarm_stays_at_its_wall_clock_time_across_a_clock_change() {
        let berlin = Tz::Europe__Berlin;
        // 2026-03-29 is the spring-forward Sunday in Europe.
        let sat_evening = berlin
            .with_ymd_and_hms(2026, 3, 28, 22, 0, 0)
            .unwrap()
            .timestamp_millis();
        let fire = next_fire_at(&alarm("07:00", &[]), &berlin, sat_evening).unwrap();
        let local = berlin.timestamp_millis_opt(fire).unwrap();
        assert_eq!((local.hour(), local.minute()), (7, 0));
        assert_eq!(local.day(), 29);
    }

    /// An alarm inside the lost hour still goes off that morning, at the moment
    /// the clock reaches the far side of the gap - rather than silently missing
    /// the day it was set for.
    #[test]
    fn an_alarm_in_the_skipped_hour_rings_when_the_clock_passes_it() {
        let berlin = Tz::Europe__Berlin;
        let sat_evening = berlin
            .with_ymd_and_hms(2026, 3, 28, 22, 0, 0)
            .unwrap()
            .timestamp_millis();
        // 02:30 does not exist on 2026-03-29 in Berlin.
        let fire = next_fire_at(&alarm("02:30", &[]), &berlin, sat_evening).unwrap();
        let local = berlin.timestamp_millis_opt(fire).unwrap();
        assert_eq!(local.day(), 29);
        assert_eq!((local.hour(), local.minute()), (3, 0));
    }

    /// A time that happens twice rings once, at the first.
    #[test]
    fn an_alarm_in_the_repeated_hour_rings_at_the_first_one() {
        let berlin = Tz::Europe__Berlin;
        // 2026-10-25 is the autumn Sunday; 02:30 occurs twice.
        let sat_evening = berlin
            .with_ymd_and_hms(2026, 10, 24, 22, 0, 0)
            .unwrap()
            .timestamp_millis();
        let fire = next_fire_at(&alarm("02:30", &[]), &berlin, sat_evening).unwrap();
        // The first 02:30 is still on summer time, two hours ahead of UTC.
        let utc_hour = chrono::DateTime::from_timestamp_millis(fire)
            .unwrap()
            .hour();
        assert_eq!(utc_hour, 0, "the earlier of the two 02:30s");
    }

    #[test]
    fn a_time_that_is_not_hh_mm_is_refused() {
        for bad in ["7:00", "07:00:00", "24:00", "07:60", "", "0700", "aa:bb"] {
            assert!(parse_hhmm(bad).is_none(), "accepted {bad:?}");
        }
        assert!(parse_hhmm("00:00").is_some());
        assert!(parse_hhmm("23:59").is_some());
    }

    #[test]
    fn an_alarm_survives_the_wire_unchanged() {
        let a = alarm("07:00", &[0, 4]);
        let back: Alarm = serde_json::from_str(&serde_json::to_string(&a).unwrap()).unwrap();
        assert_eq!(back, a);
    }
}

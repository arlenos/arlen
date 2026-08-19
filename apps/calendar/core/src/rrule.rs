//! Working out when a repeating event actually happens.
//!
//! The parser flags `RRULE` and keeps it verbatim; this turns it into dates. It
//! is deliberately a SUBSET of RFC 5545 §3.3.10, and the subset is the point:
//! the recurrence grammar is large enough that a partial implementation
//! pretending to be whole is worse than one that says what it does not know.
//!
//! **What it expands:** `FREQ` daily, weekly, monthly and yearly, with
//! `INTERVAL`, `BYDAY` (weekly), `COUNT` and `UNTIL`.
//!
//! **What it refuses:** everything else - `BYSETPOS`, `BYMONTHDAY`, `BYYEARDAY`,
//! `BYWEEKNO`, `WKST` other than the default, a `BYDAY` carrying an ordinal
//! (`3MO`, "the third Monday"). [`expand`] returns `None` for those, and the
//! surface then says the event repeats without saying when, which is what it
//! said before this existed. A rule half-applied would put a meeting on a day
//! nobody agreed to, and a calendar that does that is worse than one that admits
//! the gap.
//!
//! Bounded by a window, never by "generate the next N": an unbounded rule
//! (`FREQ=DAILY` with no `COUNT` or `UNTIL`) is infinite, and the caller always
//! knows which dates it is drawing.

use chrono::{Datelike, Duration, NaiveDate, Weekday};

/// How often a rule repeats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freq {
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

/// A recurrence rule, as far as this understands one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    pub freq: Freq,
    /// Every `interval` periods. RFC 5545's default is 1.
    pub interval: u32,
    /// The weekdays a weekly rule fires on. Empty means "the weekday the event
    /// starts on", which is what the specification says.
    pub by_day: Vec<Weekday>,
    /// Stop after this many occurrences, counting the first.
    pub count: Option<u32>,
    /// Stop after this date, inclusive.
    pub until: Option<NaiveDate>,
}

/// Parse an `RRULE` value, or `None` when it uses a part this does not model.
///
/// Refusing is the whole contract: a rule that mentions `BYSETPOS` is not
/// approximately a rule without it.
pub fn parse_rule(value: &str) -> Option<Rule> {
    let mut freq = None;
    let mut interval = 1u32;
    let mut by_day = Vec::new();
    let mut count = None;
    let mut until = None;

    for part in value.split(';').filter(|p| !p.is_empty()) {
        let (name, v) = part.split_once('=')?;
        match name.trim().to_uppercase().as_str() {
            "FREQ" => {
                freq = Some(match v.trim().to_uppercase().as_str() {
                    "DAILY" => Freq::Daily,
                    "WEEKLY" => Freq::Weekly,
                    "MONTHLY" => Freq::Monthly,
                    "YEARLY" => Freq::Yearly,
                    // SECONDLY, MINUTELY and HOURLY exist and no calendar UI
                    // this shape can show them usefully.
                    _ => return None,
                })
            }
            "INTERVAL" => interval = v.trim().parse().ok().filter(|n| *n > 0)?,
            "COUNT" => count = Some(v.trim().parse().ok().filter(|n| *n > 0)?),
            "UNTIL" => until = Some(parse_until(v.trim())?),
            "BYDAY" => {
                for day in v.split(',').map(str::trim).filter(|d| !d.is_empty()) {
                    // An ordinal prefix ("3MO") is a different rule entirely.
                    by_day.push(weekday(day)?);
                }
            }
            // WKST only matters for rules this does not expand anyway, but a
            // non-default value is a signal the rule is more than it looks.
            "WKST" if v.trim().eq_ignore_ascii_case("MO") => {}
            // Anything else: not modelled, so not expanded.
            _ => return None,
        }
    }
    Some(Rule {
        freq: freq?,
        interval,
        by_day,
        count,
        until,
    })
}

/// `UNTIL` is a DATE or a DATE-TIME (usually UTC). Only the date matters here,
/// because expansion produces dates and the caller carries the time of day from
/// the event's own start.
fn parse_until(v: &str) -> Option<NaiveDate> {
    let date = v.split('T').next()?;
    NaiveDate::parse_from_str(date, "%Y%m%d").ok()
}

fn weekday(code: &str) -> Option<Weekday> {
    Some(match code.to_uppercase().as_str() {
        "MO" => Weekday::Mon,
        "TU" => Weekday::Tue,
        "WE" => Weekday::Wed,
        "TH" => Weekday::Thu,
        "FR" => Weekday::Fri,
        "SA" => Weekday::Sat,
        "SU" => Weekday::Sun,
        _ => return None,
    })
}

/// The dates this rule puts inside `[from, to]`, starting from `start`.
///
/// `start` is the event's own first date and is itself an occurrence, per the
/// specification. Returns `None` when the rule is one [`parse_rule`] refuses.
/// The window is inclusive at both ends and the result is ordered.
pub fn expand(rule: &str, start: NaiveDate, from: NaiveDate, to: NaiveDate) -> Option<Vec<NaiveDate>> {
    let rule = parse_rule(rule)?;
    if from > to {
        return Some(Vec::new());
    }
    let mut out = Vec::new();
    let mut seen = 0u32;

    // A guard on the walk itself, not only on the window: a yearly rule asked
    // about a decade is a handful of steps, a daily rule about the same span is
    // a few thousand, and a rule whose interval never advances the date would
    // otherwise spin. Nothing legitimate needs more than this.
    const MAX_STEPS: u32 = 20_000;
    let mut steps = 0u32;

    match rule.freq {
        Freq::Weekly if !rule.by_day.is_empty() => {
            // Walk week by week from the start's own week, emitting the named
            // days in order. The interval counts WEEKS, so a fortnightly rule
            // skips the weeks in between rather than the days.
            let week_start = start - Duration::days(start.weekday().num_days_from_monday() as i64);
            let mut week = week_start;
            let mut days: Vec<Weekday> = rule.by_day.clone();
            days.sort_by_key(|d| d.num_days_from_monday());
            days.dedup();
            loop {
                steps += 1;
                if steps > MAX_STEPS || week > to {
                    break;
                }
                for d in &days {
                    let date = week + Duration::days(d.num_days_from_monday() as i64);
                    if date < start {
                        continue; // the rule does not reach back before its event
                    }
                    if rule.until.is_some_and(|u| date > u) {
                        return Some(out);
                    }
                    seen += 1;
                    if rule.count.is_some_and(|c| seen > c) {
                        return Some(out);
                    }
                    if date >= from && date <= to {
                        out.push(date);
                    }
                }
                week += Duration::weeks(rule.interval as i64);
            }
        }
        _ => {
            let mut date = start;
            loop {
                steps += 1;
                if steps > MAX_STEPS || date > to {
                    break;
                }
                if rule.until.is_some_and(|u| date > u) {
                    break;
                }
                seen += 1;
                if rule.count.is_some_and(|c| seen > c) {
                    break;
                }
                if date >= from {
                    out.push(date);
                }
                date = match advance(date, rule.freq, rule.interval, start) {
                    Some(next) => next,
                    // A step that cannot be represented (past the calendar's
                    // range) ends the walk rather than wrapping.
                    None => break,
                };
            }
        }
    }
    out.sort();
    out.dedup();
    Some(out)
}

/// One period on from `date`.
///
/// Monthly and yearly anchor on the ORIGINAL day of month, so a rule starting on
/// the 31st lands on the 31st of the months that have one - marching a
/// month-clamped date forward would drag a 31st permanently down to the 28th
/// after one February.
fn advance(date: NaiveDate, freq: Freq, interval: u32, start: NaiveDate) -> Option<NaiveDate> {
    match freq {
        Freq::Daily => date.checked_add_signed(Duration::days(interval as i64)),
        Freq::Weekly => date.checked_add_signed(Duration::weeks(interval as i64)),
        Freq::Monthly => {
            let mut months = (date.year() as i64 * 12 + date.month0() as i64) + interval as i64;
            // Skip the months that have no such day (the 31st of February),
            // which is what the specification says to do: those occurrences do
            // not exist rather than sliding to the 28th.
            for _ in 0..48 {
                let y = months.div_euclid(12) as i32;
                let m = months.rem_euclid(12) as u32 + 1;
                if let Some(d) = NaiveDate::from_ymd_opt(y, m, start.day()) {
                    return Some(d);
                }
                months += interval as i64;
            }
            None
        }
        Freq::Yearly => {
            let mut year = date.year() + interval as i32;
            // The 29th of February, again: a yearly rule on it happens in leap
            // years only.
            for _ in 0..8 {
                if let Some(d) = NaiveDate::from_ymd_opt(year, start.month(), start.day()) {
                    return Some(d);
                }
                year += interval as i32;
            }
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn a_daily_rule_fills_the_window() {
        let got = expand("FREQ=DAILY", d(2026, 8, 19), d(2026, 8, 19), d(2026, 8, 22)).unwrap();
        assert_eq!(got, vec![d(2026, 8, 19), d(2026, 8, 20), d(2026, 8, 21), d(2026, 8, 22)]);
    }

    /// The weekday standup: the days it names, not the day it starts on.
    #[test]
    fn a_weekly_rule_fires_on_the_days_it_names() {
        let got = expand(
            "FREQ=WEEKLY;BYDAY=MO,WE,FR",
            d(2026, 8, 19), // a Wednesday
            d(2026, 8, 17),
            d(2026, 8, 28),
        )
        .unwrap();
        assert_eq!(
            got,
            vec![d(2026, 8, 19), d(2026, 8, 21), d(2026, 8, 24), d(2026, 8, 26), d(2026, 8, 28)],
            "nothing before the event's own start, then Mon/Wed/Fri"
        );
    }

    /// An interval counts PERIODS. A fortnightly rule skips whole weeks, and
    /// getting this wrong shows every week - the most visible way to be wrong.
    #[test]
    fn a_fortnightly_rule_skips_the_week_between() {
        let got = expand(
            "FREQ=WEEKLY;INTERVAL=2;BYDAY=MO",
            d(2026, 8, 17),
            d(2026, 8, 1),
            d(2026, 9, 30),
        )
        .unwrap();
        assert_eq!(got, vec![d(2026, 8, 17), d(2026, 8, 31), d(2026, 9, 14), d(2026, 9, 28)]);
    }

    #[test]
    fn count_stops_the_series_wherever_the_window_is() {
        let got = expand("FREQ=DAILY;COUNT=3", d(2026, 8, 19), d(2026, 8, 1), d(2026, 12, 31)).unwrap();
        assert_eq!(got.len(), 3);
        // And the count is spent even by occurrences the window does not show.
        let got = expand("FREQ=DAILY;COUNT=3", d(2026, 8, 19), d(2026, 8, 21), d(2026, 12, 31)).unwrap();
        assert_eq!(got, vec![d(2026, 8, 21)], "two occurrences fell before the window");
    }

    #[test]
    fn until_ends_the_series_inclusively() {
        let got = expand(
            "FREQ=DAILY;UNTIL=20260821T235959Z",
            d(2026, 8, 19),
            d(2026, 8, 1),
            d(2026, 12, 31),
        )
        .unwrap();
        assert_eq!(got, vec![d(2026, 8, 19), d(2026, 8, 20), d(2026, 8, 21)]);
    }

    /// A monthly rule on the 31st happens in the months that have one. Clamping
    /// to the 28th would move every later occurrence permanently.
    #[test]
    fn a_monthly_rule_on_the_31st_skips_the_months_without_one() {
        let got = expand("FREQ=MONTHLY", d(2026, 1, 31), d(2026, 1, 1), d(2026, 6, 30)).unwrap();
        assert_eq!(got, vec![d(2026, 1, 31), d(2026, 3, 31), d(2026, 5, 31)]);
    }

    #[test]
    fn a_yearly_rule_on_the_29th_of_february_happens_in_leap_years() {
        let got = expand("FREQ=YEARLY", d(2024, 2, 29), d(2024, 1, 1), d(2033, 1, 1)).unwrap();
        assert_eq!(got, vec![d(2024, 2, 29), d(2028, 2, 29), d(2032, 2, 29)]);
    }

    /// The refusal is the contract. A rule with a part this does not model must
    /// not be approximated - "the third Monday" is not "every Monday".
    #[test]
    fn a_rule_this_does_not_model_is_refused_rather_than_approximated() {
        assert!(expand("FREQ=MONTHLY;BYSETPOS=-1;BYDAY=FR", d(2026, 8, 1), d(2026, 8, 1), d(2026, 9, 1)).is_none());
        assert!(expand("FREQ=MONTHLY;BYDAY=3MO", d(2026, 8, 1), d(2026, 8, 1), d(2026, 9, 1)).is_none());
        assert!(expand("FREQ=MONTHLY;BYMONTHDAY=1,15", d(2026, 8, 1), d(2026, 8, 1), d(2026, 9, 1)).is_none());
        assert!(expand("FREQ=HOURLY", d(2026, 8, 1), d(2026, 8, 1), d(2026, 9, 1)).is_none());
        assert!(expand("FREQ=DAILY;INTERVAL=0", d(2026, 8, 1), d(2026, 8, 1), d(2026, 9, 1)).is_none());
    }

    /// An unbounded daily rule over a decade must not be an unbounded walk.
    #[test]
    fn an_open_ended_rule_is_bounded_by_the_window() {
        let got = expand("FREQ=DAILY", d(2020, 1, 1), d(2030, 1, 1), d(2030, 1, 3)).unwrap();
        assert_eq!(got, vec![d(2030, 1, 1), d(2030, 1, 2), d(2030, 1, 3)]);
    }

    #[test]
    fn an_empty_window_yields_nothing_rather_than_everything() {
        let got = expand("FREQ=DAILY", d(2026, 8, 19), d(2026, 8, 22), d(2026, 8, 20)).unwrap();
        assert!(got.is_empty());
    }
}

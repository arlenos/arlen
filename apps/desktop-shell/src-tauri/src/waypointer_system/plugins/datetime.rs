/// DateTime / timezone plugin.
///
/// Handles queries like `time`, `date`, `time in Tokyo`, `in 3 hours`,
/// `next friday`, `yesterday`, `tomorrow`. Public helper `try_evaluate`
/// is used by the `evaluate_waypointer_input` Tauri command so the shell
/// keeps a single inline-result flow across math, units, and datetime.

use chrono::{Datelike, Duration, Local, Timelike};
use chrono_tz::Tz;

use crate::waypointer_plugins::WaypointerResult;
use crate::waypointer_system::plugin::*;

pub struct DateTimePlugin;

impl WaypointerPlugin for DateTimePlugin {
    fn id(&self) -> &str { "core.datetime" }
    fn name(&self) -> &str { "Date & Time" }
    fn description(&self) -> &str { "Current time, dates, timezones, weekdays, and relative offsets like \"in 3 hours\"." }
    fn priority(&self) -> u32 { 3 }
    fn max_results(&self) -> usize { 1 }

    fn search(&self, query: &str) -> Vec<SearchResult> {
        match try_evaluate(query) {
            Some(r) => vec![SearchResult {
                id: "datetime-result".into(),
                title: r.display.clone(),
                description: Some("Date & Time".into()),
                icon: Some("clock".into()),
                relevance: 0.95,
                action: Action::Copy { text: r.copy_value },
                plugin_id: String::new(),
            }],
            None => Vec::new(),
        }
    }

    fn execute(&self, _result: &SearchResult) -> Result<(), PluginError> {
        Ok(()) // Copy handled by shell clipboard.
    }
}

// ── Public helpers (used by evaluate_waypointer_input) ──────────────────

/// Evaluates a date/time query. Returns None if the input doesn't look like one.
pub fn try_evaluate(input: &str) -> Option<WaypointerResult> {
    let lower = input.to_lowercase();
    let lower = lower.trim();
    let now = Local::now();

    if lower == "time" || lower == "time now" || lower == "current time" {
        let display = now.format("%H:%M:%S").to_string();
        return Some(dt_result(&display, &display));
    }

    if lower == "date" || lower == "today" || lower == "current date" {
        let display = format_date(&now);
        let copy = now.format("%Y-%m-%d").to_string();
        return Some(dt_result(&display, &copy));
    }

    if lower == "yesterday" {
        let d = now - Duration::days(1);
        let display = format_date(&d);
        let copy = d.format("%Y-%m-%d").to_string();
        return Some(dt_result(&display, &copy));
    }

    if lower == "tomorrow" {
        let d = now + Duration::days(1);
        let display = format_date(&d);
        let copy = d.format("%Y-%m-%d").to_string();
        return Some(dt_result(&display, &copy));
    }

    if let Some(rest) = lower.strip_prefix("time ") {
        let rest = rest.trim();
        if let Some(display) = resolve_timezone_time(rest) {
            return Some(dt_result(&display, &display));
        }
    }

    if let Some(rest) = lower.strip_prefix("in ") {
        if let Some(r) = parse_duration_offset(rest, &now) {
            return Some(r);
        }
    }

    if let Some(rest) = lower.strip_prefix("next ") {
        if let Some(weekday) = parse_weekday(rest.trim()) {
            let mut d = now + Duration::days(1);
            while d.weekday() != weekday {
                d = d + Duration::days(1);
            }
            let display = format_date(&d);
            let copy = d.format("%Y-%m-%d").to_string();
            return Some(dt_result(&display, &copy));
        }
    }

    None
}

fn dt_result(display: &str, copy_value: &str) -> WaypointerResult {
    WaypointerResult {
        result_type: "datetime".to_string(),
        display: display.to_string(),
        copy_value: copy_value.to_string(),
    }
}

fn format_date<T: chrono::Datelike + chrono::Timelike>(dt: &T) -> String
where
    T: std::fmt::Debug,
{
    let weekday = match dt.weekday() {
        chrono::Weekday::Mon => "Monday",
        chrono::Weekday::Tue => "Tuesday",
        chrono::Weekday::Wed => "Wednesday",
        chrono::Weekday::Thu => "Thursday",
        chrono::Weekday::Fri => "Friday",
        chrono::Weekday::Sat => "Saturday",
        chrono::Weekday::Sun => "Sunday",
    };
    let month = match dt.month() {
        1 => "January", 2 => "February", 3 => "March", 4 => "April",
        5 => "May", 6 => "June", 7 => "July", 8 => "August",
        9 => "September", 10 => "October", 11 => "November", 12 => "December",
        _ => "",
    };
    format!("{}, {}. {} {}", weekday, dt.day(), month, dt.year())
}

fn resolve_timezone_time(input: &str) -> Option<String> {
    use chrono::Utc;

    let now_utc = Utc::now();

    if input == "utc" {
        return Some(now_utc.format("%H:%M:%S UTC").to_string());
    }
    if let Some(rest) = input.strip_prefix("utc") {
        if let Ok(offset) = rest.trim().parse::<i32>() {
            let secs = offset * 3600;
            let tz = chrono::FixedOffset::east_opt(secs)?;
            let t = now_utc.with_timezone(&tz);
            return Some(format!("{} UTC{:+}", t.format("%H:%M:%S"), offset));
        }
    }

    if let Some(tz) = city_to_tz(input) {
        let t = now_utc.with_timezone(&tz);
        return Some(format!("{} ({})", t.format("%H:%M:%S"), tz.name()));
    }

    if let Ok(tz) = input.parse::<Tz>() {
        let t = now_utc.with_timezone(&tz);
        return Some(format!("{} ({})", t.format("%H:%M:%S"), tz.name()));
    }

    // Retry with capitalisation (users type "europe/vienna").
    let capitalized: String = input
        .split('/')
        .map(|part| {
            let mut c = part.chars();
            match c.next() {
                Some(first) => first.to_uppercase().to_string() + &c.as_str().to_lowercase(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join("/");
    if let Ok(tz) = capitalized.parse::<Tz>() {
        let t = now_utc.with_timezone(&tz);
        return Some(format!("{} ({})", t.format("%H:%M:%S"), tz.name()));
    }

    None
}

fn city_to_tz(city: &str) -> Option<chrono_tz::Tz> {
    let iana = match city {
        "vienna" | "wien" => "Europe/Vienna",
        "berlin" => "Europe/Berlin",
        "munich" | "muenchen" | "munchen" => "Europe/Berlin",
        "zurich" | "zuerich" => "Europe/Zurich",
        "london" => "Europe/London",
        "paris" => "Europe/Paris",
        "rome" | "roma" => "Europe/Rome",
        "madrid" => "Europe/Madrid",
        "amsterdam" => "Europe/Amsterdam",
        "brussels" | "bruxelles" => "Europe/Brussels",
        "stockholm" => "Europe/Stockholm",
        "oslo" => "Europe/Oslo",
        "copenhagen" | "kopenhagen" => "Europe/Copenhagen",
        "helsinki" => "Europe/Helsinki",
        "warsaw" | "warschau" => "Europe/Warsaw",
        "prague" | "prag" => "Europe/Prague",
        "budapest" => "Europe/Budapest",
        "bucharest" | "bukarest" => "Europe/Bucharest",
        "athens" | "athen" => "Europe/Athens",
        "istanbul" => "Europe/Istanbul",
        "moscow" | "moskau" => "Europe/Moscow",
        "new york" | "nyc" => "America/New_York",
        "los angeles" | "la" => "America/Los_Angeles",
        "chicago" => "America/Chicago",
        "denver" => "America/Denver",
        "toronto" => "America/Toronto",
        "vancouver" => "America/Vancouver",
        "mexico city" => "America/Mexico_City",
        "sao paulo" => "America/Sao_Paulo",
        "buenos aires" => "America/Argentina/Buenos_Aires",
        "tokyo" => "Asia/Tokyo",
        "beijing" | "peking" => "Asia/Shanghai",
        "shanghai" => "Asia/Shanghai",
        "hong kong" => "Asia/Hong_Kong",
        "singapore" => "Asia/Singapore",
        "seoul" => "Asia/Seoul",
        "mumbai" | "bombay" => "Asia/Kolkata",
        "delhi" => "Asia/Kolkata",
        "dubai" => "Asia/Dubai",
        "sydney" => "Australia/Sydney",
        "melbourne" => "Australia/Melbourne",
        "auckland" => "Pacific/Auckland",
        "honolulu" => "Pacific/Honolulu",
        "innsbruck" => "Europe/Vienna",
        "graz" => "Europe/Vienna",
        "salzburg" => "Europe/Vienna",
        "linz" => "Europe/Vienna",
        _ => return None,
    };
    iana.parse().ok()
}

fn parse_duration_offset(
    input: &str,
    now: &chrono::DateTime<chrono::Local>,
) -> Option<WaypointerResult> {
    let parts: Vec<&str> = input.split_whitespace().collect();
    if parts.len() != 2 {
        return None;
    }
    let n: i64 = parts[0].parse().ok()?;
    let unit = parts[1];

    let delta = match unit {
        "hour" | "hours" | "h" => Duration::hours(n),
        "minute" | "minutes" | "min" | "mins" => Duration::minutes(n),
        "day" | "days" | "d" => Duration::days(n),
        "week" | "weeks" | "w" => Duration::weeks(n),
        _ => return None,
    };

    let future = *now + delta;
    let is_date_change = future.date_naive() != now.date_naive();

    let display = if is_date_change {
        format!("{}, {}", format_date(&future), future.format("%H:%M"))
    } else {
        future.format("%H:%M:%S").to_string()
    };
    let copy = if is_date_change {
        future.format("%Y-%m-%d %H:%M").to_string()
    } else {
        future.format("%H:%M:%S").to_string()
    };

    Some(dt_result(&display, &copy))
}

fn parse_weekday(s: &str) -> Option<chrono::Weekday> {
    match s {
        "monday" | "mon" => Some(chrono::Weekday::Mon),
        "tuesday" | "tue" => Some(chrono::Weekday::Tue),
        "wednesday" | "wed" => Some(chrono::Weekday::Wed),
        "thursday" | "thu" => Some(chrono::Weekday::Thu),
        "friday" | "fri" => Some(chrono::Weekday::Fri),
        "saturday" | "sat" => Some(chrono::Weekday::Sat),
        "sunday" | "sun" => Some(chrono::Weekday::Sun),
        _ => None,
    }
}

// ── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_keyword_returns_local_time() {
        let r = try_evaluate("time").unwrap();
        assert_eq!(r.result_type, "datetime");
    }

    #[test]
    fn date_keyword_returns_formatted_date() {
        let r = try_evaluate("date").unwrap();
        assert_eq!(r.result_type, "datetime");
        // Copy format is YYYY-MM-DD.
        assert_eq!(r.copy_value.len(), 10);
    }

    #[test]
    fn tomorrow_is_one_day_ahead() {
        let today = try_evaluate("today").unwrap();
        let tomorrow = try_evaluate("tomorrow").unwrap();
        assert_ne!(today.copy_value, tomorrow.copy_value);
    }

    #[test]
    fn time_prefix_with_city() {
        let r = try_evaluate("time tokyo").unwrap();
        // Display includes the IANA name after resolving the city.
        assert!(r.display.contains("Asia/Tokyo"));
    }

    #[test]
    fn time_prefix_with_utc() {
        let r = try_evaluate("time utc").unwrap();
        assert!(r.display.contains("UTC"));
    }

    #[test]
    fn time_prefix_with_offset() {
        let r = try_evaluate("time utc+3").unwrap();
        assert!(r.display.contains("UTC"));
    }

    #[test]
    fn non_datetime_returns_none() {
        assert!(try_evaluate("hello").is_none());
        assert!(try_evaluate("2+2").is_none());
        assert!(try_evaluate("").is_none());
    }

    #[test]
    fn next_weekday() {
        let r = try_evaluate("next monday").unwrap();
        assert_eq!(r.result_type, "datetime");
    }

    #[test]
    fn in_n_hours() {
        let r = try_evaluate("in 3 hours").unwrap();
        assert_eq!(r.result_type, "datetime");
    }
}

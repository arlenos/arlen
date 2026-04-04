/// Waypointer inline plugins: calculator and unit converter.

use serde::Serialize;

/// Result from evaluating a Waypointer input.
#[derive(Clone, Serialize)]
pub struct WaypointerResult {
    /// "math", "unit"
    pub result_type: String,
    /// Human-readable result for display.
    pub display: String,
    /// Value suitable for clipboard copy.
    pub copy_value: String,
}

/// Evaluates input as a math expression or unit conversion.
///
/// Runs the evaluation on a background thread with a 200ms timeout and
/// panic safety so that bad input cannot crash or hang the process.
#[tauri::command]
pub fn evaluate_waypointer_input(input: String) -> Option<WaypointerResult> {
    use std::sync::mpsc;
    use std::time::Duration;

    let input_trimmed = input.trim().to_string();
    if input_trimmed.is_empty() || input_trimmed.len() > 200 {
        return None;
    }

    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            evaluate_inner(&input_trimmed)
        }));
        let _ = tx.send(result);
    });

    match rx.recv_timeout(Duration::from_millis(200)) {
        Ok(Ok(r)) => r,
        Ok(Err(_)) => {
            log::warn!("evaluate_waypointer_input: evaluation panicked");
            Some(WaypointerResult {
                result_type: "error".to_string(),
                display: "Computation too complex".to_string(),
                copy_value: String::new(),
            })
        }
        Err(_) => {
            log::warn!("evaluate_waypointer_input: evaluation timed out");
            Some(WaypointerResult {
                result_type: "error".to_string(),
                display: "Computation too complex".to_string(),
                copy_value: String::new(),
            })
        }
    }
}

/// Inner evaluation logic, called from the sandboxed thread.
fn evaluate_inner(input: &str) -> Option<WaypointerResult> {
    // DateTime first (exact keyword matches).
    if let Some(r) = evaluate_datetime(input) {
        return Some(r);
    }
    if let Some(r) = convert_units(input) {
        return Some(r);
    }
    if let Some(r) = convert_single_unit(input) {
        return Some(r);
    }
    if let Some(r) = evaluate_math(input) {
        return Some(r);
    }
    None
}

// ===== Math evaluation =====

/// Evaluates a math expression using meval (with custom functions).
fn evaluate_math(input: &str) -> Option<WaypointerResult> {
    // Strip leading/trailing '=' that users might type.
    let expr = input.trim_start_matches('=').trim_end_matches('=').trim();
    if expr.is_empty() {
        return None;
    }

    // Must look like a math expression.
    let has_math = expr.contains('+')
        || expr.contains('-')
        || expr.contains('*')
        || expr.contains('/')
        || expr.contains('^')
        || expr.contains('%')
        || expr.contains('(')
        || input.starts_with('=')
        || input.ends_with('=');

    if !has_math {
        return None;
    }

    // Normalize common symbols.
    let normalized = expr
        .replace('x', "*")
        .replace('X', "*")
        .replace(',', "")
        .replace("**", "^");

    // Build meval context with custom functions.
    let result = meval::eval_str_with_context(&normalized, context());
    match result {
        Ok(v) if v.is_finite() => {
            let display = format!("= {}", fmt(v));
            let copy_value = fmt(v);
            Some(WaypointerResult {
                result_type: "math".to_string(),
                display,
                copy_value,
            })
        }
        _ => {
            // Fallback to evalexpr for integer ops and modulo.
            evaluate_math_evalexpr(&normalized)
        }
    }
}

/// Fallback math evaluation via evalexpr (handles integer modulo, etc.).
fn evaluate_math_evalexpr(expr: &str) -> Option<WaypointerResult> {
    let value = evalexpr::eval(expr).ok()?;
    let display = match value {
        evalexpr::Value::Float(f) if f.is_finite() => format!("= {}", fmt(f)),
        evalexpr::Value::Int(i) => format!("= {i}"),
        _ => return None,
    };
    let copy_value = display.trim_start_matches("= ").to_string();
    Some(WaypointerResult {
        result_type: "math".to_string(),
        display,
        copy_value,
    })
}

/// Builds a meval context with custom functions and constants.
fn context() -> meval::Context<'static> {
    let mut ctx = meval::Context::new();
    ctx.var("pi", std::f64::consts::PI);
    ctx.var("e", std::f64::consts::E);
    ctx.var("tau", std::f64::consts::TAU);
    ctx.func2("perm", |n, k| {
        let n = n as u64;
        let k = k as u64;
        if k > n { return 0.0; }
        ((n - k + 1)..=n).product::<u64>() as f64
    });
    ctx.func2("comb", |n, k| {
        let n = n as u64;
        let k = k as u64;
        if k > n { return 0.0; }
        let k = k.min(n - k);
        let mut result: u64 = 1;
        for i in 0..k {
            result = result * (n - i) / (i + 1);
        }
        result as f64
    });
    ctx.func("fact", |n| {
        let n = n as u64;
        (1..=n).product::<u64>() as f64
    });
    ctx
}

// ===== Unit conversion: "X unit in Y" =====

/// Converts between units with explicit target: "30 F in C".
fn convert_units(input: &str) -> Option<WaypointerResult> {
    let lower = input.to_lowercase();
    let parts: Vec<&str> = lower.split_whitespace().collect();

    let (value, from, to) = if parts.len() == 4
        && (parts[2] == "in" || parts[2] == "to")
    {
        let v: f64 = parts[0].parse().ok()?;
        (v, parts[1], parts[3])
    } else if parts.len() == 3 && (parts[1] == "in" || parts[1] == "to") {
        let (v, u) = split_number_unit(parts[0])?;
        (v, u, parts[2])
    } else {
        return None;
    };

    let (result, from_label, to_label) = convert_pair(value, from, to)?;
    let display = format!("{} {} = {} {}", fmt_input(value), from_label, result, to_label);

    Some(WaypointerResult {
        result_type: "unit".to_string(),
        display,
        copy_value: result,
    })
}

// ===== Unit conversion: single value + unit, show all =====

/// Shows all relevant conversions for a value with a unit: "30 F", "5 km".
fn convert_single_unit(input: &str) -> Option<WaypointerResult> {
    let lower = input.to_lowercase();
    let parts: Vec<&str> = lower.split_whitespace().collect();

    let (value, unit) = if parts.len() == 2 {
        let v: f64 = parts[0].parse().ok()?;
        (v, parts[1])
    } else if parts.len() == 1 {
        split_number_unit(parts[0])?
    } else {
        return None;
    };

    let targets = conversion_targets(unit)?;
    let mut results = Vec::new();
    for (to, _to_label) in &targets {
        if let Some((r, _fl, tl)) = convert_pair(value, unit, to) {
            results.push(format!("{} {}", r, tl));
        }
    }

    if results.is_empty() {
        return None;
    }

    let display = format!("{} {} = {}", fmt_input(value), canonical_label(unit), results.join(", "));
    let copy_value = results.first().map(|s| {
        s.split_whitespace().next().unwrap_or("").to_string()
    }).unwrap_or_default();

    Some(WaypointerResult {
        result_type: "unit".to_string(),
        display,
        copy_value,
    })
}

/// Returns target units for a given source unit.
fn conversion_targets(unit: &str) -> Option<Vec<(&'static str, &'static str)>> {
    match unit {
        "f" | "fahrenheit" => Some(vec![("c", "C"), ("k", "K")]),
        "c" | "celsius" => Some(vec![("f", "F"), ("k", "K")]),
        "k" | "kelvin" => Some(vec![("c", "C"), ("f", "F")]),
        "km" | "kilometers" => Some(vec![("miles", "mi"), ("m", "m")]),
        "miles" | "mi" => Some(vec![("km", "km")]),
        "m" | "meters" => Some(vec![("ft", "ft"), ("in", "in")]),
        "ft" | "feet" => Some(vec![("m", "m"), ("in", "in")]),
        "in" | "inches" => Some(vec![("cm", "cm"), ("m", "m")]),
        "cm" => Some(vec![("in", "in"), ("m", "m")]),
        "kg" | "kilograms" => Some(vec![("lb", "lb")]),
        "lb" | "lbs" | "pounds" => Some(vec![("kg", "kg")]),
        "g" | "grams" => Some(vec![("oz", "oz")]),
        "oz" | "ounces" => Some(vec![("g", "g")]),
        "mb" | "megabytes" => Some(vec![("gb", "GB")]),
        "gb" | "gigabytes" => Some(vec![("mb", "MB"), ("tb", "TB")]),
        "tb" | "terabytes" => Some(vec![("gb", "GB")]),
        "usd" => Some(vec![("eur", "EUR"), ("gbp", "GBP")]),
        "eur" => Some(vec![("usd", "USD"), ("gbp", "GBP")]),
        "gbp" => Some(vec![("usd", "USD"), ("eur", "EUR")]),
        "gallons" | "gal" => Some(vec![("liters", "L")]),
        "liters" | "l" => Some(vec![("gallons", "gal")]),
        _ => None,
    }
}

/// Returns the canonical display label for a unit.
fn canonical_label(unit: &str) -> &'static str {
    match unit {
        "f" | "fahrenheit" => "F",
        "c" | "celsius" => "C",
        "k" | "kelvin" => "K",
        "km" | "kilometers" => "km",
        "miles" | "mi" => "mi",
        "m" | "meters" => "m",
        "ft" | "feet" => "ft",
        "in" | "inches" => "in",
        "cm" => "cm",
        "kg" | "kilograms" => "kg",
        "lb" | "lbs" | "pounds" => "lb",
        "g" | "grams" => "g",
        "oz" | "ounces" => "oz",
        "mb" | "megabytes" => "MB",
        "gb" | "gigabytes" => "GB",
        "tb" | "terabytes" => "TB",
        "kb" | "kilobytes" => "KB",
        "usd" => "USD",
        "eur" => "EUR",
        "gbp" => "GBP",
        "jpy" => "JPY",
        "chf" => "CHF",
        "gallons" | "gal" => "gal",
        "liters" | "l" => "L",
        _ => Box::leak(unit.to_string().into_boxed_str()),
    }
}

// ===== Core conversion =====

/// Converts a single value between two units.
/// Returns (formatted_result, from_label, to_label).
fn convert_pair(value: f64, from: &str, to: &str) -> Option<(String, &'static str, &'static str)> {
    // Temperature
    match (from, to) {
        ("f" | "fahrenheit", "c" | "celsius") => return Some((fmt((value - 32.0) * 5.0 / 9.0), "F", "C")),
        ("c" | "celsius", "f" | "fahrenheit") => return Some((fmt(value * 9.0 / 5.0 + 32.0), "C", "F")),
        ("c" | "celsius", "k" | "kelvin") => return Some((fmt(value + 273.15), "C", "K")),
        ("k" | "kelvin", "c" | "celsius") => return Some((fmt(value - 273.15), "K", "C")),
        ("f" | "fahrenheit", "k" | "kelvin") => return Some((fmt((value - 32.0) * 5.0 / 9.0 + 273.15), "F", "K")),
        ("k" | "kelvin", "f" | "fahrenheit") => return Some((fmt((value - 273.15) * 9.0 / 5.0 + 32.0), "K", "F")),
        _ => {}
    }

    // Length
    match (from, to) {
        ("km" | "kilometers", "miles" | "mi") => return Some((fmt(value * 0.621371), "km", "mi")),
        ("miles" | "mi", "km" | "kilometers") => return Some((fmt(value * 1.60934), "mi", "km")),
        ("km" | "kilometers", "m" | "meters") => return Some((fmt(value * 1000.0), "km", "m")),
        ("m" | "meters", "km" | "kilometers") => return Some((fmt(value / 1000.0), "m", "km")),
        ("m" | "meters", "ft" | "feet") => return Some((fmt(value * 3.28084), "m", "ft")),
        ("ft" | "feet", "m" | "meters") => return Some((fmt(value / 3.28084), "ft", "m")),
        ("m" | "meters", "in" | "inches") => return Some((fmt(value * 39.3701), "m", "in")),
        ("in" | "inches", "m" | "meters") => return Some((fmt(value / 39.3701), "in", "m")),
        ("cm", "in" | "inches") => return Some((fmt(value / 2.54), "cm", "in")),
        ("in" | "inches", "cm") => return Some((fmt(value * 2.54), "in", "cm")),
        ("ft" | "feet", "in" | "inches") => return Some((fmt(value * 12.0), "ft", "in")),
        ("in" | "inches", "ft" | "feet") => return Some((fmt(value / 12.0), "in", "ft")),
        _ => {}
    }

    // Weight
    match (from, to) {
        ("kg" | "kilograms", "lb" | "lbs" | "pounds") => return Some((fmt(value * 2.20462), "kg", "lb")),
        ("lb" | "lbs" | "pounds", "kg" | "kilograms") => return Some((fmt(value / 2.20462), "lb", "kg")),
        ("g" | "grams", "oz" | "ounces") => return Some((fmt(value / 28.3495), "g", "oz")),
        ("oz" | "ounces", "g" | "grams") => return Some((fmt(value * 28.3495), "oz", "g")),
        _ => {}
    }

    // Volume
    match (from, to) {
        ("gallons" | "gal", "liters" | "l") => return Some((fmt(value * 3.78541), "gal", "L")),
        ("liters" | "l", "gallons" | "gal") => return Some((fmt(value / 3.78541), "L", "gal")),
        _ => {}
    }

    // Data
    match (from, to) {
        ("kb" | "kilobytes", "mb" | "megabytes") => return Some((fmt(value / 1024.0), "KB", "MB")),
        ("mb" | "megabytes", "kb" | "kilobytes") => return Some((fmt(value * 1024.0), "MB", "KB")),
        ("mb" | "megabytes", "gb" | "gigabytes") => return Some((fmt(value / 1024.0), "MB", "GB")),
        ("gb" | "gigabytes", "mb" | "megabytes") => return Some((fmt(value * 1024.0), "GB", "MB")),
        ("gb" | "gigabytes", "tb" | "terabytes") => return Some((fmt(value / 1024.0), "GB", "TB")),
        ("tb" | "terabytes", "gb" | "gigabytes") => return Some((fmt(value * 1024.0), "TB", "GB")),
        ("mb" | "megabytes", "tb" | "terabytes") => return Some((fmt(value / (1024.0 * 1024.0)), "MB", "TB")),
        ("tb" | "terabytes", "mb" | "megabytes") => return Some((fmt(value * 1024.0 * 1024.0), "TB", "MB")),
        _ => {}
    }

    // Currency (fixed rates)
    match (from, to) {
        ("usd", "eur") => return Some((fmt(value * 0.92), "USD", "EUR")),
        ("eur", "usd") => return Some((fmt(value / 0.92), "EUR", "USD")),
        ("usd", "gbp") => return Some((fmt(value * 0.79), "USD", "GBP")),
        ("gbp", "usd") => return Some((fmt(value / 0.79), "GBP", "USD")),
        ("eur", "gbp") => return Some((fmt(value * 0.86), "EUR", "GBP")),
        ("gbp", "eur") => return Some((fmt(value / 0.86), "GBP", "EUR")),
        ("usd", "jpy") => return Some((fmt(value * 151.0), "USD", "JPY")),
        ("jpy", "usd") => return Some((fmt(value / 151.0), "JPY", "USD")),
        ("usd", "chf") => return Some((fmt(value * 0.88), "USD", "CHF")),
        ("chf", "usd") => return Some((fmt(value / 0.88), "CHF", "USD")),
        _ => {}
    }

    None
}

// ===== Helpers =====

/// Splits "30f" into (30.0, "f").
fn split_number_unit(s: &str) -> Option<(f64, &str)> {
    let idx = s.find(|c: char| !c.is_ascii_digit() && c != '.' && c != '-')?;
    let num: f64 = s[..idx].parse().ok()?;
    let unit = &s[idx..];
    if unit.is_empty() { return None; }
    Some((num, unit))
}

/// Formats a float for display: trim trailing zeros, max 4 decimal places.
fn fmt(v: f64) -> String {
    if v == v.floor() && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        let s = format!("{:.4}", v);
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

/// Formats an input value (preserves user precision).
fn fmt_input(v: f64) -> String {
    if v == v.floor() && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        format!("{}", v)
    }
}

// ===== DateTime evaluation =====

/// Evaluates date/time queries.
fn evaluate_datetime(input: &str) -> Option<WaypointerResult> {
    use chrono::{Datelike, Duration, Local, Timelike, Weekday};
    use chrono_tz::Tz;

    let lower = input.to_lowercase();
    let lower = lower.trim();
    let now = Local::now();

    // "time" -> local time
    if lower == "time" || lower == "time now" || lower == "current time" {
        let display = now.format("%H:%M:%S").to_string();
        return Some(dt_result(&display, &display));
    }

    // "date" -> local date with weekday
    if lower == "date" || lower == "today" || lower == "current date" {
        let display = format_date(&now);
        let copy = now.format("%Y-%m-%d").to_string();
        return Some(dt_result(&display, &copy));
    }

    // "yesterday"
    if lower == "yesterday" {
        let d = now - Duration::days(1);
        let display = format_date(&d);
        let copy = d.format("%Y-%m-%d").to_string();
        return Some(dt_result(&display, &copy));
    }

    // "tomorrow"
    if lower == "tomorrow" {
        let d = now + Duration::days(1);
        let display = format_date(&d);
        let copy = d.format("%Y-%m-%d").to_string();
        return Some(dt_result(&display, &copy));
    }

    // "time <timezone/city>" or "time UTC+N"
    if let Some(rest) = lower.strip_prefix("time ") {
        let rest = rest.trim();
        if let Some(display) = resolve_timezone_time(rest) {
            return Some(dt_result(&display, &display));
        }
    }

    // "in N hours/minutes/days/weeks"
    if let Some(rest) = lower.strip_prefix("in ") {
        if let Some(r) = parse_duration_offset(rest, &now) {
            return Some(r);
        }
    }

    // "next monday", "next friday", etc.
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
    use chrono_tz::Tz;

    let now_utc = Utc::now();

    // Try "UTC", "UTC+N", "UTC-N"
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

    // Try city name lookup.
    if let Some(tz) = city_to_tz(input) {
        let t = now_utc.with_timezone(&tz);
        return Some(format!("{} ({})", t.format("%H:%M:%S"), tz.name()));
    }

    // Try direct IANA name (e.g. "europe/vienna").
    if let Ok(tz) = input.parse::<Tz>() {
        let t = now_utc.with_timezone(&tz);
        return Some(format!("{} ({})", t.format("%H:%M:%S"), tz.name()));
    }

    // Try with capitalized parts (e.g. "europe/vienna" -> "Europe/Vienna").
    let capitalized: String = input
        .split('/')
        .map(|part| {
            let mut c = part.chars();
            match c.next() {
                Some(first) => {
                    first.to_uppercase().to_string() + &c.as_str().to_lowercase()
                }
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

/// Maps common city names to IANA timezone identifiers.
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
    use chrono::Duration;

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

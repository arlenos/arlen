/// Unit conversion plugin.
///
/// Recognises patterns like `30 F in C`, `5 km to miles`, `200 cm` (implicit
/// targets) and offers one inline result. All value-and-table knowledge lives
/// here; the Tauri-facing `evaluate_waypointer_input` command delegates to
/// `try_convert` to preserve the inline-result UX on the frontend.

use crate::waypointer_plugins::WaypointerResult;
use crate::waypointer_system::plugin::*;

pub struct UnitConverterPlugin;

impl WaypointerPlugin for UnitConverterPlugin {
    fn id(&self) -> &str { "core.unit-converter" }
    fn name(&self) -> &str { "Unit Converter" }
    fn description(&self) -> &str { "Converts temperatures, lengths, weights, volumes, data sizes, and currencies." }
    fn priority(&self) -> u32 { 2 }
    fn max_results(&self) -> usize { 1 }

    fn search(&self, query: &str) -> Vec<SearchResult> {
        match try_convert(query) {
            Some(r) => vec![SearchResult {
                id: "unit-result".into(),
                title: r.display.clone(),
                description: Some("Unit conversion".into()),
                icon: Some("arrow-right-left".into()),
                relevance: 0.95,
                action: Action::Copy { text: r.copy_value },
                plugin_id: String::new(),
            }],
            None => Vec::new(),
        }
    }

    fn execute(&self, _result: &SearchResult) -> Result<(), PluginError> {
        Ok(()) // Copy is handled by the shell clipboard API.
    }
}

// ── Public helpers (used by evaluate_waypointer_input) ──────────────────

/// Tries to interpret the input as a unit conversion. First checks for an
/// explicit target (`X u in Y`), then falls back to single-unit implicit
/// targets (`30 F`).
pub fn try_convert(input: &str) -> Option<WaypointerResult> {
    convert_units(input).or_else(|| convert_single_unit(input))
}

/// Converts between units with explicit target: `30 F in C`.
fn convert_units(input: &str) -> Option<WaypointerResult> {
    let lower = input.to_lowercase();
    let parts: Vec<&str> = lower.split_whitespace().collect();

    // Four token shapes accepted:
    //   "30 f in c"         → [30, f, in, c]
    //   "30f in c"          → [30f, in, c]
    //   "30 f c"            → [30, f, c]          (implicit "to")
    //   "30f c"             → [30f, c]            (implicit "to")
    // Previously only the first two were recognised, so users who
    // omitted "in"/"to" got a silent no-op.
    let (value, from, to) = if parts.len() == 4
        && (parts[2] == "in" || parts[2] == "to")
    {
        let v: f64 = parts[0].parse().ok()?;
        (v, parts[1], parts[3])
    } else if parts.len() == 3 && (parts[1] == "in" || parts[1] == "to") {
        let (v, u) = split_number_unit(parts[0])?;
        (v, u, parts[2])
    } else if parts.len() == 3 {
        // "30 f c" — no connector word, assume "to".
        let v: f64 = parts[0].parse().ok()?;
        (v, parts[1], parts[2])
    } else if parts.len() == 2 {
        // "30f c" — glued number+unit, then target.
        let (v, u) = split_number_unit(parts[0])?;
        (v, u, parts[1])
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

/// Shows all relevant conversions for a value with a unit: `30 F`, `5 km`.
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

    // Currency (fixed rates; intentionally coarse — rates in a real build
    // should come from a live-updating module).
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

fn split_number_unit(s: &str) -> Option<(f64, &str)> {
    let idx = s.find(|c: char| !c.is_ascii_digit() && c != '.' && c != '-')?;
    let num: f64 = s[..idx].parse().ok()?;
    let unit = &s[idx..];
    if unit.is_empty() { return None; }
    Some((num, unit))
}

fn fmt(v: f64) -> String {
    if v == v.floor() && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        let s = format!("{:.4}", v);
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

fn fmt_input(v: f64) -> String {
    if v == v.floor() && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        format!("{}", v)
    }
}

// ── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn temperature_explicit_target() {
        let r = try_convert("30 F in C").unwrap();
        assert_eq!(r.result_type, "unit");
        assert!(r.display.contains("-1"));
    }

    #[test]
    fn length_km_to_miles() {
        let r = try_convert("10 km in miles").unwrap();
        assert!(r.display.contains("mi"));
    }

    #[test]
    fn single_unit_shows_all_targets() {
        let r = try_convert("5 km").unwrap();
        assert!(r.display.contains("mi"));
        assert!(r.display.contains("m"));
    }

    #[test]
    fn number_glued_to_unit() {
        let r = try_convert("100cm").unwrap();
        assert!(r.display.contains("in"));
    }

    #[test]
    fn non_unit_input_returns_none() {
        assert!(try_convert("hello world").is_none());
        assert!(try_convert("").is_none());
        assert!(try_convert("2+2").is_none());
    }

    #[test]
    fn plugin_search_returns_at_most_one() {
        let p = UnitConverterPlugin;
        assert_eq!(p.search("5 km").len(), 1);
        assert!(p.search("hello").is_empty());
    }
}

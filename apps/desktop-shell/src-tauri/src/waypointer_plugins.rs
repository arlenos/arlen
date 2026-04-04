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
#[tauri::command]
pub fn evaluate_waypointer_input(input: String) -> Option<WaypointerResult> {
    let input = input.trim();
    log::info!("evaluate_waypointer_input: {:?}", input);
    if input.is_empty() {
        return None;
    }

    // Try unit conversion first (more specific patterns).
    if let Some(r) = convert_units(input) {
        log::info!("evaluate_waypointer_input: UNIT result={:?}", r.display);
        return Some(r);
    }

    // Try math evaluation.
    if let Some(r) = evaluate_math(input) {
        log::info!("evaluate_waypointer_input: MATH result={:?}", r.display);
        return Some(r);
    }

    log::info!("evaluate_waypointer_input: no match");
    None
}

/// Evaluates a math expression using evalexpr.
fn evaluate_math(input: &str) -> Option<WaypointerResult> {
    // Strip leading/trailing '=' that users might type.
    let expr = input.trim_start_matches('=').trim_end_matches('=').trim();
    if expr.is_empty() {
        return None;
    }

    // Must contain at least one operator or function to be a math expression.
    let has_operator = expr.contains('+')
        || expr.contains('-')
        || expr.contains('*')
        || expr.contains('/')
        || expr.contains('^')
        || expr.contains('(')
        || expr.contains('%');

    if !has_operator {
        return None;
    }

    // Replace common symbols.
    let normalized = expr
        .replace('x', "*")
        .replace('X', "*")
        .replace(',', "");

    let value = evalexpr::eval(&normalized).ok()?;
    let display = match value {
        evalexpr::Value::Float(f) => {
            if f == f.floor() && f.abs() < 1e15 {
                format!("= {}", f as i64)
            } else {
                format!("= {:.6}", f).trim_end_matches('0').trim_end_matches('.').to_string()
            }
        }
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

/// Converts between units. Supports temperature, length, weight, data.
fn convert_units(input: &str) -> Option<WaypointerResult> {
    // Pattern: "<number> <from_unit> in <to_unit>" or "<number><from_unit> to <to_unit>"
    let input_lower = input.to_lowercase();
    let parts: Vec<&str> = input_lower.split_whitespace().collect();

    // Try patterns: "30 f in c", "5 km in miles", "100 usd in eur"
    let (value, from, to) = if parts.len() == 4
        && (parts[2] == "in" || parts[2] == "to")
    {
        let v: f64 = parts[0].parse().ok()?;
        (v, parts[1], parts[3])
    } else if parts.len() == 3 && (parts[1] == "in" || parts[1] == "to") {
        // "30f in c" -- number glued to unit
        let (v, u) = split_number_unit(parts[0])?;
        (v, u, parts[2])
    } else {
        return None;
    };

    let (result, from_label, to_label) = convert(value, from, to)?;

    let display = format!("{value} {from_label} = {result} {to_label}");
    let copy_value = format!("{result}");

    Some(WaypointerResult {
        result_type: "unit".to_string(),
        display,
        copy_value,
    })
}

/// Splits "30f" into (30.0, "f").
fn split_number_unit(s: &str) -> Option<(f64, &str)> {
    let idx = s.find(|c: char| !c.is_ascii_digit() && c != '.' && c != '-')?;
    let num: f64 = s[..idx].parse().ok()?;
    let unit = &s[idx..];
    if unit.is_empty() {
        return None;
    }
    Some((num, unit))
}

/// Core conversion logic. Returns (result_formatted, from_label, to_label).
fn convert(value: f64, from: &str, to: &str) -> Option<(String, &'static str, &'static str)> {
    // Temperature
    match (from, to) {
        ("f" | "fahrenheit", "c" | "celsius") => {
            let r = (value - 32.0) * 5.0 / 9.0;
            return Some((fmt(r), "F", "C"));
        }
        ("c" | "celsius", "f" | "fahrenheit") => {
            let r = value * 9.0 / 5.0 + 32.0;
            return Some((fmt(r), "C", "F"));
        }
        ("c" | "celsius", "k" | "kelvin") => {
            let r = value + 273.15;
            return Some((fmt(r), "C", "K"));
        }
        ("k" | "kelvin", "c" | "celsius") => {
            let r = value - 273.15;
            return Some((fmt(r), "K", "C"));
        }
        ("f" | "fahrenheit", "k" | "kelvin") => {
            let r = (value - 32.0) * 5.0 / 9.0 + 273.15;
            return Some((fmt(r), "F", "K"));
        }
        ("k" | "kelvin", "f" | "fahrenheit") => {
            let r = (value - 273.15) * 9.0 / 5.0 + 32.0;
            return Some((fmt(r), "K", "F"));
        }
        _ => {}
    }

    // Length
    let length_result = match (from, to) {
        ("km" | "kilometers", "miles" | "mi") => Some((value * 0.621371, "km", "miles")),
        ("miles" | "mi", "km" | "kilometers") => Some((value * 1.60934, "miles", "km")),
        ("m" | "meters", "ft" | "feet") => Some((value * 3.28084, "m", "ft")),
        ("ft" | "feet", "m" | "meters") => Some((value / 3.28084, "ft", "m")),
        ("m" | "meters", "in" | "inches") => Some((value * 39.3701, "m", "in")),
        ("in" | "inches", "m" | "meters") => Some((value / 39.3701, "in", "m")),
        ("cm", "in" | "inches") => Some((value / 2.54, "cm", "in")),
        ("in" | "inches", "cm") => Some((value * 2.54, "in", "cm")),
        ("ft" | "feet", "in" | "inches") => Some((value * 12.0, "ft", "in")),
        ("in" | "inches", "ft" | "feet") => Some((value / 12.0, "in", "ft")),
        _ => None,
    };
    if let Some((r, fl, tl)) = length_result {
        return Some((fmt(r), fl, tl));
    }

    // Weight
    let weight_result = match (from, to) {
        ("kg" | "kilograms", "lb" | "lbs" | "pounds") => Some((value * 2.20462, "kg", "lb")),
        ("lb" | "lbs" | "pounds", "kg" | "kilograms") => Some((value / 2.20462, "lb", "kg")),
        ("g" | "grams", "oz" | "ounces") => Some((value / 28.3495, "g", "oz")),
        ("oz" | "ounces", "g" | "grams") => Some((value * 28.3495, "oz", "g")),
        _ => None,
    };
    if let Some((r, fl, tl)) = weight_result {
        return Some((fmt(r), fl, tl));
    }

    // Data
    let data_result = match (from, to) {
        ("mb" | "megabytes", "gb" | "gigabytes") => Some((value / 1024.0, "MB", "GB")),
        ("gb" | "gigabytes", "mb" | "megabytes") => Some((value * 1024.0, "GB", "MB")),
        ("gb" | "gigabytes", "tb" | "terabytes") => Some((value / 1024.0, "GB", "TB")),
        ("tb" | "terabytes", "gb" | "gigabytes") => Some((value * 1024.0, "TB", "GB")),
        ("mb" | "megabytes", "tb" | "terabytes") => Some((value / (1024.0 * 1024.0), "MB", "TB")),
        ("tb" | "terabytes", "mb" | "megabytes") => Some((value * 1024.0 * 1024.0, "TB", "MB")),
        ("kb" | "kilobytes", "mb" | "megabytes") => Some((value / 1024.0, "KB", "MB")),
        ("mb" | "megabytes", "kb" | "kilobytes") => Some((value * 1024.0, "MB", "KB")),
        _ => None,
    };
    if let Some((r, fl, tl)) = data_result {
        return Some((fmt(r), fl, tl));
    }

    // Currency (fixed rates, no API)
    let currency_result = match (from, to) {
        ("usd", "eur") => Some((value * 0.92, "USD", "EUR")),
        ("eur", "usd") => Some((value / 0.92, "EUR", "USD")),
        ("usd", "gbp") => Some((value * 0.79, "USD", "GBP")),
        ("gbp", "usd") => Some((value / 0.79, "GBP", "USD")),
        ("eur", "gbp") => Some((value * 0.86, "EUR", "GBP")),
        ("gbp", "eur") => Some((value / 0.86, "GBP", "EUR")),
        ("usd", "jpy") => Some((value * 151.0, "USD", "JPY")),
        ("jpy", "usd") => Some((value / 151.0, "JPY", "USD")),
        ("usd", "chf") => Some((value * 0.88, "USD", "CHF")),
        ("chf", "usd") => Some((value / 0.88, "CHF", "USD")),
        _ => None,
    };
    if let Some((r, fl, tl)) = currency_result {
        return Some((fmt(r), fl, tl));
    }

    None
}

/// Formats a float nicely: trim trailing zeros, max 4 decimal places.
fn fmt(v: f64) -> String {
    if v == v.floor() && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        let s = format!("{:.4}", v);
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

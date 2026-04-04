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

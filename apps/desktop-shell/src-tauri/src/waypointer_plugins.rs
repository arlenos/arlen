/// Inline-result entry point for the Waypointer.
///
/// Kept as a Tauri command so the existing frontend inline-result UX
/// (a single rich row above the list for math / unit / datetime)
/// continues to work without a rewrite. All actual evaluation lives
/// in the plugin modules under `waypointer_system::plugins`; this file
/// is a thin dispatcher with a sandbox thread, timeout, and panic
/// catch so malformed input can neither hang the shell nor crash it.

use serde::Serialize;

use crate::waypointer_system::plugins::{calculator, datetime, unit_converter};

/// Result from evaluating a Waypointer input.
#[derive(Clone, Serialize)]
pub struct WaypointerResult {
    /// `"math"`, `"unit"`, `"datetime"`, or `"error"`.
    pub result_type: String,
    /// Human-readable result for display.
    pub display: String,
    /// Value suitable for clipboard copy.
    pub copy_value: String,
}

/// Evaluates input as a math expression, unit conversion, or datetime query.
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
            dispatch(&input_trimmed)
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

/// Runs the three inline evaluators in priority order. Datetime first
/// (exact keyword matches), then units (number+unit patterns), then
/// math (eager, gated on presence of an operator).
fn dispatch(input: &str) -> Option<WaypointerResult> {
    datetime::try_evaluate(input)
        .or_else(|| unit_converter::try_convert(input))
        .or_else(|| calculator::try_math(input))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispatch_math() {
        let r = dispatch("2+2").unwrap();
        assert_eq!(r.result_type, "math");
    }

    #[test]
    fn dispatch_unit() {
        let r = dispatch("30 F in C").unwrap();
        assert_eq!(r.result_type, "unit");
    }

    #[test]
    fn dispatch_datetime() {
        let r = dispatch("time").unwrap();
        assert_eq!(r.result_type, "datetime");
    }

    #[test]
    fn dispatch_nothing() {
        assert!(dispatch("firefox").is_none());
    }
}

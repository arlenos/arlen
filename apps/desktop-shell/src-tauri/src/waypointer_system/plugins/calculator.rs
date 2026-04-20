/// Calculator plugin.
///
/// Activated either via the explicit `=` prefix (user intends math) or
/// eagerly detected by `try_math` when the input looks like an
/// expression. The Tauri-facing `evaluate_waypointer_input` command
/// calls `try_math` so the inline-result flow still works without a
/// prefix (typing `2+2` shows `= 4` even without the `=`).

use crate::waypointer_plugins::WaypointerResult;
use crate::waypointer_system::plugin::*;

pub struct CalculatorPlugin;

impl WaypointerPlugin for CalculatorPlugin {
    fn id(&self) -> &str { "core.calculator" }
    fn name(&self) -> &str { "Calculator" }
    fn description(&self) -> &str { "Evaluates math expressions including pi, e, and combinatorial functions." }
    fn prefix(&self) -> Option<&str> { Some("=") }
    fn priority(&self) -> u32 { 0 }
    fn max_results(&self) -> usize { 1 }

    fn search(&self, query: &str) -> Vec<SearchResult> {
        // With the `=` prefix the user explicitly wants math, so skip
        // the has_math heuristic and evaluate directly.
        match evaluate_expression(query) {
            Some(v) => vec![SearchResult {
                id: "calc-result".into(),
                title: v.copy_value.clone(),
                description: Some(format!("{query} =")),
                icon: Some("calculator".into()),
                relevance: 1.0,
                action: Action::Copy { text: v.copy_value },
                plugin_id: String::new(),
            }],
            None => Vec::new(),
        }
    }

    fn execute(&self, _result: &SearchResult) -> Result<(), PluginError> {
        Ok(())
    }
}

// ── Public helpers ──────────────────────────────────────────────────────

/// Eager math evaluation used by `evaluate_waypointer_input`. Only returns
/// a result when the input actually contains an operator (so typing
/// `firefox` never produces a math result).
pub fn try_math(input: &str) -> Option<WaypointerResult> {
    let expr = input.trim_start_matches('=').trim_end_matches('=').trim();
    if expr.is_empty() {
        return None;
    }

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

    evaluate_expression(expr)
}

/// Evaluates `expr` (no `=` handling — the caller must strip it first).
fn evaluate_expression(expr: &str) -> Option<WaypointerResult> {
    if expr.trim().is_empty() {
        return None;
    }

    let normalized = expr
        .replace('x', "*")
        .replace('X', "*")
        .replace(',', "")
        .replace("**", "^");

    match meval::eval_str_with_context(&normalized, context()) {
        Ok(v) if v.is_finite() => {
            let copy = fmt(v);
            Some(WaypointerResult {
                result_type: "math".to_string(),
                display: format!("= {copy}"),
                copy_value: copy,
            })
        }
        _ => evaluate_with_evalexpr(&normalized),
    }
}

fn evaluate_with_evalexpr(expr: &str) -> Option<WaypointerResult> {
    let value = evalexpr::eval(expr).ok()?;
    let (display, copy) = match value {
        evalexpr::Value::Float(f) if f.is_finite() => {
            let c = fmt(f);
            (format!("= {c}"), c)
        }
        evalexpr::Value::Int(i) => (format!("= {i}"), i.to_string()),
        _ => return None,
    };
    Some(WaypointerResult {
        result_type: "math".to_string(),
        display,
        copy_value: copy,
    })
}

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

fn fmt(v: f64) -> String {
    if v == v.floor() && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        let s = format!("{:.4}", v);
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

// ── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_basic_math() {
        let p = CalculatorPlugin;
        let r = p.search("2+2");
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].title, "4");
    }

    #[test]
    fn plugin_decimal() {
        let p = CalculatorPlugin;
        let r = p.search("1/3");
        assert_eq!(r.len(), 1);
        assert!(r[0].title.starts_with("0.333"));
    }

    #[test]
    fn plugin_empty_query() {
        let p = CalculatorPlugin;
        assert!(p.search("").is_empty());
    }

    #[test]
    fn plugin_invalid_math() {
        let p = CalculatorPlugin;
        assert!(p.search("hello").is_empty());
    }

    #[test]
    fn try_math_requires_operator() {
        assert!(try_math("hello").is_none());
        assert!(try_math("2+2").is_some());
        assert!(try_math("2*3").is_some());
    }

    #[test]
    fn try_math_with_equals_prefix() {
        let r = try_math("=5+5").unwrap();
        assert_eq!(r.copy_value, "10");
    }

    #[test]
    fn try_math_with_equals_suffix() {
        let r = try_math("5+5=").unwrap();
        assert_eq!(r.copy_value, "10");
    }

    #[test]
    fn try_math_x_requires_explicit_prefix() {
        // Bare `3x4` must NOT evaluate (would false-positive on words
        // containing `x`, e.g. "firefox"). With the `=` prefix the user
        // opts in explicitly, and then `x` is normalised to `*`.
        assert!(try_math("3x4").is_none());
        let r = try_math("=3x4").unwrap();
        assert_eq!(r.copy_value, "12");
    }

    #[test]
    fn try_math_uses_pi() {
        // pi*2 = 6.2831...
        let r = try_math("pi*2").unwrap();
        assert!(r.copy_value.starts_with("6.28"));
    }

    #[test]
    fn try_math_modulo_via_evalexpr() {
        // meval may not support %; the evalexpr fallback handles it.
        let r = try_math("10%3").unwrap();
        assert_eq!(r.copy_value, "1");
    }

    #[test]
    fn try_math_display_has_prefix() {
        let r = try_math("2+2").unwrap();
        assert_eq!(r.display, "= 4");
    }
}

/// Calculator plugin: evaluates math expressions, unit conversions, datetime.

use crate::waypointer_system::plugin::*;

pub struct CalculatorPlugin;

impl WaypointerPlugin for CalculatorPlugin {
    fn id(&self) -> &str { "core.calculator" }
    fn name(&self) -> &str { "Calculator" }
    fn prefix(&self) -> Option<&str> { Some("=") }
    fn priority(&self) -> u32 { 0 }

    fn search(&self, query: &str) -> Vec<SearchResult> {
        let query = query.trim();
        if query.is_empty() {
            return Vec::new();
        }

        // Try math evaluation via meval.
        if let Ok(result) = meval::eval_str(query) {
            let display = format_number(result);
            return vec![SearchResult {
                id: "calc-result".into(),
                title: display.clone(),
                description: Some(format!("{query} =")),
                icon: Some("calculator".into()),
                relevance: 1.0,
                action: Action::Copy { text: display },
                plugin_id: String::new(),
            }];
        }

        Vec::new()
    }

    fn execute(&self, result: &SearchResult) -> Result<(), PluginError> {
        // Copy action is handled by the shell.
        let _ = result;
        Ok(())
    }
}

/// Format a number: strip trailing zeros, handle integers.
fn format_number(n: f64) -> String {
    if n == n.floor() && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        let s = format!("{:.10}", n);
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_math() {
        let p = CalculatorPlugin;
        let r = p.search("2+2");
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].title, "4");
    }

    #[test]
    fn test_decimal() {
        let p = CalculatorPlugin;
        let r = p.search("1/3");
        assert_eq!(r.len(), 1);
        assert!(r[0].title.starts_with("0.333"));
    }

    #[test]
    fn test_empty() {
        let p = CalculatorPlugin;
        assert!(p.search("").is_empty());
    }

    #[test]
    fn test_invalid() {
        let p = CalculatorPlugin;
        assert!(p.search("hello").is_empty());
    }
}

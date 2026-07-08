//! Per-provider token usage accounting over a resettable window, and the spending-cap
//! check that feeds the combo fallback.
//!
//! The proxy sees every upstream response, so it can meter the tokens each provider spent
//! (from the OpenAI-shape `usage` object the forward returns; the Anthropic transcoder
//! already maps native counts into that shape). [`UsageLedger`] accumulates those counts
//! per provider within a rolling window that resets after `window_secs`; a provider whose
//! accrued tokens reach its configured cap is reported [`reached_cap`](UsageLedger::reached_cap)
//! so `forward_combo` falls past it - the "spending cap triggers the fallback" control.
//!
//! Cost in currency is deliberately NOT modelled here: it needs per-provider pricing (the
//! models.dev seed), which is a separate design item. Token counts are the engine-portable
//! signal a cap and the transparency surface can both use today.

use std::collections::HashMap;

/// One provider's token usage within the current window.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderUsage {
    /// Prompt (input) tokens spent this window.
    pub prompt_tokens: u64,
    /// Completion (output) tokens spent this window.
    pub completion_tokens: u64,
    /// Sum of prompt + completion tokens this window (the value a cap is checked against).
    pub total_tokens: u64,
    /// Number of forwarded requests this window.
    pub requests: u64,
}

/// Live per-provider token accounting over a rolling window. Not thread-safe on its own; the
/// service wraps it in a mutex. Times are epoch seconds passed in by the caller, so the core
/// stays deterministic and unit-testable (no ambient clock).
#[derive(Debug, Clone)]
pub struct UsageLedger {
    window_secs: u64,
    window_started_at: u64,
    usage: HashMap<String, ProviderUsage>,
}

impl UsageLedger {
    /// A fresh ledger whose first window opens at `now_secs`. `window_secs` should be > 0; a
    /// zero window degenerates to "always just reset" (every provider always uncapped), which
    /// is a safe no-op rather than a panic.
    pub fn new(window_secs: u64, now_secs: u64) -> Self {
        Self { window_secs, window_started_at: now_secs, usage: HashMap::new() }
    }

    /// Whether the current window has elapsed as of `now_secs`.
    fn elapsed(&self, now_secs: u64) -> bool {
        now_secs.saturating_sub(self.window_started_at) >= self.window_secs
    }

    /// Record one forwarded call's tokens against a provider. Rolls the window first if it has
    /// elapsed (clearing every provider's counts and re-anchoring the window at `now_secs`), so
    /// usage never carries across a window boundary.
    pub fn accrue(&mut self, provider: &str, prompt: u64, completion: u64, now_secs: u64) {
        if self.elapsed(now_secs) {
            self.usage.clear();
            self.window_started_at = now_secs;
        }
        let e = self.usage.entry(provider.to_string()).or_default();
        e.prompt_tokens = e.prompt_tokens.saturating_add(prompt);
        e.completion_tokens = e.completion_tokens.saturating_add(completion);
        e.total_tokens = e.total_tokens.saturating_add(prompt).saturating_add(completion);
        e.requests = e.requests.saturating_add(1);
    }

    /// A provider's usage in the current window as of `now_secs`. Reads as zero once the window
    /// has elapsed (the stored counts are stale until the next `accrue` rolls them).
    pub fn usage_of(&self, provider: &str, now_secs: u64) -> ProviderUsage {
        if self.elapsed(now_secs) {
            return ProviderUsage::default();
        }
        self.usage.get(provider).copied().unwrap_or_default()
    }

    /// Whether a provider has reached its token cap for the current window. A just-elapsed
    /// window reads as zero usage, so it is never capped. A cap of 0 means "no headroom" and
    /// is reached immediately once the window is live.
    pub fn reached_cap(&self, provider: &str, cap: u64, now_secs: u64) -> bool {
        self.usage_of(provider, now_secs).total_tokens >= cap
    }

    /// Seconds until the current window resets (0 once it has elapsed).
    pub fn resets_in(&self, now_secs: u64) -> u64 {
        self.window_secs.saturating_sub(now_secs.saturating_sub(self.window_started_at))
    }
}

/// Extract `(prompt_tokens, completion_tokens)` from an OpenAI chat-completions response
/// body's `usage` object. Returns `None` when the body is not JSON or carries no usage (a
/// streaming chunk, an error body, or a provider that omits it), in which case the caller
/// accrues nothing rather than guessing.
pub fn tokens_from_response_body(body: &str) -> Option<(u64, u64)> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    let usage = v.get("usage")?;
    let prompt = usage.get("prompt_tokens")?.as_u64()?;
    let completion = usage.get("completion_tokens")?.as_u64()?;
    Some((prompt, completion))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accrue_accumulates_prompt_completion_and_requests() {
        let mut led = UsageLedger::new(3600, 0);
        led.accrue("openai", 100, 50, 10);
        led.accrue("openai", 200, 25, 20);
        let u = led.usage_of("openai", 30);
        assert_eq!(u.prompt_tokens, 300);
        assert_eq!(u.completion_tokens, 75);
        assert_eq!(u.total_tokens, 375);
        assert_eq!(u.requests, 2);
        // an untouched provider is zero
        assert_eq!(led.usage_of("mistral", 30), ProviderUsage::default());
    }

    #[test]
    fn the_window_rolls_and_clears_usage() {
        let mut led = UsageLedger::new(3600, 0);
        led.accrue("openai", 1000, 0, 100);
        assert_eq!(led.usage_of("openai", 200).total_tokens, 1000);
        // once the window elapses, usage reads as zero even before the next accrue
        assert_eq!(led.usage_of("openai", 3700).total_tokens, 0);
        // the next accrue re-anchors the window and only its tokens count
        led.accrue("openai", 42, 0, 3700);
        assert_eq!(led.usage_of("openai", 3800).total_tokens, 42);
    }

    #[test]
    fn reached_cap_tracks_the_window() {
        let mut led = UsageLedger::new(3600, 0);
        led.accrue("openai", 900, 0, 10);
        assert!(!led.reached_cap("openai", 1000, 20), "under cap");
        led.accrue("openai", 100, 0, 30);
        assert!(led.reached_cap("openai", 1000, 40), "at cap");
        // after the window rolls, the cap is no longer reached
        assert!(!led.reached_cap("openai", 1000, 4000), "fresh window");
    }

    #[test]
    fn resets_in_counts_down_then_floors_at_zero() {
        let led = UsageLedger::new(3600, 0);
        assert_eq!(led.resets_in(0), 3600);
        assert_eq!(led.resets_in(1000), 2600);
        assert_eq!(led.resets_in(3600), 0);
        assert_eq!(led.resets_in(9999), 0);
    }

    #[test]
    fn tokens_from_response_body_reads_the_usage_object() {
        let body = r#"{"choices":[],"usage":{"prompt_tokens":12,"completion_tokens":7,"total_tokens":19}}"#;
        assert_eq!(tokens_from_response_body(body), Some((12, 7)));
        // no usage, not JSON, or a partial usage object -> no accrual
        assert_eq!(tokens_from_response_body(r#"{"choices":[]}"#), None);
        assert_eq!(tokens_from_response_body("not json"), None);
        assert_eq!(tokens_from_response_body(r#"{"usage":{"prompt_tokens":1}}"#), None);
    }
}

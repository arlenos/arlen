/// Clipboard-history plugin: surfaces the in-memory ring buffer from
/// `clipboard_history.rs` as a Waypointer search section.
///
/// The ring buffer itself (wl-paste watcher, filters, ring storage) is
/// owned by `clipboard_history`. This plugin is the read-only facade
/// that the PluginManager exposes: it takes an `Arc<ClipboardHistory>`
/// reference, runs substring filtering on the current snapshot, and
/// dispatches `execute` back to `wl-copy` via the same helper.
///
/// Prefixes `c:`, `clip:`, and `clipboard` all route exclusively to
/// this plugin. Without a prefix the plugin stays silent: the
/// clipboard section is valuable when explicitly asked for but would
/// pollute every general search with stale text otherwise.

use serde_json::json;

use crate::clipboard_history::{copy_via_wl_copy, ClipboardEntry, ClipboardHistoryState};
use crate::waypointer_system::plugin::*;

/// Max characters shown in the result title before truncation.
const TITLE_MAX_CHARS: usize = 80;

pub struct ClipboardPlugin {
    history: ClipboardHistoryState,
}

impl ClipboardPlugin {
    pub fn new(history: ClipboardHistoryState) -> Self {
        Self { history }
    }
}

impl WaypointerPlugin for ClipboardPlugin {
    fn id(&self) -> &str { "core.clipboard" }
    fn name(&self) -> &str { "Clipboard" }
    fn description(&self) -> &str {
        "Recall recently-copied text. Opt-in via shell.toml [clipboard] enabled=true."
    }
    fn priority(&self) -> u32 { 12 }
    fn max_results(&self) -> usize { 20 }

    /// The prefix registered here is one of the three keyword triggers.
    /// The other two (`clip:`, `clipboard`) are handled by the
    /// front-end always-invoking the `search_plugin` bridge and by the
    /// plugin's own query parser below.
    fn prefix(&self) -> Option<&str> { None }

    fn search(&self, query: &str) -> Vec<SearchResult> {
        if !self.history.is_enabled() {
            return Vec::new();
        }
        let needle = strip_clipboard_prefix(query).trim().to_string();

        let entries = self.history.filter(&needle);
        entries
            .into_iter()
            .take(self.max_results())
            .enumerate()
            .map(|(idx, e)| build_result(&e, idx))
            .collect()
    }

    fn execute(&self, result: &SearchResult) -> Result<(), PluginError> {
        let Action::Custom { ref data, .. } = result.action else {
            return Err(PluginError::ExecuteFailed(
                "clipboard: expected Action::Custom".into(),
            ));
        };
        let id = data
            .get("id")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| PluginError::ExecuteFailed("clipboard: missing id".into()))?;
        let entry = self
            .history
            .find(id)
            .ok_or_else(|| PluginError::ExecuteFailed(format!("clipboard: entry {id} gone")))?;
        copy_via_wl_copy(&entry.content).map_err(PluginError::ExecuteFailed)
    }
}

/// Strip any of the accepted keyword prefixes from a query. The
/// plugin is addressed via `search_plugin` so the frontend doesn't
/// strip the prefix for us — we do it here so the user can type `c:`,
/// `clip:`, or `clipboard ` and see the same filter behaviour.
fn strip_clipboard_prefix(q: &str) -> &str {
    let trimmed = q.trim_start();
    for prefix in ["clipboard ", "clipboard", "clip:", "c:"] {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            return rest;
        }
    }
    trimmed
}

fn build_result(entry: &ClipboardEntry, order: usize) -> SearchResult {
    let title = make_title(&entry.content);
    let description = make_description(entry);
    SearchResult {
        id: format!("clip-{}", entry.id),
        title,
        // Relevance decays gently by list position so freshest entry
        // comes first. The ring buffer is already sorted most-recent-
        // first, so we just want a monotonic rank that survives the
        // PluginManager's relevance sort.
        description: Some(description),
        icon: Some("clipboard".into()),
        relevance: 1.0 - (order as f32 * 0.01).min(0.5),
        action: Action::Custom {
            handler: "clipboard_copy".into(),
            data: json!({ "id": entry.id }),
        },
        plugin_id: String::new(),
    }
}

/// Build a single-line title: collapse internal whitespace to a
/// single space, then clip to `TITLE_MAX_CHARS` with an ellipsis.
/// Multi-line clipboard entries would otherwise break the Command row
/// height.
fn make_title(content: &str) -> String {
    let single = content
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if single.chars().count() <= TITLE_MAX_CHARS {
        return single;
    }
    let trimmed: String = single.chars().take(TITLE_MAX_CHARS - 1).collect();
    format!("{trimmed}…")
}

fn make_description(entry: &ClipboardEntry) -> String {
    let rel = relative_time(entry.timestamp_ms);
    if entry.source_app_id.is_empty() {
        rel
    } else {
        format!("{rel} · from {}", entry.source_app_id)
    }
}

/// Short relative time string ("2 min ago", "3 h ago"). Kept local
/// rather than pulled from files.rs because the bucket sizes we want
/// here are slightly different — clipboard entries are typically
/// recent, so "seconds ago" carries more signal.
fn relative_time(timestamp_ms: i64) -> String {
    let now = chrono::Utc::now().timestamp_millis();
    let diff = (now - timestamp_ms).max(0);
    let s = diff / 1000;
    if s < 10 {
        return "just now".into();
    }
    if s < 60 {
        return format!("{s}s ago");
    }
    let m = s / 60;
    if m < 60 {
        return format!("{m} min ago");
    }
    let h = m / 60;
    if h < 24 {
        return format!("{h} h ago");
    }
    let d = h / 24;
    format!("{d} d ago")
}

// ── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clipboard_history::ClipboardHistory;
    use std::sync::Arc;

    fn mk_plugin(enabled: bool) -> ClipboardPlugin {
        let h = Arc::new(ClipboardHistory::new());
        h.set_enabled(enabled);
        ClipboardPlugin::new(h)
    }

    #[test]
    fn strip_prefix_variants() {
        assert_eq!(strip_clipboard_prefix("c:hello"), "hello");
        assert_eq!(strip_clipboard_prefix("clip:x"), "x");
        assert_eq!(strip_clipboard_prefix("clipboard foo"), "foo");
        assert_eq!(strip_clipboard_prefix("nothing"), "nothing");
        assert_eq!(strip_clipboard_prefix("  c:spaced"), "spaced");
    }

    #[test]
    fn make_title_single_line() {
        assert_eq!(make_title("hello"), "hello");
    }

    #[test]
    fn make_title_collapses_whitespace() {
        assert_eq!(make_title("hello\tworld\n\nline"), "hello world line");
    }

    #[test]
    fn make_title_truncates_with_ellipsis() {
        let s = "a".repeat(200);
        let title = make_title(&s);
        assert!(title.chars().count() <= TITLE_MAX_CHARS);
        assert!(title.ends_with('…'));
    }

    #[test]
    fn disabled_plugin_returns_empty() {
        let plugin = mk_plugin(false);
        // Push a dummy entry via history (would still be filtered out).
        plugin.history.push("hello".into(), "".into());
        assert!(plugin.search("hello").is_empty());
    }

    #[test]
    fn enabled_plugin_surfaces_entries() {
        let plugin = mk_plugin(true);
        plugin.history.push("the quick brown fox".into(), "firefox".into());
        plugin.history.push("another note".into(), "".into());
        let r = plugin.search("");
        assert_eq!(r.len(), 2);
        // Most-recent-first.
        assert!(r[0].title.contains("another"));
    }

    #[test]
    fn plugin_filters_by_substring() {
        let plugin = mk_plugin(true);
        plugin.history.push("hello world".into(), "".into());
        plugin.history.push("goodbye world".into(), "".into());
        plugin.history.push("hello there".into(), "".into());
        let r = plugin.search("hello");
        assert_eq!(r.len(), 2);
    }

    #[test]
    fn plugin_strips_prefix_for_search() {
        let plugin = mk_plugin(true);
        plugin.history.push("hello world".into(), "".into());
        plugin.history.push("goodbye".into(), "".into());
        let r = plugin.search("c:hello");
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn plugin_respects_max_results() {
        let plugin = mk_plugin(true);
        for i in 0..50 {
            plugin.history.push(format!("entry {i}"), "".into());
        }
        let r = plugin.search("entry");
        assert_eq!(r.len(), plugin.max_results());
    }

    #[test]
    fn execute_rejects_wrong_action() {
        let plugin = mk_plugin(true);
        let r = SearchResult {
            id: "clip-1".into(),
            title: "x".into(),
            description: None,
            icon: None,
            relevance: 1.0,
            action: Action::Copy { text: "hi".into() },
            plugin_id: "core.clipboard".into(),
        };
        assert!(plugin.execute(&r).is_err());
    }

    #[test]
    fn execute_rejects_missing_id() {
        let plugin = mk_plugin(true);
        let r = SearchResult {
            id: "clip-1".into(),
            title: "x".into(),
            description: None,
            icon: None,
            relevance: 1.0,
            action: Action::Custom {
                handler: "clipboard_copy".into(),
                data: json!({}),
            },
            plugin_id: "core.clipboard".into(),
        };
        assert!(plugin.execute(&r).is_err());
    }

    #[test]
    fn execute_rejects_unknown_id() {
        let plugin = mk_plugin(true);
        let r = SearchResult {
            id: "clip-1".into(),
            title: "x".into(),
            description: None,
            icon: None,
            relevance: 1.0,
            action: Action::Custom {
                handler: "clipboard_copy".into(),
                data: json!({ "id": 99999 }),
            },
            plugin_id: "core.clipboard".into(),
        };
        assert!(plugin.execute(&r).is_err());
    }

    #[test]
    fn relative_time_formats() {
        let now = chrono::Utc::now().timestamp_millis();
        assert_eq!(relative_time(now), "just now");
        assert!(relative_time(now - 30 * 1000).contains("s ago"));
        assert!(relative_time(now - 5 * 60 * 1000).contains("min"));
        assert!(relative_time(now - 3 * 3600 * 1000).contains("h"));
        assert!(relative_time(now - 2 * 86_400_000).contains("d"));
    }

    #[test]
    fn plugin_metadata() {
        let plugin = mk_plugin(false);
        assert_eq!(plugin.id(), "core.clipboard");
        assert_eq!(plugin.max_results(), 20);
        assert_eq!(plugin.priority(), 12);
    }

    #[test]
    fn result_order_reflects_ring_order() {
        let plugin = mk_plugin(true);
        plugin.history.push("first".into(), "".into());
        plugin.history.push("second".into(), "".into());
        plugin.history.push("third".into(), "".into());
        let r = plugin.search("");
        // Head-first means "third" is first and has highest relevance.
        assert_eq!(r[0].title, "third");
        assert!(r[0].relevance >= r[1].relevance);
        assert!(r[1].relevance >= r[2].relevance);
    }
}

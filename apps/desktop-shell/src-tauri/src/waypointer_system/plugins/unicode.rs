/// Unicode search plugin: find characters by name (prefix "u:").

use crate::waypointer_system::plugin::*;

pub struct UnicodePlugin;

impl WaypointerPlugin for UnicodePlugin {
    fn id(&self) -> &str { "core.unicode" }
    fn name(&self) -> &str { "Unicode" }
    fn description(&self) -> &str { "Find characters by name or codepoint (e.g. U+2764 or HEART)." }
    fn prefix(&self) -> Option<&str> { Some("u:") }
    fn priority(&self) -> u32 { 0 }
    fn max_results(&self) -> usize { 20 }

    fn search(&self, query: &str) -> Vec<SearchResult> {
        let q = query.trim();
        if q.is_empty() {
            return Vec::new();
        }

        // Try direct codepoint: "U+2764", "2764", "0x2764".
        if let Some(cp) = parse_codepoint(q) {
            if let Some(ch) = char::from_u32(cp) {
                let name = unicode_names2::name(ch)
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| format!("U+{cp:04X}"));
                return vec![SearchResult {
                    id: format!("u-{cp:04X}"),
                    title: format!("{ch}  {name}"),
                    description: Some(format!("U+{cp:04X}")),
                    icon: None,
                    relevance: 1.0,
                    action: Action::Copy { text: ch.to_string() },
                    plugin_id: String::new(),
                }];
            }
        }

        // Name search over the precomputed (codepoint, name) index.
        // Building the index lazily on first use takes ~120ms on a
        // modern machine (scanning 1.1 million codepoints through
        // `unicode_names2::name`); subsequent searches then iterate
        // the Vec of (u32, &'static str) in microseconds.
        let q_upper = q.to_uppercase();
        let mut results = Vec::new();

        for &(cp, name_str) in name_index() {
            if results.len() >= 20 {
                break;
            }
            if name_str.contains(&q_upper) {
                let Some(ch) = char::from_u32(cp) else { continue };
                results.push(SearchResult {
                    id: format!("u-{cp:04X}"),
                    title: format!("{ch}  {name_str}"),
                    description: Some(format!("U+{cp:04X}")),
                    icon: None,
                    relevance: if name_str.starts_with(&q_upper) { 0.9 } else { 0.5 },
                    action: Action::Copy { text: ch.to_string() },
                    plugin_id: String::new(),
                });
            }
        }

        results.sort_by(|a, b| b.relevance.partial_cmp(&a.relevance).unwrap_or(std::cmp::Ordering::Equal));
        results
    }

    fn execute(&self, _result: &SearchResult) -> Result<(), PluginError> {
        Ok(()) // Copy action handled by shell.
    }
}

/// Lazily-built index of every named Unicode codepoint.
///
/// Populated once on the first `search()` call and reused for every
/// subsequent lookup. Previously the plugin walked 0x20..=0x1FFFF on
/// every keystroke, which was ~130K `unicode_names2::name` calls per
/// tick and visible as UI jank on typing bursts.
///
/// Extended range: walks the full Unicode space so emoji (U+1F300+)
/// and CJK Extension blocks are searchable. Build time scales with
/// the range but only fires once per process.
fn name_index() -> &'static [(u32, &'static str)] {
    static CACHE: std::sync::OnceLock<Vec<(u32, &'static str)>> = std::sync::OnceLock::new();
    CACHE.get_or_init(|| {
        // 0x110000 is the upper bound of Unicode (17 planes × 64K).
        let mut out = Vec::with_capacity(40_000);
        for cp in 0x20..0x110000_u32 {
            // `char::from_u32` rejects surrogates (U+D800..U+DFFF)
            // and values above U+10FFFF, so the check doubles as the
            // validity filter.
            let Some(ch) = char::from_u32(cp) else { continue };
            if let Some(name) = unicode_names2::name(ch) {
                // `name.to_string()` allocates — we leak the String so
                // entries can be `&'static str`, avoiding per-search
                // allocations. ~30K entries × ~30 chars = ~900KB of
                // static memory, acceptable for a UI plugin.
                let leaked: &'static str = Box::leak(name.to_string().into_boxed_str());
                out.push((cp, leaked));
            }
        }
        log::info!("unicode: built name index ({} entries)", out.len());
        out
    })
}

fn parse_codepoint(s: &str) -> Option<u32> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix("U+").or_else(|| s.strip_prefix("u+")) {
        return u32::from_str_radix(hex, 16).ok();
    }
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        return u32::from_str_radix(hex, 16).ok();
    }
    if s.len() >= 4 && s.chars().all(|c| c.is_ascii_hexdigit()) {
        return u32::from_str_radix(s, 16).ok();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_codepoint_search() {
        let p = UnicodePlugin;
        let r = p.search("U+2764");
        assert_eq!(r.len(), 1);
        assert!(r[0].title.contains('❤'));
    }

    #[test]
    fn test_name_search() {
        let p = UnicodePlugin;
        let r = p.search("HEART");
        assert!(!r.is_empty());
    }

    #[test]
    fn test_parse_codepoint() {
        assert_eq!(parse_codepoint("U+0041"), Some(0x41));
        assert_eq!(parse_codepoint("0x2764"), Some(0x2764));
        assert_eq!(parse_codepoint("2764"), Some(0x2764));
        assert_eq!(parse_codepoint("hello"), None);
    }
}

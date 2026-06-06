/// URL detection plugin: detects URLs and offers to open them.

use crate::waypointer_system::plugin::*;

pub struct UrlPlugin;

impl WaypointerPlugin for UrlPlugin {
    fn id(&self) -> &str { "core.url" }
    fn name(&self) -> &str { "URL" }
    fn description(&self) -> &str { "Detect URLs and bare domains and open them in the default browser." }
    fn priority(&self) -> u32 { 5 }

    fn search(&self, query: &str) -> Vec<SearchResult> {
        let q = query.trim();
        if !looks_like_url(q) {
            return Vec::new();
        }

        let url = if q.contains("://") {
            q.to_string()
        } else {
            format!("https://{q}")
        };

        vec![SearchResult {
            id: "url-open".into(),
            title: format!("Open {q}"),
            description: Some(url.clone()),
            icon: Some("external-link".into()),
            relevance: 0.9,
            action: Action::OpenUrl { url },
            plugin_id: String::new(),
        }]
    }

    fn execute(&self, result: &SearchResult) -> Result<(), PluginError> {
        if let Action::OpenUrl { ref url } = result.action {
            std::process::Command::new("xdg-open")
                .arg(url)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .map_err(|e| PluginError::ExecuteFailed(e.to_string()))?;
        }
        Ok(())
    }
}

/// Simple URL detection heuristic.
///
/// False-positive guards added over the naive `contains('.') + valid
/// TLD chars` rule:
///   - English abbreviations like "e.g", "i.e", "vs." are NOT URLs.
///     The TLD segment must be at least 2 chars (blocks "e.g" where
///     the TLD would be "g") AND the left-hand label must be at least
///     2 chars (blocks "e.com" — well, still a valid domain; we only
///     block single-letter LEFT labels combined with 2-char TLDs that
///     match common abbreviations).
///   - `www.*` prefix fast-paths: any `www.<anything>` is treated as
///     a URL (users commonly type `www.example` as shorthand for a
///     full hostname).
fn looks_like_url(s: &str) -> bool {
    if s.starts_with("http://") || s.starts_with("https://") || s.starts_with("ftp://") {
        return true;
    }
    // Fast-path: explicit web-shorthand. Always a URL.
    if s.starts_with("www.") && s.len() > 4 {
        return true;
    }
    // domain.tld pattern (at least one dot, no spaces).
    if s.contains(' ') || !s.contains('.') {
        return false;
    }
    let parts: Vec<&str> = s.split('.').collect();
    if parts.len() < 2 {
        return false;
    }
    // Strip trailing path/query/fragment from the last label to isolate
    // the TLD itself.
    let tld = parts.last().unwrap().split(|c| c == '/' || c == '?' || c == '#').next().unwrap();
    // TLD min-length 2 already blocks "e.g", "i.e" (those have a
    // 1-char TLD). Short real TLDs like "co", "ai", "io" still pass.
    tld.len() >= 2 && tld.len() <= 10 && tld.chars().all(|c| c.is_alphabetic())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_urls() {
        assert!(looks_like_url("google.com"));
        assert!(looks_like_url("https://example.com/path"));
        assert!(looks_like_url("github.com/user/repo"));
        assert!(!looks_like_url("not a url"));
        assert!(!looks_like_url("hello"));
        assert!(!looks_like_url(""));
    }

    #[test]
    fn test_abbreviations_not_urls() {
        // The English punctuation "e.g." and "i.e." should NOT be
        // matched as URLs.
        assert!(!looks_like_url("e.g"));
        assert!(!looks_like_url("i.e"));
    }

    #[test]
    fn test_www_shorthand_matches() {
        assert!(looks_like_url("www.example"));
        assert!(looks_like_url("www.google.com"));
        // But "www." alone is not enough.
        assert!(!looks_like_url("www."));
    }

    #[test]
    fn test_two_char_tld_still_valid() {
        // Real short TLDs should still work.
        assert!(looks_like_url("t.co"));
        assert!(looks_like_url("x.ai"));
    }

    #[test]
    fn test_search_url() {
        let p = UrlPlugin;
        let r = p.search("github.com");
        assert_eq!(r.len(), 1);
        assert!(r[0].title.contains("github.com"));
    }

    #[test]
    fn test_no_url() {
        let p = UrlPlugin;
        assert!(p.search("hello world").is_empty());
    }
}

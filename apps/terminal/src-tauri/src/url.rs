//! URL opener: hand a link off to the user's browser/mail client through the
//! Arlen portal, falling back to `xdg-open` when no portal frontend is running.
//!
//! Command output contains URLs, and a terminal that draws them as plain text
//! is the one thing every terminal before this one did not do. The grid makes
//! them clickable two ways - a scanner over the printed line, and the handler
//! for programs that speak OSC 8 - and both routes end here.
//!
//! It has to leave the webview. A click that navigated in place would replace
//! the terminal with the target site in a window with no chrome and no back
//! button, and xterm's own fallback for an OSC 8 link is worse still: an English
//! `confirm()` box in every locale, then `window.open`.
//!
//! Restricted to `http`, `https` and `mailto`; anything else is refused, so this
//! cannot be used as a generic `file://`/protocol shell-out from the webview. The
//! URI is passed on as a single argument (never a shell), so it carries no
//! injection risk.

use tauri_plugin_arlen_portal::api;

const ALLOWED_SCHEMES: &[&str] = &["https://", "http://", "mailto:"];

/// Whether `url`'s scheme is on the allowlist. URI schemes are case-insensitive,
/// so the check lowercases the prefix to match (the original `url`, with its real
/// path/query casing, is what gets opened). Kept in lockstep with `SCHEMES` in
/// `src/lib/links.ts`, which decides what the grid underlines: a scheme one side
/// offers and the other refuses is a click that does nothing.
fn scheme_allowed(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    ALLOWED_SCHEMES.iter().any(|s| lower.starts_with(s))
}

/// Open `url` in the user's default handler, if its scheme is allowed.
#[tauri::command]
pub async fn open_url(url: String) -> Result<(), String> {
    if !scheme_allowed(&url) {
        return Err(format!(
            "rejected URL with disallowed scheme: {url}; only http(s) and mailto are supported"
        ));
    }
    api::open_external(&url)
        .await
        .map_err(|e| format!("open {url}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowed_schemes_pass_case_insensitively() {
        for url in [
            "https://github.com/arlenos",
            "http://example.com",
            "https://example.com/p?q=1",
            "mailto:someone@example.com",
            // Schemes are case-insensitive, and the original casing of the
            // path/query is preserved on open.
            "HTTPS://example.com/PaTh",
            "HtTp://example.com",
            "MAILTO:someone@example.com",
        ] {
            assert!(scheme_allowed(url), "expected {url} to pass");
        }
    }

    #[test]
    fn disallowed_schemes_are_rejected() {
        for url in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "data:text/html,<script>1</script>",
            "ftp://example.com",
            "  https://leading-space.example",
            // http(s) must carry `//`; a bare `https:` is not a navigable link.
            "https:no-slashes.example",
            "http:also-bad",
        ] {
            assert!(!scheme_allowed(url), "expected {url} to be rejected");
        }
    }
}

//! URL opener: hand a link off to the user's browser/mail client via
//! `xdg-open`.
//!
//! Markdown answers can contain links. Letting a click navigate the Tauri
//! webview would replace the single-page app with the target site (and there is
//! no back button), so the frontend intercepts link clicks and routes them here
//! to open externally instead.
//!
//! Restricted to the same schemes the markdown sanitizer allows (`http`,
//! `https`, `mailto`); anything else is refused, so this cannot be used as a
//! generic `file://`/protocol shell-out from the webview. The URL is passed to
//! `xdg-open` as a single argument (no shell), so it carries no injection risk.

use std::process::Command;

const ALLOWED_SCHEMES: &[&str] = &["https://", "http://", "mailto:"];

/// Whether `url`'s scheme is on the allowlist. URI schemes are
/// case-insensitive, and the markdown sanitizer accepts them that way, so the
/// check lowercases the prefix to match (the original `url`, with its real
/// path/query casing, is what gets opened). Kept in lockstep with the
/// sanitizer's `ALLOWED_URI_REGEXP` so a rendered link never reaches here only
/// to be rejected.
fn scheme_allowed(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    ALLOWED_SCHEMES.iter().any(|s| lower.starts_with(s))
}

/// Open `url` in the user's default handler, if its scheme is allowed.
#[tauri::command]
pub fn open_url(url: String) -> Result<(), String> {
    if !scheme_allowed(&url) {
        return Err(format!(
            "rejected URL with disallowed scheme: {url}; only http(s) and mailto are supported"
        ));
    }
    Command::new("xdg-open")
        .arg(&url)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("xdg-open: {e}"))?;
    Ok(())
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
            // Schemes are case-insensitive (the sanitizer accepts these too),
            // and the original casing of the path/query is preserved on open.
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
            // http(s) must carry `//`, matching the sanitizer; a bare
            // `https:` is not a navigable link.
            "https:no-slashes.example",
            "http:also-bad",
        ] {
            assert!(!scheme_allowed(url), "expected {url} to be rejected");
        }
    }
}

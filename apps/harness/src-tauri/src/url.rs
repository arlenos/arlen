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

/// Open `url` in the user's default handler, if its scheme is allowed.
#[tauri::command]
pub fn open_url(url: String) -> Result<(), String> {
    if !ALLOWED_SCHEMES.iter().any(|s| url.starts_with(s)) {
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

    fn allowed(url: &str) -> bool {
        ALLOWED_SCHEMES.iter().any(|s| url.starts_with(s))
    }

    #[test]
    fn allowed_schemes_pass() {
        for url in [
            "https://github.com/arlenos",
            "http://example.com",
            "https://example.com/p?q=1",
            "mailto:someone@example.com",
        ] {
            assert!(allowed(url), "expected {url} to pass");
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
        ] {
            assert!(!allowed(url), "expected {url} to be rejected");
        }
    }
}

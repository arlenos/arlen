//! URL opener: `xdg-open <url>`.
//!
//! Replaces the bogus `window.__TAURI__.opener?.openUrl?.(url)`
//! pattern which silently no-oped because no opener-plugin is
//! installed (Codex Sprint D review MEDIUM 2). xdg-open is
//! standard on every Linux desktop and forwards to the user's
//! configured default browser.
//!
//! Restricted to `https://` and `http://` schemes — passing
//! arbitrary `file://` or shell-meta-character URLs from
//! untrusted code paths would be a privilege-escalation surface.

use std::process::Command;

const ALLOWED_SCHEMES: &[&str] = &["https://", "http://"];

#[tauri::command]
pub fn open_url(url: String) -> Result<(), String> {
    if !ALLOWED_SCHEMES.iter().any(|s| url.starts_with(s)) {
        return Err(format!(
            "rejected URL with disallowed scheme: {url}; only http(s) is supported"
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

    /// http(s) URLs pass the scheme check. We don't actually run
    /// `xdg-open` in tests because spawning a browser from CI is
    /// rude; the validation logic is the part worth testing.
    #[test]
    fn allowed_schemes_pass_validation() {
        for url in [
            "https://github.com/lunaris-sys",
            "http://example.com",
            "https://example.com/path?query=1",
        ] {
            assert!(
                ALLOWED_SCHEMES.iter().any(|s| url.starts_with(s)),
                "expected {url} to pass"
            );
        }
    }

    /// Anything outside http(s) is rejected so this command can't
    /// be used as a generic file/protocol shell-out from JS.
    #[test]
    fn disallowed_schemes_are_rejected() {
        for url in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "ftp://example.com",
            "lunaris:///foo",
            "",
            "github.com/no-scheme",
        ] {
            let result = open_url(url.to_string());
            assert!(
                result.is_err(),
                "expected {url} to be rejected, got {result:?}"
            );
            let err = result.unwrap_err();
            assert!(
                err.contains("disallowed scheme"),
                "unexpected error for {url}: {err}"
            );
        }
    }
}

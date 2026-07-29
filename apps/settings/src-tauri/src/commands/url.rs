//! URL opener.
//!
//! Prefers the Arlen portal plugin (`tauri-plugin-arlen-portal`), which routes
//! through `org.freedesktop.portal.OpenURI` and so lands on the Arlen backend
//! like every other portal call; falls back to `xdg-open` when the portal
//! frontend is not available (CI, headless dev, stripped image). This is the
//! same portal-first shape [`super::picker`] uses, so URL opening and file
//! picking do not take two different routes out of Settings.
//!
//! Restricted to `https://` and `http://` schemes — passing arbitrary `file://`
//! or shell-meta-character URLs from untrusted code paths would be a
//! privilege-escalation surface. This check is deliberately TIGHTER than the
//! plugin's own allowlist (which also permits mailto/tel/sms/xmpp/ftps/file), so
//! it stays in front of the plugin call rather than delegating to it.

use std::process::Command;

use tauri_plugin_arlen_portal::{api, OpenUriOptions, PickerError};

const ALLOWED_SCHEMES: &[&str] = &["https://", "http://"];

/// Settings' own scheme gate, applied before any portal or shell-out call.
/// Pure, so the security-relevant part is unit-tested directly rather than by
/// driving the command (which would need a bus and a browser).
fn validate_scheme(url: &str) -> Result<(), String> {
    if !ALLOWED_SCHEMES.iter().any(|s| url.starts_with(s)) {
        return Err(format!(
            "rejected URL with disallowed scheme: {url}; only http(s) is supported"
        ));
    }
    Ok(())
}

#[tauri::command]
pub async fn open_url(url: String) -> Result<(), String> {
    validate_scheme(&url)?;

    match api::open_uri(&url, OpenUriOptions::default()).await {
        Ok(()) => return Ok(()),
        Err(PickerError::PortalUnavailable { .. }) | Err(PickerError::ConnectionLost { .. }) => {
            log::info!("portal unavailable, falling back to xdg-open");
        }
        Err(e) => return Err(format!("portal open_uri failed: {e}")),
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

    /// http(s) URLs pass the scheme check. We don't actually open anything in
    /// tests because spawning a browser from CI is rude; the validation logic is
    /// the part worth testing, and this calls the real gate rather than
    /// re-implementing the check.
    #[test]
    fn allowed_schemes_pass_validation() {
        for url in [
            "https://github.com/arlenos",
            "http://example.com",
            "https://example.com/path?query=1",
        ] {
            assert!(validate_scheme(url).is_ok(), "expected {url} to pass");
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
            "arlen:///foo",
            "",
            "github.com/no-scheme",
        ] {
            let result = validate_scheme(url);
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

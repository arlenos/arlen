// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

//! The store's URL opener: the support and homepage links on an app's page.
//!
//! Its own copy rather than a call into another app's, because a Tauri command
//! is compiled into one binary and registered on that binary's handler - the
//! store invoking Settings' `open_url` is a call the runtime rejects, however
//! well `grep` finds the name. Four apps already carry this command for the same
//! reason; this is the fifth, not a new pattern.
//!
//! Opens through `tauri_plugin_arlen_portal::api::open_external`, which prefers
//! `org.freedesktop.portal.OpenURI` and falls back to `xdg-open` when no portal
//! frontend runs.
//!
//! **http(s) only**, and deliberately tighter than the plugin's own allowlist
//! (which also permits mailto/tel/sms/xmpp/ftps/file). A store page's links come
//! from a package manifest, which is to say from whoever published the package,
//! so this is the one app where the URL is least the user's own. A `file://` or
//! handler-scheme link out of a manifest is a privilege-escalation surface, and
//! the store has no use for one.

use tauri_plugin_arlen_portal::api;

const ALLOWED_SCHEMES: &[&str] = &["https://", "http://"];

/// Whether the store will open `url` at all. Pure, so the part that matters is
/// tested without spawning a browser.
fn scheme_allowed(url: &str) -> bool {
    ALLOWED_SCHEMES.iter().any(|s| url.starts_with(s))
}

/// Open a store link in the user's browser.
#[tauri::command]
pub async fn open_url(url: String) -> Result<(), String> {
    if !scheme_allowed(&url) {
        return Err(format!(
            "rejected URL with disallowed scheme: {url}; only http(s) is supported"
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
    fn http_and_https_are_the_only_schemes_a_manifest_can_reach() {
        assert!(scheme_allowed("https://example.com/support"));
        assert!(scheme_allowed("http://example.com"));
        for url in [
            "file:///etc/passwd",
            "mailto:someone@example.com",
            "javascript:alert(1)",
            "data:text/html,<script>alert(1)</script>",
            // No scheme at all: `xdg-open` would treat it as a path.
            "/etc/passwd",
            "",
        ] {
            assert!(!scheme_allowed(url), "{url} must be refused");
        }
    }

    #[test]
    fn the_check_is_on_the_scheme_and_not_on_the_rest_of_the_url() {
        // A hostile-looking path is still just a path to a browser; refusing it
        // would be theatre, and the scheme is the part that decides who opens it.
        assert!(scheme_allowed("https://example.com/../../etc/passwd?x=';rm -rf'"));
    }
}

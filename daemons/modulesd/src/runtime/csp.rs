/// Content-Security-Policy header generator for Tier 2 iframe modules.
///
/// The shell's `module://` Tauri scheme handler asks the daemon for
/// the CSP string when serving an iframe asset; the daemon derives it
/// from the module's manifest. Putting the generator here keeps the
/// rule in one place and lets the shell-side handler stay a thin
/// pass-through.
///
/// Policy:
///   * `default-src 'self' module://{module_id}` — assets only from
///     the module's own bundle. No inline scripts, no remote images.
///   * `connect-src` — union of the manifest `network.allow` domains.
///     `'self'` is included so postMessage round trips do not error
///     out (postMessage is not gated by `connect-src` but we keep the
///     allowlist tight to avoid surprises).
///   * `script-src 'self'` — only the module's own JS bundle. We do
///     not allow `'unsafe-inline'` because every modern bundler emits
///     external scripts; iframe authors who ship inline JS were doing
///     it wrong already.
///   * `frame-ancestors 'self'` — only the desktop-shell webview can
///     embed the module. Prevents another module from inlining a
///     competitor's iframe.
///   * `style-src 'self' 'unsafe-inline'` — Svelte and Tailwind both
///     emit inline style attributes; without `'unsafe-inline'` here,
///     standard component libraries refuse to render. This is the one
///     concession we make.

use arlen_modules::ModuleCapabilities;

/// Build the per-module CSP header value.
pub fn build_csp(module_id: &str, capabilities: &ModuleCapabilities) -> String {
    let mut connect = vec!["'self'".to_string()];
    if let Some(net) = &capabilities.network {
        for domain in &net.allowed_domains {
            connect.push(format!("https://{domain}"));
        }
    }

    [
        format!("default-src 'self' module://{module_id}"),
        format!("script-src 'self'"),
        format!("style-src 'self' 'unsafe-inline'"),
        format!("img-src 'self' module://{module_id} data:"),
        format!("font-src 'self' module://{module_id} data:"),
        format!("connect-src {}", connect.join(" ")),
        format!("frame-ancestors 'self'"),
        format!("base-uri 'self'"),
        format!("form-action 'none'"),
    ]
    .join("; ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use arlen_modules::NetworkCapability;

    #[test]
    fn csp_contains_module_origin_in_default_src() {
        let csp = build_csp("com.example.weather", &ModuleCapabilities::default());
        assert!(csp.contains("default-src 'self' module://com.example.weather"));
    }

    #[test]
    fn csp_includes_network_allowlist_in_connect_src() {
        let mut caps = ModuleCapabilities::default();
        caps.network = Some(NetworkCapability {
            allowed_domains: vec!["api.example.com".into(), "cdn.example.com".into()],
        });
        let csp = build_csp("x", &caps);
        assert!(csp.contains("https://api.example.com"));
        assert!(csp.contains("https://cdn.example.com"));
    }

    #[test]
    fn csp_locks_frame_ancestors_to_self() {
        let csp = build_csp("x", &ModuleCapabilities::default());
        assert!(csp.contains("frame-ancestors 'self'"));
    }

    #[test]
    fn csp_disallows_form_submission() {
        let csp = build_csp("x", &ModuleCapabilities::default());
        assert!(csp.contains("form-action 'none'"));
    }

    #[test]
    fn csp_blocks_inline_script() {
        let csp = build_csp("x", &ModuleCapabilities::default());
        assert!(csp.contains("script-src 'self'"));
        assert!(!csp.contains("'unsafe-inline'") || !csp.contains("script-src 'self' 'unsafe-inline'"));
    }
}

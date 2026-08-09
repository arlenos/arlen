// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Open the Settings app on a particular page.
//!
//! The Knowledge app's privacy line offers to take you to the setting that
//! governs what is recorded. It invoked a command nothing registered, so the
//! link did nothing - the worst shape for a privacy affordance, because the
//! person clicking it is trying to find a control and concludes there is none.
//!
//! The shell already does this by spawning `arlen-settings --panel <name>`, and
//! Settings parses exactly that at startup. This is the same call from here
//! rather than a second mechanism: one binary, one flag, one place to change if
//! the launch contract moves.

/// The pages this may link to.
///
/// An allowlist rather than a pass-through, because the argument arrives from a
/// frontend and is handed to a process launch. Not a security boundary - the
/// same user could run the binary themselves - but a route that does not exist
/// opens Settings on nothing, and a caller typo becomes a blank window rather
/// than an error somebody can read.
const PANELS: &[&str] = &[
    "privacy",
    "knowledge",
    "appearance",
    "ai",
    "apps",
    "about",
    "notifications",
];

/// The panel name for a route like `/privacy` or `privacy`.
fn panel_of(route: &str) -> Option<&'static str> {
    let name = route.trim_start_matches('/').split('/').next()?;
    PANELS.iter().copied().find(|p| *p == name)
}

/// Open Settings on `route`.
#[tauri::command]
pub async fn open_settings_route(route: String) -> Result<(), String> {
    let panel = panel_of(&route).ok_or_else(|| format!("{route}: not a Settings page"))?;
    std::process::Command::new("arlen-settings")
        .arg("--panel")
        .arg(panel)
        .spawn()
        .map_err(|e| format!("could not open Settings: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_leading_slash_and_a_subpath_both_resolve_to_the_panel() {
        assert_eq!(panel_of("/privacy"), Some("privacy"));
        assert_eq!(panel_of("privacy"), Some("privacy"));
        assert_eq!(panel_of("/privacy/physical"), Some("privacy"));
    }

    #[test]
    fn an_unknown_route_is_refused_rather_than_opening_settings_on_nothing() {
        assert_eq!(panel_of("/nowhere"), None);
        assert_eq!(panel_of(""), None);
        assert_eq!(panel_of("/"), None);
    }
}

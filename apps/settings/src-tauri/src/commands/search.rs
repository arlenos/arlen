//! Settings search index export and CLI argument handling.

use std::path::PathBuf;
use std::sync::Mutex;

use serde::Serialize;

/// Parsed launch args. `take()` clears after first read so the
/// frontend only navigates once even if it calls the command again.
static LAUNCH_ARGS: Mutex<Option<LaunchTarget>> = Mutex::new(None);

#[derive(Debug, Clone, Serialize)]
pub struct LaunchTarget {
    pub panel: String,
    pub anchor: Option<String>,
    /// An app id, when the caller wants one app's own settings page rather than a
    /// panel. Separate from `panel` because the frontend resolves a panel against
    /// its table of known panels and drops anything absent - a path smuggled
    /// through that field would navigate nowhere, silently.
    pub app: Option<String>,
}

/// Store the parsed CLI args so `get_launch_args` can return them.
/// Called once from `lib.rs` setup before the frontend mounts.
pub fn store_launch_args() {
    if let Some((panel, anchor, app)) = parse_cli_args() {
        *LAUNCH_ARGS.lock().unwrap() = Some(LaunchTarget { panel, anchor, app });
    }
}

/// Return the launch navigation target (if any) and clear it so
/// subsequent calls return `None`. The frontend calls this once in
/// `onMount` after all stores are initialised.
#[tauri::command]
pub fn get_launch_args() -> Option<LaunchTarget> {
    LAUNCH_ARGS.lock().unwrap().take()
}

/// Where the exported index lives so Waypointer can read it without
/// starting the Settings app.
fn index_path() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("arlen")
        .join("settings-index.json")
}

/// Where the exported catalogs live: `<locale>.json` per locale, the layout
/// `arlen_i18n::Localizer::load_dir` reads.
fn catalog_dir() -> PathBuf {
    index_path()
        .parent()
        .unwrap_or(&PathBuf::from("/tmp"))
        .join("catalogs")
        .join("settings")
}

/// Write the settings search index and the catalogs it points at.
///
/// The index carries message ids, not prose, so a reader holding only the index
/// has nothing to show; the catalogs travel with it. Both come from one call, and
/// **the catalogs are written first**: a reader that sees a new index can then
/// always resolve it, where the other order leaves a window in which the index
/// names messages that are not on disk yet.
#[tauri::command]
pub fn export_settings_index(json: String, catalogs: String) -> Result<(), String> {
    let by_locale: std::collections::BTreeMap<String, serde_json::Value> =
        serde_json::from_str(&catalogs).map_err(|e| format!("catalogs: {e}"))?;

    let dir = catalog_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("create catalog dir: {e}"))?;
    for (locale, messages) in &by_locale {
        // A locale is a filename here, so it may not steer the write anywhere else.
        if locale.is_empty() || !locale.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
            return Err(format!("catalogs: `{locale}` is not a locale"));
        }
        let body = serde_json::to_string(messages).map_err(|e| format!("catalogs: {e}"))?;
        std::fs::write(dir.join(format!("{locale}.json")), body)
            .map_err(|e| format!("write catalog {locale}: {e}"))?;
    }

    let path = index_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create dir: {e}"))?;
    }
    std::fs::write(&path, &json).map_err(|e| format!("write: {e}"))?;
    log::info!(
        "settings index exported ({} bytes, {} catalog(s)) to {}",
        json.len(),
        by_locale.len(),
        path.display()
    );
    Ok(())
}

/// Parse CLI arguments into a navigation target.
///
/// `--panel <id>`, optionally `--section <name>`, and optionally `--app <app-id>`
/// for one app's own settings page. `--app` implies the apps panel, so an app
/// linking to its own settings does not have to know which panel holds them
/// (per-app-settings-plan.md 4b: every app's header menu points at its page).
///
/// Called from setup() and the result is handed to the frontend so it can
/// navigate on mount.
pub fn parse_cli_args() -> Option<(String, Option<String>, Option<String>)> {
    let args: Vec<String> = std::env::args().collect();
    parse_args(&args)
}

/// The parsing itself, over a slice. Separate from the process arguments so the
/// defaulting rules can be tested; `std::env::args` is not something a test can
/// set without affecting every other test in the binary.
fn parse_args(args: &[String]) -> Option<(String, Option<String>, Option<String>)> {
    let mut panel: Option<String> = None;
    let mut section: Option<String> = None;
    let mut app: Option<String> = None;

    let mut i = 1; // skip binary name
    while i < args.len() {
        match args[i].as_str() {
            "--panel" if i + 1 < args.len() => {
                panel = Some(args[i + 1].clone());
                i += 2;
            }
            "--section" | "--setting" if i + 1 < args.len() => {
                section = Some(args[i + 1].clone());
                i += 2;
            }
            "--app" if i + 1 < args.len() => {
                app = Some(args[i + 1].clone());
                i += 2;
            }
            _ => {
                i += 1;
            }
        }
    }

    // An app id names its page well enough on its own, so `--app` alone is a
    // complete request and defaults the panel to the list it belongs to.
    match (panel, app) {
        (p, Some(a)) => Some((p.unwrap_or_else(|| "apps".to_owned()), section, Some(a))),
        (Some(p), None) => Some((p, section, None)),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(rest: &[&str]) -> Vec<String> {
        std::iter::once("arlen-settings").chain(rest.iter().copied()).map(str::to_owned).collect()
    }

    #[test]
    fn a_panel_and_a_section_parse_as_before() {
        assert_eq!(
            parse_args(&args(&["--panel", "keyboard", "--section", "repeat"])),
            Some(("keyboard".to_owned(), Some("repeat".to_owned()), None)),
        );
        assert_eq!(parse_args(&args(&[])), None, "no arguments is no navigation");
    }

    #[test]
    fn an_app_id_alone_is_a_complete_request() {
        // So an app linking to its own settings does not have to know which panel
        // holds the list it belongs to.
        assert_eq!(
            parse_args(&args(&["--app", "org.arlen.files"])),
            Some(("apps".to_owned(), None, Some("org.arlen.files".to_owned()))),
        );
    }

    #[test]
    fn an_explicit_panel_is_kept_alongside_an_app() {
        assert_eq!(
            parse_args(&args(&["--panel", "privacy", "--app", "org.arlen.files"])),
            Some(("privacy".to_owned(), None, Some("org.arlen.files".to_owned()))),
        );
    }

    #[test]
    fn a_flag_with_no_value_is_ignored_rather_than_swallowing_the_next_one() {
        // `--app` last with nothing after it must not consume the terminator or
        // report an empty id the router would turn into `/apps/`.
        assert_eq!(parse_args(&args(&["--panel", "display", "--app"])),
            Some(("display".to_owned(), None, None)));
    }
}

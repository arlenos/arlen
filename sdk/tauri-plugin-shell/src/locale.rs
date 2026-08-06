//! The UI language, for every app that embeds this plugin.
//!
//! Settings writes `~/.config/arlen/locale.toml`; this reads it and watches it,
//! so a language chosen in one place takes effect everywhere without a restart.
//! The same shape as the theme consumer next door, and for the same reason: a
//! choice that only one surface honours is not a system setting.
//!
//! Before this existed there was no reader at all. The catalogs, the fallback
//! chain and the reactive store were all in place and nothing ever set the store,
//! so every translation in the tree was unreachable.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use notify::{Event, EventKind, RecursiveMode, Watcher};
use tauri::{AppHandle, Emitter, Runtime};

/// The language the messages are authored in, and the floor of every fallback.
///
/// This predicate also lives in `arlen_i18n::chosen`, which the desktop shell and
/// the daemons use. Not shared from there: `sdk/i18n` declares its own workspace
/// and this plugin is an `sdk` member, so a path dependency is a "multiple
/// workspace roots" error rather than a link. Both sides carry the same tests, so
/// a change to the rule that lands on one shows up as a failure on the other.
const SOURCE_LOCALE: &str = "en";

/// `~/.config/arlen/locale.toml`.
fn locale_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("arlen")
        .join("locale.toml")
}

/// The chosen UI language, or the source language when nothing has chosen.
///
/// Deliberately not `LANG`: a machine nobody has told otherwise gets the language
/// the messages were written in, rather than a guess from a locale that may have
/// been set for number and date formats alone.
fn current_locale() -> String {
    let Ok(text) = std::fs::read_to_string(locale_path()) else {
        return SOURCE_LOCALE.to_string();
    };
    let Ok(doc) = text.parse::<toml::Table>() else {
        return SOURCE_LOCALE.to_string();
    };
    doc.get("locale")
        .and_then(|l| l.get("ui"))
        .and_then(|v| v.as_str())
        .filter(|tag| is_locale_tag(tag))
        .unwrap_or(SOURCE_LOCALE)
        .to_string()
}

/// A BCP-47-shaped tag, loosely: the value reaches a catalog lookup and an
/// `Intl` constructor, so anything else is refused rather than passed on.
fn is_locale_tag(tag: &str) -> bool {
    !tag.is_empty()
        && tag.len() <= 35
        && tag
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// The current UI language tag.
#[tauri::command]
pub fn locale_get() -> String {
    current_locale()
}

/// Watch `locale.toml` and re-emit `arlen://locale-changed` when it changes, so
/// a language switch reaches an app that is already running.
pub fn spawn_locale_watcher<R: Runtime>(app: AppHandle<R>) {
    let target = locale_path();
    let Some(watch_dir) = target.parent().map(|p| p.to_path_buf()) else {
        log::warn!("locale consumer: config path has no parent dir");
        return;
    };
    let _ = std::fs::create_dir_all(&watch_dir);

    std::thread::spawn(move || {
        let app_clone = app.clone();
        let last_fire = std::sync::Mutex::new(Instant::now() - Duration::from_secs(1));

        let mut watcher = match notify::recommended_watcher(move |event: Result<Event, _>| {
            let Ok(event) = event else { return };
            if !matches!(
                event.kind,
                EventKind::Modify(_) | EventKind::Create(_) | EventKind::Remove(_)
            ) {
                return;
            }
            if !event
                .paths
                .iter()
                .any(|p| p.file_name().map(|n| n == "locale.toml").unwrap_or(false))
            {
                return;
            }
            // Debounce: an atomic write is a burst of events for one change.
            {
                let mut lf = last_fire.lock().unwrap();
                if lf.elapsed() < Duration::from_millis(100) {
                    return;
                }
                *lf = Instant::now();
            }
            std::thread::sleep(Duration::from_millis(30));

            let tag = current_locale();
            if let Err(e) = app_clone.emit("arlen://locale-changed", &tag) {
                log::warn!("locale consumer: emit failed: {e}");
            }
        }) {
            Ok(w) => w,
            Err(e) => {
                log::warn!("locale consumer: failed to create watcher: {e}");
                return;
            }
        };

        if let Err(e) = watcher.watch(&watch_dir, RecursiveMode::NonRecursive) {
            log::warn!("locale consumer: failed to watch {}: {e}", watch_dir.display());
            return;
        }

        loop {
            std::thread::sleep(Duration::from_secs(3600));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tag_is_letters_digits_and_hyphens() {
        assert!(is_locale_tag("de"));
        assert!(is_locale_tag("zh-Hant-TW"));
        // The value reaches a catalog lookup and an `Intl` constructor; a path
        // or an injected separator has no business in either.
        assert!(!is_locale_tag(""));
        assert!(!is_locale_tag("../etc"));
        assert!(!is_locale_tag("de_AT.UTF-8"));
        assert!(!is_locale_tag(&"x".repeat(36)));
    }
}

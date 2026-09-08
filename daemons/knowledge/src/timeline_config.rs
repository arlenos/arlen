//! Recording pause, read from `~/.config/arlen/graph.toml` `[timeline]`.
//!
//! The Knowledge app's timeline has a Pause switch, and its own copy says what
//! that has to mean: "Recording is paused. Nothing is added until you resume."
//! So this is not a display filter - while it is on, the writer must not admit
//! events into the store at all. Anything less would be the shape this tree has
//! spent two days removing: a surface asserting a state nobody enforces.
//!
//! Watched, not just read once. The switch is a privacy control, and a pause
//! that only takes effect after a restart is the same lie in slower motion: the
//! user asks for collection to stop, the file says it stopped, and the daemon
//! keeps writing until something else happens to restart it. `watch_paused`
//! below keeps a shared flag current so the writer sees a change within a
//! moment of the file being saved.
use serde::Deserialize;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// `[timeline]` section of `graph.toml`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct TimelineConfig {
    /// While true, no event is written to the store.
    #[serde(default)]
    pub paused: bool,
    /// Apps whose activity is never stored.
    ///
    /// Settings has written this since the Knowledge page existed and nothing
    /// read it, under a row that says "Nothing these apps do is recorded". A
    /// person excluded their password manager and the daemon went on recording
    /// it, which is the same class as the pause above and worse in kind: a pause
    /// is visible when it fails, an exclusion is not.
    #[serde(default)]
    pub excluded_apps: Vec<String>,
    /// Path prefixes whose activity is never stored. Same history as
    /// `excluded_apps`, under "Activity under these paths is never recorded".
    #[serde(default)]
    pub excluded_paths: Vec<String>,
}

impl TimelineConfig {
    /// Whether an event from `app_id` about `path` is excluded from the store.
    ///
    /// Apps match EXACTLY, case-insensitively: the id is a machine value the
    /// user picks from a list of what has been seen, and a substring rule would
    /// make `mail` exclude `mailbox-indexer` without saying so. Paths match as
    /// PREFIXES on a component boundary, which is what "activity under this
    /// path" means - `/home/t/private` covers `/home/t/private/x` and not
    /// `/home/t/private-notes`.
    ///
    /// Either field empty means that half excludes nothing, never everything: a
    /// list nobody has filled in is not a claim about all apps.
    pub fn is_excluded(&self, app_id: &str, path: &str) -> bool {
        if !app_id.is_empty()
            && self
                .excluded_apps
                .iter()
                .any(|a| a.eq_ignore_ascii_case(app_id))
        {
            return true;
        }
        !path.is_empty() && self.excluded_paths.iter().any(|p| path_under(path, p))
    }
}

/// Whether `path` is `prefix` or sits under it, on a component boundary.
fn path_under(path: &str, prefix: &str) -> bool {
    let prefix = prefix.trim_end_matches('/');
    if prefix.is_empty() {
        return false;
    }
    path == prefix || path.strip_prefix(prefix).is_some_and(|r| r.starts_with('/'))
}

/// Just enough of the file to reach `[timeline]`; the other sections have their
/// own readers and are none of this one's business.
#[derive(Debug, Clone, Default, Deserialize)]
struct GraphConfig {
    #[serde(default)]
    timeline: TimelineConfig,
}

impl TimelineConfig {
    /// Load from `~/.config/arlen/graph.toml`, defaulting to recording.
    ///
    /// Every failure defaults to NOT paused, and that direction is chosen: an
    /// unreadable config leaving recording on matches what the surface will say,
    /// while defaulting to paused would silently stop collection with the app
    /// still showing it running.
    pub fn load() -> Self {
        let Some(path) = dirs::config_dir().map(|p| p.join("arlen/graph.toml")) else {
            return Self::default();
        };
        if !path.exists() {
            return Self::default();
        }
        match std::fs::read_to_string(&path) {
            Ok(content) => match toml::from_str::<GraphConfig>(&content) {
                Ok(cfg) => cfg.timeline,
                Err(e) => {
                    tracing::warn!("{} is not valid TOML ({e}); recording stays on", path.display());
                    Self::default()
                }
            },
            Err(e) => {
                tracing::warn!("{} could not be read ({e}); recording stays on", path.display());
                Self::default()
            }
        }
    }
}

/// Keep `flag` in step with `[timeline] paused`, for as long as the daemon runs.
///
/// Polled rather than inotify-watched, on purpose. Editors save by writing a
/// temporary file and renaming it over the target, so a watch registered on the
/// path itself stops seeing changes after the first save - a failure that looks
/// exactly like "the setting does not work" and is tedious to find. A read of one
/// small file every few seconds costs nothing and cannot lose the file.
///
/// The lag is honest rather than hidden: a save takes effect within one interval,
/// not instantly, and that is the guarantee the surface should make.
/// The one flag every collector in this daemon reads.
///
/// Process-wide because the question is process-wide - "is this daemon recording"
/// has one answer, and two collectors reading two flags is how half a pause
/// happens. Threading it through both entry points would say the same thing with
/// more rope: `writer::run` and `project::watcher::run` are started side by side
/// in `main` and neither owns the setting.
static PAUSED: std::sync::OnceLock<Arc<AtomicBool>> = std::sync::OnceLock::new();

/// The shared flag, seeded from the config on first use.
pub fn paused_flag() -> Arc<AtomicBool> {
    PAUSED
        .get_or_init(|| Arc::new(AtomicBool::new(TimelineConfig::load().paused)))
        .clone()
}

/// Is collection paused right now?
pub fn is_paused() -> bool {
    paused_flag().load(Ordering::Relaxed)
}

/// The exclusion lists every collector reads, kept current beside the flag.
///
/// A `RwLock` rather than an atomic because the value is two lists, and read by
/// every admitted event: readers never block each other, and the writer takes it
/// once per poll interval only when the file changed.
static RULES: std::sync::OnceLock<Arc<std::sync::RwLock<TimelineConfig>>> =
    std::sync::OnceLock::new();

/// The shared rules, seeded from the config on first use.
pub fn rules() -> Arc<std::sync::RwLock<TimelineConfig>> {
    RULES
        .get_or_init(|| Arc::new(std::sync::RwLock::new(TimelineConfig::load())))
        .clone()
}

/// Is this event excluded right now?
///
/// A poisoned lock reads as NOT excluded, deliberately and against the usual
/// fail-closed instinct: the alternative is a panic in one collector silently
/// switching recording off for everything, and a person who notices that their
/// timeline stopped has no way to connect it to a lock. The exclusion is a
/// privacy promise, so this is the one place worth stating - if the lock is ever
/// poisoned the promise is not kept, and the log line says so.
pub fn is_excluded(app_id: &str, path: &str) -> bool {
    match rules().read() {
        Ok(cfg) => cfg.is_excluded(app_id, path),
        Err(e) => {
            tracing::error!("timeline rules unreadable ({e}); exclusions are not being applied");
            false
        }
    }
}

pub async fn watch_paused(flag: Arc<AtomicBool>) {
    const INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);
    let mut last = flag.load(Ordering::Relaxed);
    loop {
        tokio::time::sleep(INTERVAL).await;
        let cfg = TimelineConfig::load();
        // The lists travel with the flag: one read of one file answers both, and
        // a second poller would be a second answer to "what is recorded".
        if let Ok(mut guard) = rules().write() {
            if guard.excluded_apps != cfg.excluded_apps
                || guard.excluded_paths != cfg.excluded_paths
            {
                tracing::info!(
                    "timeline exclusions changed: {} app(s), {} path(s)",
                    cfg.excluded_apps.len(),
                    cfg.excluded_paths.len()
                );
            }
            *guard = cfg.clone();
        }
        let now = cfg.paused;
        if now != last {
            flag.store(now, Ordering::Relaxed);
            last = now;
            if now {
                tracing::info!("recording paused; events are read and discarded until it resumes");
            } else {
                tracing::info!("recording resumed; events are being stored again");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> TimelineConfig {
        toml::from_str::<GraphConfig>(s).map(|c| c.timeline).unwrap_or_default()
    }

    #[test]
    fn an_excluded_app_is_matched_exactly_and_case_insensitively() {
        let cfg = parse("[timeline]\nexcluded_apps = [\"dev.arlen.mail\"]\n");
        assert!(cfg.is_excluded("dev.arlen.mail", "/home/t/x"));
        assert!(cfg.is_excluded("DEV.ARLEN.MAIL", ""));
        // Not a substring rule: an id that merely contains an excluded one is a
        // different app, and excluding it without being asked is the quiet kind
        // of wrong on a privacy control.
        assert!(!cfg.is_excluded("dev.arlen.mailbox-indexer", ""));
    }

    #[test]
    fn an_excluded_path_covers_what_is_under_it_and_nothing_beside_it() {
        let cfg = parse("[timeline]\nexcluded_paths = [\"/home/t/private\"]\n");
        assert!(cfg.is_excluded("", "/home/t/private"));
        assert!(cfg.is_excluded("", "/home/t/private/notes/a.md"));
        // The component boundary is the whole point of the rule.
        assert!(!cfg.is_excluded("", "/home/t/private-notes/a.md"));
        assert!(!cfg.is_excluded("", "/home/t/public/a.md"));
    }

    #[test]
    fn a_trailing_slash_on_the_rule_changes_nothing() {
        let cfg = parse("[timeline]\nexcluded_paths = [\"/home/t/private/\"]\n");
        assert!(cfg.is_excluded("", "/home/t/private/a.md"));
    }

    #[test]
    fn empty_lists_exclude_nothing_rather_than_everything() {
        // A list nobody filled in is not a claim about all apps, and reading it
        // that way would stop recording on a fresh install.
        let cfg = parse("[timeline]\npaused = false\n");
        assert!(!cfg.is_excluded("dev.arlen.files", "/home/t/x"));
        assert!(!cfg.is_excluded("", ""));
    }

    #[test]
    fn an_empty_rule_entry_is_not_a_wildcard() {
        // An empty string in the list would prefix-match every path if the rule
        // were naive. Somebody pressing Add on a blank field must not switch
        // recording off for the whole disk.
        let cfg = parse("[timeline]\nexcluded_paths = [\"\", \"/\"]\n");
        assert!(!cfg.is_excluded("", "/home/t/x"));
    }

    #[test]
    fn absent_section_records() {
        assert!(!parse("[projects]\nmax_depth = 2\n").paused);
    }

    #[test]
    fn the_flag_is_read() {
        assert!(parse("[timeline]\npaused = true\n").paused);
        assert!(!parse("[timeline]\npaused = false\n").paused);
    }

    #[test]
    fn an_unrelated_section_does_not_pause_recording() {
        // The failure that would matter: a parse quirk reading some other key as
        // the pause and stopping collection nobody asked to stop.
        assert!(!parse("[projects]\nauto_promote_threshold = 9\n[graph]\npaused = true\n").paused);
    }
}

/// Recording pause, read from `~/.config/arlen/graph.toml` `[timeline]`.
///
/// The Knowledge app's timeline has a Pause switch, and its own copy says what
/// that has to mean: "Recording is paused. Nothing is added until you resume."
/// So this is not a display filter - while it is on, the writer must not admit
/// events into the store at all. Anything less would be the shape this tree has
/// spent two days removing: a surface asserting a state nobody enforces.
///
/// Watched, not just read once. The switch is a privacy control, and a pause
/// that only takes effect after a restart is the same lie in slower motion: the
/// user asks for collection to stop, the file says it stopped, and the daemon
/// keeps writing until something else happens to restart it. `watch_paused`
/// below keeps a shared flag current so the writer sees a change within a
/// moment of the file being saved.
use serde::Deserialize;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// `[timeline]` section of `graph.toml`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct TimelineConfig {
    /// While true, no event is written to the store.
    #[serde(default)]
    pub paused: bool,
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

pub async fn watch_paused(flag: Arc<AtomicBool>) {
    const INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);
    let mut last = flag.load(Ordering::Relaxed);
    loop {
        tokio::time::sleep(INTERVAL).await;
        let now = TimelineConfig::load().paused;
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

/// File watcher for live config reload.
///
/// Watches the parent directories of config files (to catch atomic
/// rename-writes from editors) and debounces rapid changes at 100ms.
///
/// See `docs/architecture/config-system.md` (live reload section).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde::de::DeserializeOwned;

use crate::ConfigError;

/// Debounce window: rapid changes within this period produce a single callback.
const DEBOUNCE_MS: u64 = 100;

/// How long the loop blocks before re-checking the stop flag.
const POLL_MS: u64 = 500;

/// Collapses a burst of filesystem events into one reload.
///
/// TRAILING EDGE, and that is a correction rather than a preference. The
/// previous version fired on the FIRST event of a burst and then suppressed for
/// the window, which loses the burst's outcome: the callback reloads the file at
/// fire time, so a save that lands two milliseconds later is read by nobody, and
/// nothing further arrives to correct it. The consumer then holds a stale config
/// indefinitely while every log line says the reload happened. Waiting for the
/// burst to settle and reloading once delivers the state the file actually ended
/// up in, which is the only state a consumer can act on.
///
/// It is also the reason this type exists at all rather than two `Instant`s in
/// the loop. The decision is pure - given the last event's time and the current
/// time, has the burst settled - so it can be driven by a clock the test writes
/// down, and "a burst collapses to one reload" becomes an assertion instead of a
/// sleep long enough to usually be true. `test_debounce_rapid_changes` failed CI
/// on 13 Aug by losing exactly that race on a loaded runner.
#[derive(Debug)]
struct Debounce {
    window: Duration,
    /// When the most recent event of the current burst arrived.
    latest: Option<Instant>,
}

impl Debounce {
    fn new(window: Duration) -> Self {
        Self {
            window,
            latest: None,
        }
    }

    /// Note an event. The burst is unsettled until the window passes with none.
    fn record(&mut self, now: Instant) {
        self.latest = Some(now);
    }

    /// How long until the current burst settles, or `None` when none is open.
    fn wait(&self, now: Instant) -> Option<Duration> {
        self.latest
            .map(|t| self.window.saturating_sub(now.duration_since(t)))
    }

    /// Take the settled burst, at most once per burst.
    fn take_settled(&mut self, now: Instant) -> bool {
        match self.latest {
            Some(t) if now.duration_since(t) >= self.window => {
                self.latest = None;
                true
            }
            _ => false,
        }
    }
}

/// A handle to a running config watcher. Drop or call `stop()` to clean up.
pub struct ConfigWatcher {
    running: Arc<AtomicBool>,
    // Thread handle kept for join-on-drop if needed. We don't join
    // automatically because the notify watcher blocks; stopping is
    // via the `running` flag which causes the thread to exit.
    _thread: Option<std::thread::JoinHandle<()>>,
}

impl ConfigWatcher {
    /// Watch a component's config files and call `callback` whenever the
    /// config changes on disk.
    ///
    /// Watches both system defaults and user config directories. The callback
    /// receives `Ok(T)` with the freshly merged config on valid changes, or
    /// `Err(ConfigError)` if the new file is invalid (the watcher keeps
    /// running).
    ///
    /// The watcher runs on a dedicated thread. `callback` must be `Send + 'static`.
    pub fn watch<T, F>(
        component: &str,
        defaults_path: Option<PathBuf>,
        user_path: Option<PathBuf>,
        callback: F,
    ) -> Self
    where
        T: DeserializeOwned + Send + 'static,
        F: Fn(Result<T, ConfigError>) + Send + 'static,
    {
        let running = Arc::new(AtomicBool::new(true));
        let running_clone = running.clone();

        let component = component.to_string();
        let filename = format!("{component}.toml");

        let thread = std::thread::Builder::new()
            .name(format!("cfg-watch-{component}"))
            .spawn(move || {
                run_watcher::<T, F>(
                    &filename,
                    defaults_path.as_deref(),
                    user_path.as_deref(),
                    &callback,
                    &running_clone,
                );
            })
            .expect("failed to spawn config watcher thread");

        Self {
            running,
            _thread: Some(thread),
        }
    }

    /// Stop the watcher. Also happens automatically on drop.
    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
    }
}

impl Drop for ConfigWatcher {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Internal: run the notify watcher loop on the current thread.
fn run_watcher<T, F>(
    filename: &str,
    defaults_path: Option<&Path>,
    user_path: Option<&Path>,
    callback: &F,
    running: &AtomicBool,
) where
    T: DeserializeOwned,
    F: Fn(Result<T, ConfigError>),
{
    let filename_owned = filename.to_string();
    let (tx, rx) = std::sync::mpsc::channel();

    // notify's recommended_watcher sends events through the channel.
    let mut watcher: RecommendedWatcher = match notify::recommended_watcher(
        move |res: Result<Event, notify::Error>| {
            if let Ok(event) = res {
                let _ = tx.send(event);
            }
        },
    ) {
        Ok(w) => w,
        Err(e) => {
            tracing_or_eprintln(&format!("config watcher init failed: {e}"));
            return;
        }
    };

    // Watch parent directories (not the files themselves) to catch
    // atomic rename writes from editors.
    let mut watched_dirs: Vec<PathBuf> = Vec::new();

    for path in [defaults_path, user_path].into_iter().flatten() {
        if let Some(parent) = path.parent() {
            if parent.exists() {
                if watcher
                    .watch(parent, RecursiveMode::NonRecursive)
                    .is_ok()
                {
                    watched_dirs.push(parent.to_path_buf());
                }
            }
        }
    }

    if watched_dirs.is_empty() {
        tracing_or_eprintln("config watcher: no directories to watch");
        return;
    }

    let poll = Duration::from_millis(POLL_MS);
    let mut debounce = Debounce::new(Duration::from_millis(DEBOUNCE_MS));

    while running.load(Ordering::SeqCst) {
        // Wake when the open burst settles, but never block past the poll
        // interval, so the stop flag is still honoured while nothing changes.
        let timeout = debounce
            .wait(Instant::now())
            .map_or(poll, |until_settled| until_settled.min(poll));

        match rx.recv_timeout(timeout) {
            Ok(event) => {
                if touches(&event, &filename_owned, &watched_dirs) {
                    debounce.record(Instant::now());
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                // Either the burst has settled or nothing is happening; both
                // are decided below.
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                break;
            }
        }

        if debounce.take_settled(Instant::now()) {
            // Read the file as it stands now, which after a burst is its final
            // state rather than whatever it held when the burst opened.
            let result: Result<T, ConfigError> = crate::load_from(defaults_path, user_path);
            callback(result);
        }
    }
}

/// Whether an event concerns the config file this watcher was asked about.
///
/// The watch is on the parent directory (to catch write-temp-then-rename), so
/// the raw stream carries every sibling's writes too.
fn touches(event: &Event, filename: &str, watched_dirs: &[PathBuf]) -> bool {
    let by_name = event
        .paths
        .iter()
        .any(|p| p.file_name().map(|n| n == filename).unwrap_or(false));

    by_name
        || matches!(event.kind, EventKind::Create(_) | EventKind::Modify(_))
            && event
                .paths
                .iter()
                .any(|p| p.is_dir() && watched_dirs.iter().any(|d| d == p))
}

fn tracing_or_eprintln(msg: &str) {
    // If tracing is available, use it; otherwise fall back to stderr.
    eprintln!("[arlen-config] {msg}");
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use std::io::Write;
    use std::sync::{Arc, Mutex};

    #[derive(Debug, Clone, Deserialize, PartialEq)]
    struct TestCfg {
        #[serde(default)]
        value: i32,
    }

    /// Write with `act` until `cond` holds, or give up.
    ///
    /// The deadline is a give-up point, not a window an assertion depends on: a
    /// loaded machine only makes the loop go round more times. Every sleep in
    /// this file used to be the other kind, sized to be "usually enough", which
    /// is what put `test_debounce_rapid_changes` in the CI log.
    ///
    /// Writing is retried because a write that beats the watch being armed is
    /// never seen, and no amount of waiting afterwards recovers it. But each
    /// attempt then polls WITHOUT writing, and that gap is load-bearing: a
    /// trailing-edge debouncer fires when the writes stop, so a retry loop that
    /// re-writes every few milliseconds is a burst that never ends and never
    /// settles. The first cut did exactly that and all three end-to-end tests
    /// went red with the callback having fired zero times, which reads at a
    /// glance like a broken watcher rather than a starved one.
    fn until(mut act: impl FnMut(), mut cond: impl FnMut() -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            act();
            let quiet_until = Instant::now() + Duration::from_millis(500);
            while Instant::now() < quiet_until {
                if cond() {
                    return true;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        false
    }

    fn write_cfg(path: &Path, content: &str) {
        // Atomic write: write to temp then rename.
        let tmp = path.with_extension("tmp");
        let mut f = std::fs::File::create(&tmp).unwrap();
        f.write_all(content.as_bytes()).unwrap();
        f.sync_all().unwrap();
        std::fs::rename(&tmp, path).unwrap();
    }

    #[test]
    fn test_callback_on_change() {
        let dir = tempfile::TempDir::new().unwrap();
        let cfg_path = dir.path().join("test.toml");
        write_cfg(&cfg_path, "value = 1");

        let results: Arc<Mutex<Vec<Result<TestCfg, ConfigError>>>> =
            Arc::new(Mutex::new(Vec::new()));
        let results_clone = results.clone();

        let watcher = ConfigWatcher::watch::<TestCfg, _>(
            "test",
            None,
            Some(cfg_path.clone()),
            move |r| {
                results_clone.lock().unwrap().push(r);
            },
        );

        let arrived = until(
            || write_cfg(&cfg_path, "value = 42"),
            || {
                results
                    .lock()
                    .unwrap()
                    .last()
                    .and_then(|r| r.as_ref().ok())
                    .map(|c| c.value == 42)
                    .unwrap_or(false)
            },
        );

        watcher.stop();
        assert!(arrived, "the callback never saw the changed config");
    }

    // ---- the debouncer itself, on a clock the test writes down -------------
    //
    // These replace `test_debounce_rapid_changes`, which drove real writes
    // through inotify, slept, and counted callbacks. It asserted `count <= 2`,
    // which is a claim about how fast the machine is: on a loaded CI runner a
    // gap between two writes stretches past the window, a second callback
    // fires, and the test goes red without anything being wrong. It did, on
    // 13 Aug.
    //
    // Every time below is written as a fraction or multiple of the debouncer's
    // own window, so the assertions say "inside the window" and "past it"
    // rather than naming a number of milliseconds. Nothing sleeps, so load
    // cannot change the outcome.

    const W: Duration = Duration::from_millis(100);

    #[test]
    fn a_burst_collapses_into_one_reload() {
        let mut d = Debounce::new(W);
        let t0 = Instant::now();

        // Five events, each arriving before the window since the last ran out.
        for i in 0..5 {
            let at = t0 + (W / 4) * i;
            d.record(at);
            assert!(
                !d.take_settled(at),
                "the burst is still open at event {i}, nothing should reload yet"
            );
        }

        let last = t0 + (W / 4) * 4;
        assert!(
            !d.take_settled(last + W / 2),
            "still inside the window after the last event"
        );
        assert!(
            d.take_settled(last + W),
            "the window passed with no further event, so the burst settled"
        );
        assert!(
            !d.take_settled(last + W * 10),
            "a settled burst is taken once, not once per check"
        );
    }

    #[test]
    fn a_later_burst_settles_on_its_own() {
        let mut d = Debounce::new(W);
        let t0 = Instant::now();

        d.record(t0);
        assert!(d.take_settled(t0 + W));

        let second = t0 + W * 5;
        d.record(second);
        assert!(!d.take_settled(second), "the second burst has just opened");
        assert!(d.take_settled(second + W), "and settles like the first");
    }

    #[test]
    fn nothing_settles_without_an_event() {
        let mut d = Debounce::new(W);
        let t0 = Instant::now();
        assert!(d.wait(t0).is_none(), "no burst is open");
        assert!(!d.take_settled(t0 + W * 100));
    }

    #[test]
    fn the_wait_is_what_is_left_of_the_window() {
        let d_at = |elapsed: Duration| {
            let mut d = Debounce::new(W);
            let t0 = Instant::now();
            d.record(t0);
            d.wait(t0 + elapsed).unwrap()
        };
        assert_eq!(d_at(Duration::ZERO), W, "a fresh event waits the window out");
        assert_eq!(d_at(W / 2), W / 2, "half elapsed, half to go");
        assert_eq!(
            d_at(W * 3),
            Duration::ZERO,
            "an overdue burst waits no longer, rather than underflowing"
        );
    }

    #[test]
    fn a_rapid_burst_delivers_the_value_the_file_ended_on() {
        // The end-to-end half, and the one the old leading-edge debounce got
        // wrong: it fired on the FIRST write and reloaded then, so `value = 5`
        // was never read by anyone and the callback was left holding an early
        // value for good.
        //
        // THE BURST IS WRITTEN ONCE AND NEVER RETRIED, which is the whole
        // design of the test. The first version wrote the burst inside the
        // retry loop, and it passed against the leading-edge code as happily as
        // against this one: each retry re-fired the leading edge, and by then
        // the file already held the 5 the previous burst left behind, so it
        // loaded a 5 for the wrong reason. A test that cannot fail against the
        // defect it names is worse than no test, so the arming is done first
        // and separately - one distinct value, retried until the callback
        // reports it, which proves the watch is established - and only then
        // does the burst go out, once.
        //
        // The assertion is then convergence, not a count, so a slow machine
        // only makes the poll go round more times.
        let dir = tempfile::TempDir::new().unwrap();
        let cfg_path = dir.path().join("test.toml");
        write_cfg(&cfg_path, "value = 0");

        let last: Arc<Mutex<Option<i32>>> = Arc::new(Mutex::new(None));
        let last_clone = last.clone();

        let watcher = ConfigWatcher::watch::<TestCfg, _>(
            "test",
            None,
            Some(cfg_path.clone()),
            move |r: Result<TestCfg, ConfigError>| {
                if let Ok(cfg) = r {
                    *last_clone.lock().unwrap() = Some(cfg.value);
                }
            },
        );

        // Arm: retried, because a write that beats the watch is never seen.
        assert!(
            until(
                || write_cfg(&cfg_path, "value = 7"),
                || *last.lock().unwrap() == Some(7)
            ),
            "the watch never armed, so the burst below would prove nothing"
        );

        // The burst: written once, and nothing writes again.
        for i in 1..=5 {
            write_cfg(&cfg_path, &format!("value = {i}"));
        }

        let settled = {
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                if *last.lock().unwrap() == Some(5) {
                    break true;
                }
                if Instant::now() >= deadline {
                    break false;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        };

        watcher.stop();
        assert!(
            settled,
            "the burst ended on value = 5 and the callback never saw it; got {:?}",
            *last.lock().unwrap()
        );
    }

    #[test]
    fn test_invalid_config_survives() {
        let dir = tempfile::TempDir::new().unwrap();
        let cfg_path = dir.path().join("test.toml");
        write_cfg(&cfg_path, "value = 1");

        let results: Arc<Mutex<Vec<bool>>> = Arc::new(Mutex::new(Vec::new()));
        let results_clone = results.clone();

        let watcher = ConfigWatcher::watch::<TestCfg, _>(
            "test",
            None,
            Some(cfg_path.clone()),
            move |r: Result<TestCfg, ConfigError>| {
                results_clone.lock().unwrap().push(r.is_ok());
            },
        );

        let refused = until(
            || write_cfg(&cfg_path, "this is {{{{ invalid"),
            || results.lock().unwrap().iter().any(|ok| !ok),
        );
        assert!(refused, "invalid TOML never reached the callback as an Err");

        // The point of the test: the watcher is still alive afterwards.
        let recovered = until(
            || write_cfg(&cfg_path, "value = 99"),
            || *results.lock().unwrap().last().unwrap_or(&false),
        );

        watcher.stop();
        assert!(
            recovered,
            "the watcher stopped delivering after one invalid file"
        );
    }

    #[test]
    fn test_stop_is_clean() {
        let dir = tempfile::TempDir::new().unwrap();
        let cfg_path = dir.path().join("test.toml");
        write_cfg(&cfg_path, "value = 1");

        let watcher = ConfigWatcher::watch::<TestCfg, _>(
            "test",
            None,
            Some(cfg_path),
            |_| {},
        );

        std::thread::sleep(Duration::from_millis(100));
        watcher.stop();
        std::thread::sleep(Duration::from_millis(200));
        // No panic, no hang -- clean shutdown.
    }
}

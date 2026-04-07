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

    let mut last_fire = Instant::now() - Duration::from_secs(10);

    while running.load(Ordering::SeqCst) {
        // Block with timeout so we can check the running flag periodically.
        match rx.recv_timeout(Duration::from_millis(500)) {
            Ok(event) => {
                // Filter: only care about events touching our filename.
                let dominated = event.paths.iter().any(|p| {
                    p.file_name()
                        .map(|n| n == filename_owned.as_str())
                        .unwrap_or(false)
                });
                let dominated = dominated
                    || matches!(
                        event.kind,
                        EventKind::Create(_) | EventKind::Modify(_)
                    ) && event.paths.iter().any(|p| {
                        // Also match parent dir events (inotify on dir).
                        p.is_dir()
                            && watched_dirs.iter().any(|d| d == p)
                    });

                if !dominated {
                    continue;
                }

                // Debounce: skip if too soon after last fire.
                let now = Instant::now();
                if now.duration_since(last_fire) < Duration::from_millis(DEBOUNCE_MS) {
                    continue;
                }
                last_fire = now;

                // Reload and invoke callback.
                let result: Result<T, ConfigError> =
                    crate::load_from(defaults_path, user_path);
                callback(result);
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                // Check running flag and loop.
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                break;
            }
        }
    }
}

fn tracing_or_eprintln(msg: &str) {
    // If tracing is available, use it; otherwise fall back to stderr.
    eprintln!("[lunaris-config] {msg}");
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

        // Wait for watcher to start.
        std::thread::sleep(Duration::from_millis(200));

        // Change the config.
        write_cfg(&cfg_path, "value = 42");
        std::thread::sleep(Duration::from_millis(300));

        watcher.stop();
        std::thread::sleep(Duration::from_millis(100));

        let results = results.lock().unwrap();
        assert!(!results.is_empty(), "callback should have been called");
        let last = results.last().unwrap().as_ref().unwrap();
        assert_eq!(last.value, 42);
    }

    #[test]
    fn test_debounce_rapid_changes() {
        let dir = tempfile::TempDir::new().unwrap();
        let cfg_path = dir.path().join("test.toml");
        write_cfg(&cfg_path, "value = 0");

        let call_count = Arc::new(Mutex::new(0usize));
        let count_clone = call_count.clone();

        let watcher = ConfigWatcher::watch::<TestCfg, _>(
            "test",
            None,
            Some(cfg_path.clone()),
            move |_r| {
                *count_clone.lock().unwrap() += 1;
            },
        );

        std::thread::sleep(Duration::from_millis(200));

        // Rapid-fire 5 changes within 50ms.
        for i in 1..=5 {
            write_cfg(&cfg_path, &format!("value = {i}"));
            std::thread::sleep(Duration::from_millis(10));
        }

        // Wait for debounce window + processing.
        std::thread::sleep(Duration::from_millis(400));

        watcher.stop();
        std::thread::sleep(Duration::from_millis(100));

        let count = *call_count.lock().unwrap();
        // Debounce should collapse 5 rapid writes into 1-2 callbacks.
        assert!(
            count <= 2,
            "expected at most 2 callbacks (debounce), got {count}"
        );
        assert!(count >= 1, "expected at least 1 callback, got {count}");
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

        std::thread::sleep(Duration::from_millis(200));

        // Write invalid TOML.
        write_cfg(&cfg_path, "this is {{{{ invalid");
        std::thread::sleep(Duration::from_millis(300));

        // Write valid TOML again -- watcher should still be alive.
        write_cfg(&cfg_path, "value = 99");
        std::thread::sleep(Duration::from_millis(300));

        watcher.stop();
        std::thread::sleep(Duration::from_millis(100));

        let results = results.lock().unwrap();
        // Should have at least 2 callbacks: one Err (invalid), one Ok (valid).
        assert!(
            results.len() >= 2,
            "expected at least 2 callbacks, got {}",
            results.len()
        );
        // First should be Err (invalid TOML), last should be Ok.
        assert!(!results[0], "first callback should be Err");
        assert!(*results.last().unwrap(), "last callback should be Ok");
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

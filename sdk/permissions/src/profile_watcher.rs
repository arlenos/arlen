//! File-watcher for `~/.config/permissions/` that detects profile
//! changes and surfaces them as `(app_id, exists)` events.
//!
//! Producers (this watcher + `installd` post-write) fire change
//! events; consumers (brokers + knowledge daemon) react via
//! `ConnectionAuth::refresh_profile` or token-cache invalidation.
//! See `docs/architecture/peer-auth-system.md` "In-flight
//! revocation" section.
//!
//! Design points the brokers depend on:
//!
//! * Watches the parent directory (`~/.config/permissions/`) with
//!   `RecursiveMode::NonRecursive` so editor atomic-rename writes
//!   (tmp + rename) deliver as Modify on the renamed target.
//! * Filters to `*.toml` files; debounces bursts so a scripted
//!   bulk-revoke (`sed -i` across N files) fires N callbacks
//!   spaced by the debounce window rather than 10×N raw notify
//!   events.
//! * Reports `(app_id, exists)` derived from the filename stem
//!   and a single post-debounce `metadata()` check. Brokers use
//!   `exists=false` to mean revoke-all and drop all connections
//!   from that `app_id`.
//!
//! Robustness contract (Codex adversarial review high-1 fix):
//! revocation is security-sensitive, so a missed inotify event
//! must not strand stale grants. The watcher therefore
//!
//! * Maintains a known-app set keyed by `app_id`. Each emit
//!   (incremental or resync) updates the set.
//! * **Resyncs on any notify backend error** — queue overflow,
//!   watch invalidation, dir-temporarily-gone all schedule a
//!   directory rescan that diffs against the known set and
//!   emits one `ProfileChange` per add / removal.
//! * **Periodic safety rescan every 60 s** as belt-and-suspenders
//!   — catches scenarios where notify never errored but lost
//!   events anyway (rare; observed on overcommitted inotify
//!   instances).
//! * **Initial scan on start** populates the known set and emits
//!   one event per pre-existing profile, so brokers that start
//!   after profiles already exist don't operate on a blank slate.

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, Mutex,
};
use std::thread;
use std::time::{Duration, Instant};

/// A profile-change event reported to consumers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileChange {
    /// `app_id` derived from the filename stem (e.g. `com.example`
    /// from `com.example.toml`).
    pub app_id: String,
    /// Whether the file currently exists on disk after the change.
    /// `false` for a delete (revoke-all).
    pub exists: bool,
}

/// Periodic safety rescan cadence. Defense-in-depth against
/// inotify event loss that the error-callback path didn't catch.
const PERIODIC_RESCAN_INTERVAL: Duration = Duration::from_secs(60);

/// Shared state between the notify callback (raw-event ingest),
/// the worker thread (debounce drain + periodic rescan), and the
/// public `force_rescan` method.
struct State {
    /// `app_id` → last raw-event timestamp. Worker drains entries
    /// idle longer than `debounce` and emits a ProfileChange.
    pending: HashMap<String, Instant>,
    /// Set of `app_id`s the watcher believes currently exist on
    /// disk. Used during resync: any known app whose file is now
    /// missing emits `exists=false`; any disk file not yet known
    /// emits `exists=true`.
    known: HashSet<String>,
    /// Signals the worker that a full rescan is due. Set by the
    /// notify error callback and by `force_rescan`.
    resync_due: bool,
    /// Last time a periodic safety rescan completed.
    last_periodic_rescan: Instant,
}

/// Watches the user permission profile directory and emits one
/// `ProfileChange` per app_id per debounce window. Callback runs on
/// an internal worker thread.
///
/// Drop the returned guard to stop watching — both the raw notify
/// watcher and the debounce worker exit within ~50 ms of drop.
pub struct ProfileWatcher {
    _watcher: RecommendedWatcher,
    state: Arc<Mutex<State>>,
    alive: Arc<AtomicU64>,
    worker: Option<thread::JoinHandle<()>>,
}

impl Drop for ProfileWatcher {
    fn drop(&mut self) {
        self.alive.store(0, Ordering::Release);
        if let Some(handle) = self.worker.take() {
            let _ = handle.join();
        }
    }
}

impl ProfileWatcher {
    /// Default canonical path per AUTH-CANONICAL.md §2:
    /// `~/.config/permissions/`. Honours `LUNARIS_PERMISSIONS_DIR`
    /// for tests (same env var `installd` uses).
    pub fn permissions_dir() -> PathBuf {
        if let Ok(p) = std::env::var("LUNARIS_PERMISSIONS_DIR") {
            return PathBuf::from(p);
        }
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join(".config")
            .join("permissions")
    }

    /// Start watching the default permissions dir.
    pub fn start<F>(on_change: F) -> Result<Self, notify::Error>
    where
        F: Fn(ProfileChange) + Send + Sync + 'static,
    {
        Self::start_at(Self::permissions_dir(), Duration::from_millis(150), on_change)
    }

    /// Start watching an explicit directory with a custom debounce
    /// window. Used by tests and by sites that point at a non-
    /// default profile root.
    ///
    /// The directory must exist (the parent dir creation is the
    /// caller's job — installd creates it on first write; brokers
    /// create it on startup if absent so the watcher can attach).
    pub fn start_at<F>(
        dir: PathBuf,
        debounce: Duration,
        on_change: F,
    ) -> Result<Self, notify::Error>
    where
        F: Fn(ProfileChange) + Send + Sync + 'static,
    {
        let state = Arc::new(Mutex::new(State {
            pending: HashMap::new(),
            known: HashSet::new(),
            resync_due: true, // Initial scan picks up existing files.
            last_periodic_rescan: Instant::now(),
        }));
        let on_change: Arc<dyn Fn(ProfileChange) + Send + Sync + 'static> =
            Arc::new(on_change);

        let alive = Arc::new(AtomicU64::new(1));
        let alive_for_worker = Arc::clone(&alive);
        let state_for_notify = Arc::clone(&state);

        let mut watcher = notify::recommended_watcher(
            move |res: Result<notify::Event, notify::Error>| {
                match res {
                    Ok(event) => {
                        use notify::EventKind;
                        if !matches!(
                            event.kind,
                            EventKind::Create(_)
                                | EventKind::Modify(_)
                                | EventKind::Remove(_)
                        ) {
                            return;
                        }
                        let now = Instant::now();
                        let mut guard = state_for_notify.lock().unwrap();
                        for p in &event.paths {
                            if p.extension() != Some(OsStr::new("toml")) {
                                continue;
                            }
                            if let Some(stem) =
                                p.file_stem().and_then(|s| s.to_str())
                            {
                                guard.pending.insert(stem.to_string(), now);
                            }
                        }
                    }
                    Err(e) => {
                        // Codex adversarial-review high-1: notify
                        // errors (queue overflow, watch invalidation,
                        // dir-temporarily-gone) must not silently
                        // drop revocation events. Schedule a full
                        // rescan; the worker thread picks it up
                        // within ~50 ms and emits a ProfileChange
                        // per add/remove diff against the known set.
                        let mut guard = state_for_notify.lock().unwrap();
                        guard.resync_due = true;
                        eprintln!(
                            "profile_watcher: notify backend error → rescan scheduled: {e}"
                        );
                    }
                }
            },
        )?;

        watcher.watch(&dir, RecursiveMode::NonRecursive)?;

        let dir_for_worker = dir.clone();
        let state_for_worker = Arc::clone(&state);
        let on_change_for_worker = Arc::clone(&on_change);

        // Worker thread:
        //   1. Drain debounced pending entries (idle > debounce).
        //   2. If `resync_due` was flipped (notify error, force_rescan,
        //      or initial start), run a full dir scan and emit diffs.
        //   3. Every PERIODIC_RESCAN_INTERVAL, run a safety rescan
        //      anyway — belt + suspenders against silent loss.
        // 50 ms tick so the worker exits within 50 ms of drop.
        let worker = thread::spawn(move || {
            while alive_for_worker.load(Ordering::Acquire) != 0 {
                thread::sleep(Duration::from_millis(50));

                // 1. Drain debounced pending.
                let due: Vec<String> = {
                    let mut guard = state_for_worker.lock().unwrap();
                    let now = Instant::now();
                    let due_keys: Vec<String> = guard
                        .pending
                        .iter()
                        .filter_map(|(k, t)| {
                            (now.duration_since(*t) >= debounce).then(|| k.clone())
                        })
                        .collect();
                    for k in &due_keys {
                        guard.pending.remove(k);
                    }
                    due_keys
                };
                for app_id in due {
                    let exists = dir_for_worker
                        .join(format!("{app_id}.toml"))
                        .exists();
                    {
                        let mut guard = state_for_worker.lock().unwrap();
                        if exists {
                            guard.known.insert(app_id.clone());
                        } else {
                            guard.known.remove(&app_id);
                        }
                    }
                    on_change_for_worker(ProfileChange { app_id, exists });
                }

                // 2. Rescan if flagged.
                let needs_resync = {
                    let mut guard = state_for_worker.lock().unwrap();
                    let due = guard.resync_due;
                    guard.resync_due = false;
                    due
                };
                if needs_resync {
                    run_rescan(
                        &dir_for_worker,
                        &state_for_worker,
                        &on_change_for_worker,
                    );
                    let mut guard = state_for_worker.lock().unwrap();
                    guard.last_periodic_rescan = Instant::now();
                    continue;
                }

                // 3. Periodic safety rescan.
                let should_periodic = {
                    let guard = state_for_worker.lock().unwrap();
                    guard.last_periodic_rescan.elapsed() >= PERIODIC_RESCAN_INTERVAL
                };
                if should_periodic {
                    run_rescan(
                        &dir_for_worker,
                        &state_for_worker,
                        &on_change_for_worker,
                    );
                    let mut guard = state_for_worker.lock().unwrap();
                    guard.last_periodic_rescan = Instant::now();
                }
            }
        });

        Ok(ProfileWatcher {
            _watcher: watcher,
            state,
            alive,
            worker: Some(worker),
        })
    }

    /// Trigger a directory rescan on the next worker tick. Emits
    /// `ProfileChange` for adds (files not in the known set) and
    /// removes (known files now absent). Idempotent — calling
    /// repeatedly does not amplify emissions because the known set
    /// is updated atomically with each emit.
    ///
    /// Public so test code, recovery flows, and admin tooling can
    /// trigger a manual sync; the watcher itself calls it via the
    /// `resync_due` flag whenever the notify backend reports an
    /// error.
    pub fn force_rescan(&self) {
        self.state.lock().unwrap().resync_due = true;
    }

    /// Replace the in-memory known-app set. **Test-only** —
    /// simulates the "notify lost an event" scenario so the
    /// recovery path can be tested deterministically without
    /// having to corrupt the inotify subsystem.
    #[cfg(test)]
    fn test_replace_known(&self, apps: HashSet<String>) {
        self.state.lock().unwrap().known = apps;
    }
}

/// Diff the current directory contents against the known set and
/// emit a `ProfileChange` per delta. Called by the worker on
/// `resync_due` or on the periodic timer. Conservative: emits no
/// events for files unchanged since the last rescan (known stays
/// known, present stays present).
fn run_rescan(
    dir: &PathBuf,
    state: &Arc<Mutex<State>>,
    on_change: &Arc<dyn Fn(ProfileChange) + Send + Sync>,
) {
    let mut current: HashSet<String> = HashSet::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension() != Some(OsStr::new("toml")) {
                continue;
            }
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                current.insert(stem.to_string());
            }
        }
    }

    let (added, removed): (Vec<String>, Vec<String>) = {
        let guard = state.lock().unwrap();
        let added: Vec<String> = current.difference(&guard.known).cloned().collect();
        let removed: Vec<String> =
            guard.known.difference(&current).cloned().collect();
        (added, removed)
    };

    // Update known set before emitting so a re-entrant callback
    // (consumer triggers another force_rescan synchronously) sees
    // consistent state.
    {
        let mut guard = state.lock().unwrap();
        for k in &added {
            guard.known.insert(k.clone());
        }
        for k in &removed {
            guard.known.remove(k);
        }
    }

    for app_id in added {
        on_change(ProfileChange {
            app_id,
            exists: true,
        });
    }
    for app_id in removed {
        on_change(ProfileChange {
            app_id,
            exists: false,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;
    use std::sync::mpsc;
    use tempfile::tempdir;

    fn write_profile(dir: &Path, app: &str, body: &str) {
        let path = dir.join(format!("{app}.toml"));
        fs::write(path, body).unwrap();
    }

    fn delete_profile(dir: &Path, app: &str) {
        let _ = fs::remove_file(dir.join(format!("{app}.toml")));
    }

    fn await_change(rx: &mpsc::Receiver<ProfileChange>) -> Option<ProfileChange> {
        rx.recv_timeout(Duration::from_secs(2)).ok()
    }

    fn drain(rx: &mpsc::Receiver<ProfileChange>) {
        while rx.recv_timeout(Duration::from_millis(200)).is_ok() {}
    }

    #[test]
    fn create_fires_with_exists_true() {
        let dir = tempdir().unwrap();
        let (tx, rx) = mpsc::channel();
        let _w = ProfileWatcher::start_at(
            dir.path().to_path_buf(),
            Duration::from_millis(80),
            move |c| {
                let _ = tx.send(c);
            },
        )
        .unwrap();
        drain(&rx);
        write_profile(dir.path(), "com.example", "[graph]\n");
        let change = await_change(&rx).expect("create event");
        assert_eq!(change.app_id, "com.example");
        assert!(change.exists);
    }

    #[test]
    fn delete_fires_with_exists_false() {
        let dir = tempdir().unwrap();
        write_profile(dir.path(), "com.delete", "[graph]\n");
        let (tx, rx) = mpsc::channel();
        let _w = ProfileWatcher::start_at(
            dir.path().to_path_buf(),
            Duration::from_millis(80),
            move |c| {
                let _ = tx.send(c);
            },
        )
        .unwrap();
        drain(&rx);
        delete_profile(dir.path(), "com.delete");
        let change = await_change(&rx).expect("delete event");
        assert_eq!(change.app_id, "com.delete");
        assert!(!change.exists);
    }

    #[test]
    fn debounce_collapses_burst() {
        let dir = tempdir().unwrap();
        let (tx, rx) = mpsc::channel();
        let _w = ProfileWatcher::start_at(
            dir.path().to_path_buf(),
            Duration::from_millis(120),
            move |c| {
                let _ = tx.send(c);
            },
        )
        .unwrap();
        drain(&rx);
        for _ in 0..5 {
            write_profile(dir.path(), "com.burst", "[graph]\n");
            thread::sleep(Duration::from_millis(15));
        }
        let first = await_change(&rx).expect("at least one event");
        assert_eq!(first.app_id, "com.burst");
        assert!(rx.recv_timeout(Duration::from_millis(180)).is_err());
    }

    #[test]
    fn ignores_non_toml_files() {
        let dir = tempdir().unwrap();
        let (tx, rx) = mpsc::channel();
        let _w = ProfileWatcher::start_at(
            dir.path().to_path_buf(),
            Duration::from_millis(80),
            move |c| {
                let _ = tx.send(c);
            },
        )
        .unwrap();
        drain(&rx);
        fs::write(dir.path().join("readme.txt"), "not a profile").unwrap();
        assert!(rx.recv_timeout(Duration::from_millis(200)).is_err());
    }

    /// Initial scan on start populates the known set and emits one
    /// `ProfileChange { exists: true }` per pre-existing profile.
    /// Protects the boot path: a shell that starts after some apps
    /// are already installed still gets revocation hooks armed
    /// against the right `app_id`s.
    #[test]
    fn initial_scan_emits_existing_profiles() {
        let dir = tempdir().unwrap();
        write_profile(dir.path(), "com.preexisting", "[graph]\n");
        let (tx, rx) = mpsc::channel();
        let _w = ProfileWatcher::start_at(
            dir.path().to_path_buf(),
            Duration::from_millis(80),
            move |c| {
                let _ = tx.send(c);
            },
        )
        .unwrap();
        let change = await_change(&rx).expect("initial scan emit");
        assert_eq!(change.app_id, "com.preexisting");
        assert!(change.exists);
    }

    /// Codex adversarial-review high-1: simulate "notify never told
    /// us this file was deleted" by leaving a stale entry in the
    /// known set, then triggering `force_rescan`. The watcher must
    /// emit `exists=false` so brokers drop the stale grant.
    #[test]
    fn rescan_emits_removal_for_stale_known_entry() {
        let dir = tempdir().unwrap();
        let (tx, rx) = mpsc::channel();
        let w = ProfileWatcher::start_at(
            dir.path().to_path_buf(),
            Duration::from_millis(80),
            move |c| {
                let _ = tx.send(c);
            },
        )
        .unwrap();
        drain(&rx);

        let mut stale = HashSet::new();
        stale.insert("com.stale.grant".to_string());
        w.test_replace_known(stale);

        w.force_rescan();
        let change = await_change(&rx).expect("rescan should emit removal");
        assert_eq!(change.app_id, "com.stale.grant");
        assert!(!change.exists);
    }

    /// Companion to the removal test: simulate "notify never told us
    /// this file was created" by clearing the known set while the
    /// file is on disk, then forcing a rescan.
    #[test]
    fn rescan_emits_addition_for_unseen_disk_file() {
        let dir = tempdir().unwrap();
        write_profile(dir.path(), "com.unseen", "[graph]\n");
        let (tx, rx) = mpsc::channel();
        let w = ProfileWatcher::start_at(
            dir.path().to_path_buf(),
            Duration::from_millis(80),
            move |c| {
                let _ = tx.send(c);
            },
        )
        .unwrap();
        drain(&rx);

        w.test_replace_known(HashSet::new());

        w.force_rescan();
        let change = await_change(&rx).expect("rescan should emit addition");
        assert_eq!(change.app_id, "com.unseen");
        assert!(change.exists);
    }

    /// Repeated force_rescan calls while disk and known are in sync
    /// must not amplify emissions.
    #[test]
    fn force_rescan_is_idempotent_when_state_matches_disk() {
        let dir = tempdir().unwrap();
        write_profile(dir.path(), "com.stable", "[graph]\n");
        let (tx, rx) = mpsc::channel();
        let w = ProfileWatcher::start_at(
            dir.path().to_path_buf(),
            Duration::from_millis(80),
            move |c| {
                let _ = tx.send(c);
            },
        )
        .unwrap();
        let initial = await_change(&rx).expect("initial");
        assert_eq!(initial.app_id, "com.stable");

        w.force_rescan();
        w.force_rescan();
        w.force_rescan();
        assert!(rx.recv_timeout(Duration::from_millis(400)).is_err());
    }
}

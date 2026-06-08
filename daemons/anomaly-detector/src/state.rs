//! Persisted detector state.
//!
//! The baseline (rate EWMA, known node-types), the read high-water
//! mark, the last-user-activity time, per-alert cooldowns, and the
//! suppression list survive a restart in a small JSON file under the
//! per-user data dir. A lost or corrupt file is non-fatal: the
//! detector restarts in a learning state and re-learns the baseline.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::detect::{Alert, DetectorConfig, NoveltyDetector, RateDetector};

/// Minimum spacing between two behavioural alerts of the same key, so
/// a sustained anomaly does not spam the user.
const ALERT_COOLDOWN_MICROS: i64 = 15 * 60 * 1_000_000; // 15 min

/// Critical alerts (audit tampering) surface promptly but are still
/// deduped over a short window so a flapping daemon cannot spam.
const CRITICAL_DEDUP_MICROS: i64 = 60 * 1_000_000; // 1 min

/// The detector's durable state.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct State {
    /// Highest audit index processed; the next poll starts here.
    pub hwm_index: u64,
    /// Rolling AI-query-rate baseline + spike detector.
    pub rate: RateDetector,
    /// Known graph node-types + novelty detector.
    pub novelty: NoveltyDetector,
    /// Last `window.*` (user-activity proxy) time seen this run, or
    /// carried across a restart.
    pub last_user_activity_micros: Option<i64>,
    /// Per-alert-key last-raised time, for cooldown.
    pub alert_cooldowns: HashMap<String, i64>,
    /// Alert keys the user marked as noise; dropped before dispatch.
    pub suppressed: HashSet<String>,
    /// Set once the initial catch-up to the audit read head has
    /// completed. While false the detector learns the existing log as
    /// baseline **without** dispatching alerts, so a fresh install
    /// does not fire on historical activity. Persisted, so a normal
    /// restart (which loads a warmed, bootstrapped state) alerts on
    /// the small since-last-run backlog, while a *first run that
    /// crashed mid-catch-up* (this flag still false, but with a
    /// non-zero HWM) correctly resumes learning silently until the
    /// first full catch-up finally completes.
    ///
    /// `serde(default)` is safe here precisely because this field is
    /// part of the schema from the daemon's first release: no state
    /// file lacking it can exist, so a missing value only ever comes
    /// from a fresh-default (genuine first run). A "treat any non-fresh
    /// state as bootstrapped" heuristic was deliberately rejected — it
    /// would break the mid-catch-up resume case above by forcing live
    /// alerts on the not-yet-learned historical tail.
    #[serde(default)]
    pub bootstrapped: bool,
}

impl State {
    /// Load the state file, or a fresh default if it is absent or
    /// unreadable/corrupt (logged). Never fails: the detector is
    /// advisory and must come up regardless.
    pub fn load(path: &Path) -> Self {
        match std::fs::read(path) {
            Ok(bytes) => match serde_json::from_slice(&bytes) {
                Ok(state) => state,
                Err(e) => {
                    tracing::warn!(
                        "anomaly state at {} is unreadable ({e}); starting fresh",
                        path.display()
                    );
                    State::default()
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => State::default(),
            Err(e) => {
                tracing::warn!(
                    "cannot read anomaly state {} ({e}); starting fresh",
                    path.display()
                );
                State::default()
            }
        }
    }

    /// Persist atomically: write a temp file, fsync, rename over the
    /// target, fsync the directory. Mode 0600.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;

        let dir = path.parent().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "state path has no parent")
        })?;
        let tmp = path.with_extension("json.tmp");
        let body = serde_json::to_vec_pretty(self)?;
        {
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&tmp)?;
            f.write_all(&body)?;
            f.sync_all()?;
        }
        std::fs::rename(&tmp, path)?;
        if let Ok(dir_file) = std::fs::File::open(dir) {
            let _ = dir_file.sync_all();
        }
        Ok(())
    }

    /// Whether `alert` should be dispatched now: not suppressed, and
    /// outside its cooldown window.
    pub fn should_raise(&self, alert: &Alert, now_micros: i64) -> bool {
        if self.suppressed.contains(&alert.key) {
            return false;
        }
        let cooldown = if alert.critical {
            CRITICAL_DEDUP_MICROS
        } else {
            ALERT_COOLDOWN_MICROS
        };
        match self.alert_cooldowns.get(&alert.key) {
            Some(&last) => now_micros - last >= cooldown,
            None => true,
        }
    }

    /// Record that `alert` was raised at `now_micros` (starts its
    /// cooldown).
    pub fn record_raised(&mut self, alert: &Alert, now_micros: i64) {
        self.alert_cooldowns.insert(alert.key.clone(), now_micros);
    }

    /// Note user activity at `micros` (from a `window.*` event).
    pub fn note_user_activity(&mut self, micros: i64) {
        let newer = match self.last_user_activity_micros {
            Some(prev) => micros.max(prev),
            None => micros,
        };
        self.last_user_activity_micros = Some(newer);
    }

    /// Drop cooldown entries whose window has long passed, so the map
    /// does not grow without bound over a long uptime.
    pub fn prune_cooldowns(&mut self, now_micros: i64, _cfg: &DetectorConfig) {
        let horizon = ALERT_COOLDOWN_MICROS;
        self.alert_cooldowns
            .retain(|_, &mut last| now_micros - last < horizon);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detect::Alert;

    fn tampered() -> Alert {
        Alert::audit_tampered("chain broken at 3")
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let mut s = State {
            hwm_index: 42,
            ..Default::default()
        };
        s.note_user_activity(1_000);
        s.record_raised(&tampered(), 5_000);
        s.save(&path).unwrap();
        let back = State::load(&path);
        assert_eq!(back.hwm_index, 42);
        assert_eq!(back.last_user_activity_micros, Some(1_000));
        assert!(back.alert_cooldowns.contains_key(tampered().key.as_str()));
    }

    #[test]
    fn missing_state_loads_default() {
        let dir = tempfile::tempdir().unwrap();
        let s = State::load(&dir.path().join("nope.json"));
        assert_eq!(s.hwm_index, 0);
    }

    #[test]
    fn corrupt_state_loads_default_not_panics() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        std::fs::write(&path, b"{ not json").unwrap();
        assert_eq!(State::load(&path).hwm_index, 0);
    }

    #[test]
    fn cooldown_suppresses_repeat_then_allows_after_window() {
        let mut s = State::default();
        let a = tampered();
        let t0 = 1_000_000_000;
        assert!(s.should_raise(&a, t0), "first raise allowed");
        s.record_raised(&a, t0);
        // Within the critical dedup window → suppressed.
        assert!(!s.should_raise(&a, t0 + CRITICAL_DEDUP_MICROS - 1));
        // After it → allowed again.
        assert!(s.should_raise(&a, t0 + CRITICAL_DEDUP_MICROS));
    }

    #[test]
    fn suppressed_key_is_never_raised() {
        let mut s = State::default();
        let a = tampered();
        s.suppressed.insert(a.key.clone());
        assert!(!s.should_raise(&a, 0));
    }

    #[test]
    fn prune_drops_only_expired_cooldowns() {
        let mut s = State::default();
        s.alert_cooldowns.insert("old".into(), 0);
        s.alert_cooldowns.insert("recent".into(), 100 * 60 * 1_000_000);
        s.prune_cooldowns(100 * 60 * 1_000_000, &DetectorConfig::default());
        assert!(!s.alert_cooldowns.contains_key("old"));
        assert!(s.alert_cooldowns.contains_key("recent"));
    }
}

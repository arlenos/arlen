//! The detection heuristics.
//!
//! Every detector is a pure function over injected state, the audit
//! entry (or timestamp), and config — no clocks, no sockets — so the
//! acceptance behaviour ("a sudden 10× query rate raises an alert",
//! "novel namespace access raises an alert") is unit-tested
//! deterministically. The orchestrator in [`crate::source`] wires
//! real time and the read API in around these.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

/// Kinds of anomaly the detector reports. The `key` on an [`Alert`]
/// disambiguates instances (e.g. which node-type) for cooldown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlertKind {
    /// AI query rate spiked far above the learned baseline.
    RateSpike,
    /// A graph node-type never seen before was accessed.
    NovelNodeType,
    /// An AI action ran with no recent user interaction.
    NoUserInteraction,
    /// The audit log's own integrity check failed.
    AuditTampered,
    /// A rate-limit violation reported by the graph/AI layer (S15).
    RateLimit,
}

impl AlertKind {
    /// Stable string used in the persisted cooldown map.
    pub fn as_str(self) -> &'static str {
        match self {
            AlertKind::RateSpike => "rate-spike",
            AlertKind::NovelNodeType => "novel-node-type",
            AlertKind::NoUserInteraction => "no-user-interaction",
            AlertKind::AuditTampered => "audit-tampered",
            AlertKind::RateLimit => "rate-limit",
        }
    }
}

/// One raised anomaly: what to show the user and how to key it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Alert {
    /// The kind of anomaly.
    pub kind: AlertKind,
    /// Cooldown / suppression key: `kind` plus any instance detail
    /// (e.g. the node-type), so distinct instances alert independently
    /// but a repeating one is rate-limited.
    pub key: String,
    /// Notification summary line.
    pub summary: String,
    /// Notification body.
    pub body: String,
    /// Whether this is a critical-urgency alert.
    pub critical: bool,
}

impl Alert {
    fn rate_spike(count: u64, baseline: f64) -> Self {
        Alert {
            kind: AlertKind::RateSpike,
            key: AlertKind::RateSpike.as_str().to_string(),
            summary: "Unusual AI query rate".to_string(),
            body: format!(
                "The AI layer issued {count} queries in one window, far above \
                 the usual rate (~{baseline:.1}). If you did not start a large \
                 AI task, this may indicate a compromised component."
            ),
            critical: false,
        }
    }

    fn novel_node_type(node_type: &str) -> Self {
        Alert {
            kind: AlertKind::NovelNodeType,
            key: format!("{}:{node_type}", AlertKind::NovelNodeType.as_str()),
            summary: "AI accessed a new kind of data".to_string(),
            body: format!(
                "The AI layer accessed graph data of type '{node_type}' for the \
                 first time. Review this in Settings if it was unexpected."
            ),
            critical: false,
        }
    }

    fn no_user_interaction() -> Self {
        Alert {
            kind: AlertKind::NoUserInteraction,
            key: AlertKind::NoUserInteraction.as_str().to_string(),
            summary: "AI acted with no recent activity".to_string(),
            body: "The AI layer took an action while there was no recent user \
                   interaction on this machine."
                .to_string(),
            critical: false,
        }
    }

    /// A critical alert that the audit log itself was tampered with.
    pub fn audit_tampered(detail: &str) -> Self {
        Alert {
            kind: AlertKind::AuditTampered,
            key: AlertKind::AuditTampered.as_str().to_string(),
            summary: "Audit log integrity failure".to_string(),
            body: format!(
                "The system audit log failed its integrity check ({detail}). \
                 The record of recent activity may be incomplete or altered."
            ),
            critical: true,
        }
    }
}

/// Tunable detector parameters.
#[derive(Debug, Clone)]
pub struct DetectorConfig {
    /// Width of a rate window, seconds.
    pub window_secs: i64,
    /// Completed windows observed before the rate baseline is trusted
    /// enough to judge spikes.
    pub warmup_windows: u64,
    /// A window alerts only at or above this multiple of the baseline.
    pub spike_factor: f64,
    /// ...and only at or above this absolute count, so a 0.1→1 "10×"
    /// at trivial volume is not flagged.
    pub min_events_for_spike: u64,
    /// EWMA smoothing for the rate baseline (0..1, higher = faster).
    pub ewma_alpha: f64,
    /// Entries learned before novel node-types are alerted (cold-start
    /// learning so a fresh install does not alert on first use).
    pub warmup_entries: u64,
    /// An AI action this long after the last user interaction (micros)
    /// with no activity since is flagged.
    pub no_interaction_threshold_micros: i64,
}

impl Default for DetectorConfig {
    fn default() -> Self {
        Self {
            window_secs: 60,
            warmup_windows: 5,
            spike_factor: 10.0,
            min_events_for_spike: 20,
            ewma_alpha: 0.3,
            warmup_entries: 50,
            no_interaction_threshold_micros: 5 * 60 * 1_000_000, // 5 min
        }
    }
}

impl DetectorConfig {
    fn window_micros(&self) -> i64 {
        self.window_secs.max(1) * 1_000_000
    }
}

/// Largest run of empty windows folded into the baseline on a time
/// gap (e.g. the machine slept), so a long gap is bounded work.
const MAX_FOLD: i64 = 60;

/// Rolling AI-query-rate baseline + spike detector.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RateDetector {
    /// Window index (`timestamp / window`) currently accumulating.
    window_start: Option<i64>,
    /// Queries counted in the current window.
    count: u64,
    /// EWMA of completed-window counts — the baseline.
    pub ewma: f64,
    /// Completed windows folded into the baseline so far.
    pub windows_seen: u64,
}

impl RateDetector {
    fn window_index(ts_micros: i64, cfg: &DetectorConfig) -> i64 {
        ts_micros.div_euclid(cfg.window_micros())
    }

    /// Count one AI query observed at `ts_micros`. Returns an alert if
    /// advancing into this query's window finalised an earlier window
    /// that spiked.
    pub fn observe_query(&mut self, ts_micros: i64, cfg: &DetectorConfig) -> Option<Alert> {
        let w = Self::window_index(ts_micros, cfg);
        let alert = self.advance_to(w, cfg);
        self.count += 1;
        alert
    }

    /// Finalise the current window if wall-clock `now` has moved past
    /// it (called periodically so a trailing window is judged even
    /// with no following query).
    pub fn tick(&mut self, now_micros: i64, cfg: &DetectorConfig) -> Option<Alert> {
        let w = Self::window_index(now_micros, cfg);
        self.advance_to(w, cfg)
    }

    /// Advance the active window to `w`, finalising every window from
    /// the current one up to (but not including) `w`.
    fn advance_to(&mut self, w: i64, cfg: &DetectorConfig) -> Option<Alert> {
        let cur = match self.window_start {
            None => {
                self.window_start = Some(w);
                return None;
            }
            Some(cur) => cur,
        };
        if w <= cur {
            // Same window, or an out-of-order timestamp: nothing to
            // finalise. (Out-of-order events still count into the
            // current window via the caller.)
            return None;
        }
        // Finalise the current window with its accumulated count.
        let alert = self.finalize(self.count, cfg);
        self.count = 0;
        // Fold empty intervening windows so a quiet stretch lowers the
        // baseline, bounded by MAX_FOLD against a large time jump.
        let gap = (w - cur - 1).min(MAX_FOLD);
        for _ in 0..gap {
            let _ = self.finalize(0, cfg);
        }
        self.window_start = Some(w);
        alert
    }

    /// Judge a finished window against the current baseline, then fold
    /// it in. Judging happens before the fold, so a window is compared
    /// to the baseline that preceded it.
    fn finalize(&mut self, count: u64, cfg: &DetectorConfig) -> Option<Alert> {
        let alert = if self.windows_seen >= cfg.warmup_windows
            && count >= cfg.min_events_for_spike
            && (count as f64) >= cfg.spike_factor * self.ewma.max(1.0)
        {
            Some(Alert::rate_spike(count, self.ewma))
        } else {
            None
        };
        self.ewma = if self.windows_seen == 0 {
            count as f64
        } else {
            cfg.ewma_alpha * count as f64 + (1.0 - cfg.ewma_alpha) * self.ewma
        };
        self.windows_seen += 1;
        alert
    }
}

/// Set of graph node-types ever seen + novelty detector.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NoveltyDetector {
    /// Node-types (and relations) seen so far.
    pub known: HashSet<String>,
    /// Entries observed — gates the cold-start learning period.
    pub entries_seen: u64,
}

impl NoveltyDetector {
    /// Observe the labels (node-types / relations) of one audit entry.
    /// During the warm-up it only learns; afterwards a never-seen
    /// label raises an alert (once, then it is learned).
    ///
    /// Only **label-bearing** observations count toward the warm-up:
    /// an entry with no labels teaches nothing, so it must not consume
    /// the learning budget (else, e.g., many label-less AI queries
    /// would exhaust the warm-up and the first real graph access would
    /// be flagged as novel rather than learned as baseline).
    pub fn observe(&mut self, labels: &[String], cfg: &DetectorConfig) -> Vec<Alert> {
        if labels.is_empty() {
            return Vec::new();
        }
        let warmed = self.entries_seen >= cfg.warmup_entries;
        let mut alerts = Vec::new();
        for label in labels {
            if !self.known.contains(label) {
                if warmed {
                    alerts.push(Alert::novel_node_type(label));
                }
                self.known.insert(label.clone());
            }
        }
        self.entries_seen += 1;
        alerts
    }
}

/// Decide whether an AI action at `action_micros` ran without recent
/// user interaction. `last_activity_micros` is the last `window.*`
/// event time (None if none seen this run); `grace_until_micros` is a
/// startup grace window during which we never alert (we may not have
/// observed activity yet).
pub fn check_no_user_interaction(
    action_micros: i64,
    last_activity_micros: Option<i64>,
    grace_until_micros: i64,
    cfg: &DetectorConfig,
) -> Option<Alert> {
    if action_micros < grace_until_micros {
        return None;
    }
    match last_activity_micros {
        None => Some(Alert::no_user_interaction()),
        Some(t) if action_micros - t > cfg.no_interaction_threshold_micros => {
            Some(Alert::no_user_interaction())
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> DetectorConfig {
        // Small warmups for deterministic tests.
        DetectorConfig {
            warmup_windows: 3,
            warmup_entries: 2,
            ..DetectorConfig::default()
        }
    }

    /// Feed `n` queries into window `w` (spread within it).
    fn fill_window(d: &mut RateDetector, w: i64, n: u64, c: &DetectorConfig) -> Vec<Alert> {
        let base = w * c.window_micros();
        let mut alerts = Vec::new();
        for i in 0..n {
            if let Some(a) = d.observe_query(base + i as i64, c) {
                alerts.push(a);
            }
        }
        alerts
    }

    #[test]
    fn steady_rate_does_not_alert() {
        let c = cfg();
        let mut d = RateDetector::default();
        let mut alerts = Vec::new();
        for w in 0..10 {
            alerts.extend(fill_window(&mut d, w, 5, &c));
        }
        // Finalise the last window.
        if let Some(a) = d.tick(11 * c.window_micros(), &c) {
            alerts.push(a);
        }
        assert!(alerts.is_empty(), "steady rate must not alert: {alerts:?}");
    }

    #[test]
    fn sudden_ten_x_query_rate_raises_an_alert() {
        let c = cfg();
        let mut d = RateDetector::default();
        // Warm up: 5 windows of 5 queries → baseline ~5.
        for w in 0..5 {
            assert!(fill_window(&mut d, w, 5, &c).is_empty());
        }
        // A window with 50 queries (10× the baseline of ~5).
        assert!(fill_window(&mut d, 5, 50, &c).is_empty(), "alert fires on finalise");
        // The spike window finalises when the next window opens.
        let alert = fill_window(&mut d, 6, 1, &c);
        assert_eq!(alert.len(), 1, "the 10× window must raise exactly one alert");
        assert_eq!(alert[0].kind, AlertKind::RateSpike);
    }

    #[test]
    fn spike_below_absolute_floor_does_not_alert() {
        // 10× a tiny baseline but below MIN_EVENTS_FOR_SPIKE must not
        // fire — avoids noise at trivial volumes.
        let c = cfg();
        let mut d = RateDetector::default();
        for w in 0..5 {
            assert!(fill_window(&mut d, w, 1, &c).is_empty());
        }
        assert!(fill_window(&mut d, 5, 10, &c).is_empty()); // 10× baseline 1, but < 20
        let alert = fill_window(&mut d, 6, 1, &c);
        assert!(alert.is_empty(), "below the absolute floor must not alert: {alert:?}");
    }

    #[test]
    fn rate_detector_does_not_alert_during_warmup() {
        let c = cfg();
        let mut d = RateDetector::default();
        // A huge first window, but we are still in warm-up.
        assert!(fill_window(&mut d, 0, 100, &c).is_empty());
        let alert = fill_window(&mut d, 1, 1, &c);
        assert!(alert.is_empty(), "no baseline yet → no alert during warm-up");
    }

    #[test]
    fn novel_node_type_alerts_only_after_warmup() {
        let c = cfg(); // warmup_entries = 2
        let mut d = NoveltyDetector::default();
        assert!(d.observe(&["File".into()], &c).is_empty(), "learning");
        assert!(d.observe(&["App".into()], &c).is_empty(), "learning");
        // Past warm-up: a never-seen type alerts.
        let alerts = d.observe(&["SecretVault".into()], &c);
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].kind, AlertKind::NovelNodeType);
        assert!(alerts[0].key.contains("SecretVault"));
        // Seen now → no repeat alert.
        assert!(d.observe(&["SecretVault".into()], &c).is_empty());
        // A known type from warm-up never alerts.
        assert!(d.observe(&["File".into()], &c).is_empty());
    }

    #[test]
    fn no_user_interaction_respects_grace_and_threshold() {
        let c = cfg();
        let grace = 1_000_000; // actions before t=1s are in grace
        // In grace → never alerts.
        assert!(check_no_user_interaction(500_000, None, grace, &c).is_none());
        // Past grace, recent activity → no alert.
        let now = 100 * 60 * 1_000_000;
        assert!(check_no_user_interaction(now, Some(now - 1_000_000), grace, &c).is_none());
        // Past grace, stale activity beyond threshold → alert.
        let stale = now - c.no_interaction_threshold_micros - 1;
        let a = check_no_user_interaction(now, Some(stale), grace, &c);
        assert_eq!(a.map(|x| x.kind), Some(AlertKind::NoUserInteraction));
        // Past grace, no activity ever → alert.
        assert!(check_no_user_interaction(now, None, grace, &c).is_some());
    }

    #[test]
    fn a_long_time_gap_is_bounded_and_lowers_the_baseline() {
        let c = cfg();
        let mut d = RateDetector::default();
        for w in 0..5 {
            let _ = fill_window(&mut d, w, 30, &c);
        }
        let before = d.ewma;
        // Jump far ahead; the fold must be bounded (no pathological
        // loop) and pull the baseline down toward zero.
        let _ = d.tick(100_000 * c.window_micros(), &c);
        assert!(d.ewma < before, "empty windows lower the baseline");
    }
}

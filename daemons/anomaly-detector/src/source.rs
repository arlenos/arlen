//! Orchestration: the read-API poll loop and the Event Bus consumer.
//!
//! The reliable path is the by-index poll of the audit read API; the
//! Event Bus (`audit.*` triggers, `window.*` activity) only lowers
//! latency and feeds the no-interaction signal. Two seams keep this
//! testable without a real bus: [`AuditSource`] (the read side, mocked
//! in tests) and the `notify` flag (skips the D-Bus send so tests
//! assert on state). The per-entry analysis ([`Detector::process_entry`])
//! is pure w.r.t. I/O.

use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;
use audit_proto::{AuditKind, ReadClient, ReadClientError, ReadPage, StructuralView};
use os_sdk::event_consumer::{EventConsumer, UnixEventConsumer};
use tokio::time::MissedTickBehavior;

use crate::detect::{check_no_user_interaction, Alert, DetectorConfig};
use crate::notify::Notifier;
use crate::state::State;

/// How often the poll loop drains new audit entries (also triggered
/// immediately by an `audit.ai.*` event).
const POLL_INTERVAL_SECS: u64 = 30;

/// Page size for read-API queries (the daemon clamps to its ceiling).
const PAGE: u64 = 1000;

/// Backoff between Event Bus subscribe attempts.
const SUBSCRIBE_RETRY: Duration = Duration::from_secs(5);

/// The read side, abstracted so tests can script pages and errors.
#[async_trait]
pub trait AuditSource: Send + Sync {
    /// Read one page of Structural views plus the daemon tamper flag.
    async fn read_page(
        &self,
        from: u64,
        to: u64,
        limit: u64,
        project_id: Option<&str>,
    ) -> Result<ReadPage, ReadClientError>;
}

#[async_trait]
impl AuditSource for ReadClient {
    async fn read_page(
        &self,
        from: u64,
        to: u64,
        limit: u64,
        project_id: Option<&str>,
    ) -> Result<ReadPage, ReadClientError> {
        self.read(from, to, limit, project_id).await
    }
}

/// The running detector.
pub struct Detector {
    state: State,
    cfg: DetectorConfig,
    source: Box<dyn AuditSource>,
    notifier: Option<Notifier>,
    /// When false, alerts are logged + recorded but not sent over
    /// D-Bus (tests, or a log-only mode).
    notify: bool,
    state_path: PathBuf,
    grace_until_micros: i64,
}

impl Detector {
    /// Build a detector. `grace_until_micros` is the deadline before
    /// which the no-interaction check never fires.
    pub fn new(
        state: State,
        cfg: DetectorConfig,
        source: Box<dyn AuditSource>,
        state_path: PathBuf,
        grace_until_micros: i64,
        notify: bool,
    ) -> Self {
        Self {
            state,
            cfg,
            source,
            notifier: None,
            notify,
            state_path,
            grace_until_micros,
        }
    }

    /// Feed one audit entry through the detectors, returning any
    /// alerts. Pure w.r.t. I/O: it only mutates the detector state.
    pub fn process_entry(&mut self, e: &StructuralView) -> Vec<Alert> {
        let mut alerts = Vec::new();

        if e.kind == AuditKind::Query {
            if let Some(a) = self.state.rate.observe_query(e.timestamp_micros, &self.cfg) {
                alerts.push(a);
            }
        }

        // Novel node-type / relation access.
        let mut labels = e.structural.node_types.clone();
        labels.extend(e.structural.relations.iter().cloned());
        alerts.extend(self.state.novelty.observe(&labels, &self.cfg));

        // AI action with no recent user interaction. `grace_until`
        // (= startup + grace) excludes both the initial window AND all
        // backlog entries (their timestamps predate startup), so this
        // signal is judged only for *live* entries where the observed
        // `last_user_activity` is meaningful. Backlog anomalies are
        // still caught by the rate and novelty detectors.
        if matches!(e.kind, AuditKind::Query | AuditKind::ToolCall) {
            if let Some(a) = check_no_user_interaction(
                e.timestamp_micros,
                self.state.last_user_activity_micros,
                self.grace_until_micros,
                &self.cfg,
            ) {
                alerts.push(a);
            }
        }

        alerts
    }

    /// Drain all audit entries past the high-water mark, analysing
    /// each, then — only if the catch-up fully succeeded — finalise
    /// the trailing rate window and persist.
    ///
    /// Failure handling matters for correctness: on a read error the
    /// loop breaks *without* finalising a rate window, so the EWMA
    /// timeline never advances past entries that have not been read
    /// yet (those are processed in timestamp order on the next
    /// successful poll). While the initial catch-up is still in
    /// progress (`!bootstrapped`) the detectors learn but no alert is
    /// dispatched, so a fresh install does not fire on historical log
    /// activity.
    pub async fn poll(&mut self) {
        let mut caught_up = false;
        let mut tampered_seen = false;

        loop {
            match self
                .source
                .read_page(self.state.hwm_index, u64::MAX, PAGE, None)
                .await
            {
                Ok(page) => {
                    tampered_seen |= page.tampered;
                    let n = page.entries.len() as u64;
                    let drained = page.entries.is_empty() || n < PAGE;
                    for entry in &page.entries {
                        let alerts = self.process_entry(entry);
                        if self.state.bootstrapped {
                            for alert in alerts {
                                let now = crate::now_micros();
                                self.maybe_dispatch(&alert, now).await;
                            }
                        }
                        self.state.hwm_index = entry.index + 1;
                    }
                    if drained {
                        caught_up = true;
                        break;
                    }
                }
                Err(e) => {
                    // Advisory: log and retry next cycle. Do NOT tick
                    // the rate window — the timeline must not move past
                    // entries we have not read.
                    tracing::warn!("audit read poll failed: {e}");
                    break;
                }
            }
        }

        if caught_up {
            let now = crate::now_micros();
            let trailing = self.state.rate.tick(now, &self.cfg);
            if self.state.bootstrapped {
                if let Some(alert) = trailing {
                    self.maybe_dispatch(&alert, now).await;
                }
            } else {
                // First full catch-up complete: the baseline is
                // learned from the existing log, alerts now go live.
                self.state.bootstrapped = true;
                tracing::info!("initial baseline learned; anomaly alerts now live");
            }
        }

        if tampered_seen {
            // Critical, and observed over the reliable poll path, so it
            // survives a missed Event Bus event. Never gated by
            // bootstrap.
            let now = crate::now_micros();
            let alert = Alert::audit_tampered("audit read API reports the ledger tampered");
            self.maybe_dispatch(&alert, now).await;
        }

        let now = crate::now_micros();
        self.state.prune_cooldowns(now, &self.cfg);
        if let Err(e) = self.state.save(&self.state_path) {
            tracing::warn!("anomaly state save failed: {e}");
        }
    }

    /// Dispatch an alert if it is neither suppressed nor in cooldown.
    /// Always logged; the D-Bus notification is best-effort and is
    /// skipped entirely when `notify` is false.
    async fn maybe_dispatch(&mut self, alert: &Alert, now_micros: i64) {
        if !self.state.should_raise(alert, now_micros) {
            return;
        }
        tracing::warn!(
            kind = alert.kind.as_str(),
            key = %alert.key,
            "anomaly alert: {}",
            alert.summary
        );
        if self.notify {
            if self.notifier.is_none() {
                match Notifier::connect().await {
                    Ok(n) => self.notifier = Some(n),
                    Err(e) => tracing::warn!("notification connect failed: {e}"),
                }
            }
            if let Some(n) = &self.notifier {
                if let Err(e) = n.dispatch(alert).await {
                    tracing::warn!("notification dispatch failed: {e}");
                }
            }
        }
        // Record the cooldown regardless: the alert was logged, and
        // this prevents attempt-spam if the notifier is unavailable.
        self.state.record_raised(alert, now_micros);
    }

    /// Handle one Event Bus event.
    async fn handle_event(&mut self, ev_type: &str, payload: &[u8]) {
        if ev_type.starts_with("window.") {
            // A proxy for recent human interaction.
            self.state.note_user_activity(crate::now_micros());
        } else if ev_type == "audit.tampered" {
            let detail = serde_json::from_slice::<serde_json::Value>(payload)
                .ok()
                .and_then(|v| v.get("detail").and_then(|d| d.as_str()).map(String::from))
                .unwrap_or_else(|| "integrity check failed".to_string());
            let alert = Alert::audit_tampered(&detail);
            self.maybe_dispatch(&alert, crate::now_micros()).await;
        } else if ev_type.starts_with("audit.ai.") {
            // A committed audit append: drain it (and anything missed)
            // from the read API immediately.
            self.poll().await;
        }
        // Future: a graph/AI rate-limit-violation event (S15) is
        // handled here; until S15 emits it there is nothing to do.
    }

    /// Run forever: subscribe to `audit.*` + `window.*`, poll on a
    /// timer and on `audit.ai.*` events. Re-subscribes if the feed
    /// closes.
    pub async fn run(mut self, consumer: UnixEventConsumer) {
        let mut interval = tokio::time::interval(Duration::from_secs(POLL_INTERVAL_SECS));
        interval.set_missed_tick_behavior(MissedTickBehavior::Delay);

        // Catch up on anything that accumulated while we were down.
        self.poll().await;

        loop {
            let mut rx = match consumer
                .subscribe(vec!["audit.".to_string(), "window.".to_string()])
                .await
            {
                Ok(rx) => rx,
                Err(e) => {
                    tracing::warn!(
                        "event bus subscribe failed: {e}; retrying in {}s",
                        SUBSCRIBE_RETRY.as_secs()
                    );
                    tokio::time::sleep(SUBSCRIBE_RETRY).await;
                    continue;
                }
            };
            tracing::info!("anomaly detector: subscribed to audit.* + window.*");
            loop {
                tokio::select! {
                    _ = interval.tick() => self.poll().await,
                    event = rx.recv() => match event {
                        Some(event) => self.handle_event(&event.r#type, &event.payload).await,
                        None => {
                            tracing::warn!("event feed closed; re-subscribing");
                            break;
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detect::AlertKind;
    use audit_proto::StructuralRecord;
    use std::collections::VecDeque;
    use tokio::sync::Mutex;

    /// A scripted read source: each `read_page` pops the next result.
    /// An exhausted script yields an empty, untampered page (caught up).
    struct MockSource {
        pages: Mutex<VecDeque<Result<ReadPage, ReadClientError>>>,
    }

    impl MockSource {
        fn new(script: Vec<Result<ReadPage, ReadClientError>>) -> Self {
            Self {
                pages: Mutex::new(script.into()),
            }
        }
    }

    #[async_trait]
    impl AuditSource for MockSource {
        async fn read_page(
            &self,
            _from: u64,
            _to: u64,
            _limit: u64,
            _project: Option<&str>,
        ) -> Result<ReadPage, ReadClientError> {
            self.pages.lock().await.pop_front().unwrap_or(Ok(ReadPage {
                entries: vec![],
                tampered: false,
            }))
        }
    }

    fn entry(index: u64, kind: AuditKind, ts: i64, node_types: &[&str]) -> StructuralView {
        StructuralView {
            index,
            timestamp_micros: ts,
            kind,
            actor: "ai-daemon".into(),
            structural: StructuralRecord {
                subject: "ai.query".into(),
                node_types: node_types.iter().map(|s| s.to_string()).collect(),
                relations: vec![],
                result_count: None,
                duration_ms: None,
                outcome: "completed".into(),
                depth: None,
            },
            call_chain_id: None,
            project_id: None,
            entry_hash_hex: "00".into(),
        }
    }

    fn page(entries: Vec<StructuralView>, tampered: bool) -> Result<ReadPage, ReadClientError> {
        Ok(ReadPage { entries, tampered })
    }

    fn detector(state: State, cfg: DetectorConfig, src: MockSource, grace_until: i64) -> Detector {
        Detector::new(
            state,
            cfg,
            Box::new(src),
            std::env::temp_dir().join(format!("anomaly-test-{}.json", std::process::id())),
            grace_until,
            false, // never touch the bus in tests
        )
    }

    fn novel_key(t: &str) -> String {
        format!("{}:{t}", AlertKind::NovelNodeType.as_str())
    }

    #[test]
    fn process_entry_flags_a_novel_node_type_after_warmup() {
        let cfg = DetectorConfig {
            warmup_entries: 1,
            ..DetectorConfig::default()
        };
        // grace_until = MAX silences the no-interaction check so this
        // test isolates the novelty detector.
        let mut d = detector(State::default(), cfg, MockSource::new(vec![]), i64::MAX);
        assert!(d.process_entry(&entry(0, AuditKind::Query, 1000, &["File"])).is_empty());
        let alerts = d.process_entry(&entry(1, AuditKind::Query, 2000, &["SecretVault"]));
        assert!(
            alerts.iter().any(|a| a.kind == AlertKind::NovelNodeType),
            "novel node-type must alert: {alerts:?}"
        );
    }

    #[test]
    fn process_entry_flags_action_without_user_interaction() {
        let cfg = DetectorConfig {
            warmup_entries: 1000,
            ..DetectorConfig::default()
        };
        let mut d = detector(State::default(), cfg, MockSource::new(vec![]), 0);
        let now = 1_000 * 60 * 1_000_000;
        let alerts = d.process_entry(&entry(0, AuditKind::Query, now, &[]));
        assert!(alerts.iter().any(|a| a.kind == AlertKind::NoUserInteraction));
    }

    #[tokio::test]
    async fn bootstrap_learns_silently_then_alerts_go_live() {
        let cfg = DetectorConfig {
            warmup_entries: 1,
            ..DetectorConfig::default()
        };
        // Poll #1 (bootstrap): learns "File" without alerting.
        let src = MockSource::new(vec![
            page(vec![entry(0, AuditKind::Query, 1000, &["File"])], false),
            // Poll #2 (live): a novel type → must alert.
            page(vec![entry(1, AuditKind::Query, 2000, &["SecretVault"])], false),
        ]);
        let mut d = detector(State::default(), cfg, src, i64::MAX);

        d.poll().await; // bootstrap
        assert!(d.state.bootstrapped, "first catch-up sets bootstrapped");
        assert!(
            d.state.alert_cooldowns.is_empty(),
            "bootstrap must not dispatch alerts: {:?}",
            d.state.alert_cooldowns
        );
        assert_eq!(d.state.hwm_index, 1);

        d.poll().await; // live
        assert!(
            d.state.alert_cooldowns.contains_key(&novel_key("SecretVault")),
            "a novel type after bootstrap must alert: {:?}",
            d.state.alert_cooldowns
        );
        assert_eq!(d.state.hwm_index, 2);
    }

    #[tokio::test]
    async fn tamper_flag_on_a_poll_raises_a_critical_alert() {
        let mut state = State::default();
        state.bootstrapped = true; // past bootstrap
        let src = MockSource::new(vec![page(vec![], true)]);
        let mut d = detector(state, DetectorConfig::default(), src, 0);
        d.poll().await;
        assert!(
            d.state.alert_cooldowns.contains_key(AlertKind::AuditTampered.as_str()),
            "a tampered read page must raise the critical alert"
        );
    }

    #[tokio::test]
    async fn an_interrupted_bootstrap_resumes_learning_silently() {
        // A first run that crashed mid-catch-up: bootstrapped is still
        // false but the HWM has advanced. The restart must keep
        // learning the historical tail silently (no alerts) until the
        // first full catch-up completes — NOT treat the tail as live.
        let cfg = DetectorConfig {
            warmup_entries: 1,
            ..DetectorConfig::default()
        };
        let mut state = State::default();
        state.hwm_index = 2; // mid-bootstrap progress, bootstrapped == false
        // The unprocessed historical tail includes a never-seen type.
        let src = MockSource::new(vec![page(
            vec![entry(2, AuditKind::Query, 3000, &["HistoricalType"])],
            false,
        )]);
        let mut d = detector(state, cfg, src, i64::MAX);

        d.poll().await;
        assert!(d.state.bootstrapped, "catch-up completes → bootstrapped");
        assert!(
            d.state.alert_cooldowns.is_empty(),
            "the historical tail must be learned silently, not alerted: {:?}",
            d.state.alert_cooldowns
        );
        assert!(
            d.state.novelty.known.contains("HistoricalType"),
            "the tail's type is learned as baseline"
        );
    }

    #[tokio::test]
    async fn a_read_error_does_not_advance_or_crash() {
        let mut state = State::default();
        state.bootstrapped = true;
        state.hwm_index = 7;
        let src = MockSource::new(vec![Err(ReadClientError::Transport("down".into()))]);
        let mut d = detector(state, DetectorConfig::default(), src, 0);
        d.poll().await; // must not panic
        assert_eq!(d.state.hwm_index, 7, "a failed read must not advance the HWM");
        assert!(d.state.alert_cooldowns.is_empty(), "no alerts on a read failure");
    }
}

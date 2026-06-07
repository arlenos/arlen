//! Persisted recent-alert log for the agent observability surface.
//!
//! The detector dispatches alerts as desktop notifications, which are
//! ephemeral. To let the harness "Notices" view show what fired recently, the
//! daemon also appends each dispatched alert to a small bounded log on disk
//! (newest first) that the harness reads. The daemon is the only writer; the
//! harness reads it read-only. The log is advisory, so a read or write error
//! is non-fatal and never stalls detection.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::detect::Alert;

/// How many recent alerts to keep; older ones are dropped.
const MAX_ALERTS: usize = 50;

/// One dispatched alert, in display form.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecentAlert {
    /// Stable alert-kind string (`AlertKind::as_str`).
    pub kind: String,
    /// Summary line.
    pub summary: String,
    /// Body text.
    pub body: String,
    /// Whether it was a critical-urgency alert.
    pub critical: bool,
    /// When it was dispatched, microseconds since the Unix epoch.
    pub ts_micros: i64,
}

/// A bounded, newest-first log of recently dispatched alerts.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct AlertLog {
    /// Newest first.
    pub alerts: Vec<RecentAlert>,
}

impl AlertLog {
    /// Load the log from `path`. A missing or unreadable file yields an empty
    /// log: the surface is advisory and a read error must not stall the daemon.
    pub fn load(path: &Path) -> Self {
        match std::fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    /// Record a dispatched alert at the front, capping the log length.
    pub fn record(&mut self, alert: &Alert, ts_micros: i64) {
        self.alerts.insert(
            0,
            RecentAlert {
                kind: alert.kind.as_str().to_string(),
                summary: alert.summary.clone(),
                body: alert.body.clone(),
                critical: alert.critical,
                ts_micros,
            },
        );
        self.alerts.truncate(MAX_ALERTS);
    }

    /// Persist the log atomically (temp file + rename, mode 0600), mirroring
    /// `State::save`.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;

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
        std::fs::rename(&tmp, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detect::AlertKind;

    fn alert(summary: &str) -> Alert {
        Alert {
            kind: AlertKind::RateSpike,
            key: "k".to_string(),
            summary: summary.to_string(),
            body: "b".to_string(),
            critical: false,
        }
    }

    #[test]
    fn record_is_newest_first_and_capped() {
        let mut log = AlertLog::default();
        for i in 0..(MAX_ALERTS + 5) {
            log.record(&alert(&format!("a{i}")), i as i64);
        }
        assert_eq!(log.alerts.len(), MAX_ALERTS, "capped at MAX_ALERTS");
        // The most recent record is at the front, with the kind's stable string.
        assert_eq!(log.alerts[0].summary, format!("a{}", MAX_ALERTS + 4));
        assert_eq!(log.alerts[0].kind, AlertKind::RateSpike.as_str());
    }
}

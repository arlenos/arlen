/// Knowledge Graph daemon client for the Quick Settings KG tile.
///
/// Queries the read-only daemon socket for the last 7 days of event
/// timestamps and buckets them into per-day counts client-side. The
/// per-day buckets feed the tile's inline sparkline; the total of
/// today's bucket is shown as the headline number.
///
/// Failure modes (daemon down, parse error, timeout) all degrade to
/// `available = false` + zero counts so the tile renders an offline
/// state rather than crashing.

use serde::Serialize;

use crate::projects::graph_query_async;

/// Eight days of buckets — `today + last 7` so the sparkline always
/// has a leading "today" datapoint.
const BUCKET_COUNT: usize = 8;

/// One UTC day in milliseconds.
const DAY_MS: i64 = 86_400_000;

/// Daily bucket of event counts.
#[derive(Debug, Clone, Serialize)]
pub struct KnowledgeBucket {
    /// UTC day index (timestamp / DAY_MS), absolute. Useful for the
    /// frontend to label tooltip / x-axis if it wants.
    pub day: i64,
    /// Event count in this bucket.
    pub count: u32,
}

/// Response payload — buckets ordered oldest → newest, plus a
/// daemon-availability flag so the frontend can render an offline
/// state without inferring it from "all zeros".
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeStats {
    /// True if the daemon answered the query (regardless of count).
    pub available: bool,
    /// Per-day counts, oldest → newest, length = `BUCKET_COUNT`.
    pub buckets: Vec<KnowledgeBucket>,
    /// Today's count, mirrored as a headline number.
    pub today: u32,
    /// Sum across all buckets — used for the "1.2k entries" label
    /// when no per-day breakdown is needed.
    pub total: u32,
}

fn empty_response() -> KnowledgeStats {
    let now_day = (now_ms() / DAY_MS) as i64;
    let buckets = (0..BUCKET_COUNT)
        .map(|i| KnowledgeBucket {
            day: now_day - (BUCKET_COUNT as i64 - 1 - i as i64),
            count: 0,
        })
        .collect();
    KnowledgeStats {
        available: false,
        buckets,
        today: 0,
        total: 0,
    }
}

fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Tauri command: returns the last 8 days of event counts.
///
/// Polled by the KG tile every few minutes — graph queries are not
/// real-time so a 5-minute cache is fine.
#[tauri::command]
pub async fn knowledge_daily_counts() -> KnowledgeStats {
    let now = now_ms();
    let cutoff = now - (BUCKET_COUNT as i64) * DAY_MS;

    // Fetch raw event timestamps for the last N days. Bucketing
    // client-side keeps the daemon-side query simple and avoids
    // depending on Ladybug-specific aggregate-function support.
    let cypher = format!(
        "MATCH (e:Event) WHERE e.timestamp >= {cutoff} RETURN e.timestamp"
    );

    let raw = match graph_query_async(cypher).await {
        Ok(r) => r,
        Err(e) => {
            log::debug!("knowledge_daily_counts: graph query failed: {e}");
            return empty_response();
        }
    };

    bucket_response(&raw, now)
}

/// Parse pipe-delimited timestamps and bucket them by UTC day.
/// Pulled out for testability — `bucket_response` is pure.
fn bucket_response(raw: &str, now: i64) -> KnowledgeStats {
    if raw.trim().is_empty() || raw.starts_with("ERROR") {
        return empty_response();
    }

    let now_day = now / DAY_MS;
    let oldest_day = now_day - (BUCKET_COUNT as i64 - 1);

    let mut counts = vec![0u32; BUCKET_COUNT];
    let mut lines = raw.lines();
    lines.next(); // Skip the column-header row.

    for line in lines {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let ts: i64 = match line.split('|').next().and_then(|s| s.trim().parse().ok()) {
            Some(v) => v,
            None => continue,
        };
        let day = ts / DAY_MS;
        if day < oldest_day || day > now_day {
            continue;
        }
        let idx = (day - oldest_day) as usize;
        if idx < counts.len() {
            counts[idx] = counts[idx].saturating_add(1);
        }
    }

    let total: u32 = counts.iter().sum();
    let today = *counts.last().unwrap_or(&0);

    let buckets = counts
        .into_iter()
        .enumerate()
        .map(|(i, count)| KnowledgeBucket {
            day: oldest_day + i as i64,
            count,
        })
        .collect();

    KnowledgeStats {
        available: true,
        buckets,
        today,
        total,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A typical daemon response with two events on day-0 and one
    /// event on day-1 should bucket cleanly. `now` is set to fall on
    /// day 100, so the rows targeting day 100 land in the last
    /// bucket and the day-99 row in the second-to-last.
    #[test]
    fn buckets_recent_events_into_correct_slots() {
        let now = 100 * DAY_MS + 1_234;
        let raw = format!(
            "ts\n{}\n{}\n{}\n",
            100 * DAY_MS + 100,
            100 * DAY_MS + 5_000,
            99 * DAY_MS + 1_000,
        );
        let r = bucket_response(&raw, now);
        assert!(r.available);
        assert_eq!(r.today, 2);
        assert_eq!(r.total, 3);
        assert_eq!(r.buckets.len(), BUCKET_COUNT);
        assert_eq!(r.buckets[BUCKET_COUNT - 1].count, 2);
        assert_eq!(r.buckets[BUCKET_COUNT - 2].count, 1);
    }

    /// Out-of-window rows must be ignored — we don't want a stale
    /// event to inflate today's count.
    #[test]
    fn drops_rows_outside_the_window() {
        let now = 100 * DAY_MS;
        let raw = format!(
            "ts\n{}\n{}\n",
            (100 - BUCKET_COUNT as i64 - 5) * DAY_MS, // far in the past
            100 * DAY_MS + 1,                          // today
        );
        let r = bucket_response(&raw, now);
        assert_eq!(r.total, 1);
        assert_eq!(r.today, 1);
    }

    #[test]
    fn empty_input_returns_zero_buckets() {
        let r = bucket_response("", 100 * DAY_MS);
        assert!(!r.available);
        assert_eq!(r.total, 0);
        assert!(r.buckets.iter().all(|b| b.count == 0));
    }

    #[test]
    fn error_marker_returns_offline_state() {
        let r = bucket_response("ERROR: socket closed", 100 * DAY_MS);
        assert!(!r.available);
        assert_eq!(r.buckets.len(), BUCKET_COUNT);
    }

    #[test]
    fn malformed_rows_are_skipped() {
        let now = 100 * DAY_MS;
        let raw = format!(
            "ts\nnot_a_number\n{}\n|missing\n",
            100 * DAY_MS + 50,
        );
        let r = bucket_response(&raw, now);
        assert_eq!(r.total, 1);
    }

    #[test]
    fn empty_response_has_correct_shape() {
        let r = empty_response();
        assert!(!r.available);
        assert_eq!(r.buckets.len(), BUCKET_COUNT);
        assert_eq!(r.today, 0);
        assert_eq!(r.total, 0);
    }
}

//! The canonical instant for the knowledge graph: INT64 microseconds since the
//! Unix epoch (bitemporal-knowledge-graph.md §3).
//!
//! Timestamp units were inconsistent across the codebase (the immutable event
//! log and retention use microseconds, the project store wrote milliseconds, the
//! entity layer RFC3339 strings). A bi-temporal model that compares mixed units
//! silently mis-orders history, so one unit is fixed everywhere a fact's time is
//! compared: microseconds, the unit the authoritative `events` log already uses,
//! so promotion derives valid-time with zero conversion on the hot path.
//! Conversion is done by source at each writer's known chokepoint, never by
//! guessing a value's magnitude (which would silently rescale small test and
//! fixture timestamps). Stamps stay INT64, never Kuzu's native TEMPORAL type,
//! because the typed-row reader cannot carry a temporal cell cleanly.

use chrono::{DateTime, TimeZone, Utc};

/// Microseconds since the Unix epoch: the single canonical time unit for graph
/// facts. A newtype so a raw millisecond or app-supplied value cannot be passed
/// where canonical micros are expected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Micros(pub i64);

/// The current instant in canonical microseconds.
pub fn now() -> Micros {
    Micros(Utc::now().timestamp_micros())
}

impl Micros {
    /// This instant as a `DateTime<Utc>`, or `None` for the `0` sentinel that
    /// marks an unset or absent timestamp in graph storage.
    pub fn to_dt(self) -> Option<DateTime<Utc>> {
        if self.0 == 0 {
            None
        } else {
            Utc.timestamp_micros(self.0).single()
        }
    }

    /// The canonical micros of a `DateTime<Utc>`.
    pub fn from_dt(dt: &DateTime<Utc>) -> Self {
        Micros(dt.timestamp_micros())
    }
}

/// Convert canonical micros to `Option<DateTime<Utc>>`, returning `None` for the
/// `0` unset sentinel (replaces the project store's old `millis_to_dt`).
pub fn micros_to_dt(micros: i64) -> Option<DateTime<Utc>> {
    Micros(micros).to_dt()
}

/// Convert a `DateTime<Utc>` to canonical micros for graph storage (replaces the
/// project store's old `dt_to_millis`).
pub fn dt_to_micros(dt: &DateTime<Utc>) -> i64 {
    Micros::from_dt(dt).0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn now_is_a_plausible_recent_micros_value() {
        // After 2020-01-01 in micros (1_577_836_800_000_000), so the unit is
        // microseconds, not millis or seconds.
        assert!(now().0 > 1_577_836_800_000_000);
    }

    #[test]
    fn dt_micros_round_trips() {
        let dt = Utc.timestamp_micros(1_700_000_000_123_456).single().unwrap();
        let micros = dt_to_micros(&dt);
        assert_eq!(micros, 1_700_000_000_123_456);
        assert_eq!(micros_to_dt(micros), Some(dt));
    }

    #[test]
    fn zero_is_the_unset_sentinel() {
        assert_eq!(micros_to_dt(0), None);
        assert_eq!(Micros(0).to_dt(), None);
    }

    #[test]
    fn micros_order_matches_time_order() {
        let earlier = Micros(1_700_000_000_000_000);
        let later = Micros(1_700_000_000_000_001);
        assert!(earlier < later, "plain integer compare orders instants");
    }
}

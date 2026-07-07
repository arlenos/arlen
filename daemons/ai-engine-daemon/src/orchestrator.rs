//! The autonomous-curator orchestrator (`pi-agent-adoption.md` §E), re-homed
//! from the native `ai-agent` into the engine daemon.
//!
//! The trigger spine is OURS: pi is request-driven, so the daemon subscribes the
//! event bus, coalesces a fire-storm, deterministically decides whether to act,
//! and drives either a daemon-direct deterministic curation or a bounded
//! ephemeral pi run. This module is built in increments; it starts with the
//! COALESCER, the piece §E flags as "doubly critical with pi" - never one pi-run
//! spawn per storm event.
//!
//! The coalescer is re-homed verbatim from the old `ai-agent` engine loop (gap
//! G1). It stays a pure, clock-injected, bounded structure so the whole dispatch
//! decision is testable without the event bus or a pi process.

use std::collections::{BTreeMap, HashMap};
use std::time::{Duration, SystemTime};

/// Default per-behaviour coalescing window. A burst of identical events for one
/// behaviour within this window fires it once. Short by design: long enough to
/// collapse a "x100 in a second" storm, short enough not to suppress a
/// deliberate re-trigger seconds later.
pub const DEFAULT_COALESCE_WINDOW: Duration = Duration::from_secs(1);

/// Hard cap on the coalescer's tracking map, so a storm of DISTINCT events (many
/// unique paths in one window, e.g. a build or a `find`) cannot grow it without
/// bound. At the cap, stale entries are pruned; if it is still full of fresh
/// distinct events the map is cleared (coalescing forgets recent entries, never
/// dropping a distinct dispatch).
pub const MAX_COALESCE_ENTRIES: usize = 4096;

/// Per-behaviour burst coalescer. Collapses a storm of identical
/// `(behaviour, event_type, fields, external_content)` events into one dispatch
/// per [`window`](Self::new), so an autonomous behaviour fires once per burst
/// rather than once per event. Doubly critical under pi: a storm must never fan
/// out into one bounded pi run per event.
pub struct Coalescer {
    window: Duration,
    /// Per-process random seed for the key digest, so a producer cannot craft a
    /// colliding key without knowing it.
    hasher: std::collections::hash_map::RandomState,
    /// Key digest to the time that (behaviour, event) was last admitted.
    seen: HashMap<u64, SystemTime>,
}

impl Coalescer {
    /// A coalescer with the given burst window.
    pub fn new(window: Duration) -> Self {
        Self {
            window,
            hasher: std::collections::hash_map::RandomState::new(),
            seen: HashMap::new(),
        }
    }

    /// The fixed-size digest a dispatch is coalesced on: the behaviour, the event
    /// type, the (sorted) fields and the external-content bit, hashed under a
    /// per-process seed so a producer cannot forge a colliding key.
    fn digest(
        &self,
        behaviour: &str,
        event_type: &str,
        fields: &BTreeMap<String, String>,
        external_content: bool,
    ) -> u64 {
        use std::hash::{BuildHasher as _, Hash as _, Hasher as _};
        let mut h = self.hasher.build_hasher();
        behaviour.hash(&mut h);
        event_type.hash(&mut h);
        // A BTreeMap hashes in sorted key order, so the same fields always digest
        // identically.
        fields.hash(&mut h);
        external_content.hash(&mut h);
        h.finish()
    }

    /// Decide whether to dispatch the keyed event at `now`, recording the time
    /// when it admits. Returns `true` (dispatch) when the key is new or its last
    /// dispatch is older than the window; `false` (coalesce) when a dispatch
    /// happened within the window. The window is measured from the first dispatch
    /// of a burst, not extended by each coalesced duplicate, so a sustained stream
    /// fires once per window rather than being suppressed forever.
    pub fn admit(
        &mut self,
        behaviour: &str,
        event_type: &str,
        fields: &BTreeMap<String, String>,
        external_content: bool,
        now: SystemTime,
    ) -> bool {
        let key = self.digest(behaviour, event_type, fields, external_content);
        let window = self.window;
        // Bound cost and memory. The common case (a small map) does no scan:
        // expiry is lazy, per key, on access below. Only when the map has grown to
        // the cap do we prune stale entries in one pass; if it is still full of
        // fresh distinct events, clear it entirely. Clearing only forgets recent
        // entries, so at worst a few duplicates slip through afterwards
        // (over-dispatch, never a dropped distinct event), while per-event cost
        // stays amortised O(1) and the map stays bounded under a hostile producer.
        if self.seen.len() >= MAX_COALESCE_ENTRIES {
            self.seen.retain(|_, last| {
                now.duration_since(*last)
                    .map(|elapsed| elapsed < window)
                    .unwrap_or(false)
            });
            if self.seen.len() >= MAX_COALESCE_ENTRIES {
                self.seen.clear();
            }
        }
        match self.seen.entry(key) {
            std::collections::hash_map::Entry::Occupied(mut slot) => {
                // Within the window: coalesce, without refreshing (the window is
                // measured from the first dispatch of a burst, not extended by each
                // duplicate). Expired, or future-stamped after a backwards clock
                // move: treat as stale, refresh to `now` and admit, rather than
                // suppressing the event past the window.
                let within = now
                    .duration_since(*slot.get())
                    .map(|elapsed| elapsed < window)
                    .unwrap_or(false);
                if within {
                    false
                } else {
                    slot.insert(now);
                    true
                }
            }
            std::collections::hash_map::Entry::Vacant(slot) => {
                slot.insert(now);
                true
            }
        }
    }

    /// The number of tracked keys (for tests / introspection).
    #[cfg(test)]
    pub fn tracked(&self) -> usize {
        self.seen.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn a_burst_of_identical_events_fires_once_per_window() {
        let mut c = Coalescer::new(Duration::from_secs(1));
        let t0 = SystemTime::UNIX_EPOCH;
        let f = fields(&[("path", "/a.rs")]);
        // First admits; the rest of the burst within the window coalesce.
        assert!(c.admit("auto-tag", "file.opened", &f, false, t0));
        assert!(!c.admit("auto-tag", "file.opened", &f, false, t0 + Duration::from_millis(100)));
        assert!(!c.admit("auto-tag", "file.opened", &f, false, t0 + Duration::from_millis(900)));
        // Past the window (measured from the first dispatch), it fires again.
        assert!(c.admit("auto-tag", "file.opened", &f, false, t0 + Duration::from_millis(1100)));
    }

    #[test]
    fn distinct_events_each_dispatch() {
        let mut c = Coalescer::new(Duration::from_secs(1));
        let t = SystemTime::UNIX_EPOCH;
        // Different fields, different behaviour, and different external bit are all
        // distinct keys - none coalesce against the others.
        assert!(c.admit("auto-tag", "file.opened", &fields(&[("path", "/a.rs")]), false, t));
        assert!(c.admit("auto-tag", "file.opened", &fields(&[("path", "/b.rs")]), false, t));
        assert!(c.admit("summarise", "file.opened", &fields(&[("path", "/a.rs")]), false, t));
        assert!(c.admit("auto-tag", "file.opened", &fields(&[("path", "/a.rs")]), true, t));
    }

    #[test]
    fn field_order_does_not_affect_the_key() {
        let mut c = Coalescer::new(Duration::from_secs(1));
        let t = SystemTime::UNIX_EPOCH;
        assert!(c.admit("b", "e", &fields(&[("a", "1"), ("b", "2")]), false, t));
        // The same fields in the other insertion order digest identically -> coalesce.
        assert!(!c.admit("b", "e", &fields(&[("b", "2"), ("a", "1")]), false, t));
    }

    #[test]
    fn a_backwards_clock_move_admits_rather_than_suppresses() {
        let mut c = Coalescer::new(Duration::from_secs(1));
        let f = fields(&[("path", "/a.rs")]);
        let t1 = SystemTime::UNIX_EPOCH + Duration::from_secs(10);
        assert!(c.admit("b", "e", &f, false, t1));
        // A now EARLIER than the recorded time (clock stepped back): the duration
        // is an error, treated as stale -> admit, never suppress past the window.
        assert!(c.admit("b", "e", &f, false, SystemTime::UNIX_EPOCH + Duration::from_secs(5)));
    }

    #[test]
    fn a_sustained_distinct_storm_stays_bounded() {
        let mut c = Coalescer::new(Duration::from_secs(1));
        let t = SystemTime::UNIX_EPOCH;
        // Push well past the cap with distinct keys, all within one window.
        for i in 0..(MAX_COALESCE_ENTRIES + 500) {
            let f = fields(&[("path", &format!("/f{i}.rs"))]);
            c.admit("b", "e", &f, false, t);
        }
        // The map never exceeds the cap (pruned/cleared), so a hostile producer
        // cannot grow it without bound.
        assert!(c.tracked() <= MAX_COALESCE_ENTRIES);
    }
}

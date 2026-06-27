//! In-memory store of execution receipts for the live-session undo path.
//!
//! When the live executor writes a relation it produces an
//! [`ExecutedWrite`](crate::executor::ExecutedWrite) receipt (the op_id, the
//! decision's correlation id, and the write). To let a later `compensate` (undo)
//! find the write to reverse — triggered from the activity view by the
//! decision's correlation id — the receipts are retained here, keyed by that
//! correlation id, for the daemon's lifetime.
//!
//! The store is **in-memory and bounded**. Receipts do not survive a restart,
//! and once `capacity` is reached the oldest receipt ages out and can no longer
//! be undone (a persisted, signed receipt log is the separate undo-log
//! increment). Bounding is deliberate: an unbounded store would grow without
//! limit on a long-running daemon. The type is generic over the stored value so
//! the bounded recency-map logic is testable without the executor-private
//! receipt; production uses `ReceiptStore<ExecutedWrite>`.

use std::collections::{HashMap, VecDeque};

use crate::executor::ActionReceipt;

/// A retained execution receipt together with the behaviour that produced it.
/// The receipt is stored as the [`ActionReceipt`] that `compensate` consumes
/// directly (the graph write wrapped as [`ActionReceipt::Graph`]), so the future
/// undo trigger passes `&retained.receipt` without re-wrapping, and a non-graph
/// receipt (EM-R5) is retained uniformly as [`ActionReceipt::NonGraph`]. A later
/// compensate audits the undo under the original behaviour's identity (the audit
/// links the retract to the write's decision), which the receipt does not carry,
/// so it is kept alongside here.
#[derive(Debug, Clone)]
pub struct RetainedReceipt {
    /// The compensate-ready execution receipt.
    pub receipt: ActionReceipt,
    /// The behaviour whose decision produced the write.
    pub behaviour: String,
}

/// A bounded, correlation-id-keyed store with first-in-first-out eviction.
/// Re-recording an existing key refreshes its recency rather than evicting
/// another entry.
#[derive(Debug)]
pub struct ReceiptStore<V> {
    capacity: usize,
    /// Keys in eviction order, oldest at the front. A key appears at most once.
    order: VecDeque<String>,
    map: HashMap<String, V>,
}

impl<V: Clone> ReceiptStore<V> {
    /// A store holding at most `capacity` receipts (clamped to at least 1, so a
    /// recorded receipt is always retrievable until evicted by a newer one).
    pub fn new(capacity: usize) -> Self {
        ReceiptStore {
            capacity: capacity.max(1),
            order: VecDeque::new(),
            map: HashMap::new(),
        }
    }

    /// Record a receipt under `key` (the decision's correlation id). If the key
    /// is already present its value is replaced and its recency refreshed; a new
    /// key evicts the oldest entry once capacity is exceeded.
    pub fn record(&mut self, key: String, value: V) {
        if self.map.insert(key.clone(), value).is_some() {
            // Existing key: move it to the most-recent position.
            self.order.retain(|k| k != &key);
            self.order.push_back(key);
            return;
        }
        self.order.push_back(key);
        while self.order.len() > self.capacity {
            if let Some(oldest) = self.order.pop_front() {
                self.map.remove(&oldest);
            }
        }
    }

    /// Look up the receipt for `key`, cloned for the caller. `None` if it was
    /// never recorded or has aged out.
    pub fn get(&self, key: &str) -> Option<V> {
        self.map.get(key).cloned()
    }

    /// Every retained value, cloned, oldest first. Used to list the store (e.g.
    /// the harness reads all pending proposals); the order is the eviction order.
    pub fn values(&self) -> Vec<V> {
        self.order.iter().filter_map(|k| self.map.get(k).cloned()).collect()
    }

    /// Remove the entry for `key`, returning it if present. Used when an entry is
    /// acted on (a pending proposal approved/denied) so it no longer lists.
    pub fn remove(&mut self, key: &str) -> Option<V> {
        let value = self.map.remove(key);
        if value.is_some() {
            self.order.retain(|k| k != key);
        }
        value
    }

    /// The number of retained receipts.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// Whether the store holds no receipts.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_and_retrieves_by_key() {
        let mut s: ReceiptStore<String> = ReceiptStore::new(4);
        s.record("corr-1".into(), "write-A".into());
        assert_eq!(s.get("corr-1"), Some("write-A".to_string()));
        assert_eq!(s.get("missing"), None);
        assert_eq!(s.len(), 1);
    }

    #[test]
    fn evicts_the_oldest_over_capacity() {
        let mut s: ReceiptStore<String> = ReceiptStore::new(2);
        s.record("a".into(), "1".into());
        s.record("b".into(), "2".into());
        s.record("c".into(), "3".into()); // evicts "a"
        assert_eq!(s.get("a"), None, "oldest aged out");
        assert_eq!(s.get("b"), Some("2".to_string()));
        assert_eq!(s.get("c"), Some("3".to_string()));
        assert_eq!(s.len(), 2);
    }

    #[test]
    fn re_recording_a_key_refreshes_recency_and_does_not_grow() {
        let mut s: ReceiptStore<String> = ReceiptStore::new(2);
        s.record("a".into(), "1".into());
        s.record("b".into(), "2".into());
        // Refresh "a" so it is now the most recent; the next insert evicts "b".
        s.record("a".into(), "1b".into());
        s.record("c".into(), "3".into()); // evicts "b", not "a"
        assert_eq!(s.get("a"), Some("1b".to_string()), "refreshed value survives");
        assert_eq!(s.get("b"), None, "stale entry evicted");
        assert_eq!(s.get("c"), Some("3".to_string()));
        assert_eq!(s.len(), 2);
    }

    #[test]
    fn values_lists_oldest_first_and_remove_drops_an_entry() {
        let mut s: ReceiptStore<String> = ReceiptStore::new(4);
        s.record("a".into(), "1".into());
        s.record("b".into(), "2".into());
        s.record("c".into(), "3".into());
        assert_eq!(s.values(), vec!["1".to_string(), "2".to_string(), "3".to_string()]);
        assert_eq!(s.remove("b"), Some("2".to_string()));
        assert_eq!(s.remove("b"), None, "already removed");
        assert_eq!(s.get("b"), None);
        assert_eq!(s.values(), vec!["1".to_string(), "3".to_string()], "order preserved, b gone");
        assert_eq!(s.len(), 2);
    }

    #[test]
    fn capacity_is_clamped_to_at_least_one() {
        let mut s: ReceiptStore<String> = ReceiptStore::new(0);
        s.record("a".into(), "1".into());
        // A clamped capacity of 1 keeps the most recent recorded receipt.
        assert_eq!(s.get("a"), Some("1".to_string()));
    }
}

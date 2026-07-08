//! Compensation registration for the Report verb (`pi-agent-adoption.md` Phase 1,
//! "Report: tool result for audit + compensation registration").
//!
//! When the engine reports a successful `graph.write` result, the daemon records
//! a [`RetractReceipt`] - the op-id-keyed inverse of exactly that write - so a
//! later undo can reverse it with `retract_relation`. The receipt is built from
//! the write executor's own result shape (`{op_id, from_*, to_*, relation_type}`),
//! so the inverse targets the precise edge the daemon created, never an
//! engine-supplied identifier.
//!
//! The store is in-memory and bounded (oldest ages out): receipts do not survive
//! a restart, and a persisted/signed undo log is a separate increment. The undo
//! TRIGGER (the activity-view undo) is a later consumer; this is the mechanism it
//! reads, built ahead of it like the executor was.

use std::collections::{HashMap, VecDeque};

/// The op-id-keyed inverse of a committed relation write: everything
/// `retract_relation` needs to remove exactly the edge the write created.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetractReceipt {
    /// The op id the write was stamped with; the retract is keyed to it.
    pub op_id: String,
    /// The edge endpoints + type, so the retract targets the same edge.
    pub from_type: String,
    pub from_id: String,
    pub to_type: String,
    pub to_id: String,
    pub relation_type: String,
}

impl RetractReceipt {
    /// Build a receipt directly from the daemon's own write parameters + minted
    /// op id, at the moment the executor applies the write. This is the
    /// AUTHORITATIVE path: the receipt targets exactly the edge the daemon just
    /// created, from fields the daemon validated itself, never an engine-supplied
    /// report. The op-id-keyed retract undoes exactly this write.
    #[allow(clippy::too_many_arguments)]
    pub fn for_write(
        op_id: impl Into<String>,
        from_type: impl Into<String>,
        from_id: impl Into<String>,
        to_type: impl Into<String>,
        to_id: impl Into<String>,
        relation_type: impl Into<String>,
    ) -> RetractReceipt {
        RetractReceipt {
            op_id: op_id.into(),
            from_type: from_type.into(),
            from_id: from_id.into(),
            to_type: to_type.into(),
            to_id: to_id.into(),
            relation_type: relation_type.into(),
        }
    }

    /// Build a receipt from a reported tool result, if it is a successful
    /// `graph.write` carrying the daemon's write-result shape. Any other tool, an
    /// error result, or a result missing a field yields `None` (nothing to undo).
    ///
    /// TEST-ONLY (`#[cfg(test)]`): this constructs a receipt from ENGINE-supplied
    /// result JSON (an arbitrary op_id / from / to), so it is NOT authoritative.
    /// The production write path registers only [`RetractReceipt::for_write`],
    /// built from the daemon's own minted op_id, so `compensate` retracts only a
    /// daemon-authored edge. Registering a `from_report` receipt into the live
    /// compensation store would let a non-cooperative engine plant a forged
    /// receipt and redirect a retract; keeping this test-gated makes that
    /// regression a deliberate, visible change rather than a silent one.
    #[cfg(test)]
    pub fn from_report(
        tool_name: &str,
        is_error: bool,
        result: &serde_json::Value,
    ) -> Option<RetractReceipt> {
        if tool_name != "graph.write" || is_error {
            return None;
        }
        let s = |k: &str| result.get(k).and_then(|v| v.as_str()).map(str::to_string);
        Some(RetractReceipt {
            op_id: s("op_id")?,
            from_type: s("from_type")?,
            from_id: s("from_id")?,
            to_type: s("to_type")?,
            to_id: s("to_id")?,
            relation_type: s("relation_type")?,
        })
    }
}

/// A bounded, correlation-id-keyed store of retract receipts with first-in-
/// first-out eviction. Re-recording an existing key refreshes its receipt and
/// recency rather than evicting another entry. Mirrors the agent's bounded
/// receipt store: an unbounded one would grow without limit on a long-running
/// daemon.
#[derive(Debug)]
pub struct CompensationStore {
    capacity: usize,
    /// Keys in eviction order, oldest at the front; a key appears at most once.
    order: VecDeque<String>,
    map: HashMap<String, RetractReceipt>,
}

impl CompensationStore {
    /// A store holding at most `capacity` receipts (clamped to at least 1).
    pub fn new(capacity: usize) -> Self {
        CompensationStore { capacity: capacity.max(1), order: VecDeque::new(), map: HashMap::new() }
    }

    /// Record `receipt` under `key` (the report's tool_call_id). A new key past
    /// capacity evicts the oldest; an existing key refreshes its receipt and
    /// moves it to newest.
    pub fn register(&mut self, key: impl Into<String>, receipt: RetractReceipt) {
        let key = key.into();
        if self.map.insert(key.clone(), receipt).is_some() {
            self.order.retain(|k| k != &key);
        } else if self.order.len() >= self.capacity {
            if let Some(oldest) = self.order.pop_front() {
                self.map.remove(&oldest);
            }
        }
        self.order.push_back(key);
    }

    /// The receipt recorded under `key`, if it is still retained.
    pub fn get(&self, key: &str) -> Option<&RetractReceipt> {
        self.map.get(key)
    }

    /// Drop the receipt under `key` (a no-op if absent). Called after a successful
    /// undo so `completed_actions` stops listing an action that has already been
    /// retracted - an undone edge is no longer an undoable one.
    pub fn remove(&mut self, key: &str) {
        if self.map.remove(key).is_some() {
            self.order.retain(|k| k != key);
        }
    }

    /// The retained receipts paired with their correlation-id key, OLDEST FIRST
    /// (eviction order). Backs the AIAgent1 `completed_actions` transparency read:
    /// the recently-executed, still-undoable writes, each carrying the key the
    /// `compensate(id)` undo targets. Bounded like the store, so an aged-out action
    /// is neither listed nor undoable (the same horizon as `compensate`).
    pub fn entries(&self) -> Vec<(&str, &RetractReceipt)> {
        self.order
            .iter()
            .filter_map(|k| self.map.get(k).map(|r| (k.as_str(), r)))
            .collect()
    }

    /// How many receipts are retained.
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

    fn write_result(op: &str) -> serde_json::Value {
        serde_json::json!({
            "op_id": op, "created": true,
            "from_type": "File", "from_id": "/a.rs",
            "to_type": "Project", "to_id": "proj-1",
            "relation_type": "FILE_PART_OF",
        })
    }

    #[test]
    fn a_successful_write_result_yields_a_receipt() {
        let r = RetractReceipt::from_report("graph.write", false, &write_result("op-1")).unwrap();
        assert_eq!(r.op_id, "op-1");
        assert_eq!(r.relation_type, "FILE_PART_OF");
        assert_eq!(r.to_id, "proj-1");
    }

    #[test]
    fn a_non_write_or_errored_or_partial_result_yields_nothing() {
        assert!(RetractReceipt::from_report("graph.read", false, &write_result("op")).is_none());
        assert!(RetractReceipt::from_report("graph.write", true, &write_result("op")).is_none());
        let mut partial = write_result("op");
        partial.as_object_mut().unwrap().remove("op_id");
        assert!(RetractReceipt::from_report("graph.write", false, &partial).is_none());
    }

    #[test]
    fn a_registered_receipt_is_retrievable_by_its_key() {
        let mut store = CompensationStore::new(8);
        let r = RetractReceipt::from_report("graph.write", false, &write_result("op-7")).unwrap();
        store.register("call-7", r.clone());
        assert_eq!(store.get("call-7"), Some(&r));
        assert_eq!(store.len(), 1);
        assert!(store.get("call-other").is_none());
    }

    #[test]
    fn the_oldest_receipt_ages_out_at_capacity() {
        let mut store = CompensationStore::new(2);
        for (k, op) in [("c1", "op1"), ("c2", "op2"), ("c3", "op3")] {
            store.register(k, RetractReceipt::from_report("graph.write", false, &write_result(op)).unwrap());
        }
        assert_eq!(store.len(), 2);
        assert!(store.get("c1").is_none(), "the oldest was evicted");
        assert!(store.get("c2").is_some());
        assert!(store.get("c3").is_some());
    }

    #[test]
    fn entries_lists_receipts_oldest_first_with_keys() {
        let mut store = CompensationStore::new(8);
        store.register("c1", RetractReceipt::from_report("graph.write", false, &write_result("op1")).unwrap());
        store.register("c2", RetractReceipt::from_report("graph.write", false, &write_result("op2")).unwrap());
        let listed: Vec<(&str, &str)> =
            store.entries().iter().map(|(k, r)| (*k, r.op_id.as_str())).collect();
        assert_eq!(listed, vec![("c1", "op1"), ("c2", "op2")]);
    }

    #[test]
    fn re_registering_a_key_refreshes_without_evicting() {
        let mut store = CompensationStore::new(2);
        store.register("c1", RetractReceipt::from_report("graph.write", false, &write_result("op1")).unwrap());
        store.register("c2", RetractReceipt::from_report("graph.write", false, &write_result("op2")).unwrap());
        // Re-record c1 (newest now), then add c3: c2 (oldest) evicts, not c1.
        store.register("c1", RetractReceipt::from_report("graph.write", false, &write_result("op1b")).unwrap());
        store.register("c3", RetractReceipt::from_report("graph.write", false, &write_result("op3")).unwrap());
        assert!(store.get("c1").is_some(), "refreshed key survives");
        assert_eq!(store.get("c1").unwrap().op_id, "op1b", "receipt was refreshed");
        assert!(store.get("c2").is_none(), "the genuinely-oldest evicted");
        assert!(store.get("c3").is_some());
    }
}

//! The recent-actions read: the undo signer's entries joined to the audit
//! ledger by call chain, so a surface can say who did what, when, and offer
//! the undo.
//!
//! The two stores each answer half and neither duplicates the other. The undo
//! record knows how to UN-happen an action, so its captured inverse names the
//! object; the audit ledger is the record of what happened, so it carries the
//! producer, the verb and the time. Copying either side's facts into the other
//! would be a second store of the same facts, and every drift bug we have hit
//! came from a second store, never a second view.
//!
//! The join key is the undo entry's `correlation_id`, which the action's audit
//! entries carry as their `call_chain_id`.
//!
//! One rule governs failure here: **never hide an undoable action because its
//! description is missing.** A user who cannot see that something is undoable
//! has effectively lost the undo, which is worse than an undescribed row. So a
//! failed or empty join degrades the row to `description: None` and the row is
//! still returned.

use arlen_ai_undo_core::undo_log::UndoState;
use arlen_ai_undo_proto::RecentEntry;
use audit_proto::read::StructuralView;

/// What the audit ledger knows about an action, when the join resolves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionDescription {
    /// `app_id` of the component that performed the action, kernel-attested at
    /// ingest, so this is who acted and not who claims to have acted.
    pub actor: String,
    /// The audit kind, as its wire string.
    pub kind: String,
    /// The content-free structural subject, e.g. `agent.auto-tag-by-project`.
    pub subject: String,
    /// When the action was recorded, microseconds since the Unix epoch.
    pub timestamp_micros: i64,
}

/// One row of a recent-actions surface: always undoable-or-not and always
/// naming its object, with the audit description attached when it resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UndoRow {
    /// The undo entry's op id, the handle an undo is requested with.
    pub op_id: String,
    /// The join key, kept so a surface can fetch the full chain on demand.
    pub correlation_id: String,
    /// The folded lifecycle state.
    pub state: UndoState,
    /// What undo would do, as a stable identifier the UI translates.
    pub inverse_kind: &'static str,
    /// What the action acted on, from the captured inverse.
    pub object: String,
    /// The audit facts, or `None` when the join found nothing. `None` means
    /// undescribed, never un-undoable.
    pub description: Option<ActionDescription>,
}

/// Build the rows from the signer's recent entries and whatever audit entries
/// were readable, pairing them by call chain.
///
/// `audit` may be short, empty or entirely absent (an audit daemon that is down
/// yields no entries at all); every entry still produces a row.
pub fn join_rows(recent: &[RecentEntry], audit: &[StructuralView]) -> Vec<UndoRow> {
    recent
        .iter()
        .map(|r| {
            let correlation_id = r.entry.correlation_id.clone();
            UndoRow {
                op_id: r.entry.op_id.clone(),
                state: r.state,
                inverse_kind: r.entry.inverse.inverse_kind(),
                object: r.entry.inverse.object(),
                description: describe(&correlation_id, audit),
                correlation_id,
            }
        })
        .collect()
}

/// Pick the audit entry that best describes one action's chain: the earliest,
/// which is the decision that authorised the action rather than a later
/// outcome entry. Returns `None` when the chain has no readable entry.
fn describe(correlation_id: &str, audit: &[StructuralView]) -> Option<ActionDescription> {
    audit
        .iter()
        .filter(|e| e.call_chain_id.as_deref() == Some(correlation_id))
        .min_by_key(|e| (e.timestamp_micros, e.index))
        .map(|e| ActionDescription {
            actor: e.actor.clone(),
            kind: e.kind.as_str().to_string(),
            subject: e.structural.subject.clone(),
            timestamp_micros: e.timestamp_micros,
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use arlen_ai_undo_core::effect_model::{CanonicalPath, InverseReceipt, SettingTarget};
    use arlen_ai_undo_core::undo_log::UndoEntry;
    use audit_proto::{AuditKind, StructuralRecord};

    fn entry(op: &str, chain: &str, inverse: InverseReceipt) -> RecentEntry {
        RecentEntry {
            entry: UndoEntry {
                op_id: op.to_string(),
                correlation_id: chain.to_string(),
                inverse,
            },
            state: UndoState::Committed,
        }
    }

    fn path(p: &str) -> CanonicalPath {
        CanonicalPath::new(p).unwrap()
    }

    fn view(index: u64, chain: &str, actor: &str, subject: &str, at: i64) -> StructuralView {
        StructuralView {
            index,
            timestamp_micros: at,
            kind: AuditKind::AppAction,
            actor: actor.to_string(),
            structural: StructuralRecord {
                subject: subject.to_string(),
                node_types: Vec::new(),
                relations: Vec::new(),
                result_count: None,
                duration_ms: None,
                outcome: "ok".to_string(),
                depth: None,
                capability_change: None,
            },
            call_chain_id: Some(chain.to_string()),
            project_id: None,
            entry_hash_hex: "00".to_string(),
        }
    }

    #[test]
    fn a_resolved_join_carries_the_producer_verb_and_time_from_audit() {
        let recent = vec![entry(
            "op-1",
            "run-1",
            InverseReceipt::RestorePath {
                now: path("/home/u/b.txt"),
                prior: path("/home/u/a.txt"),
            },
        )];
        let audit = vec![view(3, "run-1", "files", "fs.relocate", 100)];

        let rows = join_rows(&recent, &audit);
        assert_eq!(rows.len(), 1);
        let d = rows[0].description.as_ref().expect("chain resolved");
        assert_eq!(d.actor, "files");
        assert_eq!(d.subject, "fs.relocate");
        assert_eq!(d.timestamp_micros, 100);
        // The object comes from the receipt, never from the content-free audit row.
        assert_eq!(rows[0].object, "/home/u/b.txt");
        assert_eq!(rows[0].inverse_kind, "restore-path");
    }

    #[test]
    fn an_unresolvable_join_still_yields_an_undoable_row() {
        // The guard that matters: an audit daemon that is down, or an action
        // whose entries aged out, must not make the action disappear from the
        // surface - it would look un-undoable when it is not.
        let recent = vec![entry(
            "op-2",
            "run-missing",
            InverseReceipt::RestoreValue {
                target: SettingTarget::new("shell.toml", "layout.mode").unwrap(),
                prior: Some("floating".to_string()),
            },
        )];

        let rows = join_rows(&recent, &[]);
        assert_eq!(rows.len(), 1, "the row survives an empty audit read");
        assert!(rows[0].description.is_none(), "undescribed, not hidden");
        assert_eq!(rows[0].op_id, "op-2");
        assert_eq!(rows[0].object, "shell.toml:layout.mode");
        assert_eq!(rows[0].inverse_kind, "restore-value");
    }

    #[test]
    fn a_row_takes_only_its_own_chain_and_the_earliest_entry_of_it() {
        let recent = vec![
            entry(
                "op-a",
                "run-a",
                InverseReceipt::RestorePath {
                    now: path("/a/now"),
                    prior: path("/a/prior"),
                },
            ),
            entry(
                "op-b",
                "run-b",
                InverseReceipt::RestorePath {
                    now: path("/b/now"),
                    prior: path("/b/prior"),
                },
            ),
        ];
        let audit = vec![
            view(9, "run-a", "files", "fs.relocate.done", 300),
            view(7, "run-a", "files", "fs.relocate", 100),
            view(8, "run-b", "ai-agent", "agent.tidy", 200),
        ];

        let rows = join_rows(&recent, &audit);
        let a = rows[0].description.as_ref().unwrap();
        assert_eq!(a.subject, "fs.relocate", "the authorising entry, not the outcome");
        let b = rows[1].description.as_ref().unwrap();
        assert_eq!(b.actor, "ai-agent", "chains do not bleed into each other");
    }
}

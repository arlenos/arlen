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
use audit_proto::read_client::ReadClient;
use std::collections::BTreeSet;
use std::path::Path;

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

/// One action's chain of ledger entries, however the caller gets them.
///
/// A seam rather than a `ReadClient` field, for one reason: the guard this
/// module exists to keep - that a row survives an unresolvable description -
/// is only worth anything if it is tested against a reader that actually
/// fails, and a live audit daemon cannot be made to fail on demand.
#[async_trait::async_trait]
pub trait AuditChains: Send + Sync {
    /// The entries stamped with `correlation_id`, or an empty vec if the read
    /// found nothing OR could not be made. **The two cases are deliberately not
    /// distinguished**, because they have the same consequence for the caller
    /// (an undescribed row) and distinguishing them would invite a caller to
    /// treat one of them as a reason to hide the action.
    async fn chain(&self, correlation_id: &str) -> Vec<StructuralView>;
}

/// The real reader: the audit daemon's read socket, one query per chain.
///
/// Per chain rather than one tail page of the ledger, which is the cheaper
/// shape and the wrong one. A page is a guess that the action's entries are
/// still near the head, so on a busy ledger the descriptions vanish for exactly
/// the actions a user is most likely to want back - the degradation the guard
/// above allows for, triggered by our own shortcut rather than by a real gap.
/// The chains are small and few (a panel shows tens of rows), so the round
/// trips are cheap and the answer is the true one.
pub struct LedgerChains {
    client: ReadClient,
}

/// Entries to read per chain. One action's chain is a gate decision, an
/// execution and an outcome; this is well clear of that and still bounded.
const CHAIN_LIMIT: u64 = 32;

impl LedgerChains {
    /// A reader against the canonical audit read socket.
    pub fn at_default_socket() -> Self {
        Self {
            client: ReadClient::new(audit_proto::read::read_socket_path()),
        }
    }
}

#[async_trait::async_trait]
impl AuditChains for LedgerChains {
    async fn chain(&self, correlation_id: &str) -> Vec<StructuralView> {
        match self.client.for_call_chain(correlation_id, CHAIN_LIMIT).await {
            Ok(page) => page.entries,
            Err(e) => {
                // Logged, not propagated: an audit daemon that is down must cost
                // descriptions, never the undo itself.
                tracing::debug!(error = %e, correlation_id, "audit chain unreadable");
                Vec::new()
            }
        }
    }
}

/// The recent-actions read: the signer's entries, newest first, each described
/// by its audit chain where that resolves.
///
/// Best-effort at both halves and in different ways, which is the point. An
/// unreachable **signer** yields no rows at all - there is nothing to offer an
/// undo of, and inventing rows would be worse than an empty panel. An
/// unreachable **audit daemon** yields rows with no description, because the
/// action is still undoable and saying so is the whole job.
pub async fn recent_rows(
    signer_socket: &Path,
    audit: &dyn AuditChains,
    limit: u32,
) -> Vec<UndoRow> {
    let recent = match crate::undo_signer::fetch_recent(signer_socket, limit).await {
        Ok(entries) => entries,
        Err(e) => {
            tracing::warn!(error = %e, "undo signer unreadable; recent actions empty");
            return Vec::new();
        }
    };
    // Distinct chains only. Several receipts can share one correlation id (a run
    // that produced more than one reversible effect), and reading that chain
    // once per receipt would multiply the round trips for an identical answer.
    let chains: BTreeSet<&str> = recent
        .iter()
        .map(|r| r.entry.correlation_id.as_str())
        .collect();
    let mut views: Vec<StructuralView> = Vec::new();
    for id in chains {
        views.extend(audit.chain(id).await);
    }
    join_rows(&recent, &views)
}

#[cfg(test)]
mod tests {
    use super::*;
    use arlen_ai_undo_core::effect_model::{CanonicalPath, InverseReceipt, SettingTarget};
    use arlen_ai_undo_core::undo_log::UndoEntry;
    use arlen_ai_undo_proto::{read_request, write_response, Request, Response};
    use audit_proto::{AuditKind, StructuralRecord};
    use std::sync::{Arc, Mutex};
    use tokio::net::UnixListener;

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

    /// An audit side that records what it was asked and answers with `answer`.
    struct FakeChains {
        asked: Arc<Mutex<Vec<String>>>,
        answer: Vec<StructuralView>,
    }

    #[async_trait::async_trait]
    impl AuditChains for FakeChains {
        async fn chain(&self, correlation_id: &str) -> Vec<StructuralView> {
            self.asked.lock().unwrap().push(correlation_id.to_string());
            self.answer
                .iter()
                .filter(|e| e.call_chain_id.as_deref() == Some(correlation_id))
                .cloned()
                .collect()
        }
    }

    /// Serve one `ListRecent` on a temp socket and hand back `entries`.
    fn serve_recent(dir: &std::path::Path, entries: Vec<RecentEntry>) -> std::path::PathBuf {
        let socket = dir.join("signer.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            match read_request(&mut stream).await.unwrap() {
                Request::ListRecent { .. } => {}
                other => panic!("expected ListRecent, got {other:?}"),
            }
            write_response(&mut stream, &Response::Recent(entries)).await.unwrap();
        });
        socket
    }

    #[tokio::test]
    async fn an_unreadable_audit_still_yields_undoable_rows() {
        // The guard the seam exists for. A live audit daemon cannot be made to
        // fail on demand, so this is the only place the "undescribed, never
        // hidden" rule is actually exercised against a reader that answers
        // nothing - which is what a down daemon looks like from here.
        let dir = tempfile::tempdir().unwrap();
        let socket = serve_recent(
            dir.path(),
            vec![
                entry("op-1", "run-1", InverseReceipt::RestorePath {
                    now: path("/home/u/b.txt"),
                    prior: path("/home/u/a.txt"),
                }),
                entry("op-2", "run-2", InverseReceipt::RestoreValue {
                    target: SettingTarget::new("shell.toml", "layout.mode").unwrap(),
                    prior: Some("floating".to_string()),
                }),
            ],
        );
        let audit = FakeChains {
            asked: Arc::new(Mutex::new(Vec::new())),
            answer: Vec::new(),
        };

        let rows = recent_rows(&socket, &audit, 20).await;
        assert_eq!(rows.len(), 2, "both actions stay offerable");
        assert!(rows.iter().all(|r| r.description.is_none()), "undescribed");
        assert_eq!(rows[0].object, "/home/u/b.txt", "the object still names itself");
    }

    #[tokio::test]
    async fn one_chain_is_read_once_however_many_receipts_share_it() {
        // A run that produced two reversible effects carries one correlation id
        // on both receipts. Reading its chain twice would double the round trips
        // for an identical answer.
        let dir = tempfile::tempdir().unwrap();
        let mk = |op: &str, chain: &str| {
            entry(op, chain, InverseReceipt::RestorePath {
                now: path("/n"),
                prior: path("/p"),
            })
        };
        let socket = serve_recent(
            dir.path(),
            vec![mk("op-a", "run-1"), mk("op-b", "run-1"), mk("op-c", "run-2")],
        );
        let asked = Arc::new(Mutex::new(Vec::new()));
        let audit = FakeChains {
            asked: Arc::clone(&asked),
            answer: vec![view(1, "run-1", "files", "fs.relocate", 10)],
        };

        let rows = recent_rows(&socket, &audit, 20).await;
        assert_eq!(rows.len(), 3, "every receipt is still its own row");
        let mut asked = asked.lock().unwrap().clone();
        asked.sort();
        assert_eq!(asked, vec!["run-1".to_string(), "run-2".to_string()]);
        // The shared chain describes both of its receipts.
        assert_eq!(rows[0].description.as_ref().unwrap().actor, "files");
        assert_eq!(rows[1].description.as_ref().unwrap().actor, "files");
        assert!(rows[2].description.is_none(), "run-2 has no readable entry");
    }

    #[tokio::test]
    async fn an_unreachable_signer_yields_no_rows_rather_than_invented_ones() {
        let dir = tempfile::tempdir().unwrap();
        let audit = FakeChains {
            asked: Arc::new(Mutex::new(Vec::new())),
            answer: Vec::new(),
        };
        let rows = recent_rows(&dir.path().join("absent.sock"), &audit, 20).await;
        assert!(rows.is_empty(), "no signer means nothing to undo, not a fabricated list");
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

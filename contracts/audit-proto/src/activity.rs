//! Recent-activity view over the audit read API.
//!
//! The Settings AI page and the AI harness app both render the most
//! recent audit entries (the P9 read-only activity surface). The read
//! API paginates ascending from an index, so "most recent" needs a
//! tail-seek using the page `head`; that logic plus the frontend-facing
//! entry shape lived duplicated in both apps. It belongs here, next to
//! [`ReadClient`], as one source both apps call.
//!
//! [`ReadClient::recent`] is **advisory**: a missing or unreachable
//! daemon yields an empty page flagged `available = false` rather than an
//! error, so a caller's UI renders a quiet state instead of failing.

use serde::Serialize;

use crate::{AuditKind, ReadClient, StructuralView};

/// The largest activity page a caller may request. Matches the daemon's
/// own page ceiling so a single tail read returns everything asked for.
pub const MAX_ACTIVITY_LIMIT: u64 = 200;

/// One audit entry as an activity view consumes it: the Structural tier
/// flattened to camelCase for a frontend. Never carries Forensic data.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityEntry {
    /// Chain index of the entry.
    pub index: u64,
    /// Append time, microseconds since the Unix epoch.
    pub timestamp_micros: i64,
    /// Stable kind label (`query`, `tool-call`, `graph-access`, ...).
    pub kind: String,
    /// `app_id` of the component that performed the action.
    pub actor: String,
    /// Coarse subject: an MCP server/tool id or a graph target label.
    pub subject: String,
    /// Coarse outcome label: `ok`, `denied`, `error`, ...
    pub outcome: String,
    /// Graph node types touched, if any.
    pub node_types: Vec<String>,
    /// Graph relations traversed, if any.
    pub relations: Vec<String>,
    /// Number of results, when meaningful for this kind.
    pub result_count: Option<u64>,
    /// Wall-clock duration of the action, when measured.
    pub duration_ms: Option<u64>,
    /// MCP call-chain depth, when part of one.
    pub depth: Option<u8>,
    /// MCP call-chain id, when the entry belongs to one.
    pub call_chain_id: Option<String>,
    /// Project context, when one was active.
    pub project_id: Option<String>,
    /// Opaque per-entry reference (hex `entry_hash`).
    pub entry_ref: String,
}

/// A page of recent activity: the most recent entries (newest first)
/// plus daemon liveness and tamper status, so a caller renders its
/// state banner without a second probe.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityPage {
    /// Most recent entries first.
    pub entries: Vec<ActivityEntry>,
    /// The audit daemon answered the read.
    pub available: bool,
    /// The daemon reports its ledger tampered.
    pub tampered: bool,
    /// Total entries matching the read (one past the highest index).
    pub total: u64,
}

impl ActivityPage {
    /// The page returned when the daemon could not be reached: empty and
    /// flagged unavailable, never an error.
    fn unavailable() -> Self {
        Self {
            entries: Vec::new(),
            available: false,
            tampered: false,
            total: 0,
        }
    }
}

/// A stable, frontend-friendly label for each audit kind. Exhaustive on
/// purpose: a new [`AuditKind`] variant must be given a label here rather
/// than silently rendering as a fallback.
fn kind_label(kind: &AuditKind) -> &'static str {
    match kind {
        AuditKind::Query => "query",
        AuditKind::ToolCall => "tool-call",
        AuditKind::Confirm => "confirm",
        AuditKind::PolicyViolation => "policy-violation",
        AuditKind::GraphAccess => "graph-access",
        AuditKind::Permission => "permission",
        AuditKind::NetworkCall => "network-call",
        AuditKind::AppAction => "app-action",
    }
}

/// Map a Structural-tier view to the frontend entry shape.
fn to_entry(view: &StructuralView) -> ActivityEntry {
    ActivityEntry {
        index: view.index,
        timestamp_micros: view.timestamp_micros,
        kind: kind_label(&view.kind).to_string(),
        actor: view.actor.clone(),
        subject: view.structural.subject.clone(),
        outcome: view.structural.outcome.clone(),
        node_types: view.structural.node_types.clone(),
        relations: view.structural.relations.clone(),
        result_count: view.structural.result_count,
        duration_ms: view.structural.duration_ms,
        depth: view.structural.depth,
        call_chain_id: view.call_chain_id.clone(),
        project_id: view.project_id.clone(),
        entry_ref: view.entry_hash_hex.clone(),
    }
}

impl ReadClient {
    /// Read the most recent `limit` audit entries, newest first.
    ///
    /// The read API paginates ascending from an index, so to show the
    /// *latest* activity this reads once (which also returns the ledger
    /// `head`), then, if the ledger is larger than `limit`, seeks to the
    /// tail (`from = head - limit`) with one more read. `head` only grows
    /// (the ledger is append-only), so the computed `from` never points
    /// past the end. Advisory: any transport failure degrades to an
    /// empty, `available = false` page.
    pub async fn recent(&self, limit: u64) -> ActivityPage {
        let limit = limit.clamp(1, MAX_ACTIVITY_LIMIT);

        let first = match self.read(0, u64::MAX, limit, None).await {
            Ok(page) => page,
            Err(_) => return ActivityPage::unavailable(),
        };

        let page = if first.head > limit {
            let from = first.head - limit;
            // A tail read failure falls back to the first (oldest) page
            // rather than failing: a degraded view beats none.
            self.read(from, u64::MAX, limit, None).await.unwrap_or(first)
        } else {
            first
        };

        let total = page.head;
        let tampered = page.tampered;
        let mut entries: Vec<ActivityEntry> = page.entries.iter().map(to_entry).collect();
        // The API returns ascending by index; activity views want newest first.
        entries.reverse();

        ActivityPage {
            entries,
            available: true,
            tampered,
            total,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{StructuralRecord, StructuralView};

    fn view(index: u64, kind: AuditKind, subject: &str) -> StructuralView {
        StructuralView {
            index,
            timestamp_micros: index as i64,
            kind,
            actor: "ai-daemon".into(),
            structural: StructuralRecord {
                subject: subject.into(),
                node_types: vec![],
                relations: vec![],
                result_count: None,
                duration_ms: None,
                outcome: "ok".into(),
                depth: None,
            },
            call_chain_id: None,
            project_id: None,
            entry_hash_hex: format!("{index:064x}"),
        }
    }

    #[test]
    fn to_entry_maps_kind_and_fields() {
        let e = to_entry(&view(3, AuditKind::ToolCall, "srv"));
        assert_eq!(e.index, 3);
        assert_eq!(e.kind, "tool-call");
        assert_eq!(e.subject, "srv");
        assert_eq!(e.actor, "ai-daemon");
        assert_eq!(e.entry_ref.len(), 64);
    }

    #[test]
    fn unavailable_page_is_empty_and_flagged() {
        let p = ActivityPage::unavailable();
        assert!(!p.available);
        assert!(p.entries.is_empty());
        assert_eq!(p.total, 0);
    }

    #[tokio::test]
    async fn recent_against_a_missing_socket_is_an_unavailable_page() {
        // No daemon: advisory, never an error.
        let client = ReadClient::new("/nonexistent/audit-read.sock");
        let page = client.recent(50).await;
        assert!(!page.available);
        assert!(page.entries.is_empty());
    }
}

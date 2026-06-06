//! Agent activity read command (ai-app.md §2.2, the A4 timeline).
//!
//! Reads the audit ledger's **Structural tier** (content-free metadata,
//! never Forensic) over the daemon's read socket and exposes the most
//! recent entries to the harness agent dashboard. Read-only and
//! advisory: a missing or unreachable daemon yields an empty list
//! flagged `available = false`, so the dashboard still renders.
//!
//! This is the same shared S-U4 read surface the Settings AI activity
//! slice uses (`audit-proto` `ReadClient`); only the thin Tauri wrapper
//! is per-app. Folding the wrapper into a shared crate is a tracked
//! dedup follow-up.

use audit_proto::{read_socket_path, AuditKind, ReadClient, StructuralView};
use serde::Serialize;

/// The largest activity page the view may request. Matches the daemon's
/// own page ceiling so a single tail read returns everything asked for.
const MAX_ACTIVITY_LIMIT: u64 = 200;

/// One audit entry as the timeline consumes it. Mirrors the Structural
/// tier; camelCase for the frontend.
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

/// The timeline payload: the most recent entries (newest first) plus
/// daemon liveness and tamper status, so the dashboard renders its
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
    /// Total entries in the ledger (one past the highest index).
    pub total: u64,
}

/// Read the most recent `limit` audit entries, newest first.
///
/// The read API paginates ascending from an index, so to show the
/// *latest* activity the command reads once (which also returns the
/// ledger `head`), then, if the ledger is larger than `limit`, seeks to
/// the tail (`from = head - limit`) with one more read. Both reads are
/// advisory: any failure degrades to an empty, `available = false` page.
#[tauri::command]
pub async fn ai_activity_recent(limit: u64) -> Result<ActivityPage, String> {
    let limit = limit.clamp(1, MAX_ACTIVITY_LIMIT);
    let client = ReadClient::new(read_socket_path());

    let first = match client.read(0, u64::MAX, limit, None).await {
        Ok(page) => page,
        Err(e) => {
            log::warn!("[activity] audit read failed: {e}");
            return Ok(ActivityPage {
                entries: Vec::new(),
                available: false,
                tampered: false,
                total: 0,
            });
        }
    };

    // The first read returns the OLDEST `limit` entries. When the ledger
    // holds more, seek to the tail for the newest ones. `head` only
    // grows (the ledger is append-only), so the computed `from` never
    // points past the end.
    let page = if first.head > limit {
        let from = first.head - limit;
        match client.read(from, u64::MAX, limit, None).await {
            Ok(tail) => tail,
            Err(e) => {
                log::warn!("[activity] audit tail read failed: {e}");
                first
            }
        }
    } else {
        first
    };

    let total = page.head;
    let tampered = page.tampered;
    let mut entries: Vec<ActivityEntry> = page.entries.iter().map(to_dto).collect();
    // The API returns ascending by index; the timeline wants newest first.
    entries.reverse();

    Ok(ActivityPage {
        entries,
        available: true,
        tampered,
        total,
    })
}

/// Map a Structural-tier view to the frontend DTO.
fn to_dto(view: &StructuralView) -> ActivityEntry {
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

/// A stable, frontend-friendly label for each audit kind. Exhaustive on
/// purpose: a new `AuditKind` variant must be given a label here rather
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
    }
}

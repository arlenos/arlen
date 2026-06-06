//! Read-API wire types.
//!
//! The audit daemon's read API serves the Structural tier to the
//! user's own processes — the Settings audit viewer and the Anomaly
//! Detector. These types are the wire contract for that socket, kept
//! here in the shared crate so a *reader* (the detector) and the
//! daemon (the server) share one definition, the same way the ingest
//! types are shared.
//!
//! The Forensic tier is never representable here: [`StructuralView`]
//! has no field that can hold Forensic content, so the read path
//! cannot leak it by construction.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{AuditKind, StructuralRecord};

/// One audit entry as the read API exposes it: the Structural tier
/// only.
///
/// There is deliberately **no** forensic field. The read API must
/// never serve Forensic-tier content (foundation §8.4.7), and a type
/// that *cannot hold* it enforces that by construction rather than by
/// review discipline.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StructuralView {
    /// Chain index of the entry.
    pub index: u64,
    /// Append time, microseconds since the Unix epoch.
    pub timestamp_micros: i64,
    /// What kind of action the entry records.
    pub kind: AuditKind,
    /// `app_id` of the component that performed the action.
    pub actor: String,
    /// The content-free structural metadata.
    pub structural: StructuralRecord,
    /// MCP call-chain id, when the entry belongs to one.
    pub call_chain_id: Option<String>,
    /// Project context, when one was active.
    pub project_id: Option<String>,
    /// Hex-encoded `entry_hash` — an opaque per-entry reference id.
    pub entry_hash_hex: String,
}

/// A read query: the half-open index range `[from, to)`, capped by
/// `limit`, optionally filtered to one project.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadRequest {
    /// First index to include (default 0).
    #[serde(default)]
    pub from: u64,
    /// First index to exclude. Use `u64::MAX` for "to the end".
    pub to: u64,
    /// Maximum entries to return; the daemon clamps it to its own
    /// page ceiling.
    pub limit: u64,
    /// When set, only entries recorded under this project — the basis
    /// of the project-scoped export.
    #[serde(default)]
    pub project_id: Option<String>,
}

/// The reply to a [`ReadRequest`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadResponse {
    /// A page of Structural-tier views, ascending by index. To page,
    /// the caller advances `from` past the last index returned.
    Page {
        /// The matching entries.
        entries: Vec<StructuralView>,
        /// Whether the daemon's startup integrity check found the
        /// ledger tampered. Carried on every page so a reader (the
        /// Anomaly Detector) learns tamper state over the reliable
        /// poll path, not only via a best-effort Event Bus event it
        /// might miss while down. `serde(default)` keeps old/new peers
        /// compatible.
        #[serde(default)]
        tampered: bool,
        /// One past the highest index among entries matching this
        /// page's filter. For an unfiltered read this is the total
        /// entry count (indices are contiguous from 0); for a
        /// `project_id`-scoped read it is scoped to that project, so a
        /// scoped read never discloses the global ledger volume.
        /// Carried on every page so a reader that wants the *most
        /// recent* entries can seek toward the tail
        /// (`from = head - limit`) in one round trip, instead of paging
        /// ascending from 0. `serde(default)` keeps old/new peers
        /// compatible; an old daemon reports 0 (a client then falls
        /// back to forward paging).
        #[serde(default)]
        head: u64,
    },
    /// The query could not be served.
    Error {
        /// Human-readable reason.
        reason: String,
    },
}

/// A successfully read page: the entries plus the daemon's current
/// tamper status. What [`crate::ReadClient::read`] returns.
#[derive(Debug, Clone)]
pub struct ReadPage {
    /// The matching entries, ascending by index.
    pub entries: Vec<StructuralView>,
    /// Whether the audit daemon reports its ledger as tampered.
    pub tampered: bool,
    /// One past the highest index among entries matching the request's
    /// filter (the total entry count for an unfiltered read; scoped to
    /// the project for a `project_id` read, so a scoped read never
    /// leaks the global volume). Lets a caller seek toward the tail for
    /// the most recent entries; 0 from a daemon that predates this
    /// field.
    pub head: u64,
}

/// Resolve the read socket path:
/// `$XDG_RUNTIME_DIR/lunaris/audit-read.sock`, falling back to
/// `/run/lunaris/audit-read.sock`.
pub fn read_socket_path() -> PathBuf {
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/run"));
    base.join("lunaris").join("audit-read.sock")
}

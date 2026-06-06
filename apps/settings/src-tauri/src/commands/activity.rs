//! AI Activity read command (P9 read-only transparency surface).
//!
//! Thin wrapper over the shared recent-activity reader in `audit-proto`
//! (`ReadClient::recent`) — the same source the AI harness app's activity
//! timeline uses, so the tail-seek logic and the frontend entry shape
//! live in one place (`audit_proto::activity`). It reads the Structural
//! tier (content-free, never Forensic) over the daemon's read socket.
//!
//! Read-only and advisory: a missing or unreachable daemon yields an
//! empty `available = false` page (the reader never errors), so the page
//! still renders instead of failing.

use audit_proto::{read_socket_path, ActivityPage, ReadClient};

/// Read the most recent `limit` audit entries, newest first, for the
/// Settings AI Activity view.
#[tauri::command]
pub async fn ai_activity_recent(limit: u64) -> ActivityPage {
    ReadClient::new(read_socket_path()).recent(limit).await
}

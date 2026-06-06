//! Agent activity read command (ai-app.md §2.2, the A4 timeline).
//!
//! Thin wrapper over the shared recent-activity reader in `audit-proto`
//! (`ReadClient::recent`) — the same source the Settings AI activity
//! slice uses, so the tail-seek logic and the frontend entry shape live
//! in one place. Read-only and advisory: a missing or unreachable daemon
//! yields an empty `available = false` page (the reader never errors).

use audit_proto::{read_socket_path, ActivityPage, ReadClient};

/// Read the most recent `limit` audit entries, newest first, for the
/// agent dashboard timeline.
#[tauri::command]
pub async fn ai_activity_recent(limit: u64) -> ActivityPage {
    ReadClient::new(read_socket_path()).recent(limit).await
}

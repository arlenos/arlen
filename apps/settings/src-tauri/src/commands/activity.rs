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

/// What the ledger records about one app, for its own settings page.
///
/// Filtered daemon-side by the kernel-attested actor, so the page states this
/// app's history without being handed anyone else's, and `total` is this app's
/// count rather than the ledger's size.
///
/// The id is not validated here, deliberately, unlike the commands that join it
/// onto a path. It becomes a bound query parameter and nothing else, so a string
/// that is not an app id matches no rows and the honest answer is an empty page -
/// which is also the true answer for an app that has simply never acted. A shape
/// check would refuse some of those inputs and let the rest through to the same
/// result, which is ceremony rather than a boundary.
#[tauri::command]
pub async fn settings_app_audit(app_id: String, limit: u64) -> ActivityPage {
    ReadClient::new(read_socket_path())
        .recent_for_actor(&app_id, limit)
        .await
}

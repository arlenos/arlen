//! What the ledger records about ONE APP, for its own settings page.
//!
//! Thin wrapper over the shared reader in `audit-proto`, so the tail-seek logic
//! and the frontend entry shape live in one place (`audit_proto::activity`). It
//! reads the Structural tier (content-free, never Forensic) over the daemon's
//! read socket.
//!
//! Read-only and advisory: a missing or unreachable daemon yields an empty
//! `available = false` page (the reader never errors), so the page still renders
//! instead of failing - and says which of the two it is, because a record that
//! could not be read is not a record of nothing.
//!
//! THE UNFILTERED READ IS NOT HERE ANY MORE. An `ai_activity_recent` sat beside
//! this until 9 September, for a Settings AI Activity view that does not exist:
//! the AI page's own first paragraph says reviewing what the AI did lives in the
//! harness, one activity home and one config home. The harness has its own copy
//! of that command with its own callers. This one had none, and a second reader
//! of the whole ledger, in the app that decided not to show it, is a door left in
//! a wall nobody uses.

use audit_proto::{read_socket_path, ActivityPage, ReadClient};

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

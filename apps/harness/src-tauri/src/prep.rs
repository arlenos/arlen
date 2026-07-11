//! Prep-for-this (agent-work-surfaces-plan.md surface 3): the ranked context view
//! for an entity, pulled on demand.
//!
//! "Prep me for my 2pm with X" - the harness asks the knowledge daemon to gather
//! everything related to a subject entity and rank it by liveness, so the view
//! leads with what is live-and-important and drops the stale noise. Pure read, no
//! gate; the daemon scopes the result to this app's read scope.

use os_sdk::graph::PrepItem;
use os_sdk::UnixGraphClient;

/// The default number of prep items to request.
const DEFAULT_PREP_LIMIT: i64 = 20;

/// The knowledge graph client, over `ARLEN_DAEMON_SOCKET` (else the default
/// `$XDG_RUNTIME_DIR/arlen/knowledge.sock`).
fn graph_client() -> UnixGraphClient {
    let path = os_sdk::runtime::socket_path("ARLEN_DAEMON_SOCKET", "knowledge.sock");
    UnixGraphClient::new(path.to_string_lossy().into_owned())
}

/// Prep for a subject entity: the ranked related-context view (live-and-important
/// first). `limit` bounds the number of items (default when `None`).
#[tauri::command]
pub async fn prep_for(subject_id: String, limit: Option<i64>) -> Result<Vec<PrepItem>, String> {
    let limit = limit.unwrap_or(DEFAULT_PREP_LIMIT);
    graph_client()
        .prep(&subject_id, limit)
        .await
        .map_err(|e| e.to_string())
}

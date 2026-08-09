//! The timeline's Delete control, wired to the daemon that carries it out.
//!
//! The app's copy says "Removes {range} from the timeline. This cannot be undone",
//! and `bitemporal-knowledge-graph.md` settles what that means against an
//! otherwise close-never-delete graph: a HARD delete. The activity is the user's
//! own data; the audit ledger that keeps the SYSTEM accountable is a different
//! store and is untouched.
//!
//! Nothing is decided here. The daemon owns the destruction, the caller gate and
//! the audit-before-act order; this is the app end of the wire, and its one job is
//! to report the truth back. An error must surface, because the store puts the
//! range back and tells the user their history is still there - a delete that
//! failed while the screen went empty would be the worst lie in the app.

/// Delete everything recorded at or after `from` (Unix seconds).
///
/// Answers how many nodes went, which the surface may use to say "nothing to
/// remove" rather than implying it deleted something. A failure is returned, never
/// swallowed: see the module note.
#[tauri::command]
pub async fn knowledge_timeline_delete(from: i64) -> Result<u64, String> {
    // A negative or zero boundary would mean "everything ever recorded" via a
    // control that names a range, so it is refused here rather than passed on.
    // The daemon would carry it out faithfully; the point is that no range the UI
    // can offer produces it, so it can only arrive from a bug or a caller that is
    // not the timeline.
    if from <= 0 {
        return Err("a delete needs a real range boundary".to_string());
    }
    let socket = os_sdk::runtime::socket_path("ARLEN_KNOWLEDGE_SOCKET", "knowledge.sock");
    let client = os_sdk::graph::UnixGraphClient::new(socket.to_string_lossy().into_owned());
    client.delete_activity(from).await.map_err(|e| e.to_string())
}

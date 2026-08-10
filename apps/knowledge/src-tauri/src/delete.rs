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

/// Is `from` a boundary this command will act on?
///
/// Zero is legitimate and means everything ever recorded - `TimelineView` offers
/// exactly that as "all", alongside "today". An earlier version of this guard
/// refused it on the reasoning that no range the surface could offer would be
/// zero, which was simply false: the surface offers it by name, and the refusal
/// turned the whole "delete all my history" path into the message *"Nothing was
/// deleted. The recorded range is still there."*
///
/// A NEGATIVE boundary is still refused. It cannot come from any control - both
/// ranges are zero or a midnight timestamp - so it can only arrive from a bug or
/// from a caller that is not the timeline, and answering it would delete
/// everything on the strength of a number nobody meant.
fn is_usable_boundary(from: i64) -> bool {
    from >= 0
}

/// Delete everything recorded at or after `from` (Unix seconds).
///
/// Answers how many nodes went, which the surface may use to say "nothing to
/// remove" rather than implying it deleted something. A failure is returned, never
/// swallowed: see the module note.
#[tauri::command]
pub async fn knowledge_timeline_delete(from: i64) -> Result<u64, String> {
    if !is_usable_boundary(from) {
        return Err("a delete needs a range boundary at or after the epoch".to_string());
    }
    let socket = os_sdk::runtime::socket_path("ARLEN_KNOWLEDGE_SOCKET", "knowledge.sock");
    let client = os_sdk::graph::UnixGraphClient::new(socket.to_string_lossy().into_owned());
    client.delete_activity(from).await.map_err(|e| crate::report::graph_call_failed("knowledge_timeline_delete", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The two boundaries the timeline actually offers, pinned so the guard
    /// cannot be re-tightened into refusing one of them. `TimelineView` sets
    /// `from: 0` for "all" and a midnight timestamp for "today"; refusing zero
    /// broke the first of those and reported it to the user as a delete that
    /// simply did not happen.
    #[test]
    fn both_ranges_the_timeline_offers_are_usable() {
        assert!(is_usable_boundary(0), "\"all\" is offered as zero");
        assert!(is_usable_boundary(1_786_000_000), "\"today\" is a midnight stamp");
    }

    #[test]
    fn a_negative_boundary_is_still_refused() {
        // No control produces one, so it is a bug or a stranger, and acting on it
        // would delete everything on the strength of a number nobody meant.
        assert!(!is_usable_boundary(-1));
        assert!(!is_usable_boundary(i64::MIN));
    }
}

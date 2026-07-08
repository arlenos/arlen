//! The `org.arlen.AIAgent1` pull-transparency + undo surface, re-homed from the
//! retired ai-agent onto the engine-daemon (pi-agent-adoption step 9 + the
//! planner's AIAgent1-fork ruling): DROP the approval queue (reversible writes
//! run autonomously under `executor_live`, irreversible ones Confirm via the
//! gate's consent path), KEEP the review-after-the-fact + undo methods. The engine
//! owns `org.arlen.AIAgent1` and serves exactly: status, completed_actions,
//! working_set, action_state, compensate, set_autonomous_app.
//!
//! This module grows one method at a time; each lands only when its engine-side
//! backing is real (no dormant stubs). It is registered on the engine's
//! `org.arlen.AI1` connection only once complete, because the ai-agent still owns
//! the name exclusively (`DoNotQueue`) until it is deleted.

use crate::compensation::CompensationStore;
use serde::Serialize;
use std::sync::{Arc, Mutex};

/// The object path the interface is served at (unchanged from the ai-agent, so
/// existing callers reach the re-homed surface without a path change).
pub const AGENT_OBJECT_PATH: &str = "/org/arlen/AIAgent1";
/// The well-known name the engine owns for the agent surface.
pub const AGENT_BUS_NAME: &str = "org.arlen.AIAgent1";

/// One recently-executed, still-undoable write for the `completed_actions` feed.
/// Content-bounded: the edge written, never node content (the audit subject stays
/// content-free).
#[derive(Debug, Serialize)]
struct CompletedAction {
    /// The decision correlation id: the exact handle `compensate(id)` undoes by,
    /// so the harness's Undo button needs no extra lookup.
    id: String,
    /// The graph write's operation id (the durable retract key).
    op_id: String,
    /// The relation type written, for a quiet done-line.
    relation: String,
    /// The edge source node as `type/id`.
    from: String,
    /// The edge target node as `type/id`.
    to: String,
}

/// Render the completed-actions JSON array from the compensation store, oldest
/// first. Pure and testable; the D-Bus method just locks the store and calls this.
fn completed_actions_json(store: &CompensationStore) -> String {
    let actions: Vec<CompletedAction> = store
        .entries()
        .into_iter()
        .map(|(id, r)| CompletedAction {
            id: id.to_string(),
            op_id: r.op_id.clone(),
            relation: r.relation_type.clone(),
            from: format!("{}/{}", r.from_type, r.from_id),
            to: format!("{}/{}", r.to_type, r.to_id),
        })
        .collect();
    serde_json::to_string(&actions).unwrap_or_else(|_| "[]".to_string())
}

/// The `org.arlen.AIAgent1` interface object. Holds the shared compensation store
/// so `completed_actions` (and, later, `compensate`) see the live executed writes.
pub struct AgentAdminInterface {
    compensation: Arc<Mutex<CompensationStore>>,
}

impl AgentAdminInterface {
    /// Build the interface over the daemon's shared compensation store.
    pub fn new(compensation: Arc<Mutex<CompensationStore>>) -> Self {
        Self { compensation }
    }
}

#[zbus::interface(name = "org.arlen.AIAgent1")]
impl AgentAdminInterface {
    /// The agent's recently-completed actions: the executed (silent-done) writes
    /// retained for the live-session undo path, oldest first, as a JSON array the
    /// harness renders as quiet done-lines each with an `[Undo]`. Each carries the
    /// decision correlation id that `compensate(id)` undoes by. Read-only,
    /// content-bounded, and bounded to the store's horizon (an aged-out action can
    /// neither be listed nor undone). Empty when nothing has executed.
    #[zbus(name = "completed_actions")]
    async fn completed_actions(&self) -> String {
        self.compensation
            .lock()
            .map(|store| completed_actions_json(&store))
            .unwrap_or_else(|_| "[]".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compensation::RetractReceipt;
    use serde_json::Value;

    fn receipt(op: &str) -> RetractReceipt {
        RetractReceipt::for_write(op, "File", "f-1", "Project", "proj-1", "FILE_PART_OF")
    }

    #[test]
    fn completed_actions_render_oldest_first_with_the_undo_handle() {
        let mut store = CompensationStore::new(8);
        store.register("corr-1", receipt("op-1"));
        store.register("corr-2", receipt("op-2"));
        let json = completed_actions_json(&store);
        let v: Value = serde_json::from_str(&json).unwrap();
        let arr = v.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["id"], "corr-1");
        assert_eq!(arr[0]["op_id"], "op-1");
        assert_eq!(arr[0]["relation"], "FILE_PART_OF");
        assert_eq!(arr[0]["from"], "File/f-1");
        assert_eq!(arr[0]["to"], "Project/proj-1");
        assert_eq!(arr[1]["id"], "corr-2");
    }

    #[test]
    fn an_empty_store_renders_an_empty_array() {
        assert_eq!(completed_actions_json(&CompensationStore::new(8)), "[]");
    }
}

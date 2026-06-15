//! Live active-project source for project-scoped read anchoring (GAP-21).
//!
//! Focus Mode owns "the active project": the shell emits `focus.activated`
//! (carrying the project's KG node id) and `focus.deactivated` to the Event
//! Bus. This source tracks the latest, so the query path can anchor a
//! `ProjectScoped` read to the active project's subgraph (the mandatory
//! compile-time `WHERE EXISTS` in `arlen_ai_core::graph_query`) — or refuse
//! the read when no project is active, so a project-scoped session never
//! widens to reading its tier's labels across every project.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use os_sdk::event_consumer::{EventConsumer, UnixEventConsumer};
use os_sdk::proto::FocusActivatedPayload;
use prost::Message;
use tracing::{info, warn};

/// Event-type prefix this source subscribes to: covers both
/// `focus.activated` and `focus.deactivated`.
const FOCUS_NAMESPACE: &str = "focus.";

/// Backoff between Event Bus subscribe attempts. The bus may come up after
/// this daemon, so a failed subscribe is retried rather than fatal.
const SUBSCRIBE_RETRY: Duration = Duration::from_secs(2);

/// The active Focus-Mode project, shared between the bus-listener task and
/// the query path. Cheap to clone (one `Arc`); the default is "no active
/// project", which is the fail-closed state for project-scoped reads.
///
/// Trust boundary: the project id is taken from the `focus.activated`
/// producer on the Event Bus, which today authenticates the UID but not the
/// per-event-type publish right (GAP-17, planned). So the anchor confines a
/// project-scoped read to *a* project the user owns that a co-resident
/// same-UID process can select, not provably *the* genuinely-focused one,
/// until per-producer publish authorization lands. The residual is bounded
/// to the user's own KG data (no cross-user or cross-app-data escalation),
/// and is strictly tighter than the prior state (project-scoped read of
/// every project unconditionally).
#[derive(Clone, Default)]
pub struct ActiveProject {
    current: Arc<Mutex<Option<String>>>,
}

impl ActiveProject {
    /// A source with no active project (the fail-closed default).
    pub fn new() -> Self {
        Self::default()
    }

    /// The active project's KG node id, or `None` when Focus Mode is off.
    pub fn current(&self) -> Option<String> {
        self.current
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    fn set(&self, id: Option<String>) {
        *self.current.lock().unwrap_or_else(|p| p.into_inner()) = id;
    }

    /// Set the active project directly. Test affordance for exercising the
    /// query-path anchor without a live Event Bus; not a public setter, so
    /// only the bus listener can change the live state in production.
    #[cfg(test)]
    pub(crate) fn set_current_for_test(&self, id: Option<String>) {
        self.set(id);
    }

    /// Apply one focus event to the shared cell. Factored out of the bus
    /// loop so it is unit-testable without a live Event Bus; ignores
    /// anything it cannot decode rather than panicking on hostile bytes.
    fn apply(&self, event_type: &str, payload: &[u8]) {
        match event_type {
            "focus.activated" => match FocusActivatedPayload::decode(payload) {
                Ok(p) if !p.project_id.trim().is_empty() => self.set(Some(p.project_id)),
                Ok(_) => warn!("active-project: focus.activated with empty project id"),
                Err(err) => {
                    warn!(%err, "active-project: undecodable focus.activated payload")
                }
            },
            "focus.deactivated" => self.set(None),
            // The `focus.` subscription is a prefix, so a future
            // `focus.*` event lands here harmlessly.
            _ => {}
        }
    }

    /// Subscribe to the `focus.*` Event Bus namespace and keep the active
    /// project in step, forever. Retries the subscribe until it succeeds
    /// and re-subscribes if the feed later closes (the mcp-discovery loop
    /// pattern).
    pub async fn run(self, consumer: UnixEventConsumer) {
        loop {
            let mut rx = match consumer.subscribe(vec![FOCUS_NAMESPACE.to_string()]).await {
                Ok(rx) => rx,
                Err(err) => {
                    warn!(
                        "active-project: focus subscribe failed: {err}; retrying in {}s",
                        SUBSCRIBE_RETRY.as_secs()
                    );
                    tokio::time::sleep(SUBSCRIBE_RETRY).await;
                    continue;
                }
            };
            info!("active-project: subscribed to the focus.* event namespace");
            while let Some(event) = rx.recv().await {
                self.apply(&event.r#type, &event.payload);
            }
            warn!("active-project: focus feed closed; re-subscribing");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activated_sets_then_deactivated_clears() {
        let ap = ActiveProject::new();
        assert_eq!(ap.current(), None);
        let payload = FocusActivatedPayload {
            project_id: "proj-1".into(),
            ..Default::default()
        }
        .encode_to_vec();
        ap.apply("focus.activated", &payload);
        assert_eq!(ap.current().as_deref(), Some("proj-1"));
        ap.apply("focus.deactivated", &[]);
        assert_eq!(ap.current(), None);
    }

    #[test]
    fn empty_or_whitespace_project_id_does_not_set_an_active_project() {
        let ap = ActiveProject::new();
        ap.apply("focus.activated", &FocusActivatedPayload::default().encode_to_vec());
        assert_eq!(ap.current(), None);
        let blank = FocusActivatedPayload {
            project_id: "   ".into(),
            ..Default::default()
        }
        .encode_to_vec();
        ap.apply("focus.activated", &blank);
        assert_eq!(ap.current(), None);
    }

    #[test]
    fn undecodable_payload_is_ignored_not_panicked() {
        let ap = ActiveProject::new();
        ap.apply("focus.activated", &[0xff, 0xff, 0xff]);
        assert_eq!(ap.current(), None);
    }

    #[test]
    fn an_unrelated_event_leaves_the_active_project_unchanged() {
        let ap = ActiveProject::new();
        let payload = FocusActivatedPayload {
            project_id: "p".into(),
            ..Default::default()
        }
        .encode_to_vec();
        ap.apply("focus.activated", &payload);
        ap.apply("window.focused", &[]);
        assert_eq!(ap.current().as_deref(), Some("p"));
    }
}

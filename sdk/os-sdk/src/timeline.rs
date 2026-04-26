/// `shell.timeline` — first-party app surface for writing persistent
/// semantic events to the Knowledge Graph timeline.
///
/// Foundation §468 spec: apps write completed actions (export finished,
/// build succeeded, document saved) so the timeline reflects user-meaningful
/// moments rather than raw eBPF noise. Timeline differs from Presence in
/// two ways: (1) timeline events are persistent, presence is ephemeral,
/// (2) timeline supports point-in-time *and* duration events via
/// `started_at` / `ended_at`.
///
/// Routes through the Event Bus: `record` emits `app.timeline.record`.
/// The Knowledge Daemon promotes the event into a UserAction node with
/// category `"timeline"`, action `<type>`, subject `<label>`. Metadata
/// stays in the SQLite event log so the graph stays lightweight.

use std::collections::HashMap;
use std::future::Future;

use prost::Message;
use serde::{Deserialize, Serialize};

use crate::event::{EmitError, EventEmitter};
use crate::proto::TimelineRecordPayload;

/// Parameters for [`Timeline::record`].
///
/// Mirrors the foundation §468 surface. Either `started_at` *and*
/// `ended_at` are set (duration event) or both are `None` (point-in-time).
/// Mixing — only `started_at` set without `ended_at` — is treated as
/// "started at this moment, ongoing"; the daemon uses `started_at` as
/// the timeline timestamp in that case.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelineParams {
    /// User-facing summary, e.g. `"Exported PDF"`. Surfaces in the
    /// `~/.timeline/` FUSE view and AI queries.
    pub label: String,
    /// Subject of the event — typically a file path, project name, or
    /// URL. Becomes the `subject` field on the resulting UserAction.
    pub subject: String,
    /// App-defined category like `"export"`, `"build"`, `"deploy"`,
    /// `"save"`. Becomes the `action` field on the resulting UserAction.
    pub r#type: String,
    /// Microseconds since Unix epoch. `None` for a point-in-time event.
    pub started_at: Option<i64>,
    /// Microseconds since Unix epoch. `None` for a point-in-time event.
    pub ended_at: Option<i64>,
    /// Free-form structured metadata. Stays in the SQLite event log.
    pub metadata: HashMap<String, String>,
}

/// Surface for the `shell.timeline` API.
///
/// Construct with [`Timeline::new`] passing an [`EventEmitter`] and the
/// app's identifier. Generic over the emitter for testability.
pub struct Timeline<E: EventEmitter> {
    emitter: E,
    app_id: String,
}

impl<E: EventEmitter> Timeline<E> {
    /// Create a new timeline surface bound to a specific emitter and app.
    pub fn new(emitter: E, app_id: impl Into<String>) -> Self {
        Self {
            emitter,
            app_id: app_id.into(),
        }
    }

    /// Record a completed semantic event.
    ///
    /// # Errors
    /// Returns [`EmitError`] if the emitter cannot reach the Event Bus.
    /// Protobuf encoding cannot fail for our schema.
    pub fn record(
        &self,
        params: TimelineParams,
    ) -> impl Future<Output = Result<(), EmitError>> + Send + '_ {
        let payload = TimelineRecordPayload {
            app_id: self.app_id.clone(),
            label: params.label,
            subject: params.subject,
            r#type: params.r#type,
            started_at: params.started_at.unwrap_or(0),
            ended_at: params.ended_at.unwrap_or(0),
            metadata: params.metadata,
        };
        let mut buf = Vec::with_capacity(payload.encoded_len());
        payload
            .encode(&mut buf)
            .expect("TimelineRecordPayload encode is infallible");
        async move { self.emitter.emit("app.timeline.record", buf).await }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::MockEventEmitter;

    fn decode(bytes: &[u8]) -> TimelineRecordPayload {
        TimelineRecordPayload::decode(bytes).expect("valid TimelineRecordPayload")
    }

    #[tokio::test]
    async fn record_duration_event() {
        let emitter = MockEventEmitter::new();
        let timeline = Timeline::new(emitter.clone(), "com.example.builder");

        timeline
            .record(TimelineParams {
                label: "Build succeeded".into(),
                subject: "coffeeshop".into(),
                r#type: "build".into(),
                started_at: Some(1_000_000),
                ended_at: Some(5_500_000),
                metadata: HashMap::from([("target".into(), "release".into())]),
            })
            .await
            .unwrap();

        let events = emitter.emitted().await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "app.timeline.record");

        let p = decode(&events[0].payload);
        assert_eq!(p.app_id, "com.example.builder");
        assert_eq!(p.label, "Build succeeded");
        assert_eq!(p.subject, "coffeeshop");
        assert_eq!(p.r#type, "build");
        assert_eq!(p.started_at, 1_000_000);
        assert_eq!(p.ended_at, 5_500_000);
        assert_eq!(p.metadata.get("target"), Some(&"release".to_string()));
    }

    #[tokio::test]
    async fn record_point_in_time_event() {
        let emitter = MockEventEmitter::new();
        let timeline = Timeline::new(emitter.clone(), "com.example.editor");

        timeline
            .record(TimelineParams {
                label: "Exported PDF".into(),
                subject: "/home/tim/report.pdf".into(),
                r#type: "export".into(),
                started_at: None,
                ended_at: None,
                metadata: HashMap::new(),
            })
            .await
            .unwrap();

        let p = decode(&emitter.emitted().await[0].payload);
        // Both timestamps absent — daemon falls back to envelope
        // timestamp. We round-trip the literal zero across the wire.
        assert_eq!(p.started_at, 0);
        assert_eq!(p.ended_at, 0);
        assert_eq!(p.label, "Exported PDF");
    }
}

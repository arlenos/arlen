/// `shell.presence` — first-party app surface for declaring the user's
/// current ephemeral activity.
///
/// Foundation §354 spec: apps describe what the user is *currently* doing
/// (editing, reading, building) so the Knowledge Graph gets semantically
/// precise UserAction nodes without relying on eBPF inference. Presence is
/// ephemeral: when the app loses focus or the user moves on, presence is
/// cleared. This is distinct from `shell.timeline.record`, which writes
/// completed (persistent) events.
///
/// The implementation routes through the Event Bus: `set` emits
/// `app.presence.set`, `clear` emits `app.presence.clear`. The Knowledge
/// Daemon's promotion task picks them up and creates UserAction graph nodes.
/// Apps never write to the graph directly — that path is reserved for the
/// daemon and requires capability tokens.
///
/// For Tauri apps that want auto-clear-on-blur semantics (per spec), the
/// recommended pattern is to wire the auto-clear path in the TypeScript
/// layer using Tauri's window-blur event. The Rust API only emits the
/// events; orchestration of *when* to clear is the consumer's choice.

use std::collections::HashMap;
use std::future::Future;

use prost::Message;
use serde::{Deserialize, Serialize};

use crate::event::{EmitError, EventEmitter};
use crate::proto::{PresenceClearPayload, PresenceSetPayload};

/// Auto-clear policy for a presence record.
///
/// The Rust SDK does not enforce auto-clear itself — it just stores the
/// hint in the emitted event. Consumers (typically the TypeScript shell-
/// API helper) wire the actual clear trigger.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AutoClear {
    /// Clear automatically when the app's window loses focus.
    OnBlur,
    /// Clear automatically when the user is detected idle.
    OnIdle,
    /// Caller must call `clear()` explicitly.
    Manual,
}

impl AutoClear {
    fn as_proto_str(self) -> &'static str {
        match self {
            Self::OnBlur => "on-blur",
            Self::OnIdle => "on-idle",
            Self::Manual => "",
        }
    }
}

/// Parameters for [`Presence::set`].
///
/// Mirrors the foundation §354 surface. `activity` and `subject` are
/// the only required fields: project inherits from Focus Mode if empty,
/// metadata defaults to empty, auto_clear defaults to `Manual`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresenceParams {
    /// Activity verb. Spec recommends one of "editing", "reading",
    /// "reviewing", "building", but custom values are accepted so apps
    /// can describe domain-specific verbs.
    pub activity: String,
    /// Free-form subject — typically a file path, document name, or URL.
    pub subject: String,
    /// Optional project context. Empty inherits Focus Mode's active
    /// project (resolved on the daemon side).
    pub project: Option<String>,
    /// Optional structured metadata. Free-form key/value pairs that
    /// stay in the SQLite event log; the graph node only carries
    /// activity + subject.
    pub metadata: HashMap<String, String>,
    /// Auto-clear policy. Defaults to `Manual` when omitted.
    pub auto_clear: Option<AutoClear>,
}

/// Surface for the `shell.presence` API.
///
/// Construct with [`Presence::new`] passing an [`EventEmitter`] and the
/// app's identifier. The app_id is included in every emitted event so
/// the daemon can attribute presence to the right app.
///
/// `Presence` is generic over the emitter type so tests can use
/// `MockEventEmitter` and production can use `UnixEventEmitter`.
pub struct Presence<E: EventEmitter> {
    emitter: E,
    app_id: String,
}

impl<E: EventEmitter> Presence<E> {
    /// Create a new presence surface bound to a specific emitter and app.
    pub fn new(emitter: E, app_id: impl Into<String>) -> Self {
        Self {
            emitter,
            app_id: app_id.into(),
        }
    }

    /// Declare the user's current activity. Replaces any previous
    /// presence the app had set.
    ///
    /// # Errors
    /// Returns [`EmitError`] if the emitter cannot reach the Event Bus
    /// or the payload cannot be serialised. The protobuf encoder is
    /// infallible for our schema, so in practice errors are connection
    /// failures.
    pub fn set(&self, params: PresenceParams) -> impl Future<Output = Result<(), EmitError>> + Send + '_ {
        let payload = PresenceSetPayload {
            app_id: self.app_id.clone(),
            activity: params.activity,
            subject: params.subject,
            project: params.project.unwrap_or_default(),
            auto_clear: params
                .auto_clear
                .unwrap_or(AutoClear::Manual)
                .as_proto_str()
                .to_string(),
            metadata: params.metadata,
        };
        let mut buf = Vec::with_capacity(payload.encoded_len());
        // Encoding into an exactly-sized Vec cannot fail: prost only
        // returns an error on `encode` for buffer-too-small, which
        // doesn't apply here.
        payload
            .encode(&mut buf)
            .expect("PresenceSetPayload encode is infallible into a sized Vec");
        async move { self.emitter.emit("app.presence.set", buf).await }
    }

    /// Clear the app's presence. Idempotent — safe to call when no
    /// presence is set.
    pub fn clear(&self) -> impl Future<Output = Result<(), EmitError>> + Send + '_ {
        let payload = PresenceClearPayload {
            app_id: self.app_id.clone(),
        };
        let mut buf = Vec::with_capacity(payload.encoded_len());
        payload
            .encode(&mut buf)
            .expect("PresenceClearPayload encode is infallible");
        async move { self.emitter.emit("app.presence.clear", buf).await }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::MockEventEmitter;

    fn decode_set(bytes: &[u8]) -> PresenceSetPayload {
        PresenceSetPayload::decode(bytes).expect("valid PresenceSetPayload")
    }

    fn decode_clear(bytes: &[u8]) -> PresenceClearPayload {
        PresenceClearPayload::decode(bytes).expect("valid PresenceClearPayload")
    }

    #[tokio::test]
    async fn set_emits_event_with_app_id_and_fields() {
        let emitter = MockEventEmitter::new();
        let presence = Presence::new(emitter.clone(), "com.example.editor");

        presence
            .set(PresenceParams {
                activity: "editing".into(),
                subject: "/home/tim/notes.md".into(),
                project: Some("coffeeshop".into()),
                metadata: HashMap::from([("language".into(), "rust".into())]),
                auto_clear: Some(AutoClear::OnBlur),
            })
            .await
            .unwrap();

        let events = emitter.emitted().await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "app.presence.set");

        let p = decode_set(&events[0].payload);
        assert_eq!(p.app_id, "com.example.editor");
        assert_eq!(p.activity, "editing");
        assert_eq!(p.subject, "/home/tim/notes.md");
        assert_eq!(p.project, "coffeeshop");
        assert_eq!(p.auto_clear, "on-blur");
        assert_eq!(p.metadata.get("language"), Some(&"rust".to_string()));
    }

    #[tokio::test]
    async fn set_default_auto_clear_is_manual_serialised_as_empty() {
        let emitter = MockEventEmitter::new();
        let presence = Presence::new(emitter.clone(), "com.example.app");

        presence
            .set(PresenceParams {
                activity: "reading".into(),
                subject: "doc.md".into(),
                project: None,
                metadata: HashMap::new(),
                auto_clear: None,
            })
            .await
            .unwrap();

        let p = decode_set(&emitter.emitted().await[0].payload);
        // Manual is the daemon-side absence sentinel — empty string in
        // the proto wire format. Round-tripping the literal string lets
        // the daemon distinguish "never auto-clear" from "auto-clear on
        // blur" without an extra wrapper type.
        assert_eq!(p.auto_clear, "");
    }

    #[tokio::test]
    async fn clear_emits_event_with_only_app_id() {
        let emitter = MockEventEmitter::new();
        let presence = Presence::new(emitter.clone(), "com.example.editor");

        presence.clear().await.unwrap();

        let events = emitter.emitted().await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "app.presence.clear");
        assert_eq!(decode_clear(&events[0].payload).app_id, "com.example.editor");
    }

    #[tokio::test]
    async fn empty_project_is_serialised_as_empty_string_for_focus_mode_inheritance() {
        // Per foundation §354: an absent project inherits Focus Mode.
        // The proto wire format uses empty string as the absence
        // sentinel; the daemon resolves it.
        let emitter = MockEventEmitter::new();
        let presence = Presence::new(emitter.clone(), "com.example.app");

        presence
            .set(PresenceParams {
                activity: "editing".into(),
                subject: "x".into(),
                project: None,
                metadata: HashMap::new(),
                auto_clear: None,
            })
            .await
            .unwrap();

        assert_eq!(decode_set(&emitter.emitted().await[0].payload).project, "");
    }
}

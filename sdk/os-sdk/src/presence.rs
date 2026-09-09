//! `shell.presence` — first-party app surface for declaring the user's
//! current ephemeral activity.
//!
//! Foundation §354 spec: apps describe what the user is *currently* doing
//! (editing, reading, building) so the Knowledge Graph gets semantically
//! precise UserAction nodes without relying on eBPF inference. Presence is
//! ephemeral: when the app loses focus or the user moves on, presence is
//! cleared. This is distinct from `shell.timeline.record`, which writes
//! completed (persistent) events.
//!
//! The implementation routes through the Event Bus: `set` emits
//! `app.presence.set`, `clear` emits `app.presence.clear`. The Knowledge
//! Daemon's promotion task picks them up and creates UserAction graph nodes.
//! Apps never write to the graph directly — that path is reserved for the
//! daemon and requires capability tokens.
//!
//! For Tauri apps that want auto-clear-on-blur semantics (per spec), the
//! recommended pattern is to wire the auto-clear path in the TypeScript
//! layer using Tauri's window-blur event. The Rust API only emits the
//! events; orchestration of *when* to clear is the consumer's choice.

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
///
/// THOSE DEFAULTS ARE REAL NOW. They were written here as prose and not as
/// `#[serde(default)]`, so this struct in fact required all five - and the TS
/// wrapper declares three of them optional. A frontend sending the documented
/// minimum got a deserialize error out of `presence_set`, which every app in the
/// tree discards, because a window that cannot reach the bus is still a window.
/// The file manager published `browsing` with no metadata, nothing arrived, and
/// nothing anywhere said why; a drive watching the wire is what found it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresenceParams {
    /// Activity verb. Spec recommends one of "editing", "reading",
    /// "reviewing", "building", but custom values are accepted so apps
    /// can describe domain-specific verbs.
    pub activity: String,
    /// Free-form subject — typically a file path, document name, or URL.
    pub subject: String,
    /// Optional project context.
    ///
    /// NOT RESOLVED AND NOT STORED, and this line said the opposite until
    /// 9 September: "Empty inherits Focus Mode's active project (resolved on the
    /// daemon side)". The knowledge daemon does not consume `focus.activated` at
    /// all and has no notion of an active project, and `UserAction` has no
    /// project column for one to land in - `promote_presence_set` reads
    /// `activity` and `subject` and nothing else. So a value set here rides along
    /// into the SQLite event row with the metadata and reaches no graph node,
    /// and an empty one inherits nothing.
    ///
    /// What the old sentence would need: the daemon subscribing to the focus
    /// events the shell already publishes, a project field on the node, and a
    /// decision about which wins when an app names one AND a focus is active.
    /// Until then this is context in the event log rather than a link.
    ///
    /// `#[serde(default)]`, and it was missing until 9 September. Serde requires
    /// every field it is not told to default - `Option` included - so a caller
    /// that left this out got a deserialize error rather than a `None`, which is
    /// the opposite of what the three doc lines here promise.
    #[serde(default)]
    pub project: Option<String>,
    /// Optional structured metadata. Free-form key/value pairs that
    /// stay in the SQLite event log; the graph node only carries
    /// activity + subject.
    #[serde(default)]
    pub metadata: HashMap<String, String>,
    /// Auto-clear policy. Defaults to `Manual` when omitted.
    #[serde(default)]
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

    /// THE JSON A WEBVIEW SENDS, which is the shape nothing tested.
    ///
    /// Every other test here builds the struct in Rust and so never crosses the
    /// deserialize boundary the plugin command actually sits on. The TS wrapper
    /// declares `project`, `metadata` and `auto_clear` optional and the doc above
    /// promises defaults for all three; serde required them anyway.
    #[test]
    fn the_documented_minimum_deserialises() {
        let minimum = r#"{"activity":"browsing","subject":"/home/tim/Documents"}"#;
        let p: PresenceParams = serde_json::from_str(minimum).expect("the documented minimum");
        assert_eq!(p.activity, "browsing");
        assert_eq!(p.project, None);
        assert!(p.metadata.is_empty());
        assert!(p.auto_clear.is_none());
    }

    /// And the shape the apps that DO carry context send, so the default does not
    /// quietly swallow a real value.
    #[test]
    fn a_full_payload_still_deserialises() {
        let full = r#"{"activity":"editing","subject":"/a.rs","metadata":{"language":"rust"},"auto_clear":"on-blur"}"#;
        let p: PresenceParams = serde_json::from_str(full).expect("a full payload");
        assert_eq!(p.metadata.get("language").map(String::as_str), Some("rust"));
        assert!(matches!(p.auto_clear, Some(AutoClear::OnBlur)));
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

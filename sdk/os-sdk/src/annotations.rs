/// `shell.annotations` — first-party app surface for attaching
/// structured per-app metadata to existing Knowledge Graph nodes.
///
/// Foundation §395: applications attach data within their own
/// namespace. Re-setting on the same `(target, namespace)` triple
/// replaces the previous value. The Knowledge Daemon promotes
/// `app.annotation.set` and `app.annotation.cleared` Event Bus
/// events into Annotation graph nodes keyed by a deterministic
/// UUIDv5 derived from the triple.
///
/// Reads (`get`) go through the [`GraphClient`] using a Cypher
/// match keyed on the same triple. Cross-namespace reads are
/// permitted by this SDK layer today; the daemon-side enforcement
/// follows the capability-token Phase 3.2 milestone. First-party
/// apps that read other apps' namespaces should declare the
/// permission in their manifest so the future hardening is not a
/// breaking change.
///
/// Subscriptions (`onChanged` per spec) are deferred — they need an
/// Event Bus consumer in os-sdk which only has a producer today.

use std::collections::HashMap;
use std::future::Future;

use prost::Message;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::event::{EmitError, EventEmitter};
use crate::event_consumer::{EventConsumer, SubscribeError};
use crate::graph::{GraphClient, QueryError};
use crate::proto::{AnnotationClearPayload, AnnotationSetPayload};

/// Fixed UUIDv5 namespace for deriving deterministic Annotation ids.
/// Must match `knowledge::promotion::ANNOTATION_UUID_NAMESPACE`
/// byte-for-byte — both sides hash the same triple to the same id so
/// SDK reads find what the daemon wrote. Changing one without the
/// other orphans annotations on the next set.
const ANNOTATION_UUID_NAMESPACE: Uuid = Uuid::from_bytes([
    0x6e, 0xed, 0x73, 0x05, 0xc4, 0x83, 0x4d, 0x73, 0xa6, 0x86, 0xc1, 0x73, 0x4d, 0xb1, 0x29, 0x7e,
]);

/// Derive the deterministic annotation id from the composite identity.
/// Mirror of the daemon-side derivation; documented as part of the
/// wire contract.
fn annotation_id(target_type: &str, target_id: &str, namespace: &str) -> Uuid {
    let key = format!("{target_type}\x1f{target_id}\x1f{namespace}");
    Uuid::new_v5(&ANNOTATION_UUID_NAMESPACE, key.as_bytes())
}

/// What the annotation is attached to. Mirrors foundation §403's
/// `target: { type, ... }` shape — each variant carries the
/// minimal identifier the graph node uses for that type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AnnotationTarget {
    /// Attach to a File node, identified by absolute path.
    File { path: String },
    /// Attach to an App node, identified by reverse-domain id.
    App { id: String },
    /// Attach to a Project node, identified by UUID string.
    Project { id: String },
    /// Attach to a Session node.
    Session { id: String },
}

impl AnnotationTarget {
    fn target_type(&self) -> &'static str {
        match self {
            Self::File { .. } => "File",
            Self::App { .. } => "App",
            Self::Project { .. } => "Project",
            Self::Session { .. } => "Session",
        }
    }

    fn target_id(&self) -> &str {
        match self {
            Self::File { path } => path,
            Self::App { id } | Self::Project { id } | Self::Session { id } => id,
        }
    }
}

/// Parameters for [`Annotations::set`] — attach or replace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnnotationSetParams {
    pub target: AnnotationTarget,
    /// Reverse-domain namespace, conventionally matching the app id.
    pub namespace: String,
    /// Opaque app-defined data. Encoded to JSON on the wire.
    pub data: serde_json::Value,
}

/// Parameters for [`Annotations::get`] and [`Annotations::clear`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnnotationLookup {
    pub target: AnnotationTarget,
    pub namespace: String,
}

/// A retrieved annotation row.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnnotationRecord {
    pub data: serde_json::Value,
    /// Microseconds since Unix epoch.
    pub created_at: i64,
    /// Microseconds since Unix epoch.
    pub last_modified: i64,
}

/// Surface for the `shell.annotations` API.
///
/// Generic over the emitter and graph client for testability.
pub struct Annotations<E: EventEmitter, G: GraphClient> {
    emitter: E,
    graph: G,
    app_id: String,
}

impl<E: EventEmitter, G: GraphClient> Annotations<E, G> {
    pub fn new(emitter: E, graph: G, app_id: impl Into<String>) -> Self {
        Self {
            emitter,
            graph,
            app_id: app_id.into(),
        }
    }

    /// Attach or replace an annotation. Re-setting on the same
    /// (target, namespace) triple updates `data` and `last_modified`
    /// and preserves `created_at`.
    ///
    /// # Errors
    /// [`EmitError`] when the Event Bus is unreachable. JSON encoding
    /// of `data` cannot fail because `serde_json::Value` is always
    /// representable.
    pub fn set(
        &self,
        params: AnnotationSetParams,
    ) -> impl Future<Output = Result<(), EmitError>> + Send + '_ {
        let payload = AnnotationSetPayload {
            app_id: self.app_id.clone(),
            namespace: params.namespace,
            target_type: params.target.target_type().to_string(),
            target_id: params.target.target_id().to_string(),
            data_json: params.data.to_string(),
        };
        let mut buf = Vec::with_capacity(payload.encoded_len());
        payload
            .encode(&mut buf)
            .expect("AnnotationSetPayload encode is infallible");
        async move { self.emitter.emit("app.annotation.set", buf).await }
    }

    /// Remove an annotation. Idempotent: clearing a missing annotation
    /// is silently a no-op (the daemon's MATCH+DELETE affects zero
    /// rows).
    pub fn clear(
        &self,
        lookup: AnnotationLookup,
    ) -> impl Future<Output = Result<(), EmitError>> + Send + '_ {
        let payload = AnnotationClearPayload {
            app_id: self.app_id.clone(),
            namespace: lookup.namespace,
            target_type: lookup.target.target_type().to_string(),
            target_id: lookup.target.target_id().to_string(),
        };
        let mut buf = Vec::with_capacity(payload.encoded_len());
        payload
            .encode(&mut buf)
            .expect("AnnotationClearPayload encode is infallible");
        async move { self.emitter.emit("app.annotation.cleared", buf).await }
    }

    /// Fetch the current annotation for `(target, namespace)`.
    ///
    /// Returns `None` when no annotation matches. Uses the same
    /// deterministic-id derivation as the daemon, so this is a
    /// single-row primary-key lookup rather than a property scan.
    ///
    /// # Cross-namespace reads
    ///
    /// Reading another app's namespace is permitted by the SDK today
    /// (the daemon's read-only Cypher socket has no per-namespace
    /// authorisation yet). Apps that intend to read foreign namespaces
    /// must declare them in their manifest under
    /// `permissions.graph.annotations_read_cross_namespace` — see
    /// `sdk/permissions::GraphPermissions::can_read_annotations_from`.
    /// Declaring now avoids a breaking change when the Phase 3.2-full
    /// token-authenticated write path lands and the daemon starts
    /// enforcing.
    ///
    /// # Errors
    /// [`QueryError`] from the underlying graph client. Malformed JSON
    /// in the returned `data` field is reported as
    /// [`QueryError::InvalidQuery`] — the daemon should never store
    /// non-JSON, but defensively we surface it rather than panic.
    pub fn get(
        &self,
        lookup: AnnotationLookup,
    ) -> impl Future<Output = Result<Option<AnnotationRecord>, QueryError>> + Send + '_ {
        let id = annotation_id(
            lookup.target.target_type(),
            lookup.target.target_id(),
            &lookup.namespace,
        );
        // The id is a UUID string with no special characters that need
        // escaping for Cypher; using `replace` defensively for `'`.
        let id_str = id.to_string().replace('\'', "");
        let cypher = format!(
            "MATCH (a:Annotation {{id: '{id_str}'}}) \
             RETURN a.data AS data, a.created_at AS created_at, \
             a.last_modified AS last_modified"
        );

        async move {
            let rows = self.graph.query(&cypher, HashMap::new()).await?;
            let Some(row) = rows.into_iter().next() else {
                return Ok(None);
            };
            let data_str = row
                .get("data")
                .and_then(|v| v.as_str())
                .unwrap_or("null");
            let data: serde_json::Value = serde_json::from_str(data_str)
                .map_err(|e| QueryError::InvalidQuery(format!("annotation data not JSON: {e}")))?;
            let created_at = row
                .get("created_at")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let last_modified = row
                .get("last_modified")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            Ok(Some(AnnotationRecord {
                data,
                created_at,
                last_modified,
            }))
        }
    }
}

/// One side of an annotation change observed via [`Annotations::on_changed`].
///
/// Serialised tagged-union form on the wire (`{ kind: "set", ...}`
/// / `{ kind: "cleared", ... }`) so Tauri-plugin frontends can
/// pattern-match cleanly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum AnnotationChange {
    /// The annotation was set or replaced. `data` is the new payload.
    Set {
        target: AnnotationTarget,
        namespace: String,
        app_id: String,
        data: serde_json::Value,
    },
    /// The annotation was cleared.
    Cleared {
        target: AnnotationTarget,
        namespace: String,
        app_id: String,
    },
}

/// Abort-on-drop guard for a forwarder task. Holding this keeps
/// the subscription alive; dropping it aborts the forwarder and
/// (transitively) closes the upstream bus connection.
pub struct AbortOnDrop(tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// RAII handle for a live `on_changed` subscription.
///
/// Drop the handle to unsubscribe — the internal forwarder task
/// is aborted and the underlying Event Bus connection drops,
/// causing the bus registry to release the consumer entry.
///
/// For consumers (e.g. the Tauri plugin) that want to store the
/// abort handle separately from the receiver, use [`Self::split`].
pub struct Subscription {
    abort_on_drop: AbortOnDrop,
    rx: mpsc::Receiver<AnnotationChange>,
}

impl Subscription {
    /// Receive the next matching change. Returns `None` when the
    /// subscription has ended (forwarder exited or bus closed
    /// unrecoverably).
    pub async fn recv(&mut self) -> Option<AnnotationChange> {
        self.rx.recv().await
    }

    /// Split the subscription into an abort-on-drop guard and the
    /// underlying receiver. Hold both halves to keep the
    /// subscription alive; drop the guard to unsubscribe even if
    /// the receiver is still being drained elsewhere.
    pub fn split(self) -> (AbortOnDrop, mpsc::Receiver<AnnotationChange>) {
        (self.abort_on_drop, self.rx)
    }
}

impl<E: EventEmitter, G: GraphClient> Annotations<E, G> {
    /// Subscribe to annotation changes for `(target, namespace)`.
    ///
    /// The returned [`Subscription`] yields [`AnnotationChange`]
    /// values matching the filter. Drop the handle to unsubscribe.
    ///
    /// Implementation: registers as a fresh Event Bus consumer
    /// with prefix `app.annotation.`, decodes each event's
    /// payload as either [`AnnotationSetPayload`] or
    /// [`AnnotationClearPayload`], and forwards only those whose
    /// `(target_type, target_id, namespace)` match.
    ///
    /// # Snapshot semantics
    ///
    /// Subscribers see future events only. Callers that need the
    /// current state should call [`Self::get`] first; there is a
    /// small race window between the two calls (FA8 in
    /// `docs/architecture/annotations-api.md`).
    ///
    /// # Errors
    /// [`SubscribeError::ConnectionFailed`] if the bus is
    /// unreachable after the initial-connect retry budget.
    pub async fn on_changed<C: EventConsumer + 'static>(
        &self,
        consumer: &C,
        target: AnnotationTarget,
        namespace: String,
    ) -> Result<Subscription, SubscribeError> {
        let raw_rx = consumer
            .subscribe(vec!["app.annotation.".to_string()])
            .await?;

        let (tx, rx) = mpsc::channel::<AnnotationChange>(64);
        let target_type = target.target_type().to_string();
        let target_id = target.target_id().to_string();
        let namespace_filter = namespace;

        let task = tokio::spawn(async move {
            let mut raw_rx = raw_rx;
            while let Some(event) = raw_rx.recv().await {
                let Some(change) = decode_change(
                    &event,
                    &target_type,
                    &target_id,
                    &namespace_filter,
                ) else {
                    continue;
                };
                if tx.send(change).await.is_err() {
                    return; // caller dropped the receiver
                }
            }
        });

        Ok(Subscription {
            abort_on_drop: AbortOnDrop(task),
            rx,
        })
    }
}

/// Decode the bus event into an [`AnnotationChange`] iff it
/// matches the target+namespace filter. Returns `None` for
/// non-matching or undecodable events (E11, E12).
fn decode_change(
    event: &crate::proto::Event,
    target_type: &str,
    target_id: &str,
    namespace: &str,
) -> Option<AnnotationChange> {
    match event.r#type.as_str() {
        "app.annotation.set" => {
            let p = AnnotationSetPayload::decode(event.payload.as_slice()).ok()?;
            if p.target_type != target_type
                || p.target_id != target_id
                || p.namespace != namespace
            {
                return None;
            }
            let data = serde_json::from_str(&p.data_json).unwrap_or(serde_json::Value::Null);
            Some(AnnotationChange::Set {
                target: rebuild_target(&p.target_type, p.target_id),
                namespace: p.namespace,
                app_id: p.app_id,
                data,
            })
        }
        "app.annotation.cleared" => {
            let p = AnnotationClearPayload::decode(event.payload.as_slice()).ok()?;
            if p.target_type != target_type
                || p.target_id != target_id
                || p.namespace != namespace
            {
                return None;
            }
            Some(AnnotationChange::Cleared {
                target: rebuild_target(&p.target_type, p.target_id),
                namespace: p.namespace,
                app_id: p.app_id,
            })
        }
        _ => None,
    }
}

/// Reconstruct an [`AnnotationTarget`] from the wire pair. Falls
/// back to `App { id }` for unknown target types so the caller
/// always gets a structured value (E12 strict-by-default; future
/// types degrade gracefully into App).
fn rebuild_target(target_type: &str, target_id: String) -> AnnotationTarget {
    match target_type {
        "File" => AnnotationTarget::File { path: target_id },
        "App" => AnnotationTarget::App { id: target_id },
        "Project" => AnnotationTarget::Project { id: target_id },
        "Session" => AnnotationTarget::Session { id: target_id },
        _ => AnnotationTarget::App { id: target_id },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::{MockEventConsumer, MockEventEmitter, MockGraphClient};

    fn decode_set(bytes: &[u8]) -> AnnotationSetPayload {
        AnnotationSetPayload::decode(bytes).expect("valid AnnotationSetPayload")
    }

    fn decode_clear(bytes: &[u8]) -> AnnotationClearPayload {
        AnnotationClearPayload::decode(bytes).expect("valid AnnotationClearPayload")
    }

    #[tokio::test]
    async fn set_emits_event_with_target_and_namespace() {
        let emitter = MockEventEmitter::new();
        let graph = MockGraphClient::new();
        let ann = Annotations::new(emitter.clone(), graph, "com.example.editor");

        ann.set(AnnotationSetParams {
            target: AnnotationTarget::File {
                path: "/home/tim/notes.md".into(),
            },
            namespace: "com.example.editor".into(),
            data: serde_json::json!({"word_count": 1240}),
        })
        .await
        .unwrap();

        let events = emitter.emitted().await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "app.annotation.set");

        let p = decode_set(&events[0].payload);
        assert_eq!(p.app_id, "com.example.editor");
        assert_eq!(p.namespace, "com.example.editor");
        assert_eq!(p.target_type, "File");
        assert_eq!(p.target_id, "/home/tim/notes.md");
        assert_eq!(p.data_json, r#"{"word_count":1240}"#);
    }

    #[tokio::test]
    async fn clear_emits_event_with_target_only() {
        let emitter = MockEventEmitter::new();
        let graph = MockGraphClient::new();
        let ann = Annotations::new(emitter.clone(), graph, "com.example.editor");

        ann.clear(AnnotationLookup {
            target: AnnotationTarget::Project {
                id: "550e8400-e29b-41d4-a716-446655440000".into(),
            },
            namespace: "com.example.editor".into(),
        })
        .await
        .unwrap();

        let events = emitter.emitted().await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "app.annotation.cleared");

        let p = decode_clear(&events[0].payload);
        assert_eq!(p.target_type, "Project");
        assert_eq!(p.target_id, "550e8400-e29b-41d4-a716-446655440000");
    }

    #[tokio::test]
    async fn get_returns_none_when_no_match() {
        let emitter = MockEventEmitter::new();
        let graph = MockGraphClient::new();
        let ann = Annotations::new(emitter, graph, "com.example.editor");

        let result = ann
            .get(AnnotationLookup {
                target: AnnotationTarget::File {
                    path: "/missing".into(),
                },
                namespace: "com.example.editor".into(),
            })
            .await
            .unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn get_decodes_data_and_timestamps() {
        let emitter = MockEventEmitter::new();
        // The mock matches queries by exact string, so we have to
        // build the same query the SDK constructs. The id is the
        // deterministic UUIDv5 derivation.
        let id = annotation_id("File", "/x.md", "com.example.editor");
        let id_str = id.to_string();
        let cypher = format!(
            "MATCH (a:Annotation {{id: '{id_str}'}}) \
             RETURN a.data AS data, a.created_at AS created_at, \
             a.last_modified AS last_modified"
        );
        let mut row = HashMap::new();
        row.insert("data".to_string(), serde_json::json!(r#"{"k":"v"}"#));
        row.insert("created_at".to_string(), serde_json::json!(100i64));
        row.insert("last_modified".to_string(), serde_json::json!(200i64));
        let graph = MockGraphClient::new().with_response(cypher, vec![row]);

        let ann = Annotations::new(emitter, graph, "com.example.editor");
        let got = ann
            .get(AnnotationLookup {
                target: AnnotationTarget::File {
                    path: "/x.md".into(),
                },
                namespace: "com.example.editor".into(),
            })
            .await
            .unwrap()
            .expect("annotation should be returned");
        assert_eq!(got.data, serde_json::json!({"k": "v"}));
        assert_eq!(got.created_at, 100);
        assert_eq!(got.last_modified, 200);
    }

    #[test]
    fn annotation_id_is_stable_across_invocations() {
        // Wire-contract guarantee: SDK and daemon must derive the same
        // id from the same triple. Both sides reuse this exact bytes-
        // for-bytes namespace UUID; if either drifts, set + get
        // disagree.
        let a = annotation_id("File", "/x", "com.app");
        let b = annotation_id("File", "/x", "com.app");
        assert_eq!(a, b);
        let c = annotation_id("File", "/y", "com.app");
        assert_ne!(a, c);
    }

    #[test]
    fn target_variants_serialise_to_correct_strings() {
        assert_eq!(
            AnnotationTarget::File {
                path: "/x".into()
            }
            .target_type(),
            "File"
        );
        assert_eq!(AnnotationTarget::App { id: "y".into() }.target_type(), "App");
        assert_eq!(
            AnnotationTarget::Project { id: "z".into() }.target_type(),
            "Project"
        );
        assert_eq!(
            AnnotationTarget::Session { id: "s".into() }.target_type(),
            "Session"
        );
    }

    /// Helper: build an `app.annotation.set` Event envelope with
    /// the given payload fields. Used by on_changed tests.
    fn build_set_event(
        app_id: &str,
        namespace: &str,
        target_type: &str,
        target_id: &str,
        data_json: &str,
    ) -> crate::proto::Event {
        let payload = AnnotationSetPayload {
            app_id: app_id.into(),
            namespace: namespace.into(),
            target_type: target_type.into(),
            target_id: target_id.into(),
            data_json: data_json.into(),
        };
        crate::proto::Event {
            id: "evt".into(),
            r#type: "app.annotation.set".into(),
            timestamp: 1,
            source: format!("app:{app_id}"),
            pid: 0,
            session_id: "s".into(),
            payload: payload.encode_to_vec(),
            uid: 1000,
            project_id: String::new(),
        }
    }

    fn build_clear_event(
        app_id: &str,
        namespace: &str,
        target_type: &str,
        target_id: &str,
    ) -> crate::proto::Event {
        let payload = AnnotationClearPayload {
            app_id: app_id.into(),
            namespace: namespace.into(),
            target_type: target_type.into(),
            target_id: target_id.into(),
        };
        crate::proto::Event {
            id: "evt-c".into(),
            r#type: "app.annotation.cleared".into(),
            timestamp: 2,
            source: format!("app:{app_id}"),
            pid: 0,
            session_id: "s".into(),
            payload: payload.encode_to_vec(),
            uid: 1000,
            project_id: String::new(),
        }
    }

    #[tokio::test]
    async fn on_changed_yields_matching_set_event() {
        let bus = MockEventConsumer::new();
        let ann = Annotations::new(
            MockEventEmitter::new(),
            MockGraphClient::new(),
            "com.example.editor",
        );
        let mut sub = ann
            .on_changed(
                &bus,
                AnnotationTarget::File {
                    path: "/notes.md".into(),
                },
                "com.example.editor".into(),
            )
            .await
            .unwrap();

        // Allow forwarder spawn to register on broadcast.
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        bus.push(build_set_event(
            "com.example.editor",
            "com.example.editor",
            "File",
            "/notes.md",
            r#"{"word_count":1240}"#,
        ));

        let change = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            sub.recv(),
        )
        .await
        .expect("recv in time")
        .expect("change");
        match change {
            AnnotationChange::Set {
                target,
                namespace,
                data,
                ..
            } => {
                assert_eq!(
                    target,
                    AnnotationTarget::File {
                        path: "/notes.md".into()
                    }
                );
                assert_eq!(namespace, "com.example.editor");
                assert_eq!(data, serde_json::json!({"word_count": 1240}));
            }
            other => panic!("expected Set, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn on_changed_filters_by_target_and_namespace() {
        let bus = MockEventConsumer::new();
        let ann = Annotations::new(
            MockEventEmitter::new(),
            MockGraphClient::new(),
            "com.example.editor",
        );
        let mut sub = ann
            .on_changed(
                &bus,
                AnnotationTarget::File {
                    path: "/wanted.md".into(),
                },
                "com.example.editor".into(),
            )
            .await
            .unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        // Wrong target id — must be filtered.
        bus.push(build_set_event(
            "com.example.editor",
            "com.example.editor",
            "File",
            "/other.md",
            r#"{}"#,
        ));
        // Wrong namespace — must be filtered.
        bus.push(build_set_event(
            "com.other.app",
            "com.other.app",
            "File",
            "/wanted.md",
            r#"{}"#,
        ));
        // Matching event — must arrive.
        bus.push(build_set_event(
            "com.example.editor",
            "com.example.editor",
            "File",
            "/wanted.md",
            r#"{"good":true}"#,
        ));

        let change = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            sub.recv(),
        )
        .await
        .expect("recv")
        .expect("change");
        match change {
            AnnotationChange::Set { data, .. } => {
                assert_eq!(data, serde_json::json!({"good": true}));
            }
            other => panic!("expected Set, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn on_changed_yields_clear_events() {
        let bus = MockEventConsumer::new();
        let ann = Annotations::new(
            MockEventEmitter::new(),
            MockGraphClient::new(),
            "com.example.editor",
        );
        let mut sub = ann
            .on_changed(
                &bus,
                AnnotationTarget::Project {
                    id: "550e8400-e29b-41d4-a716-446655440000".into(),
                },
                "com.example.editor".into(),
            )
            .await
            .unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        bus.push(build_clear_event(
            "com.example.editor",
            "com.example.editor",
            "Project",
            "550e8400-e29b-41d4-a716-446655440000",
        ));

        let change = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            sub.recv(),
        )
        .await
        .expect("recv")
        .expect("change");
        assert!(matches!(change, AnnotationChange::Cleared { .. }));
    }

    /// Regression for Sprint-A8 / Codex adversarial review
    /// finding 1 (listener-registration race).
    ///
    /// The Tauri plugin's two-step subscribe protocol relies on
    /// the SDK-side property that events arriving between
    /// `Subscription::split()` and the receiver being drained
    /// are buffered in the mpsc, not lost. This test simulates
    /// the Pending phase: hold the receiver without polling for
    /// 100 ms while events flow, then drain — and verify all
    /// events surface in order.
    #[tokio::test]
    async fn split_receiver_buffers_events_until_drained() {
        let bus = MockEventConsumer::new();
        let ann = Annotations::new(
            MockEventEmitter::new(),
            MockGraphClient::new(),
            "com.example.editor",
        );
        let sub = ann
            .on_changed(
                &bus,
                AnnotationTarget::File {
                    path: "/buffered.md".into(),
                },
                "com.example.editor".into(),
            )
            .await
            .unwrap();
        let (_abort, mut rx) = sub.split();

        // Allow forwarder to register on broadcast.
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        // Push three matching events while NOTHING is draining.
        // This stands in for the prepare-to-start window in the
        // Tauri plugin: events accumulate in the rx buffer while
        // no pump task exists yet.
        for i in 0..3 {
            bus.push(build_set_event(
                "com.example.editor",
                "com.example.editor",
                "File",
                "/buffered.md",
                &format!(r#"{{"i":{i}}}"#),
            ));
        }

        // Simulate the IPC roundtrip + listen() registration
        // delay before the pump starts draining.
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Drain in order — none lost, FIFO preserved.
        for i in 0..3 {
            let change = tokio::time::timeout(
                std::time::Duration::from_secs(1),
                rx.recv(),
            )
            .await
            .expect("recv in time")
            .expect("change");
            match change {
                AnnotationChange::Set { data, .. } => {
                    assert_eq!(data, serde_json::json!({"i": i}));
                }
                other => panic!("expected Set, got {other:?}"),
            }
        }
    }

    #[tokio::test]
    async fn drop_subscription_unsubscribes() {
        let bus = MockEventConsumer::new();
        let ann = Annotations::new(
            MockEventEmitter::new(),
            MockGraphClient::new(),
            "com.example.editor",
        );
        let sub = ann
            .on_changed(
                &bus,
                AnnotationTarget::File {
                    path: "/x".into(),
                },
                "com.example.editor".into(),
            )
            .await
            .unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        assert_eq!(bus.subscriber_count(), 1);

        drop(sub);
        // Nudge the mock-internal forwarder with a *matching*
        // event so it actually attempts to send into the now-
        // dropped mpsc (which triggers its exit and the broadcast
        // unsubscription). A non-matching default event would be
        // filtered by the mock and not exercise the send path.
        bus.push(build_set_event(
            "ignored",
            "ignored",
            "File",
            "/x",
            r#"{}"#,
        ));
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert_eq!(bus.subscriber_count(), 0);
    }
}

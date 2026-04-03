/// Mock implementations of the `os-sdk` traits for use in tests.
///
/// Import with `use os_sdk::mock::*` in test modules.
/// The mocks are compile-time verified against the real implementations:
/// any interface change that breaks a mock will fail compilation.
use crate::event::{EmitError, EventEmitter};
use crate::graph::{GraphClient, QueryError};
use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};

type ResponseMap = Mutex<HashMap<String, Vec<HashMap<String, serde_json::Value>>>>;

/// A recorded emission captured by [`MockEventEmitter`].
#[derive(Debug, Clone)]
pub struct EmittedEvent {
    /// The event type string, e.g. `file.opened`.
    pub event_type: String,
    /// The raw protobuf payload bytes.
    pub payload: Vec<u8>,
}

/// Mock [`EventEmitter`] that records all emitted events in memory.
///
/// Use [`MockEventEmitter::emitted`] to inspect what was emitted after
/// running the code under test.
///
/// # Example
/// ```
/// use os_sdk::mock::MockEventEmitter;
/// use os_sdk::event::EventEmitter;
///
/// #[tokio::test]
/// async fn my_test() {
///     let emitter = MockEventEmitter::new();
///     emitter.emit("file.opened", vec![]).await.unwrap();
///
///     let events = emitter.emitted().await;
///     assert_eq!(events.len(), 1);
///     assert_eq!(events[0].event_type, "file.opened");
/// }
/// ```
#[derive(Clone, Default)]
pub struct MockEventEmitter {
    events: Arc<Mutex<Vec<EmittedEvent>>>,
}

impl MockEventEmitter {
    /// Create a new mock emitter with an empty event log.
    pub fn new() -> Self {
        Self::default()
    }

    /// Return all events emitted so far, in order.
    pub async fn emitted(&self) -> Vec<EmittedEvent> {
        self.events.lock().unwrap().clone()
    }

    /// Return the number of events emitted so far.
    pub async fn emit_count(&self) -> usize {
        self.events.lock().unwrap().len()
    }

    /// Clear the event log.
    pub async fn reset(&self) {
        self.events.lock().unwrap().clear();
    }
}

impl EventEmitter for MockEventEmitter {
    #[allow(clippy::manual_async_fn)]
    fn emit<'a>(
        &'a self,
        event_type: &'a str,
        payload: Vec<u8>,
    ) -> impl Future<Output = Result<(), EmitError>> + Send + 'a {
        async move {
            self.events.lock().unwrap().push(EmittedEvent {
                event_type: event_type.to_string(),
                payload,
            });
            Ok(())
        }
    }
}

/// Mock [`GraphClient`] that returns predefined responses for known queries.
///
/// Queries not in the response map return an empty result set.
/// Use [`MockGraphClient::with_response`] to register expected responses.
///
/// # Example
/// ```
/// use os_sdk::mock::MockGraphClient;
/// use os_sdk::graph::GraphClient;
/// use std::collections::HashMap;
///
/// #[tokio::test]
/// async fn my_test() {
///     let client = MockGraphClient::new()
///         .with_response(
///             "MATCH (f:File) RETURN f.path LIMIT 1",
///             vec![HashMap::from([
///                 ("f.path".to_string(), serde_json::json!("/home/tim/report.md")),
///             ])],
///         );
///
///     let rows = client
///         .query("MATCH (f:File) RETURN f.path LIMIT 1", HashMap::new())
///         .await
///         .unwrap();
///
///     assert_eq!(rows.len(), 1);
/// }
/// ```
#[derive(Clone, Default)]
pub struct MockGraphClient {
    #[allow(clippy::type_complexity)]
    responses: Arc<ResponseMap>,
}

impl MockGraphClient {
    /// Create a new mock client with no predefined responses.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a response for a specific Cypher query string.
    ///
    /// The match is exact: the query must match character-for-character.
    pub fn with_response(
        self,
        query: impl Into<String>,
        response: Vec<HashMap<String, serde_json::Value>>,
    ) -> Self {
        // We need a blocking lock here since this is called in synchronous context.
        // This is safe because we are building the mock before the async runtime
        // starts using it.
        self.responses
            .lock().unwrap().insert(query.into(), response);
        self
    }
}

impl GraphClient for MockGraphClient {
    #[allow(clippy::manual_async_fn)]
    fn query<'a>(
        &'a self,
        cypher: &'a str,
        _params: HashMap<String, serde_json::Value>,
    ) -> impl Future<Output = Result<Vec<HashMap<String, serde_json::Value>>, QueryError>> + Send + 'a
    {
        async move {
            let responses = self.responses.lock().unwrap();
            Ok(responses.get(cypher).cloned().unwrap_or_default())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::EventEmitter;
    use crate::graph::GraphClient;

    #[tokio::test]
    async fn mock_emitter_records_events() {
        let emitter = MockEventEmitter::new();
        emitter.emit("file.opened", vec![1, 2, 3]).await.unwrap();
        emitter.emit("window.focused", vec![]).await.unwrap();

        let events = emitter.emitted().await;
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].event_type, "file.opened");
        assert_eq!(events[0].payload, vec![1, 2, 3]);
        assert_eq!(events[1].event_type, "window.focused");
    }

    #[tokio::test]
    async fn mock_emitter_reset_clears_log() {
        let emitter = MockEventEmitter::new();
        emitter.emit("file.opened", vec![]).await.unwrap();
        emitter.reset().await;
        assert_eq!(emitter.emit_count().await, 0);
    }

    #[tokio::test]
    async fn mock_graph_client_returns_registered_response() {
        let client = MockGraphClient::new().with_response(
            "MATCH (f:File) RETURN f.path LIMIT 1",
            vec![HashMap::from([(
                "f.path".to_string(),
                serde_json::json!("/home/tim/report.md"),
            )])],
        );

        let rows = client
            .query("MATCH (f:File) RETURN f.path LIMIT 1", HashMap::new())
            .await
            .unwrap();

        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].get("f.path").unwrap(),
            &serde_json::json!("/home/tim/report.md")
        );
    }

    #[tokio::test]
    async fn mock_graph_client_returns_empty_for_unknown_query() {
        let client = MockGraphClient::new();
        let rows = client
            .query("MATCH (n) RETURN n", HashMap::new())
            .await
            .unwrap();
        assert!(rows.is_empty());
    }
}

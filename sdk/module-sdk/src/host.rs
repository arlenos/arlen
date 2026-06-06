/// Host imports surfaced to module authors.
///
/// On the WASM side these functions are linked at component-load time
/// against the `lunaris:host/*` interfaces defined in `wit/host.wit`;
/// on the native side (running tests in a host crate, no WASM) they
/// route through a thread-local mock that lets tests pre-program
/// answers. The signatures match between both paths so the same
/// module source compiles natively for unit tests and to WASM for
/// production.
///
/// The capability gating happens in `lunaris-modulesd`. Every host
/// import returns a typed `HostError` on denial; modules are expected
/// to handle that gracefully.

use thiserror::Error;

#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum HostError {
    #[error("permission denied: {0}")]
    Denied(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("network: {0}")]
    Network(String),
    #[error("internal: {0}")]
    Internal(String),
}

pub mod graph {
    use super::HostError;

    /// Read-only Cypher query against the Knowledge Graph. The host
    /// extracts the namespace from the query and matches it against
    /// the module's `graph.read` allowlist; mismatched queries return
    /// `HostError::Denied`.
    ///
    /// The shape of `rows` is JSON. Modules deserialise it with
    /// `serde_json` per their query.
    pub fn query(_cypher: &str) -> Result<String, HostError> {
        #[cfg(target_arch = "wasm32")]
        {
            // WASM path: link against lunaris:host/graph#query. The
            // wit-bindgen-generated stub goes here once S5 wires it
            // up alongside the Currency-Konverter dogfood. Until
            // then this is a deliberate compile-time placeholder
            // that fails closed.
            Err(HostError::Internal(
                "module-sdk: host/graph not linked yet (S5)".into(),
            ))
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            super::native_mock::graph_query(_cypher)
        }
    }

    /// Write to the Knowledge Graph. Same gating rules as `query`.
    pub fn write(_cypher: &str) -> Result<String, HostError> {
        #[cfg(target_arch = "wasm32")]
        {
            Err(HostError::Internal(
                "module-sdk: host/graph not linked yet (S5)".into(),
            ))
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            super::native_mock::graph_write(_cypher)
        }
    }
}

pub mod network {
    use super::HostError;

    /// HTTP GET. Body is returned raw. Capability: `network.allow`.
    pub fn fetch(_url: &str) -> Result<Vec<u8>, HostError> {
        #[cfg(target_arch = "wasm32")]
        {
            Err(HostError::Internal(
                "module-sdk: host/network not linked yet (S5)".into(),
            ))
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            super::native_mock::network_fetch(_url)
        }
    }
}

pub mod events {
    use super::HostError;

    /// Emit an event to the Lunaris Event Bus. Capability:
    /// `event_bus.publish` allowlist on the event type prefix.
    pub fn emit(_event_type: &str, _payload: &[u8]) -> Result<(), HostError> {
        #[cfg(target_arch = "wasm32")]
        {
            Err(HostError::Internal(
                "module-sdk: host/events not linked yet (S5)".into(),
            ))
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            super::native_mock::events_emit(_event_type, _payload)
        }
    }
}

pub mod log {
    /// Emit an info-level log line. Always allowed.
    pub fn info(message: &str) {
        #[cfg(target_arch = "wasm32")]
        {
            // wit-bindgen stub goes here in S5.
            let _ = message;
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            super::native_mock::log_info(message);
        }
    }
}

/// Native test mock. Lets tests pre-program responses for host
/// imports so module logic can be unit-tested without spinning up a
/// daemon. WASM builds skip this whole module.
#[cfg(not(target_arch = "wasm32"))]
pub mod native_mock {
    use super::HostError;
    use std::cell::RefCell;
    use std::collections::HashMap;

    thread_local! {
        static GRAPH_RESPONSES: RefCell<HashMap<String, Result<String, HostError>>> =
            RefCell::new(HashMap::new());
        static NETWORK_RESPONSES: RefCell<HashMap<String, Result<Vec<u8>, HostError>>> =
            RefCell::new(HashMap::new());
        static LOG_LINES: RefCell<Vec<String>> = RefCell::new(Vec::new());
    }

    pub fn set_graph_response(query: &str, response: Result<String, HostError>) {
        GRAPH_RESPONSES.with(|m| {
            m.borrow_mut().insert(query.to_string(), response);
        });
    }

    pub fn set_network_response(url: &str, response: Result<Vec<u8>, HostError>) {
        NETWORK_RESPONSES.with(|m| {
            m.borrow_mut().insert(url.to_string(), response);
        });
    }

    pub(super) fn graph_query(cypher: &str) -> Result<String, HostError> {
        GRAPH_RESPONSES.with(|m| {
            m.borrow()
                .get(cypher)
                .cloned()
                .unwrap_or_else(|| Err(HostError::NotFound(cypher.to_string())))
        })
    }

    pub(super) fn graph_write(cypher: &str) -> Result<String, HostError> {
        GRAPH_RESPONSES.with(|m| {
            m.borrow()
                .get(cypher)
                .cloned()
                .unwrap_or_else(|| Err(HostError::NotFound(cypher.to_string())))
        })
    }

    pub(super) fn network_fetch(url: &str) -> Result<Vec<u8>, HostError> {
        NETWORK_RESPONSES.with(|m| {
            m.borrow()
                .get(url)
                .cloned()
                .unwrap_or_else(|| Err(HostError::NotFound(url.to_string())))
        })
    }

    pub(super) fn events_emit(_event_type: &str, _payload: &[u8]) -> Result<(), HostError> {
        Ok(())
    }

    pub(super) fn log_info(message: &str) {
        LOG_LINES.with(|m| m.borrow_mut().push(message.to_string()));
    }

    /// Inspect logged lines. Useful for assertions in tests.
    pub fn captured_log_lines() -> Vec<String> {
        LOG_LINES.with(|m| m.borrow().clone())
    }

    /// Reset all mock state. Call at the start of each test so
    /// thread-local state from previous tests does not leak in.
    pub fn reset() {
        GRAPH_RESPONSES.with(|m| m.borrow_mut().clear());
        NETWORK_RESPONSES.with(|m| m.borrow_mut().clear());
        LOG_LINES.with(|m| m.borrow_mut().clear());
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graph_query_returns_mocked_response() {
        native_mock::reset();
        native_mock::set_graph_response(
            "MATCH (n) RETURN n",
            Ok("[{\"n\": 1}]".into()),
        );
        let rows = graph::query("MATCH (n) RETURN n").unwrap();
        assert_eq!(rows, "[{\"n\": 1}]");
    }

    #[test]
    fn graph_query_unmocked_returns_not_found() {
        native_mock::reset();
        let err = graph::query("MATCH (x) RETURN x").unwrap_err();
        assert!(matches!(err, HostError::NotFound(_)));
    }

    #[test]
    fn network_fetch_returns_mocked_body() {
        native_mock::reset();
        native_mock::set_network_response(
            "https://api.example.com/x",
            Ok(b"hello".to_vec()),
        );
        let body = network::fetch("https://api.example.com/x").unwrap();
        assert_eq!(body, b"hello");
    }

    #[test]
    fn network_fetch_unmocked_returns_not_found() {
        native_mock::reset();
        let err = network::fetch("https://nope.example.com").unwrap_err();
        assert!(matches!(err, HostError::NotFound(_)));
    }

    #[test]
    fn log_captures_lines() {
        native_mock::reset();
        log::info("module: hello");
        log::info("module: world");
        let lines = native_mock::captured_log_lines();
        assert_eq!(lines, vec!["module: hello", "module: world"]);
    }

    #[test]
    fn events_emit_is_silent_success() {
        native_mock::reset();
        events::emit("test.event", b"{}").unwrap();
    }
}

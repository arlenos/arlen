//! Knowledge-graph adapter for the read executor.
//!
//! Bridges the os-sdk [`UnixGraphClient`] (which speaks the Knowledge Daemon's
//! Unix-socket Cypher protocol) onto the ai-core [`GraphQuerier`] trait the
//! [`CypherPipeline`](arlen_ai_core::pipeline::CypherPipeline) depends on, so the
//! Phase-1 [`GraphReadExecutor`](crate::read_executor::GraphReadExecutor) can run
//! a bounded read against the real graph.
//!
//! This mirrors the ai-daemon's adapter and is kept daemon-local on purpose:
//! ai-core deliberately does not depend on os-sdk (the bridge is the daemon's,
//! not core logic), and os-sdk cannot depend on ai-core (the trait lives there),
//! so neither crate is the right home and the thin glue lives in each daemon.
//! The os-sdk `GraphClient` trait uses return-position `impl Trait` (not object-
//! safe); `GraphQuerier` uses `async_trait`, so the pipeline holds it behind an
//! `Arc<dyn _>`. This adapter is the glue between the two.

use arlen_ai_core::pipeline::{GraphQuerier, GraphQueryError, GraphRow};
use async_trait::async_trait;
use os_sdk::graph::{GraphClient, QueryError, UnixGraphClient};
use std::collections::HashMap;

/// A [`GraphQuerier`] backed by the os-sdk Unix-socket graph client, pointed at
/// the Knowledge Daemon socket.
pub struct OsSdkGraphQuerier {
    client: UnixGraphClient,
}

impl OsSdkGraphQuerier {
    /// Build an adapter pointing at the Knowledge Daemon socket. Construction is
    /// lazy (the client dials per query), so this never blocks on the daemon
    /// being up.
    pub fn new(socket_path: impl Into<String>) -> Self {
        Self { client: UnixGraphClient::new(socket_path) }
    }
}

#[async_trait]
impl GraphQuerier for OsSdkGraphQuerier {
    async fn run(&self, cypher: &str) -> Result<Vec<GraphRow>, GraphQueryError> {
        self.client.query(cypher, HashMap::new()).await.map_err(|err| match err {
            QueryError::ConnectionFailed(msg) => GraphQueryError::Unreachable(msg),
            QueryError::InvalidQuery(msg) => GraphQueryError::Rejected(msg),
            QueryError::PermissionDenied => {
                GraphQueryError::Rejected("permission denied".to_string())
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_adapter_constructs_without_dialing() {
        // Construction is lazy: pointing the adapter at a socket that does not
        // exist must not panic or block (the client dials only on a query).
        let _ = OsSdkGraphQuerier::new("/nonexistent/arlen/knowledge.sock");
    }
}

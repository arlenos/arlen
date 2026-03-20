use std::future::Future;
use std::collections::HashMap;

/// Executes read-only Cypher queries against the Graph Daemon.
/// Implemented by the real Unix socket client and a mock for tests.
pub trait GraphClient: Send + Sync {
    fn query<'a>(
        &'a self,
        cypher: &'a str,
        params: HashMap<String, serde_json::Value>,
    ) -> impl Future<Output = Result<Vec<HashMap<String, serde_json::Value>>, QueryError>> + Send + 'a;
}

#[derive(Debug)]
pub enum QueryError {
    ConnectionFailed(String),
    InvalidQuery(String),
    PermissionDenied,
}

impl std::fmt::Display for QueryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            QueryError::ConnectionFailed(msg) => write!(f, "connection failed: {msg}"),
            QueryError::InvalidQuery(msg) => write!(f, "invalid query: {msg}"),
            QueryError::PermissionDenied => write!(f, "permission denied"),
        }
    }
}

impl std::error::Error for QueryError {}

use std::collections::HashMap;
use std::future::Future;
use std::path::Path;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::sync::Mutex;

/// Error type for graph query failures.
#[derive(Debug)]
pub enum QueryError {
    /// The connection to the Graph Daemon could not be established or was lost.
    ConnectionFailed(String),
    /// The Cypher query was rejected by the daemon (syntax error or write attempt).
    InvalidQuery(String),
    /// The caller does not have permission to access the requested data.
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

/// Executes read-only Cypher queries against the Lunaris Knowledge Graph.
///
/// Implemented by [`UnixGraphClient`] for production use and by
/// [`crate::mock::MockGraphClient`] for testing.
pub trait GraphClient: Send + Sync {
    /// Execute a read-only Cypher query and return the results.
    ///
    /// Each row in the result is a map of column name to JSON value.
    /// Write queries (CREATE, MERGE, DELETE, SET, REMOVE, DROP) are rejected
    /// by the daemon with [`QueryError::InvalidQuery`].
    ///
    /// # Errors
    /// Returns [`QueryError::ConnectionFailed`] if the Knowledge Daemon is unreachable.
    /// Returns [`QueryError::InvalidQuery`] if the query is malformed or a write query.
    /// Returns [`QueryError::PermissionDenied`] if the capability token is insufficient.
    fn query<'a>(
        &'a self,
        cypher: &'a str,
        params: HashMap<String, serde_json::Value>,
    ) -> impl Future<Output = Result<Vec<HashMap<String, serde_json::Value>>, QueryError>> + Send + 'a;
}

/// Production [`GraphClient`] that queries the Knowledge Graph over a Unix socket.
///
/// Connects lazily on first query and reconnects automatically if the connection
/// is lost. Thread-safe: clone freely across async tasks.
///
/// # Example
/// ```no_run
/// use os_sdk::graph::{GraphClient, UnixGraphClient};
/// use std::collections::HashMap;
///
/// #[tokio::main]
/// async fn main() {
///     let client = UnixGraphClient::new("/run/lunaris/knowledge.sock");
///     let rows = client
///         .query("MATCH (f:File) RETURN f.path LIMIT 10", HashMap::new())
///         .await
///         .unwrap();
///     for row in rows {
///         println!("{row:?}");
///     }
/// }
/// ```
#[derive(Clone)]
pub struct UnixGraphClient {
    socket_path: String,
    stream: Arc<Mutex<Option<UnixStream>>>,
}

impl UnixGraphClient {
    /// Create a new client that will connect to the given socket path.
    ///
    /// Does not connect immediately; the connection is established on the
    /// first call to [`query`](GraphClient::query).
    pub fn new(socket_path: impl Into<String>) -> Self {
        Self {
            socket_path: socket_path.into(),
            stream: Arc::new(Mutex::new(None)),
        }
    }
}

impl GraphClient for UnixGraphClient {
    #[allow(clippy::manual_async_fn)]
    fn query<'a>(
        &'a self,
        cypher: &'a str,
        _params: HashMap<String, serde_json::Value>,
    ) -> impl Future<Output = Result<Vec<HashMap<String, serde_json::Value>>, QueryError>> + Send + 'a
    {
        async move {
            // For Phase 1A params are not yet used; the daemon accepts raw Cypher only.
            // Parameterized queries will be added in Phase 2 with capability tokens.
            let query_bytes = cypher.as_bytes();
            let len = u32::try_from(query_bytes.len())
                .map_err(|e| QueryError::InvalidQuery(e.to_string()))?
                .to_be_bytes();

            let mut guard = self.stream.lock().await;

            for attempt in 0..2u8 {
                if guard.is_none() {
                    match UnixStream::connect(Path::new(&self.socket_path)).await {
                        Ok(s) => *guard = Some(s),
                        Err(e) => {
                            return Err(QueryError::ConnectionFailed(e.to_string()));
                        }
                    }
                }

                let stream = guard.as_mut().expect("just connected");

                let result = async {
                    stream.write_all(&len).await?;
                    stream.write_all(query_bytes).await?;

                    // Read response
                    let mut resp_len_buf = [0u8; 4];
                    stream.read_exact(&mut resp_len_buf).await?;
                    let resp_len = u32::from_be_bytes(resp_len_buf) as usize;

                    let mut resp_buf = vec![0u8; resp_len];
                    stream.read_exact(&mut resp_buf).await?;

                    Ok::<_, std::io::Error>(String::from_utf8_lossy(&resp_buf).to_string())
                }
                .await;

                match result {
                    Ok(response) => {
                        if response.starts_with("ERROR:") {
                            let msg = response.trim_start_matches("ERROR:").trim().to_string();
                            if msg.contains("permission") {
                                return Err(QueryError::PermissionDenied);
                            }
                            return Err(QueryError::InvalidQuery(msg));
                        }
                        // For Phase 1A the daemon returns a raw string result.
                        // We wrap it as a single row with a "result" column.
                        // Phase 2 will add structured JSON responses.
                        let row =
                            HashMap::from([("result".to_string(), serde_json::Value::String(response))]);
                        return Ok(vec![row]);
                    }
                    Err(_) if attempt == 0 => {
                        *guard = None;
                    }
                    Err(e) => {
                        *guard = None;
                        return Err(QueryError::ConnectionFailed(e.to_string()));
                    }
                }
            }

            Err(QueryError::ConnectionFailed("failed after reconnect".to_string()))
        }
    }
}

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

/// Largest response frame the client will allocate for the legacy text path.
/// Generous, because a display query can legitimately return a large blob;
/// the limit only exists so a buggy or compromised daemon cannot exhaust
/// client memory by advertising a multi-gigabyte frame.
const MAX_RESPONSE_BYTES: usize = 64 * 1024 * 1024;

/// Largest response frame the client will allocate for the typed path. Tighter
/// than [`MAX_RESPONSE_BYTES`] and matched to the daemon's own typed payload
/// cap (a few MiB of cell content plus JSON structure), so a bounded frame
/// cannot be parsed into a much larger tree of values.
const MAX_TYPED_RESPONSE_BYTES: usize = 16 * 1024 * 1024;

/// Largest column count the client will accept in a typed result, matching the
/// daemon's own cap. Bounds work before any per-row map is built.
const MAX_TYPED_COLUMNS: usize = 256;

/// Largest row count the client will accept in a typed result, matching the
/// daemon's own cap. Without this an under-frame-limit body of many empty rows
/// could still be amplified into one map allocation per row.
const MAX_TYPED_ROWS: usize = 10_000;

/// Largest length, in bytes, of a single column name. Graph column names and
/// aliases are short identifiers, so this is generous for real queries; the
/// cap exists because each cell clones its column name into the row map, so an
/// uncapped name length multiplied by the cell budget would be a key-clone
/// amplification vector. With this and [`MAX_TYPED_CELLS`] the total cloned
/// key bytes are bounded.
const MAX_TYPED_COLUMN_NAME_BYTES: usize = 128;

/// Largest total cell count (`columns x rows`) the client will materialize.
/// The row and column caps alone permit a daemon-legal worst case of
/// `10_000 x 256` cells, which fits well under the frame cap yet would build
/// millions of map entries. This bound matches the daemon's own typed payload
/// budget (a 4 MiB cost cap at a minimum 8 bytes per cell), so a well-behaved
/// daemon is never rejected while a malicious one cannot amplify a bounded
/// frame into a large heap of per-cell allocations.
const MAX_TYPED_CELLS: usize = 4 * 1024 * 1024 / 8;

/// Internal result of a single framed round trip, kept separate from
/// [`QueryError`] so the retry loop can distinguish a fatal framing error (do
/// not retry, the socket is desynchronised) from a transient I/O error.
enum FrameError {
    /// The daemon advertised a response larger than [`MAX_RESPONSE_BYTES`].
    Oversized(usize),
    /// A socket read or write failed.
    Io(std::io::Error),
}

impl From<std::io::Error> for FrameError {
    fn from(e: std::io::Error) -> Self {
        FrameError::Io(e)
    }
}

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

    /// Send a length-framed request body and read the length-framed response
    /// as **raw bytes**, reconnecting once on a dropped connection. Shared by
    /// the text and typed-row query paths.
    ///
    /// The response length the daemon advertises is rejected before any
    /// allocation if it exceeds `max_bytes`, so a buggy or compromised daemon
    /// cannot OOM the caller with an oversized frame. The caller passes the
    /// limit appropriate to its path (tighter for typed rows than for display
    /// text). Decoding (lossy for display text, strict for typed rows) is left
    /// to the caller.
    async fn round_trip(&self, body: &[u8], max_bytes: usize) -> Result<Vec<u8>, QueryError> {
        let len = u32::try_from(body.len())
            .map_err(|e| QueryError::InvalidQuery(e.to_string()))?
            .to_be_bytes();

        let mut guard = self.stream.lock().await;
        for attempt in 0..2u8 {
            // Take the stream OUT of the mutex before any I/O. If this future
            // is cancelled mid-round-trip (e.g. a caller timeout) after the
            // request is written but before the response is read, the
            // half-used stream is dropped rather than left cached with a
            // pending response that the next query would misread as its own.
            let mut stream = match guard.take() {
                Some(s) => s,
                None => match UnixStream::connect(Path::new(&self.socket_path)).await {
                    Ok(s) => s,
                    Err(e) => return Err(QueryError::ConnectionFailed(e.to_string())),
                },
            };
            let result = async {
                stream.write_all(&len).await?;
                stream.write_all(body).await?;
                let mut resp_len_buf = [0u8; 4];
                stream.read_exact(&mut resp_len_buf).await?;
                let resp_len = u32::from_be_bytes(resp_len_buf) as usize;
                if resp_len > max_bytes {
                    // Reject before allocating. This is a framing/trust error,
                    // not a transient I/O error, so do not retry: the socket
                    // is desynchronised (the oversized body is still pending).
                    return Err(FrameError::Oversized(resp_len));
                }
                let mut resp_buf = vec![0u8; resp_len];
                stream
                    .read_exact(&mut resp_buf)
                    .await
                    .map_err(FrameError::Io)?;
                Ok::<_, FrameError>(resp_buf)
            }
            .await;

            match result {
                // Cache the stream again only after a complete round trip.
                Ok(response) => {
                    *guard = Some(stream);
                    return Ok(response);
                }
                // An oversized frame is fatal: the stream is dropped (not
                // cached) and we do not retry it as a connection blip.
                Err(FrameError::Oversized(n)) => {
                    return Err(QueryError::InvalidQuery(format!(
                        "daemon response of {n} bytes exceeds the {max_bytes}-byte limit"
                    )));
                }
                // On an I/O error the stream is dropped (not cached); retry once.
                Err(FrameError::Io(_)) if attempt == 0 => {}
                Err(FrameError::Io(e)) => return Err(QueryError::ConnectionFailed(e.to_string())),
            }
        }
        Err(QueryError::ConnectionFailed("failed after reconnect".to_string()))
    }

    /// Map a daemon `ERROR:` response to a [`QueryError`].
    fn check_error(response: &str) -> Result<(), QueryError> {
        if let Some(rest) = response.strip_prefix("ERROR:") {
            let msg = rest.trim().to_string();
            if msg.contains("permission") {
                return Err(QueryError::PermissionDenied);
            }
            return Err(QueryError::InvalidQuery(msg));
        }
        Ok(())
    }

    /// Execute a read-only Cypher query and return **typed** rows.
    ///
    /// Unlike [`GraphClient::query`], which surfaces the daemon's
    /// pipe-delimited result text as a single `result` value (lossy: a value
    /// containing `|` or a newline corrupts it), this requests the daemon's
    /// structured JSON `RowSet` mode and returns one map per row, keyed by
    /// column name, with each cell a properly-typed JSON value. Use this when
    /// the result drives a decision rather than display.
    pub async fn query_rows(
        &self,
        cypher: &str,
    ) -> Result<Vec<HashMap<String, serde_json::Value>>, QueryError> {
        // A leading 0x01 byte selects the daemon's structured-row mode.
        let mut body = Vec::with_capacity(cypher.len() + 1);
        body.push(0x01);
        body.extend_from_slice(cypher.as_bytes());

        let bytes = self.round_trip(&body, MAX_TYPED_RESPONSE_BYTES).await?;
        // The daemon reports errors as plaintext ("ERROR: ..."), not JSON.
        // Only an error body is decoded as text (errors are short); a valid
        // typed body is never lossily decoded just to look for the prefix.
        if bytes.starts_with(b"ERROR:") {
            Self::check_error(&String::from_utf8_lossy(&bytes))?;
        }
        parse_row_set(&bytes)
    }
}

/// Parse the daemon's structured `{"columns": [...], "rows": [[..], ..]}`
/// JSON into rows keyed by column name.
///
/// Validation is strict and fails closed: a non-string or duplicate column
/// name, a row that is not an array, or a row whose cell count does not match
/// the column count is an [`QueryError::InvalidQuery`] rather than a coerced
/// partial row. Callers drive decisions from these rows, so a corrupt or
/// version-skewed body must surface as an error, never as plausible-looking
/// data.
fn parse_row_set(json: &[u8]) -> Result<Vec<HashMap<String, serde_json::Value>>, QueryError> {
    // `from_slice` requires valid UTF-8, so invalid bytes fail closed here
    // instead of being lossily replaced and parsed as plausible typed data.
    let value: serde_json::Value = serde_json::from_slice(json)
        .map_err(|e| QueryError::InvalidQuery(format!("malformed typed result: {e}")))?;

    let columns_json = value
        .get("columns")
        .and_then(|c| c.as_array())
        .ok_or_else(|| QueryError::InvalidQuery("typed result missing 'columns'".to_string()))?;
    if columns_json.len() > MAX_TYPED_COLUMNS {
        return Err(QueryError::InvalidQuery(format!(
            "typed result has {} columns, more than the {MAX_TYPED_COLUMNS} allowed",
            columns_json.len()
        )));
    }
    let mut columns: Vec<String> = Vec::with_capacity(columns_json.len());
    for col in columns_json {
        let name = col.as_str().ok_or_else(|| {
            QueryError::InvalidQuery("typed result has a non-string column name".to_string())
        })?;
        if name.len() > MAX_TYPED_COLUMN_NAME_BYTES {
            return Err(QueryError::InvalidQuery(format!(
                "typed result has a column name of {} bytes, more than the {MAX_TYPED_COLUMN_NAME_BYTES} allowed",
                name.len()
            )));
        }
        if columns.iter().any(|existing| existing == name) {
            return Err(QueryError::InvalidQuery(format!(
                "typed result has a duplicate column name {name:?}"
            )));
        }
        columns.push(name.to_string());
    }

    let rows_json = value
        .get("rows")
        .and_then(|r| r.as_array())
        .ok_or_else(|| QueryError::InvalidQuery("typed result missing 'rows'".to_string()))?;
    if rows_json.len() > MAX_TYPED_ROWS {
        return Err(QueryError::InvalidQuery(format!(
            "typed result has {} rows, more than the {MAX_TYPED_ROWS} allowed",
            rows_json.len()
        )));
    }
    // Bound total work before materializing any row. With the shape check
    // below every row carries exactly `columns.len()` cells, so this product
    // is the cell count the body claims; rejecting it here avoids building
    // millions of map entries from a frame that is itself within the limit.
    let total_cells = columns.len().saturating_mul(rows_json.len());
    if total_cells > MAX_TYPED_CELLS {
        return Err(QueryError::InvalidQuery(format!(
            "typed result claims {total_cells} cells, more than the {MAX_TYPED_CELLS} allowed"
        )));
    }
    let mut rows = Vec::with_capacity(rows_json.len());
    for row in rows_json {
        let cells = row.as_array().ok_or_else(|| {
            QueryError::InvalidQuery("typed result row is not an array".to_string())
        })?;
        if cells.len() != columns.len() {
            return Err(QueryError::InvalidQuery(format!(
                "typed result row has {} cells, expected {}",
                cells.len(),
                columns.len()
            )));
        }
        rows.push(
            columns
                .iter()
                .cloned()
                .zip(cells.iter().cloned())
                .collect::<HashMap<String, serde_json::Value>>(),
        );
    }
    Ok(rows)
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
            // Params are not sent on the wire; the daemon accepts raw Cypher
            // (typed/parameterised reads use `query_rows`). The daemon returns
            // its result as a raw string, surfaced as a single `result` column
            // — see `query_rows` for typed, column-keyed rows.
            let bytes = self.round_trip(cypher.as_bytes(), MAX_RESPONSE_BYTES).await?;
            // Text results are display-only, so lossy decoding is acceptable.
            let response = String::from_utf8_lossy(&bytes).to_string();
            Self::check_error(&response)?;
            let row = HashMap::from([("result".to_string(), serde_json::Value::String(response))]);
            Ok(vec![row])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_typed_columns_and_rows() {
        let json = r#"{"columns":["id","root_path"],"rows":[["p1","/home/tim/proj"],["p2","/x"]]}"#;
        let rows = parse_row_set(json.as_bytes()).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["id"], serde_json::Value::String("p1".into()));
        assert_eq!(rows[0]["root_path"], serde_json::Value::String("/home/tim/proj".into()));
    }

    #[test]
    fn preserves_cell_types() {
        let json = r#"{"columns":["n","ok","missing"],"rows":[[3,true,null]]}"#;
        let rows = parse_row_set(json.as_bytes()).unwrap();
        assert_eq!(rows[0]["n"], serde_json::json!(3));
        assert_eq!(rows[0]["ok"], serde_json::Value::Bool(true));
        assert_eq!(rows[0]["missing"], serde_json::Value::Null);
    }

    #[test]
    fn a_delimiter_or_newline_in_a_value_round_trips_intact() {
        // The point of the typed path: JSON escaping means a value with `|`
        // or a newline can no longer corrupt parsing (the old pipe-delimited
        // text format would split it into a forged row).
        let json = r#"{"columns":["name"],"rows":[["a|b\nc"]]}"#;
        let rows = parse_row_set(json.as_bytes()).unwrap();
        assert_eq!(rows[0]["name"], serde_json::Value::String("a|b\nc".into()));
    }

    #[test]
    fn rejects_malformed_or_incomplete_results() {
        assert!(parse_row_set(b"not json").is_err());
        assert!(parse_row_set(br#"{"rows":[]}"#).is_err()); // missing columns
        assert!(parse_row_set(br#"{"columns":["id"]}"#).is_err()); // missing rows
    }

    #[test]
    fn rejects_corrupt_row_shapes_rather_than_coercing() {
        // Non-string column name.
        assert!(parse_row_set(br#"{"columns":[1],"rows":[]}"#).is_err());
        // Duplicate column name would silently overwrite in a map.
        assert!(parse_row_set(br#"{"columns":["id","id"],"rows":[]}"#).is_err());
        // A row that is not an array.
        assert!(parse_row_set(br#"{"columns":["id"],"rows":[{"id":"x"}]}"#).is_err());
        // Short row: fewer cells than columns.
        assert!(parse_row_set(br#"{"columns":["a","b"],"rows":[["x"]]}"#).is_err());
        // Long row: more cells than columns.
        assert!(parse_row_set(br#"{"columns":["a"],"rows":[["x","y"]]}"#).is_err());
    }

    #[test]
    fn rejects_amplifying_row_and_column_counts() {
        // Many empty rows fit under the byte cap but would each become a map.
        let mut body = br#"{"columns":[],"rows":["#.to_vec();
        for i in 0..(MAX_TYPED_ROWS + 1) {
            if i > 0 {
                body.push(b',');
            }
            body.extend_from_slice(b"[]");
        }
        body.extend_from_slice(b"]}");
        assert!(parse_row_set(&body).is_err());

        // More columns than the daemon would ever emit.
        let cols: Vec<String> = (0..=MAX_TYPED_COLUMNS).map(|i| format!("\"c{i}\"")).collect();
        let body = format!(r#"{{"columns":[{}],"rows":[]}}"#, cols.join(","));
        assert!(parse_row_set(body.as_bytes()).is_err());

        // A single over-long column name (a per-cell key-clone amplifier).
        let long = "x".repeat(MAX_TYPED_COLUMN_NAME_BYTES + 1);
        let body = format!(r#"{{"columns":["{long}"],"rows":[]}}"#);
        assert!(parse_row_set(body.as_bytes()).is_err());

        // Within the row and column caps, but the cell product exceeds the
        // total budget. Empty row arrays keep the test body tiny; the product
        // check fires before any per-row shape validation.
        let full_cols: Vec<String> = (0..MAX_TYPED_COLUMNS).map(|i| format!("\"c{i}\"")).collect();
        let row_count = MAX_TYPED_CELLS / MAX_TYPED_COLUMNS + 2;
        let empty_rows = vec!["[]"; row_count].join(",");
        let body = format!(
            r#"{{"columns":[{}],"rows":[{}]}}"#,
            full_cols.join(","),
            empty_rows
        );
        assert!(parse_row_set(body.as_bytes()).is_err());
    }

    #[test]
    fn invalid_utf8_fails_closed_rather_than_decoding_lossily() {
        // A 0xFF byte is not valid UTF-8. The typed path must reject it, not
        // replace it with U+FFFD and parse the surrounding bytes as a row.
        let mut body = br#"{"columns":["name"],"rows":[[""#.to_vec();
        body.push(0xFF);
        body.extend_from_slice(br#""]]}"#);
        assert!(parse_row_set(&body).is_err());
    }

    #[tokio::test]
    async fn an_oversized_response_frame_is_rejected_before_allocation() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::UnixListener;

        let path = std::env::temp_dir().join("lunaris-os-sdk-oversized-frame-test.sock");
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).unwrap();

        let server = tokio::spawn(async move {
            let (mut conn, _) = listener.accept().await.unwrap();
            // Drain the request frame (4-byte length + body).
            let mut len_buf = [0u8; 4];
            conn.read_exact(&mut len_buf).await.unwrap();
            let req_len = u32::from_be_bytes(len_buf) as usize;
            let mut req = vec![0u8; req_len];
            conn.read_exact(&mut req).await.unwrap();
            // Advertise a response one byte past the client's limit, then send
            // no body: a well-behaved client must reject on the length alone.
            let huge = (MAX_RESPONSE_BYTES as u32 + 1).to_be_bytes();
            conn.write_all(&huge).await.unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        });

        let client = UnixGraphClient::new(path.to_string_lossy().to_string());
        let result = client.query("MATCH (n) RETURN n", HashMap::new()).await;
        let _ = server.await;
        let _ = std::fs::remove_file(&path);

        match result {
            Err(QueryError::InvalidQuery(msg)) => assert!(msg.contains("exceeds")),
            other => panic!("expected oversized-frame rejection, got {other:?}"),
        }
    }
}

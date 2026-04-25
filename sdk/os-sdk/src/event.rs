use std::future::Future;
use std::path::Path;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::net::UnixStream;
use tokio::sync::Mutex;
use prost::Message as _;

/// Error type for event emission failures.
#[derive(Debug)]
pub enum EmitError {
    /// The connection to the Event Bus could not be established or was lost.
    ConnectionFailed(String),
    /// The event could not be serialized to protobuf.
    SerializationFailed(String),
}

impl std::fmt::Display for EmitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EmitError::ConnectionFailed(msg) => write!(f, "connection failed: {msg}"),
            EmitError::SerializationFailed(msg) => write!(f, "serialization failed: {msg}"),
        }
    }
}

impl std::error::Error for EmitError {}

/// Emits structured events onto the Lunaris Event Bus.
///
/// Implemented by [`UnixEventEmitter`] for production use and by
/// [`crate::mock::MockEventEmitter`] for testing.
pub trait EventEmitter: Send + Sync {
    /// Emit an event to the Event Bus.
    ///
    /// The event type string follows the `category.action` convention,
    /// for example `file.opened` or `window.focused`.
    /// The payload is an encoded protobuf message specific to the event type.
    ///
    /// # Errors
    /// Returns [`EmitError::ConnectionFailed`] if the Event Bus is unreachable.
    /// Returns [`EmitError::SerializationFailed`] if the payload cannot be encoded.
    fn emit<'a>(
        &'a self,
        event_type: &'a str,
        payload: Vec<u8>,
    ) -> impl Future<Output = Result<(), EmitError>> + Send + 'a;
}

/// Production [`EventEmitter`] that sends events over a Unix socket to the Event Bus.
///
/// Connects lazily on first emit and reconnects automatically if the connection
/// is lost. Thread-safe: clone freely across async tasks.
///
/// # Example
/// ```no_run
/// use os_sdk::event::{EventEmitter, UnixEventEmitter};
///
/// #[tokio::main]
/// async fn main() {
///     let emitter = UnixEventEmitter::new("/run/lunaris/event-bus-producer.sock");
///     emitter.emit("app.action", vec![]).await.unwrap();
/// }
/// ```
#[derive(Clone)]
pub struct UnixEventEmitter {
    socket_path: String,
    /// Shared, lazily initialized connection.
    /// `None` means not yet connected or previously failed.
    stream: Arc<Mutex<Option<UnixStream>>>,
    app_id: String,
    session_id: String,
}

impl UnixEventEmitter {
    /// Create a new emitter that will connect to the given socket path.
    ///
    /// Does not connect immediately; the connection is established on the
    /// first call to [`emit`](EventEmitter::emit).
    pub fn new(socket_path: impl Into<String>) -> Self {
        Self {
            socket_path: socket_path.into(),
            stream: Arc::new(Mutex::new(None)),
            app_id: std::env::var("LUNARIS_APP_ID").unwrap_or_else(|_| "unknown".to_string()),
            session_id: std::env::var("LUNARIS_SESSION_ID")
                .unwrap_or_else(|_| "unknown".to_string()),
        }
    }
}

impl EventEmitter for UnixEventEmitter {
    #[allow(clippy::manual_async_fn)]
    fn emit<'a>(
        &'a self,
        event_type: &'a str,
        payload: Vec<u8>,
    ) -> impl Future<Output = Result<(), EmitError>> + Send + 'a {
        async move {
            let event = crate::proto::Event {
                id: uuid::Uuid::now_v7().to_string(),
                r#type: event_type.to_string(),
                timestamp: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_micros() as i64,
                source: format!("app:{}", self.app_id),
                pid: std::process::id(),
                session_id: self.session_id.clone(),
                payload,
                // uid is enriched by the bus via SO_PEERCRED on the
                // accept socket; sending 0 here is the documented
                // "let the daemon fill it in" path. project_id is
                // optional audit-log scoping that apps don't set
                // themselves — focus events propagate context.
                uid: 0,
                project_id: String::new(),
            };

            let encoded = event.encode_to_vec();
            let len = u32::try_from(encoded.len())
                .map_err(|e| EmitError::SerializationFailed(e.to_string()))?
                .to_be_bytes();

            let mut guard = self.stream.lock().await;

            // Try to send; reconnect once if the connection is broken.
            for attempt in 0..2u8 {
                if guard.is_none() {
                    match UnixStream::connect(Path::new(&self.socket_path)).await {
                        Ok(s) => *guard = Some(s),
                        Err(e) => {
                            return Err(EmitError::ConnectionFailed(e.to_string()));
                        }
                    }
                }

                let stream = guard.as_mut().expect("just connected");
                let result = async {
                    stream.write_all(&len).await?;
                    stream.write_all(&encoded).await?;
                    Ok::<_, std::io::Error>(())
                }
                .await;

                match result {
                    Ok(()) => return Ok(()),
                    Err(_) if attempt == 0 => {
                        // Connection broken; drop it and retry once.
                        *guard = None;
                    }
                    Err(e) => {
                        *guard = None;
                        return Err(EmitError::ConnectionFailed(e.to_string()));
                    }
                }
            }

            Err(EmitError::ConnectionFailed("failed after reconnect".to_string()))
        }
    }
}

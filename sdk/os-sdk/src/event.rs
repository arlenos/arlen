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

/// Emits structured events onto the Arlen Event Bus.
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
///     let emitter = UnixEventEmitter::new("/run/arlen/event-bus-producer.sock");
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
    origin: String,
}

/// The session this producer belongs to, or an empty string and a loud complaint.
///
/// `arlen-session` mints `ARLEN_SESSION_ID` once per login and every session-side
/// producer reads it. Substituting anything for an absent id makes the graph look
/// joined while joining nothing, so absent is reported rather than replaced.
fn session_origin() -> String {
    match std::env::var("ARLEN_SESSION_ID") {
        Ok(id) if !id.is_empty() => id,
        _ => {
            tracing::error!(
                "ARLEN_SESSION_ID is unset: this producer runs in a session but \
                 cannot name it, so the bus will refuse its events"
            );
            String::new()
        }
    }
}

impl UnixEventEmitter {
    /// Create a new emitter that will connect to the given socket path.
    ///
    /// Does not connect immediately; the connection is established on the
    /// first call to [`emit`](EventEmitter::emit).
    /// An emitter for a producer that runs OUTSIDE any login session.
    ///
    /// `producer` names it - "journald-parser", "powerd" - and the origin becomes
    /// `system:<producer>`. That is a claim the caller is positioned to make, which
    /// is the whole difference from the fallback this replaced: a library guessing
    /// "system" for a producer that turned out to be session-side is how a file the
    /// user opened gets attributed to the machine.
    ///
    /// Session-side producers use [`Self::new`], which reads the id the session
    /// minted and refuses to invent one. The producer comes FIRST so the call site
    /// reads as what it claims to be before it says where it sends.
    pub fn for_system_named(producer: &str, socket_path: impl Into<String>) -> Self {
        let app_id = std::env::var("ARLEN_APP_ID").unwrap_or_else(|_| producer.to_string());
        Self {
            socket_path: socket_path.into(),
            stream: Arc::new(Mutex::new(None)),
            app_id,
            origin: format!("system:{producer}"),
        }
    }

    pub fn new(socket_path: impl Into<String>) -> Self {
        let app_id = std::env::var("ARLEN_APP_ID").unwrap_or_else(|_| "unknown".to_string());
        Self {
            socket_path: socket_path.into(),
            stream: Arc::new(Mutex::new(None)),
            app_id: app_id.clone(),
            // The session, or nothing - never a stand-in.
            //
            // This fell back to "unknown", then to `system:{app_id}`, and the second
            // was the more dangerous because it reads like an answer: for a producer
            // inside a login it attributes what the USER did to the machine, in the
            // one field the transparency surface uses to tell those apart.
            //
            // A producer that genuinely has no session uses `for_system` below and
            // names itself. Absent here is a deployment defect: the bus refuses an
            // empty origin, so the events stop and the log says why.
            origin: session_origin(),
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
                origin: self.origin.clone(),
                payload,
                // uid is enriched by the bus via SO_PEERCRED on the
                // accept socket; sending 0 here is the documented
                // "let the daemon fill it in" path. project_id is
                // optional audit-log scoping that apps don't set
                // themselves — focus events propagate context.
                uid: 0,
                project_id: String::new(),
                // Bus-stamped from the producer's SO_PEERCRED-attested identity
                // (like uid); sending empty is the "let the bus fill it in" path.
                // A producer-supplied value would be overwritten anyway.
                authenticated_origin: String::new(),
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

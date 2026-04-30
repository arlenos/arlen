//! Event Bus consumer-side client.
//!
//! Companion to [`crate::event::UnixEventEmitter`]. Where the
//! emitter pushes events into the bus, the consumer pulls them
//! out: a Tauri app calls `subscribe()` with one or more event-
//! type prefixes and receives a `tokio::sync::mpsc::Receiver` of
//! decoded protobuf [`Event`] envelopes.
//!
//! See `docs/architecture/annotations-api.md` for the wire and
//! lifecycle semantics. Highlights:
//!
//! - **3-line plaintext registration** (`<id>\n<types-csv>\n<uid>\n`)
//!   matching the bus's [protocol][bus-proto].
//! - **Eager initial connect with 3× 100 ms retry**, then silent
//!   exponential-backoff reconnect (capped 30 s) on any later
//!   disconnect.
//! - **Consumer-id format `os-sdk-{app_id}-{uuidv7}`** so multiple
//!   `subscribe()` calls from the same app never collide.
//! - **mpsc channel capacity 64**, lossy backpressure (slow
//!   callers lose events; bus-side per-consumer buffer is 1024).
//! - **Drop the receiver to unsubscribe.** The forwarder task
//!   detects the closed channel, drops the bus connection, and
//!   exits.
//!
//! [bus-proto]: ../../../../event-bus/src/socket.rs

use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use prost::Message as _;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::sync::mpsc;

use crate::proto::Event;

/// Default channel capacity between the forwarder task and the
/// caller's receiver. Picked to mirror clipboard subscribe.
const RECEIVER_BUFFER: usize = 64;

/// Maximum frame body length the bus will deliver. Mirror of the
/// 1 MB cap enforced server-side; reject anything larger as a
/// protocol violation.
const MAX_FRAME_BYTES: usize = 1024 * 1024;

/// Initial-connect retry policy. 3 attempts × 100 ms = ~400 ms
/// worst-case before `subscribe()` returns `ConnectionFailed`.
const INITIAL_CONNECT_ATTEMPTS: u8 = 3;
const INITIAL_CONNECT_BACKOFF: Duration = Duration::from_millis(100);

/// Reconnect backoff bounds for the long-lived forwarder task.
const RECONNECT_INITIAL: Duration = Duration::from_millis(250);
const RECONNECT_MAX: Duration = Duration::from_secs(30);

/// Errors returned by [`EventConsumer::subscribe`].
#[derive(Debug, thiserror::Error)]
pub enum SubscribeError {
    /// All initial connect attempts failed.
    #[error("connect to event bus consumer socket: {0}")]
    ConnectionFailed(String),
    /// Failed to send the registration handshake.
    #[error("event bus registration: {0}")]
    Registration(String),
    /// Internal I/O error during setup.
    #[error("event bus I/O: {0}")]
    Io(#[from] std::io::Error),
}

/// Consumer-side counterpart to [`crate::event::EventEmitter`].
///
/// Implementors return an mpsc receiver yielding decoded
/// [`Event`] envelopes that match `subscribed_types`.
pub trait EventConsumer: Send + Sync {
    /// Subscribe to one or more event-type filters.
    ///
    /// `subscribed_types` follows the bus's prefix-match semantics:
    /// `"file.opened"` is exact, `"file."` is prefix, `"*"` is
    /// wildcard. Filters are OR-ed together inside the bus.
    ///
    /// Returns a receiver that yields events as they arrive. Drop
    /// the receiver to unsubscribe.
    ///
    /// # Errors
    /// [`SubscribeError::ConnectionFailed`] if the bus is
    /// unreachable after the eager-retry budget.
    fn subscribe<'a>(
        &'a self,
        subscribed_types: Vec<String>,
    ) -> impl Future<Output = Result<mpsc::Receiver<Event>, SubscribeError>> + Send + 'a;
}

/// Production [`EventConsumer`] talking to a Unix socket.
///
/// Cheap to clone — the socket path is a small string and
/// nothing else is shared. Each `subscribe()` opens its own
/// connection because the bus enforces one filter per
/// connection at the protocol level.
#[derive(Clone)]
pub struct UnixEventConsumer {
    socket_path: String,
    app_id: String,
    uid: u32,
}

impl UnixEventConsumer {
    /// Construct a new consumer client. Does not connect.
    /// Connection happens on `subscribe()`.
    pub fn new(socket_path: impl Into<String>) -> Self {
        let app_id =
            std::env::var("LUNARIS_APP_ID").unwrap_or_else(|_| "unknown".to_string());
        let uid = unsafe { libc::getuid() };
        Self {
            socket_path: socket_path.into(),
            app_id,
            uid,
        }
    }
}

impl EventConsumer for UnixEventConsumer {
    fn subscribe<'a>(
        &'a self,
        subscribed_types: Vec<String>,
    ) -> impl Future<Output = Result<mpsc::Receiver<Event>, SubscribeError>> + Send + 'a {
        async move {
            let consumer_id = format!(
                "os-sdk-{}-{}",
                self.app_id,
                uuid::Uuid::now_v7()
            );

            // Eager initial connect: surface failure synchronously
            // so the caller does not get a silently dead receiver.
            let stream = connect_with_retry(
                &self.socket_path,
                INITIAL_CONNECT_ATTEMPTS,
                INITIAL_CONNECT_BACKOFF,
            )
            .await?;

            // Send the 3-line registration before spawning the
            // long-lived task. If registration fails the caller
            // gets the error immediately.
            let registration = format_registration(
                &consumer_id,
                &subscribed_types,
                self.uid,
            );
            let stream = send_registration(stream, &registration).await?;

            let (tx, rx) = mpsc::channel::<Event>(RECEIVER_BUFFER);

            // Long-lived forwarder. Owns the (now-registered)
            // stream and pushes decoded events to the caller.
            // On disconnect, exponential-backoff reconnect with a
            // freshly-derived UUIDv7 consumer-id (so old registry
            // entries do not get reused after the bus drops them).
            let socket_path = PathBuf::from(&self.socket_path);
            let app_id = self.app_id.clone();
            let uid = self.uid;
            let subscribed_types = Arc::new(subscribed_types);
            tokio::spawn(forwarder(
                stream,
                tx,
                socket_path,
                app_id,
                uid,
                subscribed_types,
            ));

            Ok(rx)
        }
    }
}

/// Long-lived task: read framed events from the bus and forward
/// to the caller's mpsc. Auto-reconnect on disconnect with
/// exponential backoff. Exits when the receiver is dropped.
async fn forwarder(
    mut stream: UnixStream,
    tx: mpsc::Sender<Event>,
    socket_path: PathBuf,
    app_id: String,
    uid: u32,
    subscribed_types: Arc<Vec<String>>,
) {
    let mut backoff = RECONNECT_INITIAL;
    loop {
        // Pump events from the current stream until disconnect.
        match pump_events(&mut stream, &tx).await {
            PumpResult::ChannelClosed => {
                // Caller dropped the receiver — exit cleanly.
                return;
            }
            PumpResult::StreamClosed => {
                // Fall through to reconnect.
            }
        }

        // Reconnect loop. Each iteration generates a fresh
        // consumer-id so the bus does not see stale Vec entries.
        loop {
            tokio::time::sleep(backoff).await;
            backoff = (backoff * 2).min(RECONNECT_MAX);

            let consumer_id = format!("os-sdk-{}-{}", app_id, uuid::Uuid::now_v7());
            let new_stream = match UnixStream::connect(&socket_path).await {
                Ok(s) => s,
                Err(_) => continue,
            };
            let registration = format_registration(&consumer_id, &subscribed_types, uid);
            let new_stream = match send_registration(new_stream, &registration).await {
                Ok(s) => s,
                Err(_) => continue,
            };
            // Reset the backoff once we're back in a good state.
            backoff = RECONNECT_INITIAL;
            stream = new_stream;
            break;
        }
    }
}

/// Result of one pump cycle.
enum PumpResult {
    /// Caller dropped the receiver. Exit the forwarder.
    ChannelClosed,
    /// Bus closed our connection or sent malformed data. Reconnect.
    StreamClosed,
}

async fn pump_events(stream: &mut UnixStream, tx: &mpsc::Sender<Event>) -> PumpResult {
    loop {
        let mut len_buf = [0u8; 4];
        if stream.read_exact(&mut len_buf).await.is_err() {
            return PumpResult::StreamClosed;
        }
        let len = u32::from_be_bytes(len_buf) as usize;
        if len == 0 || len > MAX_FRAME_BYTES {
            return PumpResult::StreamClosed;
        }
        let mut body = vec![0u8; len];
        if stream.read_exact(&mut body).await.is_err() {
            return PumpResult::StreamClosed;
        }
        let event = match Event::decode(body.as_slice()) {
            Ok(e) => e,
            Err(_) => continue, // E11: malformed payload, skip and keep pumping
        };
        if tx.send(event).await.is_err() {
            return PumpResult::ChannelClosed;
        }
    }
}

/// Open a fresh `UnixStream`. Tries `attempts` times with
/// `backoff` between attempts. Returns the connected stream or
/// the last error wrapped in `ConnectionFailed`.
async fn connect_with_retry(
    socket_path: &str,
    attempts: u8,
    backoff: Duration,
) -> Result<UnixStream, SubscribeError> {
    let mut last_err: Option<std::io::Error> = None;
    for attempt in 0..attempts {
        match UnixStream::connect(socket_path).await {
            Ok(s) => return Ok(s),
            Err(e) => {
                last_err = Some(e);
                if attempt + 1 < attempts {
                    tokio::time::sleep(backoff).await;
                }
            }
        }
    }
    Err(SubscribeError::ConnectionFailed(format!(
        "{}: {}",
        socket_path,
        last_err
            .map(|e| e.to_string())
            .unwrap_or_else(|| "unknown".to_string())
    )))
}

fn format_registration(consumer_id: &str, subscribed_types: &[String], uid: u32) -> String {
    let types_csv = subscribed_types.join(",");
    format!("{consumer_id}\n{types_csv}\n{uid}\n")
}

async fn send_registration(
    mut stream: UnixStream,
    registration: &str,
) -> Result<UnixStream, SubscribeError> {
    stream
        .write_all(registration.as_bytes())
        .await
        .map_err(|e| SubscribeError::Registration(e.to_string()))?;
    stream
        .flush()
        .await
        .map_err(|e| SubscribeError::Registration(e.to_string()))?;
    Ok(stream)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registration_format_matches_bus_protocol() {
        let r = format_registration(
            "os-sdk-com.example-abc",
            &["app.annotation.".to_string(), "app.intent.".to_string()],
            1000,
        );
        assert_eq!(
            r,
            "os-sdk-com.example-abc\napp.annotation.,app.intent.\n1000\n"
        );
    }

    #[test]
    fn registration_with_empty_types_yields_empty_middle_line() {
        let r = format_registration("id", &[], 1000);
        assert_eq!(r, "id\n\n1000\n");
    }

    #[test]
    fn consumer_id_uses_app_id_and_uuid() {
        // Sanity check on the format string used at runtime.
        let id = format!("os-sdk-{}-{}", "com.example.app", uuid::Uuid::now_v7());
        assert!(id.starts_with("os-sdk-com.example.app-"));
        // UUIDv7 has fixed 36-char string length.
        let uuid_part = id.rsplit('-').next().unwrap();
        // Exactly the last segment of a UUIDv7 hyphenated form.
        // Cheap sanity (12 hex chars).
        assert_eq!(uuid_part.len(), 12);
    }
}

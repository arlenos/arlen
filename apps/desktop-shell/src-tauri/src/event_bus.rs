/// Event Bus consumer for the Lunaris desktop shell.
///
/// Subscribes to window events from the Event Bus and forwards them
/// to the TypeScript frontend via Tauri events.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;
use tauri::{AppHandle, Emitter};

const DEFAULT_CONSUMER_SOCKET: &str = "/run/lunaris/event-bus-consumer.sock";
const CONSUMER_ID: &str = "desktop-shell";
const SUBSCRIPTIONS: &str = "window.";

/// Window event payload forwarded to the TypeScript frontend.
#[derive(Debug, Clone, serde::Serialize)]
pub struct WindowEventPayload {
    pub event_type: String,
    pub app_id: String,
    pub title: String,
}

/// Start the Event Bus consumer in a background thread.
///
/// Connects to the consumer socket, registers subscriptions, and
/// forwards received events to the Tauri frontend.
/// Reconnects automatically if the connection is lost.
pub fn start(app: AppHandle) {
    let socket_path = std::env::var("LUNARIS_CONSUMER_SOCKET")
        .unwrap_or_else(|_| DEFAULT_CONSUMER_SOCKET.to_string());

    std::thread::spawn(move || {
        loop {
            if let Err(e) = run_consumer(&app, &socket_path) {
                log::warn!("Event Bus consumer disconnected: {e}, reconnecting in 2s");
            }
            std::thread::sleep(Duration::from_secs(2));
        }
    });
}

fn run_consumer(app: &AppHandle, socket_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    log::info!("connecting to Event Bus at {socket_path}");

    let stream = UnixStream::connect(socket_path)?;
    let mut writer = stream.try_clone()?;
    let reader = BufReader::new(stream);

    // Register: send consumer ID and subscriptions
    writer.write_all(format!("{CONSUMER_ID}\n{SUBSCRIPTIONS}\n").as_bytes())?;
    writer.flush()?;

    log::info!("registered as consumer, subscribed to {SUBSCRIPTIONS}");

    // Read length-prefixed protobuf events
    let mut reader = reader;
    loop {
        // Read 4-byte length prefix
        let mut len_buf = [0u8; 4];
        use std::io::Read;
        reader.get_mut().read_exact(&mut len_buf)?;
        let len = u32::from_be_bytes(len_buf) as usize;

        if len == 0 || len > 1024 * 1024 {
            return Err(format!("invalid message length: {len}").into());
        }

        let mut buf = vec![0u8; len];
        reader.get_mut().read_exact(&mut buf)?;

        // Decode protobuf Event
        if let Ok(event) = decode_event(&buf) {
            forward_to_frontend(app, event);
        }
    }
}

mod proto {
    include!(concat!(env!("OUT_DIR"), "/lunaris.eventbus.rs"));
}

fn decode_event(buf: &[u8]) -> Result<proto::Event, prost::DecodeError> {
    use prost::Message;
    proto::Event::decode(buf)
}

fn forward_to_frontend(app: &AppHandle, event: proto::Event) {
    let payload = WindowEventPayload {
        event_type: event.r#type.clone(),
        app_id: extract_app_id(&event),
        title: extract_title(&event),
    };

    log::debug!("forwarding {} for {}", event.r#type, payload.app_id);

    // Emit to TypeScript frontend
    let tauri_event = match event.r#type.as_str() {
        "window.focused" => "lunaris://window-focused",
        "window.opened"  => "lunaris://window-opened",
        "window.closed"  => "lunaris://window-closed",
        _ => return,
    };

    if let Err(e) = app.emit(tauri_event, &payload) {
        log::warn!("failed to emit Tauri event: {e}");
    }
}

/// Extract app_id from event payload.
/// The compositor encodes app_id in the payload as UTF-8 JSON.
fn extract_app_id(event: &proto::Event) -> String {
    parse_payload_field(event, "app_id")
        .unwrap_or_else(|| event.source.clone())
}

fn extract_title(event: &proto::Event) -> String {
    parse_payload_field(event, "title").unwrap_or_default()
}

fn parse_payload_field(event: &proto::Event, field: &str) -> Option<String> {
    let json: serde_json::Value = serde_json::from_slice(&event.payload).ok()?;
    json.get(field)?.as_str().map(|s| s.to_string())
}

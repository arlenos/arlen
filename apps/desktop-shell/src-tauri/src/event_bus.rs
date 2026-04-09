/// Event Bus consumer for the Lunaris desktop shell.
///
/// Subscribes to window and config events from the Event Bus and forwards
/// them to the TypeScript frontend via Tauri events.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;
use tauri::{AppHandle, Emitter};

const DEFAULT_CONSUMER_SOCKET: &str = "/run/lunaris/event-bus-consumer.sock";
const CONSUMER_ID: &str = "desktop-shell";
/// Subscribe to window, config, and project events.
const SUBSCRIPTIONS: &str = "window.,config.,project.";

/// Window event payload forwarded to the TypeScript frontend.
#[derive(Debug, Clone, serde::Serialize)]
pub struct WindowEventPayload {
    pub event_type: String,
    pub app_id: String,
    pub title: String,
}

/// Config change event payload forwarded to the TypeScript frontend.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ConfigChangedPayload {
    pub component: String,
    pub path: String,
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

    // Phase 3.1: 3-line registration (ID, patterns, UID).
    let uid = unsafe { libc::getuid() };
    writer.write_all(format!("{CONSUMER_ID}\n{SUBSCRIPTIONS}\n{uid}\n").as_bytes())?;
    writer.flush()?;

    log::info!("registered as consumer, subscribed to {SUBSCRIPTIONS}");

    let mut reader = BufReader::new(stream);
    loop {
        // Read 4-byte length prefix.
        let mut len_buf = [0u8; 4];
        use std::io::Read;
        reader.get_mut().read_exact(&mut len_buf)?;
        let len = u32::from_be_bytes(len_buf) as usize;

        if len == 0 || len > 1024 * 1024 {
            return Err(format!("invalid message length: {len}").into());
        }

        let mut buf = vec![0u8; len];
        reader.get_mut().read_exact(&mut buf)?;

        // Decode protobuf Event.
        if let Ok(event) = decode_event(&buf) {
            forward_to_frontend(app, event);
        }
    }
}

mod proto {
    #![allow(dead_code)]
    #![allow(clippy::doc_markdown)]
    include!(concat!(env!("OUT_DIR"), "/lunaris.eventbus.rs"));
}

fn decode_event(buf: &[u8]) -> Result<proto::Event, prost::DecodeError> {
    use prost::Message;
    proto::Event::decode(buf)
}

fn forward_to_frontend(app: &AppHandle, event: proto::Event) {
    let event_type = event.r#type.as_str();

    // Window events.
    if event_type.starts_with("window.") {
        let payload = WindowEventPayload {
            event_type: event.r#type.clone(),
            app_id: extract_payload_field(&event, "app_id")
                .unwrap_or_else(|| event.source.clone()),
            title: extract_payload_field(&event, "title").unwrap_or_default(),
        };

        let tauri_event = match event_type {
            "window.focused" => "lunaris://window-focused",
            "window.opened" => "lunaris://window-opened",
            "window.closed" => "lunaris://window-closed",
            _ => return,
        };

        if let Err(e) = app.emit(tauri_event, &payload) {
            log::warn!("failed to emit Tauri event: {e}");
        }
        return;
    }

    // Project events (protobuf payloads).
    if event_type.starts_with("project.") {
        forward_project_event(app, event_type, &event.payload);
        return;
    }

    // Config events.
    if event_type.starts_with("config.") {
        let payload = ConfigChangedPayload {
            component: extract_payload_field(&event, "component").unwrap_or_default(),
            path: extract_payload_field(&event, "path").unwrap_or_default(),
        };

        let tauri_event = match event_type {
            "config.changed" => "lunaris://config-changed",
            "config.reload_requested" => "lunaris://config-reload",
            _ => return,
        };

        log::debug!("config event: {event_type} component={}", payload.component);

        if let Err(e) = app.emit(tauri_event, &payload) {
            log::warn!("failed to emit config Tauri event: {e}");
        }
    }
}

/// Forward project lifecycle events to the frontend.
fn forward_project_event(app: &AppHandle, event_type: &str, payload: &[u8]) {
    use prost::Message;

    match event_type {
        "project.created" => {
            if let Ok(p) = proto::ProjectCreatedPayload::decode(payload) {
                let project = crate::projects::Project {
                    id: p.project_id,
                    name: p.name,
                    description: None,
                    root_path: p.root_path,
                    accent_color: None,
                    icon: None,
                    status: "active".into(),
                    created_at: 0,
                    last_accessed: None,
                    inferred: p.inferred,
                    confidence: p.confidence as u8,
                    promoted: !p.inferred, // explicit projects are promoted
                };
                log::info!("project.created: {} (inferred={})", project.name, project.inferred);
                let _ = app.emit("project:created", &project);
            }
        }
        "project.updated" => {
            if let Ok(p) = proto::ProjectUpdatedPayload::decode(payload) {
                let _ = app.emit("project:updated", serde_json::json!({
                    "projectId": p.project_id,
                    "name": p.name,
                }));
            }
        }
        "project.archived" => {
            if let Ok(p) = proto::ProjectArchivedPayload::decode(payload) {
                let _ = app.emit("project:archived", serde_json::json!({
                    "projectId": p.project_id,
                }));
            }
        }
        _ => {}
    }
}

/// Extract a string field from the JSON-encoded event payload.
fn extract_payload_field(event: &proto::Event, field: &str) -> Option<String> {
    let json: serde_json::Value = serde_json::from_slice(&event.payload).ok()?;
    json.get(field)?.as_str().map(|s| s.to_string())
}

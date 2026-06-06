/// Event Bus consumer for the Arlen desktop shell.
///
/// Subscribes to window and config events from the Event Bus and forwards
/// them to the TypeScript frontend via Tauri events.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;
use tauri::{AppHandle, Emitter};

const DEFAULT_CONSUMER_SOCKET: &str = "/run/arlen/event-bus-consumer.sock";
const CONSUMER_ID: &str = "desktop-shell";
/// Subscribe to window, config, project, and the app.* state
/// surfaces (toolbar, shortcut, badge, ambient).
///
/// Note: per-app state for these surfaces is NOT pruned on
/// process exit in this iteration. Stale HashMap entries are
/// harmless (rendered only when the app id matches the focused
/// window's app id, and bounded in size by app count).
/// Process-exit cleanup + TTL fallback (FA8 in
/// topbar-toolbar.md) lands in a Phase-6 hardening pass once
/// pid→app_id mapping infrastructure exists.
const SUBSCRIPTIONS: &str =
    "window.,config.,project.,app.toolbar.,app.shortcut.,app.badge.,app.ambient.";

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
pub fn start(app: AppHandle, shortcuts_state: crate::app_state::ShortcutsState) {
    let socket_path = std::env::var("ARLEN_CONSUMER_SOCKET")
        .unwrap_or_else(|_| DEFAULT_CONSUMER_SOCKET.to_string());

    std::thread::spawn(move || {
        loop {
            if let Err(e) = run_consumer(&app, &socket_path, &shortcuts_state) {
                log::warn!("Event Bus consumer disconnected: {e}, reconnecting in 2s");
            }
            std::thread::sleep(Duration::from_secs(2));
        }
    });
}

fn run_consumer(
    app: &AppHandle,
    socket_path: &str,
    shortcuts_state: &crate::app_state::ShortcutsState,
) -> Result<(), Box<dyn std::error::Error>> {
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
            forward_to_frontend(app, event, shortcuts_state);
        }
    }
}

mod proto {
    #![allow(dead_code)]
    #![allow(clippy::doc_markdown)]
    include!(concat!(env!("OUT_DIR"), "/arlen.eventbus.rs"));
}

fn decode_event(buf: &[u8]) -> Result<proto::Event, prost::DecodeError> {
    use prost::Message;
    proto::Event::decode(buf)
}

fn forward_to_frontend(
    app: &AppHandle,
    event: proto::Event,
    shortcuts_state: &crate::app_state::ShortcutsState,
) {
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
            "window.focused" => "arlen://window-focused",
            "window.opened" => "arlen://window-opened",
            "window.closed" => "arlen://window-closed",
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

    // App.toolbar events. Decoded and forwarded to the frontend
    // as Tauri events. The frontend store keys by app_id and
    // derives focused-app render. State stays in memory until
    // the app emits `cleared` or the shell process exits.
    if event_type.starts_with("app.toolbar.") {
        forward_toolbar_event(app, event_type, &event.payload);
        return;
    }

    // App.shortcut / app.badge / app.ambient: same per-app
    // forwarding pattern.
    if event_type.starts_with("app.shortcut.") {
        forward_shortcut_event(app, event_type, &event.payload, shortcuts_state);
        return;
    }
    if event_type.starts_with("app.badge.") {
        forward_badge_event(app, event_type, &event.payload);
        return;
    }
    if event_type.starts_with("app.ambient.") {
        forward_ambient_event(app, event_type, &event.payload);
        return;
    }

    // Config events.
    if event_type.starts_with("config.") {
        let payload = ConfigChangedPayload {
            component: extract_payload_field(&event, "component").unwrap_or_default(),
            path: extract_payload_field(&event, "path").unwrap_or_default(),
        };

        let tauri_event = match event_type {
            "config.changed" => "arlen://config-changed",
            "config.reload_requested" => "arlen://config-reload",
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

/// Forward `app.toolbar.*` events to the frontend. Events are
/// re-emitted as `arlen://toolbar-{event}` Tauri events with
/// the decoded payload as JSON; the frontend keys per-app
/// state by `app_id` and renders only the focused app's slot.
fn forward_toolbar_event(app: &AppHandle, event_type: &str, payload: &[u8]) {
    use prost::Message;

    match event_type {
        "app.toolbar.quick_actions" => {
            if let Ok(p) = proto::ToolbarQuickActionsPayload::decode(payload) {
                let json = serde_json::json!({
                    "appId": p.app_id,
                    "windowId": p.window_id,
                    "actions": p.actions.into_iter().map(|a| serde_json::json!({
                        "icon": a.icon,
                        "action": a.action,
                        "tooltip": a.tooltip,
                        "toggle": a.toggle,
                        "active": a.active,
                    })).collect::<Vec<_>>(),
                });
                let _ = app.emit("arlen://toolbar-quick-actions", &json);
            }
        }
        "app.toolbar.breadcrumb" => {
            if let Ok(p) = proto::ToolbarBreadcrumbPayload::decode(payload) {
                let json = serde_json::json!({
                    "appId": p.app_id,
                    "windowId": p.window_id,
                    "items": p.items.into_iter().map(|i| serde_json::json!({
                        "label": i.label,
                        "action": i.action,
                    })).collect::<Vec<_>>(),
                });
                let _ = app.emit("arlen://toolbar-breadcrumb", &json);
            }
        }
        "app.toolbar.progress" => {
            if let Ok(p) = proto::ToolbarProgressPayload::decode(payload) {
                let json = serde_json::json!({
                    "appId": p.app_id,
                    "windowId": p.window_id,
                    "value": p.value,
                    "label": if p.label.is_empty() { None } else { Some(p.label) },
                });
                let _ = app.emit("arlen://toolbar-progress", &json);
            }
        }
        "app.toolbar.progress_cleared" => {
            if let Ok(p) = proto::ToolbarProgressClearedPayload::decode(payload) {
                let json = serde_json::json!({
                    "appId": p.app_id,
                    "windowId": p.window_id,
                });
                let _ = app.emit("arlen://toolbar-progress-cleared", &json);
            }
        }
        "app.toolbar.cleared" => {
            if let Ok(p) = proto::ToolbarClearedPayload::decode(payload) {
                let json = serde_json::json!({
                    "appId": p.app_id,
                    "windowId": p.window_id,
                });
                let _ = app.emit("arlen://toolbar-cleared", &json);
            }
        }
        _ => {}
    }
}

/// Forward `app.shortcut.*` events to the frontend AND mirror
/// into the backend shortcuts state so the Waypointer plugin
/// can read them without a frontend round-trip.
fn forward_shortcut_event(
    app: &AppHandle,
    event_type: &str,
    payload: &[u8],
    shortcuts_state: &crate::app_state::ShortcutsState,
) {
    use prost::Message;
    match event_type {
        "app.shortcut.register" => {
            if let Ok(p) = proto::ShortcutRegisterPayload::decode(payload) {
                // Mirror to backend store.
                let entries: Vec<crate::app_state::ShortcutEntry> = p
                    .shortcuts
                    .iter()
                    .map(|s| crate::app_state::ShortcutEntry {
                        label: s.label.clone(),
                        icon: s.icon.clone(),
                        action: s.action.clone(),
                        context: s.context.clone(),
                        confirm: if s.confirm.is_empty() {
                            None
                        } else {
                            Some(s.confirm.clone())
                        },
                        enabled: true,
                        badge: None,
                    })
                    .collect();
                crate::app_state::apply_register(shortcuts_state, p.app_id.clone(), entries);

                let json = serde_json::json!({
                    "appId": p.app_id,
                    "shortcuts": p.shortcuts.into_iter().map(|s| serde_json::json!({
                        "label": s.label,
                        "icon": s.icon,
                        "action": s.action,
                        "context": s.context,
                        "confirm": if s.confirm.is_empty() { None } else { Some(s.confirm) },
                    })).collect::<Vec<_>>(),
                });
                let _ = app.emit("arlen://shortcut-register", &json);
            }
        }
        "app.shortcut.state_changed" => {
            if let Ok(p) = proto::ShortcutStateChangedPayload::decode(payload) {
                crate::app_state::apply_state_changed(
                    shortcuts_state,
                    &p.app_id,
                    &p.action,
                    p.enabled,
                    p.badge.clone(),
                );
                let json = serde_json::json!({
                    "appId": p.app_id,
                    "action": p.action,
                    "enabled": p.enabled,
                    "badge": p.badge,
                });
                let _ = app.emit("arlen://shortcut-state-changed", &json);
            }
        }
        "app.shortcut.cleared" => {
            if let Ok(p) = proto::ShortcutClearedPayload::decode(payload) {
                crate::app_state::apply_cleared(shortcuts_state, &p.app_id);
                let json = serde_json::json!({ "appId": p.app_id });
                let _ = app.emit("arlen://shortcut-cleared", &json);
            }
        }
        _ => {}
    }
}

/// Forward `app.badge.*` events to the frontend.
fn forward_badge_event(app: &AppHandle, event_type: &str, payload: &[u8]) {
    use prost::Message;
    match event_type {
        "app.badge.set" => {
            if let Ok(p) = proto::BadgeSetPayload::decode(payload) {
                let json = serde_json::json!({
                    "appId": p.app_id,
                    "variant": p.variant,
                    "count": p.count,
                    "status": p.status,
                    "progressValue": p.progress_value,
                });
                let _ = app.emit("arlen://badge-set", &json);
            }
        }
        "app.badge.cleared" => {
            if let Ok(p) = proto::BadgeClearedPayload::decode(payload) {
                let json = serde_json::json!({ "appId": p.app_id });
                let _ = app.emit("arlen://badge-cleared", &json);
            }
        }
        _ => {}
    }
}

/// Forward `app.ambient.*` events to the frontend.
fn forward_ambient_event(app: &AppHandle, event_type: &str, payload: &[u8]) {
    use prost::Message;
    match event_type {
        "app.ambient.set" => {
            if let Ok(p) = proto::AmbientSetPayload::decode(payload) {
                let json = serde_json::json!({
                    "appId": p.app_id,
                    "effect": p.effect,
                    "color": p.color,
                    "intensity": p.intensity,
                    "speed": p.speed,
                    "reason": p.reason,
                    "autoClearMs": p.auto_clear_ms,
                });
                let _ = app.emit("arlen://ambient-set", &json);
            }
        }
        "app.ambient.cleared" => {
            if let Ok(p) = proto::AmbientClearedPayload::decode(payload) {
                let json = serde_json::json!({ "appId": p.app_id });
                let _ = app.emit("arlen://ambient-cleared", &json);
            }
        }
        _ => {}
    }
}

/// One-shot producer-side emit. Used by the action-dispatch
/// path which needs to push an `app.toolbar.action_invoked`
/// event into the bus when the user clicks a Quick Action or
/// Breadcrumb. Synchronous + connection-per-call: actions are
/// rare so the cost is negligible, and avoiding a long-lived
/// emitter keeps this module sync-`std::os::unix` flavour.
fn emit_event(event_type: &str, payload: Vec<u8>) -> Result<(), Box<dyn std::error::Error>> {
    use prost::Message;
    let producer_socket = std::env::var("ARLEN_PRODUCER_SOCKET")
        .unwrap_or_else(|_| "/run/arlen/event-bus-producer.sock".to_string());

    let event = proto::Event {
        id: uuid::Uuid::now_v7().to_string(),
        r#type: event_type.to_string(),
        timestamp: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_micros() as i64,
        source: "desktop-shell".to_string(),
        pid: std::process::id(),
        session_id: std::env::var("ARLEN_SESSION_ID").unwrap_or_else(|_| "shell".into()),
        payload,
        uid: 0,
        project_id: String::new(),
    };

    let encoded = event.encode_to_vec();
    let len = u32::try_from(encoded.len())?.to_be_bytes();

    let mut stream = UnixStream::connect(producer_socket)?;
    stream.write_all(&len)?;
    stream.write_all(&encoded)?;
    Ok(())
}

/// Public API: push an `app.toolbar.action_invoked` event so
/// the source app's tauri-plugin-shell consumer can forward it
/// to its specific webview. `window_id` is the originating
/// webview label captured when the toolbar state was set;
/// empty string falls back to app-wide broadcast on the
/// receiver side.
pub fn emit_toolbar_action_invoked(app_id: &str, window_id: &str, action: &str) {
    use prost::Message;
    let payload = proto::ToolbarActionInvokedPayload {
        app_id: app_id.to_string(),
        action: action.to_string(),
        window_id: window_id.to_string(),
    };
    let encoded = payload.encode_to_vec();
    if let Err(e) = emit_event("app.toolbar.action_invoked", encoded) {
        log::warn!("emit toolbar action_invoked failed: {e}");
    }
}

/// Same shape as [`emit_toolbar_action_invoked`] but for the
/// shortcut surface (Waypointer-driven dispatch). Distinct
/// event type for audit clarity; the plugin-side consumer
/// routes both into the same `arlen://app-action` frontend
/// event.
pub fn emit_shortcut_action_invoked(app_id: &str, window_id: &str, action: &str) {
    use prost::Message;
    let payload = proto::ShortcutActionInvokedPayload {
        app_id: app_id.to_string(),
        action: action.to_string(),
        window_id: window_id.to_string(),
    };
    let encoded = payload.encode_to_vec();
    if let Err(e) = emit_event("app.shortcut.action_invoked", encoded) {
        log::warn!("emit shortcut action_invoked failed: {e}");
    }
}

/// Public API: push an `app.intent.dispatched` event for
/// `shell.intents.dispatch` Knowledge-Graph promotion. **`subject`
/// is the intent type (`url` / `file` / `text` / `email` /
/// `project`), NOT the user-supplied data field.** Same audit
/// discipline as the broker's audit-log lines (see
/// `intent-system.md` §6) — the graph can learn that an app
/// dispatched intents of a given type without ever recording
/// the URL / path / text content.
///
/// `handler_id` records which dispatcher actually ran
/// (`builtin.url` / `builtin.file` / etc.), enabling Phase-7
/// follow-on analytics that compare built-in default coverage
/// vs registered third-party handlers without re-instrumenting.
pub fn emit_intent_dispatched(
    app_id: &str,
    action: &str,
    intent_type: &str,
    handler_id: &str,
) {
    use prost::Message;
    let mut metadata = std::collections::HashMap::new();
    metadata.insert("handler".to_string(), handler_id.to_string());
    metadata.insert("source_app_id".to_string(), app_id.to_string());
    let payload = proto::AppActionPayload {
        category: "intent".to_string(),
        action: action.to_string(),
        subject: intent_type.to_string(),
        metadata,
    };
    let encoded = payload.encode_to_vec();
    if let Err(e) = emit_event("app.intent.dispatched", encoded) {
        log::warn!("emit intent dispatched failed: {e}");
    }
}

/// Extract a string field from the JSON-encoded event payload.
fn extract_payload_field(event: &proto::Event, field: &str) -> Option<String> {
    let json: serde_json::Value = serde_json::from_slice(&event.payload).ok()?;
    json.get(field)?.as_str().map(|s| s.to_string())
}

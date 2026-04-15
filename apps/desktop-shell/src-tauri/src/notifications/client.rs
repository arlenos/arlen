/// Unix socket client for the notification daemon.
///
/// Connects to the daemon, sends a ClientHello, then runs a read loop
/// that emits Tauri events for each ServerMessage received.

use std::path::PathBuf;
use std::sync::Arc;

use notification_proto::proto;
use tauri::{AppHandle, Emitter};
use tokio::io::WriteHalf;
use tokio::net::UnixStream;
use tokio::sync::Mutex;

use super::protocol::{read_message, write_message};
use super::types::{DndState, Notification, SyncPayload};

/// Resolve icon name to base64 data URL via the shared icon resolver.
///
/// Searches freedesktop icon theme directories for the icon name and
/// returns a base64 data URL. Falls back to checking the app index
/// for .desktop file icons.
fn resolve_icon(notification: &mut Notification) {
    log::info!(
        "resolve_icon: id={} app_icon='{}' app_name='{}'",
        notification.id,
        notification.app_icon,
        notification.app_name
    );
    if notification.app_icon.is_empty() {
        // No icon name from D-Bus, try lowercase app_name.
        let app_lower = notification.app_name.to_lowercase();
        if !app_lower.is_empty() {
            if let Some(data_url) =
                crate::shell_overlay_client::resolve_app_icon(app_lower.clone())
            {
                log::info!("resolve_icon: resolved via app_name '{app_lower}'");
                notification.app_icon = data_url;
                return;
            }
        }
        return;
    }
    // Already a data URL -- keep as-is.
    if notification.app_icon.starts_with("data:") {
        return;
    }
    // Absolute path: read directly.
    if notification.app_icon.starts_with('/') {
        if let Some(data_url) =
            crate::shell_overlay_client::resolve_app_icon(notification.app_icon.clone())
        {
            log::info!("resolve_icon: resolved absolute path");
            notification.app_icon = data_url;
        } else {
            log::info!("resolve_icon: absolute path not found, clearing");
            notification.app_icon.clear();
        }
        return;
    }
    // Try resolving as icon name (e.g. "firefox" -> hicolor theme lookup).
    if let Some(data_url) =
        crate::shell_overlay_client::resolve_app_icon(notification.app_icon.clone())
    {
        log::info!("resolve_icon: resolved '{}'", notification.app_icon);
        notification.app_icon = data_url;
        return;
    }
    // Try resolving with lowercase app_name as fallback.
    let app_lower = notification.app_name.to_lowercase();
    if app_lower != notification.app_icon {
        if let Some(data_url) =
            crate::shell_overlay_client::resolve_app_icon(app_lower.clone())
        {
            log::info!("resolve_icon: resolved via fallback '{app_lower}'");
            notification.app_icon = data_url;
            return;
        }
    }
    log::info!(
        "resolve_icon: NOT FOUND '{}' (app: '{}')",
        notification.app_icon,
        notification.app_name
    );
    notification.app_icon.clear();
}

/// Shared write-half of the socket connection.
pub type SocketWriter = Arc<Mutex<Option<WriteHalf<UnixStream>>>>;

/// Default socket path.
pub fn default_socket_path() -> PathBuf {
    let uid = unsafe { libc::getuid() };
    PathBuf::from(format!("/run/user/{uid}/lunaris/notification.sock"))
}

/// Connect to the notification daemon and start the read loop.
///
/// Returns the shared writer for sending commands.
pub fn start(app: AppHandle) -> SocketWriter {
    let writer: SocketWriter = Arc::new(Mutex::new(None));
    let writer_clone = writer.clone();

    tauri::async_runtime::spawn(async move {
        loop {
            match try_connect(&app, &writer_clone).await {
                Ok(()) => log::info!("notification client: disconnected, reconnecting"),
                Err(e) => log::warn!("notification client: {e}, retrying in 2s"),
            }
            tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
        }
    });

    writer
}

/// Try to connect once and run until disconnect.
async fn try_connect(app: &AppHandle, writer: &SocketWriter) -> Result<(), String> {
    let path = default_socket_path();
    let stream = UnixStream::connect(&path)
        .await
        .map_err(|e| format!("connect {}: {e}", path.display()))?;

    let (mut reader, write_half) = tokio::io::split(stream);

    // Store writer for commands.
    *writer.lock().await = Some(write_half);

    // Send hello.
    {
        let hello = proto::ClientMessage {
            msg: Some(proto::client_message::Msg::Hello(proto::ClientHello {
                client_name: "desktop-shell".into(),
            })),
        };
        let mut w = writer.lock().await;
        if let Some(ref mut w) = *w {
            write_message(w, &hello).await?;
        }
    }

    log::info!("notification client: connected to {}", path.display());

    // Read loop.
    loop {
        let msg: Option<proto::ServerMessage> = read_message(&mut reader).await?;
        let Some(msg) = msg else {
            // Clean disconnect.
            *writer.lock().await = None;
            return Ok(());
        };

        let Some(inner) = msg.msg else { continue };

        match inner {
            proto::server_message::Msg::Added(added) => {
                if let Some(n) = added.notification {
                    let mut notification = Notification::from(n);
                    resolve_icon(&mut notification);
                    let _ = app.emit("notification:new", &notification);
                }
            }
            proto::server_message::Msg::Closed(closed) => {
                let _ = app.emit("notification:closed", serde_json::json!({
                    "id": closed.id,
                    "reason": closed.reason,
                }));
            }
            proto::server_message::Msg::ActionInvoked(ai) => {
                let _ = app.emit("notification:action", serde_json::json!({
                    "id": ai.id,
                    "action_key": ai.action_key,
                }));
            }
            proto::server_message::Msg::NotificationRead(nr) => {
                let _ = app.emit("notification:read", serde_json::json!({
                    "id": nr.id,
                }));
            }
            proto::server_message::Msg::AllCleared(_) => {
                let _ = app.emit("notification:cleared", ());
            }
            proto::server_message::Msg::DndChanged(dc) => {
                let _ = app.emit("notification:dnd_changed", DndState {
                    mode: super::types::dnd_mode_str(dc.mode),
                });
            }
            proto::server_message::Msg::Sync(sync) => {
                let mut payload = SyncPayload::from(sync);
                for n in &mut payload.pending {
                    resolve_icon(n);
                }
                let _ = app.emit("notification:sync", &payload);
            }
            proto::server_message::Msg::History(hist) => {
                let mut notifications: Vec<Notification> = hist
                    .notifications
                    .into_iter()
                    .map(Notification::from)
                    .collect();
                for n in &mut notifications {
                    resolve_icon(n);
                }
                let _ = app.emit("notification:history", serde_json::json!({
                    "notifications": notifications,
                    "has_more": hist.has_more,
                }));
            }
            proto::server_message::Msg::CountUpdate(cu) => {
                let _ = app.emit("notification:count", serde_json::json!({
                    "pending": cu.pending_count,
                    "unread": cu.unread_count,
                }));
            }
            proto::server_message::Msg::KnownApps(ka) => {
                let _ = app.emit("notification:known_apps", serde_json::json!({
                    "app_names": ka.app_names,
                }));
            }
        }
    }
}

/// Send a command to the daemon via the shared writer.
pub async fn send_command(writer: &SocketWriter, msg: proto::ClientMessage) -> Result<(), String> {
    let mut guard = writer.lock().await;
    let w = guard
        .as_mut()
        .ok_or_else(|| "not connected to notification daemon".to_string())?;
    write_message(w, &msg).await
}

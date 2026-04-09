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
                    let notification = Notification::from(n);
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
                let mode = match dc.mode {
                    x if x == proto::DndMode::DndOn as i32 => "on",
                    x if x == proto::DndMode::DndScheduled as i32 => "scheduled",
                    _ => "off",
                };
                let _ = app.emit("notification:dnd_changed", DndState {
                    mode: mode.into(),
                });
            }
            proto::server_message::Msg::Sync(sync) => {
                let payload = SyncPayload::from(sync);
                let _ = app.emit("notification:sync", &payload);
            }
            proto::server_message::Msg::History(hist) => {
                let notifications: Vec<Notification> = hist
                    .notifications
                    .into_iter()
                    .map(Notification::from)
                    .collect();
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

/// Notification system: socket client to the lunaris-notifyd daemon.
///
/// Replaces the old in-process D-Bus server. The daemon handles
/// persistence, DND, rate limiting; the shell handles display.

pub mod client;
pub mod protocol;
pub mod types;

use notification_proto::proto;
use tauri::State;

use client::SocketWriter;

/// Start the notification client (called during Tauri setup).
pub fn start(app: tauri::AppHandle) -> SocketWriter {
    client::start(app)
}

// ── Tauri Commands ───────────────────────────────────────────────────────

#[tauri::command]
pub async fn notification_dismiss(
    writer: State<'_, SocketWriter>,
    id: u32,
) -> Result<(), String> {
    let msg = proto::ClientMessage {
        msg: Some(proto::client_message::Msg::Dismiss(
            proto::DismissNotification { id },
        )),
    };
    client::send_command(&writer, msg).await
}

#[tauri::command]
pub async fn notification_invoke_action(
    writer: State<'_, SocketWriter>,
    id: u32,
    action_key: String,
) -> Result<(), String> {
    let msg = proto::ClientMessage {
        msg: Some(proto::client_message::Msg::InvokeAction(
            proto::InvokeAction { id, action_key },
        )),
    };
    client::send_command(&writer, msg).await
}

#[tauri::command]
pub async fn notification_mark_read(
    writer: State<'_, SocketWriter>,
    id: u32,
) -> Result<(), String> {
    let msg = proto::ClientMessage {
        msg: Some(proto::client_message::Msg::MarkRead(proto::MarkRead {
            id,
        })),
    };
    client::send_command(&writer, msg).await
}

#[tauri::command]
pub async fn notification_clear_all(
    writer: State<'_, SocketWriter>,
) -> Result<(), String> {
    let msg = proto::ClientMessage {
        msg: Some(proto::client_message::Msg::ClearAll(proto::ClearAll {})),
    };
    client::send_command(&writer, msg).await
}

#[tauri::command]
pub async fn notification_set_dnd(
    writer: State<'_, SocketWriter>,
    mode: String,
) -> Result<(), String> {
    let dnd_mode = match mode.as_str() {
        "priority" | "on" => proto::DndMode::DndPriority as i32,
        "alarms" => proto::DndMode::DndAlarms as i32,
        "total" => proto::DndMode::DndTotal as i32,
        "scheduled" => proto::DndMode::DndScheduled as i32,
        _ => proto::DndMode::DndOff as i32,
    };
    let msg = proto::ClientMessage {
        msg: Some(proto::client_message::Msg::SetDnd(proto::SetDndMode {
            mode: dnd_mode,
        })),
    };
    client::send_command(&writer, msg).await
}

#[tauri::command]
pub async fn notification_get_history(
    writer: State<'_, SocketWriter>,
    limit: u32,
    before_timestamp: String,
    app_name: String,
) -> Result<(), String> {
    let msg = proto::ClientMessage {
        msg: Some(proto::client_message::Msg::GetHistory(proto::GetHistory {
            limit,
            before_timestamp,
            app_name,
        })),
    };
    client::send_command(&writer, msg).await
}

/// Request the list of distinct app names known to the daemon. The
/// response is delivered asynchronously via the `notification:known_apps`
/// Tauri event (see client.rs).
#[tauri::command]
pub async fn notification_get_known_apps(
    writer: State<'_, SocketWriter>,
) -> Result<(), String> {
    let msg = proto::ClientMessage {
        msg: Some(proto::client_message::Msg::GetKnownApps(
            proto::GetKnownApps {},
        )),
    };
    client::send_command(&writer, msg).await
}

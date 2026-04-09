/// Notification types for the Tauri frontend.
///
/// These are serialized as JSON to the Svelte frontend via Tauri events
/// and command responses.

use notification_proto::proto;
use serde::{Deserialize, Serialize};

/// A notification as seen by the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Notification {
    pub id: u32,
    pub app_name: String,
    pub summary: String,
    pub body: String,
    pub app_icon: String,
    pub actions: Vec<NotificationAction>,
    pub priority: String,
    pub category: String,
    pub timestamp: String,
    pub read: bool,
}

/// An action button on a notification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationAction {
    pub key: String,
    pub label: String,
}

/// DND state as seen by the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DndState {
    pub mode: String,
}

/// Sync payload sent on initial connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncPayload {
    pub pending: Vec<Notification>,
    pub unread_count: u32,
    pub dnd_mode: String,
}

/// Convert a proto Notification to a frontend Notification.
impl From<proto::Notification> for Notification {
    fn from(p: proto::Notification) -> Self {
        let priority = match p.priority {
            x if x == proto::Priority::Low as i32 => "low",
            x if x == proto::Priority::High as i32 => "high",
            x if x == proto::Priority::Critical as i32 => "critical",
            _ => "normal",
        };
        Self {
            id: p.id,
            app_name: p.app_name,
            summary: p.summary,
            body: p.body,
            app_icon: p.app_icon,
            actions: p
                .actions
                .into_iter()
                .map(|a| NotificationAction {
                    key: a.key,
                    label: a.label,
                })
                .collect(),
            priority: priority.into(),
            category: p.category,
            timestamp: p.timestamp,
            read: p.read,
        }
    }
}

fn dnd_mode_str(mode: i32) -> String {
    match mode {
        x if x == proto::DndMode::DndOn as i32 => "on",
        x if x == proto::DndMode::DndScheduled as i32 => "scheduled",
        _ => "off",
    }
    .into()
}

impl From<proto::SyncResponse> for SyncPayload {
    fn from(s: proto::SyncResponse) -> Self {
        Self {
            pending: s.pending.into_iter().map(Notification::from).collect(),
            unread_count: s.unread_count,
            dnd_mode: dnd_mode_str(s.dnd_mode),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_notification_from_proto() {
        let p = proto::Notification {
            id: 1,
            app_name: "Firefox".into(),
            summary: "Done".into(),
            body: "file.zip".into(),
            app_icon: "firefox".into(),
            actions: vec![proto::Action {
                key: "open".into(),
                label: "Open".into(),
            }],
            priority: proto::Priority::High as i32,
            category: "transfer.complete".into(),
            timestamp: "2026-04-09T12:00:00Z".into(),
            read: false,
        };
        let n = Notification::from(p);
        assert_eq!(n.id, 1);
        assert_eq!(n.priority, "high");
        assert_eq!(n.actions.len(), 1);
    }

    #[test]
    fn test_dnd_mode_str() {
        assert_eq!(dnd_mode_str(proto::DndMode::DndOff as i32), "off");
        assert_eq!(dnd_mode_str(proto::DndMode::DndOn as i32), "on");
        assert_eq!(dnd_mode_str(proto::DndMode::DndScheduled as i32), "scheduled");
    }
}

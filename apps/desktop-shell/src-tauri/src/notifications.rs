// SPDX-License-Identifier: GPL-3.0-only

//! Simplified notification receiver: owns `org.freedesktop.Notifications` on
//! the session D-Bus, maps urgency → priority, and emits
//! `lunaris://notification-show` Tauri events.
//!
//! Full notification daemon with persistence, action callbacks, and
//! do-not-disturb is deferred to Phase 4C.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};

use serde::{Deserialize, Serialize};
use tauri::Emitter;
use zbus::zvariant::{OwnedValue, Value};
use zbus::{connection, interface};

/// Payload emitted as `lunaris://notification-show`.
#[derive(Serialize, Deserialize, Clone)]
pub struct NotificationPayload {
    /// Monotonically increasing notification ID (starts at 1).
    pub id: u32,
    /// Sending application name.
    pub app_name: String,
    /// Short notification summary (title).
    pub summary: String,
    /// Optional notification body.
    pub body: String,
    /// Lunaris priority: `"critical"` | `"high"` | `"normal"` | `"low"`.
    pub priority: String,
}

/// D-Bus interface implementation for `org.freedesktop.Notifications`.
struct NotificationsServer {
    app: tauri::AppHandle,
    next_id: AtomicU32,
}

#[interface(name = "org.freedesktop.Notifications")]
impl NotificationsServer {
    /// Receives an incoming notification from any application.
    async fn notify(
        &self,
        app_name: &str,
        _replaces_id: u32,
        _app_icon: &str,
        summary: &str,
        body: &str,
        _actions: Vec<String>,
        hints: HashMap<String, OwnedValue>,
        expire_timeout: i32,
    ) -> u32 {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);

        let urgency: u8 = hints
            .get("urgency")
            .and_then(|v| match &**v {
                Value::U8(u) => Some(*u),
                _ => None,
            })
            .unwrap_or(1);

        // D-Bus urgency: 0=Low, 1=Normal, 2=Critical.
        // expire_timeout 0 means never auto-dismiss (treat as critical).
        let priority = match (urgency, expire_timeout) {
            (2, _) => "critical",
            (_, 0) => "critical",
            (1, _) => "high",
            _ => "normal",
        };

        let payload = NotificationPayload {
            id,
            app_name: app_name.to_owned(),
            summary: summary.to_owned(),
            body: body.to_owned(),
            priority: priority.to_owned(),
        };

        if let Err(e) = self.app.emit("lunaris://notification-show", payload) {
            log::error!("notifications: emit failed: {e}");
        }

        id
    }

    /// Returns the list of supported capabilities.
    fn get_capabilities(&self) -> Vec<String> {
        vec!["body".to_owned(), "persistence".to_owned()]
    }

    /// Returns server identification as (name, vendor, version, spec_version).
    fn get_server_information(&self) -> (String, String, String, String) {
        (
            "Lunaris".to_owned(),
            "Lunaris OS".to_owned(),
            "0.1.0".to_owned(),
            "1.2".to_owned(),
        )
    }

    /// No-op: full close/dismiss support is Phase 4C.
    fn close_notification(&self, _id: u32) {}
}

/// Starts the `org.freedesktop.Notifications` D-Bus server on the session bus.
///
/// Runs inside the Tauri async runtime (tokio). If another notification daemon
/// is already running, the name request will fail and this function logs a
/// warning and exits silently -- the existing daemon continues to operate.
pub fn start(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        let server = NotificationsServer {
            app,
            next_id: AtomicU32::new(1),
        };

        let builder = match connection::Builder::session()
            .and_then(|b| b.name("org.freedesktop.Notifications"))
            .and_then(|b| b.serve_at("/org/freedesktop/Notifications", server))
        {
            Ok(b) => b,
            Err(e) => {
                log::warn!("notifications: could not configure D-Bus connection: {e}");
                return;
            }
        };

        let conn = match builder.build().await {
            Ok(c) => c,
            Err(e) => {
                log::warn!("notifications: could not start D-Bus server: {e}");
                return;
            }
        };

        log::info!("notifications: D-Bus server started");

        // Keep the connection alive for the process lifetime.
        std::future::pending::<()>().await;
        drop(conn);
    });
}

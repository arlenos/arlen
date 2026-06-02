//! Alert dispatch over `org.freedesktop.Notifications`.
//!
//! A thin wrapper around a `Notify` D-Bus call to the notification
//! daemon. The decision of *whether* to raise an alert (suppression,
//! cooldown) lives in [`crate::state::State`] and is tested there;
//! this module only performs the I/O. A failed call is logged and
//! ignored — the detector is advisory and must not crash because the
//! notification daemon is momentarily unavailable.

use std::collections::HashMap;

use zbus::zvariant::Value;
use zbus::Connection;

use crate::detect::Alert;

/// Urgency hint values per the freedesktop notification spec.
const URGENCY_NORMAL: u8 = 1;
const URGENCY_CRITICAL: u8 = 2;

/// Dispatches alerts to the session notification daemon.
pub struct Notifier {
    conn: Connection,
}

impl Notifier {
    /// Connect to the session bus.
    pub async fn connect() -> zbus::Result<Self> {
        Ok(Self {
            conn: Connection::session().await?,
        })
    }

    /// Show `alert` as a desktop notification. Returns the assigned
    /// notification id. Errors are the caller's to log; they are not
    /// fatal.
    pub async fn dispatch(&self, alert: &Alert) -> zbus::Result<u32> {
        let urgency = if alert.critical {
            URGENCY_CRITICAL
        } else {
            URGENCY_NORMAL
        };
        let mut hints: HashMap<&str, Value<'_>> = HashMap::new();
        hints.insert("urgency", Value::U8(urgency));

        let reply = self
            .conn
            .call_method(
                Some("org.freedesktop.Notifications"),
                "/org/freedesktop/Notifications",
                Some("org.freedesktop.Notifications"),
                "Notify",
                &(
                    "Lunaris Security",      // app_name
                    0u32,                    // replaces_id
                    "dialog-warning",        // app_icon
                    alert.summary.as_str(),  // summary
                    alert.body.as_str(),     // body
                    Vec::<String>::new(),    // actions
                    hints,                   // hints
                    -1i32,                   // expire_timeout (server default)
                ),
            )
            .await?;
        let id: u32 = reply.body().deserialize()?;
        Ok(id)
    }
}

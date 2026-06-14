//! Low / critical battery desktop notifications (system-services-plan.md PWR-R6).
//!
//! On entering the low or critical battery level, the daemon raises a desktop
//! notification via the freedesktop `org.freedesktop.Notifications` service (the
//! Arlen notification daemon serves it) - "no display needed" here: the daemon
//! only *sends* the notification, it does not render it. The send fires once per
//! crossing because the [`crate::battery`] level machine is hysteretic, so there
//! is no per-percent notification spam.
//!
//! The message text is a pure function of the level + charge ([`notification_text`])
//! so it is unit-tested without a bus; the send wraps it around the `Notify` call.

use std::collections::HashMap;

use zbus::zvariant::Value;

use crate::battery::BatteryLevel;

/// freedesktop urgency levels for the notification `urgency` hint.
const URGENCY_NORMAL: u8 = 1;
const URGENCY_CRITICAL: u8 = 2;

/// The notification summary, body and urgency for a battery level, or `None`
/// when the level warrants no notification (Normal: charging or comfortably
/// above the low threshold).
pub fn notification_text(level: BatteryLevel, percentage: u8) -> Option<(&'static str, String, u8)> {
    match level {
        BatteryLevel::Normal => None,
        BatteryLevel::Low => Some((
            "Battery low",
            format!("{percentage}% remaining. Consider plugging in."),
            URGENCY_NORMAL,
        )),
        BatteryLevel::Critical => Some((
            "Battery critically low",
            format!("{percentage}% remaining. Plug in now to avoid shutdown."),
            URGENCY_CRITICAL,
        )),
    }
}

/// Send the low/critical battery notification for the given level over the
/// session bus, if the level warrants one. Best-effort: a missing notification
/// service or a send error is logged, never fatal (the published `power.*`
/// transition event is the durable signal; the notification is the convenience).
pub async fn send_battery_notification(
    conn: &zbus::Connection,
    level: BatteryLevel,
    percentage: u8,
) {
    let Some((summary, body, urgency)) = notification_text(level, percentage) else {
        return;
    };
    let proxy = match zbus::Proxy::new(
        conn,
        "org.freedesktop.Notifications",
        "/org/freedesktop/Notifications",
        "org.freedesktop.Notifications",
    )
    .await
    {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("battery notification: no notification service: {e}");
            return;
        }
    };
    let hints: HashMap<&str, Value> = HashMap::from([("urgency", Value::U8(urgency))]);
    // Notify(app_name, replaces_id, app_icon, summary, body, actions, hints, expire_timeout).
    let reply: zbus::Result<u32> = proxy
        .call(
            "Notify",
            &(
                "arlen-powerd",
                0u32,
                "battery-caution",
                summary,
                body.as_str(),
                Vec::<&str>::new(),
                hints,
                -1i32,
            ),
        )
        .await;
    if let Err(e) = reply {
        tracing::warn!("battery notification send failed: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_warrants_no_notification() {
        assert!(notification_text(BatteryLevel::Normal, 80).is_none());
    }

    #[test]
    fn low_is_normal_urgency_and_names_the_charge() {
        let (summary, body, urgency) = notification_text(BatteryLevel::Low, 18).unwrap();
        assert_eq!(summary, "Battery low");
        assert!(body.contains("18%"));
        assert_eq!(urgency, URGENCY_NORMAL);
    }

    #[test]
    fn critical_is_critical_urgency() {
        let (summary, _body, urgency) = notification_text(BatteryLevel::Critical, 4).unwrap();
        assert_eq!(summary, "Battery critically low");
        assert_eq!(urgency, URGENCY_CRITICAL);
    }
}

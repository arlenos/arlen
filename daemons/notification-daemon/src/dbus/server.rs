/// D-Bus server implementing `org.freedesktop.Notifications` (spec 1.2).
///
/// Receives notifications from applications, assigns IDs, determines
/// priority, and stores them in an in-memory list. Emits
/// `NotificationClosed` and `ActionInvoked` signals as required.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, OnceLock};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use zbus::interface;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{OwnedValue, Value};

use crate::dbus::icon_hints;
use crate::manager::NotificationManager;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Priority level for a notification, determined from D-Bus hints and
/// category strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Priority {
    /// Background info, no popup. Count only.
    Low,
    /// Standard notification. 4-second toast.
    Normal,
    /// Important. 8-second toast.
    High,
    /// Urgent. Persistent toast until dismissed.
    Critical,
}

/// Reason why a notification was closed (per spec).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u32)]
pub enum CloseReason {
    /// The notification expired (timeout).
    Expired = 1,
    /// The notification was dismissed by the user.
    Dismissed = 2,
    /// The notification was closed by `CloseNotification`.
    Closed = 3,
    /// Undefined/reserved.
    Undefined = 4,
}

/// A stored notification with all metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Notification {
    /// Unique notification ID (monotonically increasing).
    pub id: u32,
    /// Sending application name.
    pub app_name: String,
    /// Short summary (title).
    pub summary: String,
    /// Optional body text.
    pub body: String,
    /// Freedesktop icon name or path.
    pub app_icon: String,
    /// Action identifiers and labels: `[(id, label), ...]`.
    pub actions: Vec<(String, String)>,
    /// Computed priority.
    pub priority: Priority,
    /// D-Bus urgency hint value (0=Low, 1=Normal, 2=Critical).
    pub urgency: u8,
    /// Category string from hints (e.g. "im.received", "email.arrived").
    pub category: String,
    /// ISO 8601 timestamp of when the notification was received.
    pub timestamp: String,
    /// Original expire_timeout from the client (-1=server decides, 0=never).
    pub expire_timeout: i32,
    /// Whether this notification has been read/seen.
    pub read: bool,
}

/// Event emitted internally when the notification list changes.
#[derive(Debug, Clone)]
pub enum NotifyEvent {
    /// A new notification was added.
    Added(Notification),
    /// A notification was closed.
    Closed { id: u32, reason: CloseReason },
    /// An action was invoked on a notification.
    ActionInvoked { id: u32, action_key: String },
    /// A notification was marked as read.
    Read { id: u32 },
    /// All notifications were cleared.
    AllCleared,
    /// DND mode changed.
    DndChanged { mode: crate::config::DndMode },
}

// ---------------------------------------------------------------------------
// Priority determination
// ---------------------------------------------------------------------------

/// Determine the priority of a notification from D-Bus hints.
///
/// Rules (highest priority wins):
/// 1. `urgency` hint: 2 = Critical, 0 = Low
/// 2. `expire_timeout == 0` (never expire) -> Critical
/// 3. `category` hint: "im.received" / "email.arrived" -> High
/// 4. Default: Normal
pub fn determine_priority(urgency: u8, expire_timeout: i32, category: &str) -> Priority {
    // Urgency 2 is always critical.
    if urgency >= 2 {
        return Priority::Critical;
    }

    // Timeout 0 = never dismiss -> treat as critical.
    if expire_timeout == 0 {
        return Priority::Critical;
    }

    // Urgency 0 is always low.
    if urgency == 0 {
        return Priority::Low;
    }

    // Category-based promotion.
    match category {
        "im.received" | "im" | "email.arrived" | "email" | "presence.online" => Priority::High,
        "device.error" | "network.error" => Priority::High,
        "transfer.complete" | "device.added" | "device.removed" => Priority::Normal,
        _ => Priority::Normal,
    }
}

/// Resolved app ids permitted to raise a DND-piercing `Critical` notification
/// (GAP-7). Critical is the only tier that pierces Do-Not-Disturb and shows a
/// persistent toast, so an arbitrary `notify-send -u critical` from any app
/// must not claim it. These are the system alert sources: the power daemon
/// (critical-battery) and the anomaly detector (security alerts), both
/// root-owned `/usr/lib/arlen/libexec/` binaries with canonical
/// `path_to_app_id` entries. A new trusted Critical source must be added here.
const CRITICAL_NOTIFIER_ALLOWLIST: &[&str] = &["powerd", "anomalyd"];

/// Whether a caller may keep a `Critical` priority, by attested identity. Root
/// (uid 0) is always allowed as a belt for a future system alerter; otherwise
/// the resolved app id must be a known system notifier. An unresolved caller
/// (`None`) is untrusted — Critical fails closed to High rather than letting an
/// app that cannot prove a system identity pierce DND.
fn caller_may_set_critical(uid: Option<u32>, app_id: Option<&str>) -> bool {
    if uid == Some(0) {
        return true;
    }
    matches!(app_id, Some(id) if CRITICAL_NOTIFIER_ALLOWLIST.contains(&id))
}

/// Clamp a determined priority to a caller's authority. A `Critical` raised by
/// an untrusted caller is demoted to `High` (still important, an 8s toast, but
/// it does not pierce Do-Not-Disturb or persist). Every other priority is
/// returned unchanged; a trusted caller's `Critical` survives.
pub fn clamp_critical(priority: Priority, may_set_critical: bool) -> Priority {
    if priority == Priority::Critical && !may_set_critical {
        Priority::High
    } else {
        priority
    }
}

/// Resolve the calling app's Arlen identity and uid from the D-Bus connection.
///
/// The session bus daemon attests the sender's PID and uid
/// (`GetConnectionUnixProcessID` / `GetConnectionUnixUser`, not client-supplied
/// values), and `app_id_from_pid` resolves `/proc/<pid>/exe` through the F3
/// `path_to_app_id` chain — the same identity model the other daemons use. Any
/// failure yields `None`, which [`caller_may_set_critical`] treats as untrusted
/// (fail-closed for the Critical privilege). No PID-reuse guard: the only
/// consequence of a sub-millisecond reuse race here is one notification's DND
/// tier, far below a token handout.
async fn resolve_caller(
    header: &zbus::message::Header<'_>,
    connection: &zbus::Connection,
) -> (Option<u32>, Option<String>) {
    let Some(sender) = header.sender() else {
        return (None, None);
    };
    let Ok(proxy) = zbus::fdo::DBusProxy::new(connection).await else {
        return (None, None);
    };
    let uid = proxy
        .get_connection_unix_user(sender.clone().into())
        .await
        .ok();
    let app_id = match proxy.get_connection_unix_process_id(sender.clone().into()).await {
        Ok(pid) => arlen_permissions::identity::app_id_from_pid(pid).ok(),
        Err(_) => None,
    };
    (uid, app_id)
}

/// Parse the actions array from D-Bus (flat list of alternating key/label pairs)
/// into a `Vec<(String, String)>`.
fn parse_actions(raw: &[String]) -> Vec<(String, String)> {
    raw.chunks(2)
        .filter_map(|chunk| {
            if chunk.len() == 2 {
                Some((chunk[0].clone(), chunk[1].clone()))
            } else {
                None
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// D-Bus Server
// ---------------------------------------------------------------------------

/// Shared state for the notification server.
///
/// The D-Bus interface is intentionally thin: it generates IDs,
/// extracts urgency/category hints, and then delegates the full
/// pipeline (sanitisation, rate limit, DND, SQLite storage, broadcast)
/// to `NotificationManager`. Previously this struct kept its own
/// in-memory `Vec<Notification>` and bypassed the manager entirely —
/// that left SQLite empty, DND dead, and rate limiting unused.
pub struct NotificationServer {
    next_id: AtomicU32,
    events: broadcast::Sender<NotifyEvent>,
    /// Installed after construction via [`set_manager`]. `OnceLock`
    /// keeps the struct constructible before the manager exists
    /// (manager needs `event_sender()` from here).
    manager: OnceLock<Arc<NotificationManager>>,
}

impl NotificationServer {
    /// Create a new notification server. The manager must be wired up
    /// via [`set_manager`] before the D-Bus connection starts
    /// accepting messages, otherwise `notify()` will fail closed.
    pub fn new() -> (Self, broadcast::Receiver<NotifyEvent>) {
        let (tx, rx) = broadcast::channel(256);
        (
            Self {
                next_id: AtomicU32::new(1),
                events: tx,
                manager: OnceLock::new(),
            },
            rx,
        )
    }

    /// Inject the notification manager. Must be called before the
    /// D-Bus server is registered, and exactly once.
    pub fn set_manager(&self, manager: Arc<NotificationManager>) {
        let _ = self.manager.set(manager);
    }

    /// Get the event sender for subscribing to changes.
    pub fn event_sender(&self) -> broadcast::Sender<NotifyEvent> {
        self.events.clone()
    }
}

#[interface(name = "org.freedesktop.Notifications")]
impl NotificationServer {
    /// Receive an incoming notification.
    ///
    /// Returns the assigned notification ID.
    async fn notify(
        &self,
        app_name: &str,
        replaces_id: u32,
        app_icon: &str,
        summary: &str,
        body: &str,
        actions: Vec<String>,
        hints: HashMap<String, OwnedValue>,
        expire_timeout: i32,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> u32 {
        let id = if replaces_id > 0 {
            replaces_id
        } else {
            self.next_id.fetch_add(1, Ordering::SeqCst)
        };

        // Extract urgency from hints (default: 1 = Normal).
        let urgency: u8 = hints
            .get("urgency")
            .and_then(|v| match &**v {
                Value::U8(u) => Some(*u),
                _ => None,
            })
            .unwrap_or(1);

        // Extract category from hints.
        let category = hints
            .get("category")
            .and_then(|v| match &**v {
                Value::Str(s) => Some(s.to_string()),
                _ => None,
            })
            .unwrap_or_default();

        // Resolve the effective icon from hints + positional arg per
        // FDO priority: image-data > image-path > app_icon. Apps like
        // Discord / Slack / Firefox ship per-notification favicons via
        // `image-data`; without this step those would all show letter
        // fallbacks in the shell.
        let resolved_icon = icon_hints::resolve_icon(&hints, app_icon);

        tracing::info!(
            id,
            app_name,
            app_icon_arg = app_icon,
            resolved_icon_kind = icon_kind(&resolved_icon),
            %summary,
            urgency,
            %category,
            "D-Bus notify received"
        );

        // Resolve the caller's attested identity to decide whether it may raise
        // a DND-piercing Critical (GAP-7). Fail-closed: an unresolved caller is
        // untrusted, so its Critical is later clamped to High.
        let (caller_uid, caller_app_id) = resolve_caller(&header, connection).await;
        let may_set_critical = caller_may_set_critical(caller_uid, caller_app_id.as_deref());
        if !may_set_critical {
            tracing::debug!(
                app_name,
                caller = caller_app_id.as_deref().unwrap_or("<unresolved>"),
                "notify: caller not trusted for Critical; will clamp"
            );
        }

        // Delegate to the manager. Ownership of the full pipeline
        // (validation, rate limit, DND, SQLite persistence, broadcast)
        // lives there; this D-Bus method is now a thin adapter.
        let Some(manager) = self.manager.get() else {
            tracing::error!(
                "D-Bus notify: manager not wired up, dropping notification"
            );
            return 0;
        };

        manager
            .handle_notify(
                id,
                app_name,
                &resolved_icon,
                summary,
                body,
                &actions,
                urgency,
                &category,
                expire_timeout,
                may_set_critical,
            )
            .await
    }

    /// Close a notification by ID. Delegates dismissal to the manager
    /// (which updates SQLite + broadcasts). The D-Bus `NotificationClosed`
    /// signal is emitted here because it is a D-Bus concept.
    async fn close_notification(
        &self,
        id: u32,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) {
        if let Some(manager) = self.manager.get() {
            manager.handle_close(id, CloseReason::Closed).await;
        } else {
            tracing::error!("D-Bus close: manager not wired up");
        }
        let _ = Self::notification_closed(&emitter, id, CloseReason::Closed as u32).await;
        tracing::debug!(id, "notification closed via D-Bus");
    }

    /// Return supported capabilities.
    ///
    /// `body-markup` is intentionally **not** advertised: the shell
    /// renders every notification body as plain text and the daemon
    /// strips any HTML that arrives (see `manager::validation::strip_markup`).
    /// Announcing a markup capability we do not honour would mislead
    /// well-behaved senders into shipping tags that the user never sees
    /// in their styled form.
    fn get_capabilities(&self) -> Vec<String> {
        vec![
            "body".to_owned(),
            "actions".to_owned(),
            "icon-static".to_owned(),
            "persistence".to_owned(),
        ]
    }

    /// Return server identification.
    fn get_server_information(&self) -> (String, String, String, String) {
        (
            "Arlen".to_owned(),
            "Arlen OS".to_owned(),
            env!("CARGO_PKG_VERSION").to_owned(),
            "1.2".to_owned(),
        )
    }

    // ── Signals ──────────────────────────────────────────────────────────

    /// Emitted when a notification is closed.
    #[zbus(signal)]
    async fn notification_closed(
        emitter: &SignalEmitter<'_>,
        id: u32,
        reason: u32,
    ) -> zbus::Result<()>;

    /// Emitted when the user invokes an action on a notification.
    #[zbus(signal)]
    async fn action_invoked(
        emitter: &SignalEmitter<'_>,
        id: u32,
        action_key: &str,
    ) -> zbus::Result<()>;
}

/// Classify the resolved icon string for structured logging. Avoids
/// dumping megabyte-long base64 data URLs into `tracing::info` output
/// while still capturing which of the three FDO sources was picked.
fn icon_kind(icon: &str) -> &'static str {
    if icon.is_empty() {
        "none"
    } else if icon.starts_with("data:") {
        "image-data"
    } else if icon.starts_with('/') {
        "image-path"
    } else {
        "theme-name"
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ── Priority determination ───────────────────────────────────────────

    #[test]
    fn test_urgency_critical() {
        assert_eq!(determine_priority(2, -1, ""), Priority::Critical);
        assert_eq!(determine_priority(2, 5000, ""), Priority::Critical);
        assert_eq!(determine_priority(2, 0, ""), Priority::Critical);
    }

    #[test]
    fn test_urgency_low() {
        assert_eq!(determine_priority(0, -1, ""), Priority::Low);
        assert_eq!(determine_priority(0, 5000, ""), Priority::Low);
    }

    #[test]
    fn test_urgency_low_with_timeout_zero() {
        // urgency 0 BUT timeout 0 -> urgency check (0 < 2) passes,
        // then timeout 0 check -> Critical wins.
        assert_eq!(determine_priority(0, 0, ""), Priority::Critical);
    }

    #[test]
    fn test_timeout_zero_critical() {
        assert_eq!(determine_priority(1, 0, ""), Priority::Critical);
    }

    #[test]
    fn test_normal_default() {
        assert_eq!(determine_priority(1, -1, ""), Priority::Normal);
        assert_eq!(determine_priority(1, 5000, ""), Priority::Normal);
    }

    #[test]
    fn test_category_im_high() {
        assert_eq!(determine_priority(1, -1, "im.received"), Priority::High);
        assert_eq!(determine_priority(1, 5000, "im"), Priority::High);
    }

    #[test]
    fn test_category_email_high() {
        assert_eq!(determine_priority(1, -1, "email.arrived"), Priority::High);
        assert_eq!(determine_priority(1, 5000, "email"), Priority::High);
    }

    #[test]
    fn test_category_device_error_high() {
        assert_eq!(determine_priority(1, -1, "device.error"), Priority::High);
        assert_eq!(determine_priority(1, -1, "network.error"), Priority::High);
    }

    #[test]
    fn test_category_transfer_normal() {
        assert_eq!(
            determine_priority(1, -1, "transfer.complete"),
            Priority::Normal
        );
    }

    #[test]
    fn test_unknown_category_normal() {
        assert_eq!(determine_priority(1, -1, "x-custom.thing"), Priority::Normal);
    }

    #[test]
    fn test_urgency_overrides_category() {
        // urgency=2 should be critical even with a "normal" category.
        assert_eq!(
            determine_priority(2, -1, "transfer.complete"),
            Priority::Critical
        );
        // urgency=0 should be low even with "im.received" category.
        assert_eq!(determine_priority(0, -1, "im.received"), Priority::Low);
    }

    // ── Critical-tier clamp (GAP-7) ─────────────────────────────────────

    #[test]
    fn untrusted_critical_clamps_to_high() {
        assert_eq!(
            clamp_critical(Priority::Critical, false),
            Priority::High,
            "an untrusted caller's Critical must demote to High"
        );
        assert_eq!(
            clamp_critical(Priority::Critical, true),
            Priority::Critical,
            "a trusted caller keeps Critical"
        );
        // Non-Critical priorities are never touched by the clamp.
        for p in [Priority::Low, Priority::Normal, Priority::High] {
            assert_eq!(clamp_critical(p, false), p);
            assert_eq!(clamp_critical(p, true), p);
        }
    }

    #[test]
    fn only_system_notifiers_and_root_may_set_critical() {
        // The trusted system alert sources.
        assert!(caller_may_set_critical(None, Some("powerd")));
        assert!(caller_may_set_critical(Some(1000), Some("anomalyd")));
        // Root is allowed regardless of resolved app id.
        assert!(caller_may_set_critical(Some(0), None));
        // An arbitrary app is not.
        assert!(!caller_may_set_critical(Some(1000), Some("com.example.spammer")));
        // An unresolved caller fails closed (untrusted).
        assert!(!caller_may_set_critical(None, None));
    }

    // ── Actions parsing ─────────────────────────────────────────────────

    #[test]
    fn test_parse_actions_empty() {
        assert!(parse_actions(&[]).is_empty());
    }

    #[test]
    fn test_parse_actions_pairs() {
        let raw = vec![
            "default".to_string(),
            "Open".to_string(),
            "dismiss".to_string(),
            "Dismiss".to_string(),
        ];
        let actions = parse_actions(&raw);
        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0], ("default".into(), "Open".into()));
        assert_eq!(actions[1], ("dismiss".into(), "Dismiss".into()));
    }

    #[test]
    fn test_parse_actions_odd_length() {
        let raw = vec![
            "default".to_string(),
            "Open".to_string(),
            "orphan".to_string(),
        ];
        let actions = parse_actions(&raw);
        assert_eq!(actions.len(), 1);
    }

    // ── Notification struct ─────────────────────────────────────────────

    #[test]
    fn test_notification_default_fields() {
        let n = Notification {
            id: 1,
            app_name: "test".into(),
            summary: "Hello".into(),
            body: "World".into(),
            app_icon: "".into(),
            actions: vec![],
            priority: Priority::Normal,
            urgency: 1,
            category: "".into(),
            timestamp: "2026-04-09T00:00:00Z".into(),
            expire_timeout: -1,
            read: false,
        };
        assert_eq!(n.id, 1);
        assert!(!n.read);
        assert_eq!(n.priority, Priority::Normal);
    }

    #[test]
    fn test_notification_serialization() {
        let n = Notification {
            id: 42,
            app_name: "Firefox".into(),
            summary: "Download complete".into(),
            body: "file.zip".into(),
            app_icon: "firefox".into(),
            actions: vec![("open".into(), "Open".into())],
            priority: Priority::High,
            urgency: 1,
            category: "transfer.complete".into(),
            timestamp: "2026-04-09T12:00:00Z".into(),
            expire_timeout: 5000,
            read: false,
        };
        let json = serde_json::to_string(&n).unwrap();
        assert!(json.contains("\"id\":42"));
        assert!(json.contains("\"priority\":\"High\""));

        let deserialized: Notification = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.id, 42);
        assert_eq!(deserialized.priority, Priority::High);
    }

    // ── Server construction ─────────────────────────────────────────────

    #[test]
    fn test_server_capabilities() {
        let (server, _rx) = NotificationServer::new();
        let caps = server.get_capabilities();
        assert!(caps.contains(&"body".to_owned()));
        assert!(caps.contains(&"actions".to_owned()));
        assert!(caps.contains(&"persistence".to_owned()));
    }

    #[test]
    fn test_server_info() {
        let (server, _rx) = NotificationServer::new();
        let (name, vendor, _version, spec) = server.get_server_information();
        assert_eq!(name, "Arlen");
        assert_eq!(vendor, "Arlen OS");
        assert_eq!(spec, "1.2");
    }
}

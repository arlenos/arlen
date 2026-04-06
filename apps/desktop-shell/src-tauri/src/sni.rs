/// StatusNotifierItem (SNI) system tray watcher.
///
/// Implements `org.kde.StatusNotifierWatcher` on the session D-Bus. Tray-capable
/// applications (Discord, nm-applet, blueman, etc.) register themselves via
/// `RegisterStatusNotifierItem`. The watcher tracks registered items, fetches
/// their properties, and notifies the frontend via Tauri events.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tauri::Emitter;
use zbus::object_server::SignalContext;
use zbus::{connection, interface, Connection};

/// A single StatusNotifierItem (system tray entry).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SniItem {
    /// D-Bus bus name of the item owner.
    pub service: String,
    /// Application identifier (consistent between sessions).
    pub id: String,
    /// Category: ApplicationStatus, Communications, SystemServices, Hardware.
    pub category: String,
    /// Status: Passive, Active, NeedsAttention.
    pub status: String,
    /// Human-readable title.
    pub title: String,
    /// Freedesktop icon name (may be empty if app uses pixmap).
    pub icon_name: String,
}

/// Shared state for tracked SNI items.
pub type SniItems = Arc<Mutex<HashMap<String, SniItem>>>;

/// D-Bus interface: `org.kde.StatusNotifierWatcher`.
///
/// Most tray apps register with the KDE variant rather than the
/// freedesktop variant, so we use `org.kde.StatusNotifierWatcher`.
struct StatusNotifierWatcher {
    items: SniItems,
    app_handle: tauri::AppHandle,
}

#[interface(name = "org.kde.StatusNotifierWatcher")]
impl StatusNotifierWatcher {
    /// Called by tray applications to register themselves.
    async fn register_status_notifier_item(
        &self,
        service: &str,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(signal_context)] ctx: SignalContext<'_>,
    ) {
        // Resolve the actual bus name. The `service` argument may be:
        // - A well-known name: "org.kde.StatusNotifierItem-1234-1"
        // - A unique name: ":1.42"
        // - Just an object path: "/StatusNotifierItem" (bus name is the sender)
        let bus_name = if service.starts_with('/') {
            header
                .sender()
                .map(|s| s.to_string())
                .unwrap_or_else(|| service.to_string())
        } else {
            service.to_string()
        };

        log::info!("sni: item registered: {bus_name}");

        match fetch_item_properties(&bus_name, service).await {
            Ok(item) => {
                self.items.lock().unwrap().insert(bus_name.clone(), item);
                let _ = Self::status_notifier_item_registered(&ctx, &bus_name).await;
                let _ = self.app_handle.emit("sni-items-changed", ());
            }
            Err(e) => {
                log::warn!("sni: failed to fetch properties for {bus_name}: {e}");
            }
        }
    }

    /// Called by tray hosts to register themselves. We are the host, so this is
    /// a no-op.
    async fn register_status_notifier_host(&self, _service: &str) {}

    /// List of currently registered item bus names.
    #[zbus(property)]
    async fn registered_status_notifier_items(&self) -> Vec<String> {
        self.items.lock().unwrap().keys().cloned().collect()
    }

    /// Whether a host is registered (always true since we are the host).
    #[zbus(property)]
    async fn is_status_notifier_host_registered(&self) -> bool {
        true
    }

    /// Protocol version (0 per spec).
    #[zbus(property)]
    async fn protocol_version(&self) -> i32 {
        0
    }

    /// Emitted when a new item registers.
    #[zbus(signal)]
    async fn status_notifier_item_registered(
        ctx: &SignalContext<'_>,
        service: &str,
    ) -> zbus::Result<()>;

    /// Emitted when an item disappears.
    #[zbus(signal)]
    async fn status_notifier_item_unregistered(
        ctx: &SignalContext<'_>,
        service: &str,
    ) -> zbus::Result<()>;

    /// Emitted when a new host registers.
    #[zbus(signal)]
    async fn status_notifier_host_registered(ctx: &SignalContext<'_>) -> zbus::Result<()>;
}

/// Fetch SNI properties from a registered item via D-Bus.
async fn fetch_item_properties(bus_name: &str, service: &str) -> Result<SniItem, String> {
    let conn = Connection::session()
        .await
        .map_err(|e| format!("session bus: {e}"))?;

    // Determine object path from the service string.
    let obj_path = if service.starts_with('/') {
        service.to_string()
    } else if service.contains('/') {
        format!("/{}", service.split_once('/').unwrap().1)
    } else {
        "/StatusNotifierItem".to_string()
    };

    // Most apps implement org.kde.StatusNotifierItem.
    let proxy = zbus::Proxy::new(
        &conn,
        bus_name,
        obj_path.as_str(),
        "org.kde.StatusNotifierItem",
    )
    .await
    .map_err(|e| format!("proxy: {e}"))?;

    let id: String = proxy.get_property("Id").await.unwrap_or_default();
    let category: String = proxy
        .get_property("Category")
        .await
        .unwrap_or_else(|_| "ApplicationStatus".to_string());
    let status: String = proxy
        .get_property("Status")
        .await
        .unwrap_or_else(|_| "Active".to_string());
    let title: String = proxy
        .get_property("Title")
        .await
        .unwrap_or_else(|_| id.clone());
    let icon_name: String = proxy.get_property("IconName").await.unwrap_or_default();

    Ok(SniItem {
        service: bus_name.to_string(),
        id,
        category,
        status,
        title,
        icon_name,
    })
}

/// Parse a service string into (bus_name, object_path).
fn parse_service_path(service: &str) -> (&str, String) {
    if service.contains('/') && !service.starts_with('/') {
        let (name, path) = service.split_once('/').unwrap();
        (name, format!("/{path}"))
    } else {
        (service, "/StatusNotifierItem".to_string())
    }
}

/// Returns all active SNI items, sorted with NeedsAttention first.
#[tauri::command]
pub async fn get_sni_items(items: tauri::State<'_, SniItems>) -> Result<Vec<SniItem>, String> {
    let mut result: Vec<SniItem> = items
        .lock()
        .unwrap()
        .values()
        .filter(|item| item.status != "Passive")
        .cloned()
        .collect();

    result.sort_by(|a, b| {
        let att_a = a.status == "NeedsAttention";
        let att_b = b.status == "NeedsAttention";
        att_b.cmp(&att_a).then_with(|| a.title.cmp(&b.title))
    });

    Ok(result)
}

/// Activate a tray item (primary left-click action).
#[tauri::command]
pub async fn activate_sni_item(service: String) -> Result<(), String> {
    let conn = Connection::session()
        .await
        .map_err(|e| e.to_string())?;

    let (bus_name, obj_path) = parse_service_path(&service);

    let proxy = zbus::Proxy::new(
        &conn,
        bus_name,
        obj_path.as_str(),
        "org.kde.StatusNotifierItem",
    )
    .await
    .map_err(|e| e.to_string())?;

    proxy
        .call_method("Activate", &(0i32, 0i32))
        .await
        .map_err(|e| e.to_string())?;

    Ok(())
}

/// Starts the `org.kde.StatusNotifierWatcher` D-Bus service.
///
/// Runs inside the Tauri async runtime (tokio). If another watcher is already
/// running (e.g. KDE Plasma's), the name request will fail and this logs a
/// warning.
pub fn start(app: tauri::AppHandle, items: SniItems) {
    tauri::async_runtime::spawn(async move {
        let watcher = StatusNotifierWatcher {
            items: items.clone(),
            app_handle: app.clone(),
        };

        let builder = match connection::Builder::session()
            .and_then(|b| b.name("org.kde.StatusNotifierWatcher"))
            .and_then(|b| b.serve_at("/StatusNotifierWatcher", watcher))
        {
            Ok(b) => b,
            Err(e) => {
                log::warn!("sni: D-Bus config failed: {e}");
                return;
            }
        };

        let conn = match builder.build().await {
            Ok(c) => c,
            Err(e) => {
                log::warn!("sni: D-Bus start failed: {e}");
                return;
            }
        };

        log::info!("sni: StatusNotifierWatcher started");

        // Monitor D-Bus NameOwnerChanged to detect items disappearing.
        let items_clone = items.clone();
        let app_clone = app.clone();
        let conn_clone = conn.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(e) = monitor_name_changes(conn_clone, items_clone, app_clone).await {
                log::warn!("sni: name monitor error: {e}");
            }
        });

        // Keep connection alive for the process lifetime.
        std::future::pending::<()>().await;
        drop(conn);
    });
}

/// Watches `NameOwnerChanged` signals to remove items when their D-Bus owner
/// disappears (application exit).
async fn monitor_name_changes(
    conn: Connection,
    items: SniItems,
    app: tauri::AppHandle,
) -> Result<(), zbus::Error> {
    let proxy = zbus::fdo::DBusProxy::new(&conn).await?;
    let mut stream = proxy.receive_name_owner_changed().await?;

    while let Some(signal) = stream.next().await {
        if let Ok(args) = signal.args() {
            // new_owner is Optional<UniqueName>: None means the name was lost.
            if args.new_owner().is_none() {
                let name = args.name().to_string();
                let mut locked = items.lock().unwrap();
                if locked.remove(&name).is_some() {
                    log::info!("sni: item removed (owner lost): {name}");
                    let _ = app.emit("sni-items-changed", ());
                }
            }
        }
    }

    Ok(())
}

/// StatusNotifierItem (SNI) system tray watcher.
///
/// Implements `org.kde.StatusNotifierWatcher` on the session D-Bus. Tray-capable
/// applications (Discord, nm-applet, blueman, etc.) register themselves via
/// `RegisterStatusNotifierItem`. The watcher tracks registered items, fetches
/// their properties, and notifies the frontend via Tauri events.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use base64::Engine;
use futures_util::StreamExt;
use image::{ImageBuffer, Rgba};
use serde::{Deserialize, Serialize};
use tauri::Emitter;
use zbus::object_server::SignalContext;
use zbus::zvariant::{OwnedValue, Value};
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
    /// Base64-encoded PNG data URL from IconPixmap (if icon_name is empty).
    pub icon_pixmap: Option<String>,
    /// Tooltip title (from the ToolTip property struct).
    pub tooltip_title: Option<String>,
    /// Tooltip description (from the ToolTip property struct).
    pub tooltip_description: Option<String>,
    /// D-Bus object path for the com.canonical.dbusmenu interface (if any).
    pub menu_path: Option<String>,
}

/// A single entry in a DBusMenu (com.canonical.dbusmenu) tree.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbusMenuItem {
    /// Menu item ID (used in Event calls).
    pub id: i32,
    /// Item type: "standard", "separator".
    pub item_type: String,
    /// Display label (mnemonics stripped).
    pub label: String,
    /// Whether the item is clickable.
    pub enabled: bool,
    /// Whether the item is visible.
    pub visible: bool,
    /// Whether a toggle item is checked.
    pub checked: bool,
    /// Child items (submenus).
    pub children: Vec<DbusMenuItem>,
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

        let obj_path = if service.starts_with('/') {
            service.to_string()
        } else {
            "/StatusNotifierItem".to_string()
        };

        match fetch_item_properties(&bus_name, service).await {
            Ok(item) => {
                self.items.lock().unwrap().insert(bus_name.clone(), item);
                let _ = Self::status_notifier_item_registered(&ctx, &bus_name).await;
                let _ = self.app_handle.emit("sni-items-changed", ());

                // Monitor property change signals from this item.
                let items_clone = self.items.clone();
                let app_clone = self.app_handle.clone();
                let svc = bus_name.clone();
                let path = obj_path.clone();
                tauri::async_runtime::spawn(async move {
                    monitor_item_signals(&svc, &path, items_clone, app_clone).await;
                });
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
    let raw_title: String = proxy
        .get_property("Title")
        .await
        .unwrap_or_default();
    let icon_name: String = proxy.get_property("IconName").await.unwrap_or_default();

    // Fetch IconPixmap if IconName is empty (e.g. Electron apps like Discord).
    // D-Bus type: a(iiay) -- array of (width, height, ARGB bytes).
    let icon_pixmap = if icon_name.is_empty() {
        fetch_icon_pixmap(&proxy).await
    } else {
        None
    };

    // Fetch ToolTip: D-Bus type (sa(iiay)ss) -- (icon_name, icon_data, title, body).
    let (tooltip_title, tooltip_description) = fetch_tooltip(&proxy).await;

    // Best display name: tooltip > title > icon_name > process name > id.
    // Electron apps often return generic IDs like "chrome_status_icon_1",
    // but their tooltip usually contains the real app name.
    let title = if let Some(ref tt) = tooltip_title {
        if !tt.trim().is_empty() { tt.clone() }
        else { String::new() }
    } else { String::new() };

    let title = if !title.is_empty() {
        title
    } else if let Some(ref td) = tooltip_description {
        if !td.trim().is_empty() { td.clone() } else { String::new() }
    } else { String::new() };

    let title = if !title.is_empty() {
        title
    } else if !raw_title.trim().is_empty()
        && !raw_title.to_lowercase().contains("chrome_status")
        && !raw_title.to_lowercase().contains("status_icon")
    {
        raw_title
    } else if !icon_name.trim().is_empty() && !icon_name.contains("chrome") {
        capitalize(&icon_name)
    } else if let Some(proc_name) = get_process_name(&conn, bus_name).await {
        proc_name
    } else {
        id.clone()
    };

    // Fetch Menu object path (com.canonical.dbusmenu interface).
    let menu_path: Option<String> = match proxy
        .get_property::<zbus::zvariant::OwnedObjectPath>("Menu")
        .await
    {
        Ok(p) => {
            let path = p.to_string();
            log::debug!("sni: {bus_name} menu path: {path}");
            if path == "/" { None } else { Some(path) }
        }
        Err(e) => {
            log::debug!("sni: {bus_name} no menu path: {e}");
            None
        }
    };

    Ok(SniItem {
        service: bus_name.to_string(),
        id,
        category,
        status,
        title,
        icon_name,
        icon_pixmap,
        tooltip_title,
        tooltip_description,
        menu_path,
    })
}

/// Fetch the IconPixmap property and convert the largest icon to a base64 PNG
/// data URL. Returns `None` if the property is missing or conversion fails.
async fn fetch_icon_pixmap(proxy: &zbus::Proxy<'_>) -> Option<String> {
    let value: OwnedValue = proxy.get_property("IconPixmap").await.ok()?;
    // IconPixmap is a(iiay): Vec<(i32, i32, Vec<u8>)>.
    // zbus deserializes this as Vec<(i32, i32, Vec<u8>)>.
    let pixmaps: Vec<(i32, i32, Vec<u8>)> = zbus::zvariant::Value::try_from(value)
        .ok()
        .and_then(|v| TryFrom::try_from(v).ok())?;
    if pixmaps.is_empty() {
        return None;
    }
    // Pick the largest icon.
    let (width, height, data) = pixmaps
        .into_iter()
        .max_by_key(|(w, h, _)| (*w as i64) * (*h as i64))?;
    convert_argb_to_png(width, height, &data).ok()
}

/// Fetch the ToolTip property. Returns (title, description), both optional.
async fn fetch_tooltip(proxy: &zbus::Proxy<'_>) -> (Option<String>, Option<String>) {
    // ToolTip is (sa(iiay)ss): (icon_name, icon_pixmaps, title, body).
    // We only care about title (index 2) and body (index 3).
    let value: OwnedValue = match proxy.get_property("ToolTip").await {
        Ok(v) => v,
        Err(_) => return (None, None),
    };

    // Try to destructure as a 4-tuple. The icon_pixmap array makes direct
    // deserialization fragile, so extract via zvariant Structure fields.
    let structure = match zbus::zvariant::Value::try_from(value) {
        Ok(zbus::zvariant::Value::Structure(s)) => s,
        _ => return (None, None),
    };

    let fields = structure.fields();
    if fields.len() < 4 {
        return (None, None);
    }

    let title = match &fields[2] {
        zbus::zvariant::Value::Str(s) if !s.is_empty() => Some(s.to_string()),
        _ => None,
    };
    let body = match &fields[3] {
        zbus::zvariant::Value::Str(s) if !s.is_empty() => Some(s.to_string()),
        _ => None,
    };

    (title, body)
}

/// Convert ARGB pixel data (network byte order) to a base64 PNG data URL.
fn convert_argb_to_png(width: i32, height: i32, argb_data: &[u8]) -> Result<String, String> {
    let w = width as u32;
    let h = height as u32;
    let expected = (w * h * 4) as usize;
    if argb_data.len() < expected {
        return Err(format!(
            "pixel data too short: {} < {expected}",
            argb_data.len()
        ));
    }

    // ARGB (network byte order) to RGBA.
    let mut rgba = Vec::with_capacity(expected);
    for chunk in argb_data[..expected].chunks_exact(4) {
        let a = chunk[0];
        let r = chunk[1];
        let g = chunk[2];
        let b = chunk[3];
        rgba.extend_from_slice(&[r, g, b, a]);
    }

    let img: ImageBuffer<Rgba<u8>, Vec<u8>> =
        ImageBuffer::from_raw(w, h, rgba).ok_or("failed to create image buffer")?;

    let mut png_bytes: Vec<u8> = Vec::new();
    img.write_to(
        &mut std::io::Cursor::new(&mut png_bytes),
        image::ImageFormat::Png,
    )
    .map_err(|e| e.to_string())?;

    let b64 = base64::engine::general_purpose::STANDARD.encode(&png_bytes);
    Ok(format!("data:image/png;base64,{b64}"))
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

/// Layout node type from DBusMenu `GetLayout`: `(i32, a{sv}, av)`.
type LayoutNode = (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>);

/// Fetch the menu tree for an SNI item via com.canonical.dbusmenu.
///
/// Calls `GetLayout` on the item's Menu object path and recursively parses
/// the layout tree into `DbusMenuItem` entries.
#[tauri::command]
pub async fn get_sni_menu(
    service: String,
    menu_path: String,
) -> Result<Vec<DbusMenuItem>, String> {
    let conn = Connection::session()
        .await
        .map_err(|e| e.to_string())?;

    let proxy = zbus::Proxy::new(
        &conn,
        service.as_str(),
        menu_path.as_str(),
        "com.canonical.dbusmenu",
    )
    .await
    .map_err(|e| e.to_string())?;

    log::debug!("sni: getting menu from {service} at {menu_path}");

    // GetLayout(parentId: i32, recursionDepth: i32, propertyNames: as)
    // Returns: (revision: u32, layout: (i32, a{sv}, av))
    let empty_props: Vec<String> = Vec::new();
    let reply = proxy
        .call_method("GetLayout", &(0i32, -1i32, &empty_props))
        .await
        .map_err(|e| format!("GetLayout: {e}"))?;

    log::debug!("sni: GetLayout succeeded");

    let (_revision, root_layout): (u32, LayoutNode) = reply
        .body()
        .deserialize()
        .map_err(|e| format!("deserialize: {e}"))?;

    let (_root_id, _root_props, children) = root_layout;
    let items = parse_children(children);

    log::debug!("sni: parsed {} menu items", items.len());

    Ok(items)
}

/// Recursively parse child nodes from a DBusMenu layout.
fn parse_children(children: Vec<OwnedValue>) -> Vec<DbusMenuItem> {
    let mut items = Vec::new();
    for child in children {
        let node: LayoutNode = match child.try_into() {
            Ok(n) => n,
            Err(e) => {
                log::debug!("sni: skip child: {e}");
                continue;
            }
        };
        let (id, props, sub_children) = node;

        let item_type = prop_string(&props, "type").unwrap_or_else(|| "standard".into());
        let label = prop_string(&props, "label")
            .unwrap_or_default()
            .replace('_', "");
        let enabled = prop_bool(&props, "enabled").unwrap_or(true);
        let visible = prop_bool(&props, "visible").unwrap_or(true);
        let toggle_state = prop_i32(&props, "toggle-state").unwrap_or(-1);

        let children_items = if sub_children.is_empty() {
            Vec::new()
        } else {
            parse_children(sub_children)
        };

        items.push(DbusMenuItem {
            id,
            item_type,
            label,
            enabled,
            visible,
            checked: toggle_state == 1,
            children: children_items,
        });
    }
    items
}

/// Extract a string property from an `a{sv}` dict.
fn prop_string(props: &HashMap<String, OwnedValue>, key: &str) -> Option<String> {
    let v = props.get(key)?;
    let val = Value::try_from(v.clone()).ok()?;
    match unwrap_value(&val) {
        Value::Str(s) => Some(s.to_string()),
        _ => None,
    }
}

/// Extract a bool property from an `a{sv}` dict.
fn prop_bool(props: &HashMap<String, OwnedValue>, key: &str) -> Option<bool> {
    let v = props.get(key)?;
    let val = Value::try_from(v.clone()).ok()?;
    match unwrap_value(&val) {
        Value::Bool(b) => Some(*b),
        _ => None,
    }
}

/// Extract an i32 property from an `a{sv}` dict.
fn prop_i32(props: &HashMap<String, OwnedValue>, key: &str) -> Option<i32> {
    let v = props.get(key)?;
    let val = Value::try_from(v.clone()).ok()?;
    match unwrap_value(&val) {
        Value::I32(n) => Some(*n),
        _ => None,
    }
}

/// Resolve a human-readable process name from a D-Bus service via its PID.
///
/// Tries `/proc/<pid>/cmdline` first (scans for a non-generic executable name),
/// then `/proc/<pid>/comm`, then `/proc/<pid>/exe`. Skips generic names like
/// "electron", "chrome", "chromium".
async fn get_process_name(conn: &Connection, service: &str) -> Option<String> {
    let dbus_proxy = zbus::fdo::DBusProxy::new(conn).await.ok()?;
    let pid = dbus_proxy
        .get_connection_unix_process_id(service.try_into().ok()?)
        .await
        .ok()?;

    const GENERIC: &[&str] = &["electron", "chrome", "chromium"];

    // 1. /proc/<pid>/cmdline -- null-separated, often contains the real app name.
    if let Ok(cmdline) = std::fs::read_to_string(format!("/proc/{pid}/cmdline")) {
        let args: Vec<&str> = cmdline.split('\0').filter(|s| !s.is_empty()).collect();

        for arg in &args {
            let path = std::path::Path::new(arg);
            let Some(file_name) = path.file_name() else { continue };
            let name = file_name.to_string_lossy();
            let lower = name.to_lowercase();

            if name.starts_with('-')
                || lower.starts_with("lib")
                || lower.ends_with(".so")
                || GENERIC.contains(&lower.as_str())
            {
                continue;
            }

            // Looks like a real executable name.
            let cap = capitalize(&name);
            if !cap.is_empty() {
                return Some(cap);
            }
        }
    }

    // 2. /proc/<pid>/comm -- short name (max 16 chars), skip generic.
    if let Ok(raw) = std::fs::read_to_string(format!("/proc/{pid}/comm")) {
        let name = raw.trim();
        if !name.is_empty() && !GENERIC.contains(&name.to_lowercase().as_str()) {
            return Some(capitalize(name));
        }
    }

    // 3. /proc/<pid>/exe -- symlink target; try parent dir name then basename.
    if let Ok(target) = std::fs::read_link(format!("/proc/{pid}/exe")) {
        // Parent directory often is the app name (e.g. /opt/vesktop/vesktop).
        if let Some(parent) = target.parent().and_then(|p| p.file_name()) {
            let pname = parent.to_string_lossy().to_lowercase();
            if !["bin", "usr", "opt", "lib", "sbin"].contains(&pname.as_str()) {
                return Some(capitalize(&parent.to_string_lossy()));
            }
        }
        if let Some(name) = target.file_name() {
            let n = name.to_string_lossy();
            if !GENERIC.contains(&n.to_lowercase().as_str()) {
                return Some(capitalize(&n));
            }
        }
    }

    None
}

/// Capitalize the first character of a string.
fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first.to_uppercase().chain(chars).collect(),
    }
}

/// Unwrap nested D-Bus variant containers.
fn unwrap_value<'a>(v: &'a Value<'a>) -> &'a Value<'a> {
    match v {
        Value::Value(inner) => unwrap_value(inner),
        other => other,
    }
}

/// Send a click event to a DBusMenu item.
#[tauri::command]
pub async fn click_sni_menu_item(
    service: String,
    menu_path: String,
    item_id: i32,
) -> Result<(), String> {
    let conn = Connection::session()
        .await
        .map_err(|e| e.to_string())?;

    let proxy = zbus::Proxy::new(
        &conn,
        service.as_str(),
        menu_path.as_str(),
        "com.canonical.dbusmenu",
    )
    .await
    .map_err(|e| e.to_string())?;

    // Event(id: i32, eventId: String, data: Variant, timestamp: u32)
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as u32;

    proxy
        .call_method(
            "Event",
            &(item_id, "clicked", Value::I32(0), timestamp),
        )
        .await
        .map_err(|e| format!("Event: {e}"))?;

    Ok(())
}

/// Starts the `org.kde.StatusNotifierWatcher` D-Bus service.
///
/// Monitor property-change signals from a registered SNI item.
///
/// Listens for `NewTitle`, `NewIcon`, `NewStatus`, `NewToolTip` signals and
/// re-fetches the item properties when any of them fire.
async fn monitor_item_signals(
    bus_name: &str,
    obj_path: &str,
    items: SniItems,
    app: tauri::AppHandle,
) {
    let conn = match Connection::session().await {
        Ok(c) => c,
        Err(_) => return,
    };

    let proxy = match zbus::Proxy::new(
        &conn,
        bus_name,
        obj_path,
        "org.kde.StatusNotifierItem",
    )
    .await
    {
        Ok(p) => p,
        Err(_) => return,
    };

    let mut stream = match proxy.receive_all_signals().await {
        Ok(s) => s,
        Err(_) => return,
    };

    let service = bus_name.to_string();

    while let Some(signal) = stream.next().await {
        let name = signal
            .header()
            .member()
            .map(|m| m.to_string())
            .unwrap_or_default();

        match name.as_str() {
            "NewTitle" | "NewIcon" | "NewStatus" | "NewToolTip" | "NewIconThemePath" => {
                log::debug!("sni: signal {name} from {service}");
                if let Ok(updated) = fetch_item_properties(&service, &service).await {
                    items.lock().unwrap().insert(service.clone(), updated);
                    let _ = app.emit("sni-items-changed", ());
                }
            }
            _ => {}
        }
    }
}

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

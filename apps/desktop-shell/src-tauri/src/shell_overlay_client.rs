/// Wayland client for the `lunaris-shell-overlay-v1` protocol.
///
/// Binds the compositor's overlay global, accumulates context menu events, and
/// emits Tauri events to Svelte. Also exposes Tauri commands so Svelte can send
/// `activate` / `dismiss` requests back to the compositor.

// Protocol bindings following the pattern documented in wayland-scanner's lib.rs.
//
// The `mod protocol` wrapper is required so that `use wayland_client;` brings
// the external crate into scope as a name - `generate_client_code!` generates code
// that references `super::wayland_client` and `super::INTERFACE_CONSTANT`.
// `generate_interfaces!` must run first (inside `__interfaces`) to define those
// constants, which are then re-exported with `use self::__interfaces::*`.
mod protocol {
    // Make the `wayland_client` crate name accessible to generated sub-modules.
    #[allow(unused_imports)]
    pub use wayland_client;
    pub use wayland_client::protocol::*;

    pub mod __interfaces {
        use wayland_client::protocol::__interfaces::*;
        wayland_scanner::generate_interfaces!("protocols/lunaris-shell-overlay.xml");
    }
    use self::__interfaces::*;

    wayland_scanner::generate_client_code!("protocols/lunaris-shell-overlay.xml");
}

use protocol::lunaris_shell_overlay_v1 as overlay;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use wayland_client::{
    Connection, Dispatch, QueueHandle,
    globals::{GlobalListContents, registry_queue_init},
    protocol::wl_registry,
};

/// Type alias for the generated proxy type.
type OverlayProxy = overlay::LunarisShellOverlayV1;

/// Describes which parts of the layer-shell surface accept pointer input.
#[derive(Debug, Clone, Copy)]
enum InputRegionMode {
    /// Only the top 36px bar.
    BarOnly,
    /// Full screen (context menu active).
    FullScreen,
    /// Top bar plus a notification area in the top-right corner.
    WithNotifications,
    /// Top bar plus a centered popover area below the bar.
    WithPopover,
}

/// Set the input region on the GTK layer-shell window and flush immediately.
///
/// Calls `input_shape_combine_region` directly on the `gtk::Window`
/// obtained via `webview.inner().toplevel()` -- the same surface that
/// `layer_shell.rs` configures.
fn set_input_region(app: &tauri::AppHandle, mode: InputRegionMode) {
    let Some(w) = app.get_webview_window("main") else {
        return;
    };
    let _ = w.with_webview(move |webview| {
        use gtk::prelude::{Cast, WidgetExt};
        use gtk::cairo::{RectangleInt, Region};

        let Some(toplevel) = webview.inner().toplevel() else { return };
        let Ok(gtk_window) = toplevel.downcast::<gtk::Window>() else { return };

        let region = match mode {
            InputRegionMode::FullScreen => {
                log::info!("set_input_region: FullScreen (0,0 32767x32767)");
                Region::create_rectangle(&RectangleInt::new(0, 0, 32767, 32767))
            }
            InputRegionMode::BarOnly => {
                log::info!("set_input_region: BarOnly (0,0 32767x36)");
                Region::create_rectangle(&RectangleInt::new(0, 0, 32767, 36))
            }
            InputRegionMode::WithNotifications => {
                // Top bar: full width, 36px tall.
                let r = Region::create_rectangle(&RectangleInt::new(0, 0, 32767, 36));
                // Notification area: rightmost 420px, below bar, 300px tall.
                // Use allocated_width directly -- it reflects the compositor-assigned size.
                let alloc_w = gtk_window.allocated_width();
                let notif_w = 380;
                let notif_x = if alloc_w > notif_w { alloc_w - notif_w } else { 0 };
                log::info!(
                    "set_input_region: WithNotifications allocated_width={} \
                     notif_rect=({}, 36, {}, 300)",
                    alloc_w, notif_x, notif_w,
                );
                let notif = Region::create_rectangle(&RectangleInt::new(
                    notif_x, 36, notif_w, 300,
                ));
                r.union(&notif);
                r
            }
            InputRegionMode::WithPopover => {
                // Top bar + centered popover below bar.
                let r = Region::create_rectangle(&RectangleInt::new(0, 0, 32767, 36));
                let alloc_w = gtk_window.allocated_width();
                // Popover: 600px wide, 200px tall, centered horizontally.
                let pop_w = 600;
                let pop_h = 200;
                let pop_x = if alloc_w > pop_w { (alloc_w - pop_w) / 2 } else { 0 };
                log::info!(
                    "set_input_region: WithPopover allocated_width={} \
                     popover_rect=({}, 36, {}, {})",
                    alloc_w, pop_x, pop_w, pop_h,
                );
                let popover = Region::create_rectangle(&RectangleInt::new(
                    pop_x, 36, pop_w, pop_h,
                ));
                r.union(&popover);
                r
            }
        };

        gtk_window.input_shape_combine_region(Some(&region));
        gtk_window.queue_draw();

        if let Some(display) = gtk::gdk::Display::default() {
            display.flush();
        }
    });
}

/// Tracks whether a context menu or notifications are currently active.
/// Both may be true simultaneously; the correct input region is computed
/// from the combination.
static MENU_ACTIVE: AtomicBool = AtomicBool::new(false);
static NOTIFICATIONS_ACTIVE: AtomicBool = AtomicBool::new(false);
static POPOVER_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Recomputes and applies the correct input region from current state.
fn update_input_region(app: &tauri::AppHandle) {
    let menu = MENU_ACTIVE.load(Ordering::SeqCst);
    let notif = NOTIFICATIONS_ACTIVE.load(Ordering::SeqCst);
    let popover = POPOVER_ACTIVE.load(Ordering::SeqCst);
    let mode = if menu {
        InputRegionMode::FullScreen
    } else if popover {
        InputRegionMode::WithPopover
    } else if notif {
        InputRegionMode::WithNotifications
    } else {
        InputRegionMode::BarOnly
    };
    log::info!(
        "update_input_region: menu={} popover={} notif={} -> {:?}",
        menu, popover, notif, mode,
    );
    set_input_region(app, mode);
}

/// Called when a context menu opens or closes.
fn set_menu_active(app: &tauri::AppHandle, active: bool) {
    MENU_ACTIVE.store(active, Ordering::SeqCst);
    update_input_region(app);
}

// ===== Payload types emitted as Tauri events =====

/// A single entry in a context menu. Both regular items and separators are
/// represented in the same flat array so Svelte can render them in order.
#[derive(Clone, Serialize)]
pub struct MenuItemPayload {
    /// Position index as assigned by the compositor (stable across the menu lifetime).
    pub index: u32,
    /// `"entry"` for a clickable item, `"separator"` for a visual divider.
    pub kind: String,
    /// WindowAction value (1-18). `None` for separators.
    pub action: Option<u32>,
    /// Human-readable label derived from `action`. `None` for separators.
    pub label: Option<String>,
    /// Whether the item is in a toggled/checked state. `None` for separators.
    pub toggled: Option<bool>,
    /// Whether the item is disabled/grayed out. `None` for separators.
    pub disabled: Option<bool>,
    /// Keyboard shortcut label (may be an empty string). `None` for separators.
    pub shortcut: Option<String>,
}

// ===== Tab bar payload types =====

#[derive(Clone, Serialize)]
struct TabBarShowPayload {
    stack_id: u32,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

#[derive(Clone, Serialize)]
struct TabBarHidePayload {
    stack_id: u32,
}

#[derive(Clone, Serialize)]
struct TabAddedPayload {
    stack_id: u32,
    index: u32,
    title: String,
    app_id: String,
    active: bool,
}

#[derive(Clone, Serialize)]
struct TabRemovedPayload {
    stack_id: u32,
    index: u32,
}

#[derive(Clone, Serialize)]
struct TabActivatedPayload {
    stack_id: u32,
    index: u32,
}

#[derive(Clone, Serialize)]
struct TabTitleChangedPayload {
    stack_id: u32,
    index: u32,
    title: String,
}

// ===== Window header payload types =====

#[derive(Clone, Serialize)]
struct WindowHeaderShowPayload {
    surface_id: u32,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    title: String,
    activated: bool,
    has_minimize: bool,
    has_maximize: bool,
}

#[derive(Clone, Serialize)]
struct WindowHeaderUpdatePayload {
    surface_id: u32,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    title: String,
    activated: bool,
}

#[derive(Clone, Serialize)]
struct WindowHeaderHidePayload {
    surface_id: u32,
}

// ===== Zoom toolbar payload types =====

#[derive(Clone, Serialize)]
struct ZoomToolbarShowPayload {
    level: f64,
    increment: u32,
    movement: u32,
}

#[derive(Clone, Serialize)]
struct ZoomToolbarUpdatePayload {
    level: f64,
}

// ===== Indicator payload types =====

#[derive(Clone, Serialize)]
struct IndicatorShowPayload {
    kind: u32,
    edges: u32,
    direction: u32,
    shortcut1: String,
    shortcut2: String,
}

#[derive(Clone, Serialize)]
struct IndicatorHidePayload {
    kind: u32,
}

// ===== Context menu payload types =====

#[derive(Clone, Serialize)]
struct ContextMenuShowPayload {
    menu_id: u32,
    /// X position in compositor global coordinates.
    x: i32,
    /// Y position in compositor global coordinates.
    y: i32,
    items: Vec<MenuItemPayload>,
}

#[derive(Clone, Serialize)]
struct ContextMenuHidePayload {
    menu_id: u32,
}

// ===== Accumulator for in-flight menus =====

struct PendingMenu {
    x: i32,
    y: i32,
    items: Vec<MenuItemPayload>,
}

// ===== Dispatch state for the client thread =====

struct AppData {
    app_handle: AppHandle,
    /// Menus currently being assembled (between `context_menu_begin` and `context_menu_done`).
    pending_menus: HashMap<u32, PendingMenu>,
}

/// No-op impl required by `registry_queue_init::<AppData>`.
impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for AppData {
    fn event(
        _state: &mut Self,
        _registry: &wl_registry::WlRegistry,
        _event: wl_registry::Event,
        _data: &GlobalListContents,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<OverlayProxy, ()> for AppData {
    fn event(
        state: &mut Self,
        _proxy: &OverlayProxy,
        event: overlay::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            overlay::Event::ContextMenuBegin { menu_id, x, y } => {
                log::info!(
                    "shell_overlay_client: context_menu_begin menu_id={menu_id} x={x} y={y}"
                );
                state.pending_menus.insert(
                    menu_id,
                    PendingMenu {
                        x,
                        y,
                        items: Vec::new(),
                    },
                );
            }

            overlay::Event::ContextMenuItem {
                menu_id,
                index,
                action,
                toggled,
                disabled,
                shortcut,
            } => {
                if let Some(menu) = state.pending_menus.get_mut(&menu_id) {
                    let action_u32 = match action {
                        wayland_client::WEnum::Value(v) => v as u32,
                        wayland_client::WEnum::Unknown(v) => v,
                    };
                    menu.items.push(MenuItemPayload {
                        index,
                        kind: "entry".into(),
                        action: Some(action_u32),
                        label: Some(action_label(action_u32)),
                        toggled: Some(toggled != 0),
                        disabled: Some(disabled != 0),
                        shortcut: Some(shortcut),
                    });
                }
            }

            overlay::Event::ContextMenuSeparator { menu_id, index } => {
                if let Some(menu) = state.pending_menus.get_mut(&menu_id) {
                    menu.items.push(MenuItemPayload {
                        index,
                        kind: "separator".into(),
                        action: None,
                        label: None,
                        toggled: None,
                        disabled: None,
                        shortcut: None,
                    });
                }
            }

            overlay::Event::ContextMenuDone { menu_id } => {
                if let Some(menu) = state.pending_menus.remove(&menu_id) {
                    let payload = ContextMenuShowPayload {
                        menu_id,
                        x: menu.x,
                        y: menu.y,
                        items: menu.items,
                    };
                    log::info!(
                        "shell_overlay_client: emitting context-menu-show \
                         menu_id={menu_id} items={}",
                        payload.items.len()
                    );
                    if let Err(e) = state.app_handle.emit("lunaris://context-menu-show", payload) {
                        log::error!("shell_overlay_client: emit context-menu-show failed: {e}");
                    }
                    // Open the full window to input so menu items can receive clicks.
                    set_menu_active(&state.app_handle, true);
                }
            }

            overlay::Event::ContextMenuClosed { menu_id } => {
                log::info!("shell_overlay_client: ContextMenuClosed received menu_id={menu_id}");
                state.pending_menus.remove(&menu_id);
                let _ = state
                    .app_handle
                    .emit("lunaris://context-menu-hide", ContextMenuHidePayload { menu_id });
                // Restore click-through below the top bar with immediate flush.
                set_menu_active(&state.app_handle, false);
            }

            overlay::Event::TabBarShow { stack_id, x, y, width, height } => {
                let _ = state.app_handle.emit(
                    "lunaris://tab-bar-show",
                    TabBarShowPayload { stack_id, x, y, width, height },
                );
            }

            overlay::Event::TabBarHide { stack_id } => {
                let _ = state.app_handle.emit(
                    "lunaris://tab-bar-hide",
                    TabBarHidePayload { stack_id },
                );
            }

            overlay::Event::TabAdded { stack_id, index, title, app_id, active } => {
                let _ = state.app_handle.emit(
                    "lunaris://tab-added",
                    TabAddedPayload { stack_id, index, title, app_id, active: active != 0 },
                );
            }

            overlay::Event::TabRemoved { stack_id, index } => {
                let _ = state.app_handle.emit(
                    "lunaris://tab-removed",
                    TabRemovedPayload { stack_id, index },
                );
            }

            overlay::Event::TabActivated { stack_id, index } => {
                let _ = state.app_handle.emit(
                    "lunaris://tab-activated",
                    TabActivatedPayload { stack_id, index },
                );
            }

            overlay::Event::TabTitleChanged { stack_id, index, title } => {
                let _ = state.app_handle.emit(
                    "lunaris://tab-title-changed",
                    TabTitleChangedPayload { stack_id, index, title },
                );
            }

            overlay::Event::IndicatorShow { kind, edges, direction, shortcut1, shortcut2 } => {
                let kind_u32 = match kind {
                    wayland_client::WEnum::Value(v) => v as u32,
                    wayland_client::WEnum::Unknown(v) => v,
                };
                let _ = state.app_handle.emit(
                    "lunaris://indicator-show",
                    IndicatorShowPayload { kind: kind_u32, edges, direction, shortcut1, shortcut2 },
                );
            }

            overlay::Event::IndicatorHide { kind } => {
                let kind_u32 = match kind {
                    wayland_client::WEnum::Value(v) => v as u32,
                    wayland_client::WEnum::Unknown(v) => v,
                };
                let _ = state.app_handle.emit(
                    "lunaris://indicator-hide",
                    IndicatorHidePayload { kind: kind_u32 },
                );
            }

            overlay::Event::ZoomToolbarShow { level, increment, movement } => {
                let movement_u32 = match movement {
                    wayland_client::WEnum::Value(v) => v as u32,
                    wayland_client::WEnum::Unknown(v) => v,
                };
                let _ = state.app_handle.emit(
                    "lunaris://zoom-toolbar-show",
                    ZoomToolbarShowPayload {
                        level,
                        increment,
                        movement: movement_u32,
                    },
                );
            }

            overlay::Event::ZoomToolbarUpdate { level } => {
                let _ = state.app_handle.emit(
                    "lunaris://zoom-toolbar-update",
                    ZoomToolbarUpdatePayload { level },
                );
            }

            overlay::Event::ZoomToolbarHide => {
                let _ = state.app_handle.emit("lunaris://zoom-toolbar-hide", ());
            }

            overlay::Event::WindowHeaderShow {
                surface_id, x, y, width, height, title, activated, has_minimize, has_maximize,
            } => {
                let _ = state.app_handle.emit(
                    "lunaris://window-header-show",
                    WindowHeaderShowPayload {
                        surface_id, x, y, width, height, title,
                        activated: activated != 0,
                        has_minimize: has_minimize != 0,
                        has_maximize: has_maximize != 0,
                    },
                );
            }

            overlay::Event::WindowHeaderUpdate {
                surface_id, x, y, width, height, title, activated,
            } => {
                let _ = state.app_handle.emit(
                    "lunaris://window-header-update",
                    WindowHeaderUpdatePayload {
                        surface_id, x, y, width, height, title,
                        activated: activated != 0,
                    },
                );
            }

            overlay::Event::WindowHeaderHide { surface_id } => {
                let _ = state.app_handle.emit(
                    "lunaris://window-header-hide",
                    WindowHeaderHidePayload { surface_id },
                );
            }

            overlay::Event::WaypointerOpen => {
                log::info!("shell_overlay_client: WaypointerOpen received");
                crate::waypointer::toggle(&state.app_handle);
            }

            _ => {}
        }
    }
}

/// Returns a human-readable label for a WindowAction value (1-18).
fn action_label(action: u32) -> String {
    match action {
        1 => "Minimize",
        2 => "Maximize",
        3 => "Fullscreen",
        4 => "Tiled",
        5 => "Move",
        6 => "Resize (Top)",
        7 => "Resize (Left)",
        8 => "Resize (Right)",
        9 => "Resize (Bottom)",
        10 => "Stack",
        11 => "Unstack Tab",
        12 => "Unstack All",
        13 => "Screenshot",
        14 => "Move to Previous Workspace",
        15 => "Move to Next Workspace",
        16 => "Sticky",
        17 => "Close",
        18 => "Close All",
        _ => "Unknown",
    }
    .into()
}

// ===== Shared state for Tauri commands =====

/// Holds the overlay proxy and connection so Tauri commands can send requests
/// back to the compositor from any thread.
pub struct ShellOverlaySender {
    proxy: Mutex<Option<OverlayProxy>>,
    conn: Mutex<Option<Connection>>,
}

impl ShellOverlaySender {
    /// Creates an empty sender. The proxy and connection are populated once
    /// the Wayland client thread successfully binds the global.
    pub fn new() -> Self {
        Self {
            proxy: Mutex::new(None),
            conn: Mutex::new(None),
        }
    }

    /// Sends an `activate` request and flushes the connection immediately.
    pub fn activate(&self, menu_id: u32, index: u32) {
        if let Some(p) = self.proxy.lock().unwrap().as_ref() {
            p.activate(menu_id, index);
            self.flush();
        }
    }

    /// Sends a `dismiss` request and flushes the connection immediately.
    pub fn dismiss(&self, menu_id: u32) {
        if let Some(p) = self.proxy.lock().unwrap().as_ref() {
            p.dismiss(menu_id);
            self.flush();
        }
    }

    /// Sends a `tab_activate` request and flushes the connection immediately.
    pub fn tab_activate(&self, stack_id: u32, index: u32) {
        if let Some(p) = self.proxy.lock().unwrap().as_ref() {
            p.tab_activate(stack_id, index);
            self.flush();
        }
    }

    pub fn zoom_increase(&self) {
        if let Some(p) = self.proxy.lock().unwrap().as_ref() {
            p.zoom_increase();
            self.flush();
        }
    }

    pub fn zoom_decrease(&self) {
        if let Some(p) = self.proxy.lock().unwrap().as_ref() {
            p.zoom_decrease();
            self.flush();
        }
    }

    pub fn zoom_close(&self) {
        if let Some(p) = self.proxy.lock().unwrap().as_ref() {
            p.zoom_close();
            self.flush();
        }
    }

    pub fn zoom_set_increment(&self, value: u32) {
        if let Some(p) = self.proxy.lock().unwrap().as_ref() {
            p.zoom_set_increment(value);
            self.flush();
        }
    }

    pub fn window_header_action(&self, surface_id: u32, action: u32) {
        if let Some(p) = self.proxy.lock().unwrap().as_ref() {
            if let Ok(a) = overlay::WindowHeaderActionType::try_from(action) {
                p.window_header_action(surface_id, a);
                self.flush();
            }
        }
    }

    pub fn zoom_set_movement(&self, mode: u32) {
        if let Some(p) = self.proxy.lock().unwrap().as_ref() {
            if let Ok(m) = overlay::ZoomMovement::try_from(mode) {
                p.zoom_set_movement(m);
                self.flush();
            }
        }
    }

    fn flush(&self) {
        if let Some(c) = self.conn.lock().unwrap().as_ref() {
            if let Err(e) = c.flush() {
                log::warn!("shell_overlay_client: flush failed: {e}");
            }
        }
    }
}

// ===== Thread entry point =====

/// Spawns the overlay client thread and populates `sender` once the global is bound.
pub fn start(app_handle: AppHandle, sender: Arc<ShellOverlaySender>) {
    std::thread::spawn(move || {
        let conn = loop {
            match Connection::connect_to_env() {
                Ok(c) => break c,
                Err(e) => {
                    log::debug!(
                        "shell_overlay_client: compositor not ready, retrying in 1s: {e}"
                    );
                    std::thread::sleep(std::time::Duration::from_secs(1));
                }
            }
        };

        let (globals, mut event_queue) = match registry_queue_init::<AppData>(&conn) {
            Ok(r) => r,
            Err(e) => {
                log::error!("shell_overlay_client: registry init failed: {e}");
                return;
            }
        };

        let qh = event_queue.handle();

        let overlay_proxy = match globals.bind::<OverlayProxy, AppData, ()>(&qh, 1..=1, ()) {
            Ok(p) => p,
            Err(e) => {
                log::warn!(
                    "shell_overlay_client: compositor does not expose \
                     lunaris_shell_overlay_v1: {e}"
                );
                return;
            }
        };

        log::info!("shell_overlay_client: lunaris_shell_overlay_v1 global bound successfully");

        // Share the proxy and connection so Tauri commands can send requests.
        *sender.proxy.lock().unwrap() = Some(overlay_proxy);
        *sender.conn.lock().unwrap() = Some(conn);

        let mut app_data = AppData {
            app_handle,
            pending_menus: HashMap::new(),
        };

        loop {
            if let Err(e) = event_queue.blocking_dispatch(&mut app_data) {
                log::error!("shell_overlay_client: dispatch error, thread exiting: {e}");
                break;
            }
        }
    });
}

// ===== Tauri commands =====

/// Called by Svelte when the user clicks a context menu item.
#[tauri::command]
pub fn context_menu_activate(
    state: tauri::State<Arc<ShellOverlaySender>>,
    menu_id: u32,
    index: u32,
) {
    state.activate(menu_id, index);
}

/// Called by Svelte when the user dismisses the context menu without selecting an item.
#[tauri::command]
pub fn context_menu_dismiss(state: tauri::State<Arc<ShellOverlaySender>>, menu_id: u32) {
    state.dismiss(menu_id);
}

/// Called by Svelte when the user clicks a tab in a tab bar.
#[tauri::command]
pub fn tab_activate(
    state: tauri::State<Arc<ShellOverlaySender>>,
    stack_id: u32,
    index: u32,
) {
    state.tab_activate(stack_id, index);
}

#[tauri::command]
pub fn zoom_increase(state: tauri::State<Arc<ShellOverlaySender>>) {
    state.zoom_increase();
}

#[tauri::command]
pub fn zoom_decrease(state: tauri::State<Arc<ShellOverlaySender>>) {
    state.zoom_decrease();
}

#[tauri::command]
pub fn zoom_close(state: tauri::State<Arc<ShellOverlaySender>>) {
    state.zoom_close();
}

#[tauri::command]
pub fn zoom_set_increment(state: tauri::State<Arc<ShellOverlaySender>>, value: u32) {
    state.zoom_set_increment(value);
}

#[tauri::command]
pub fn zoom_set_movement(state: tauri::State<Arc<ShellOverlaySender>>, mode: u32) {
    state.zoom_set_movement(mode);
}

#[tauri::command]
pub fn window_header_action(
    state: tauri::State<Arc<ShellOverlaySender>>,
    surface_id: u32,
    action: u32,
) {
    state.window_header_action(surface_id, action);
}

/// Expand or restore the input region for toast notifications.
///
/// When `expanded` is true, the input region includes a 420x300 rectangle
/// in the top-right corner (below the 36px bar) so toasts are clickable.
/// When false, it reverts to the bar-only region (unless a context menu is active).
#[tauri::command]
pub fn set_notification_input_region(app: tauri::AppHandle, expanded: bool) {
    log::info!(
        "set_notification_input_region: expanded={} (was {})",
        expanded, NOTIFICATIONS_ACTIVE.load(Ordering::SeqCst),
    );
    NOTIFICATIONS_ACTIVE.store(expanded, Ordering::SeqCst);
    update_input_region(&app);
}

/// Expand or restore the input region for the workspace popover.
#[tauri::command]
pub fn set_popover_input_region(app: tauri::AppHandle, expanded: bool) {
    log::info!(
        "set_popover_input_region: expanded={} (was {})",
        expanded, POPOVER_ACTIVE.load(Ordering::SeqCst),
    );
    POPOVER_ACTIVE.store(expanded, Ordering::SeqCst);
    update_input_region(&app);
}

/// Resolves a freedesktop app icon path for the given `app_id`.
///
/// Searches standard icon theme directories for a matching `.png` or `.svg`.
/// Returns `None` if no icon is found.
/// Resolves a freedesktop app icon and returns it as a base64 data URL.
#[tauri::command]
pub fn resolve_app_icon(app_id: String) -> Option<String> {
    use base64::Engine;

    // If the icon_name is an absolute path, read it directly.
    if app_id.starts_with('/') {
        let path = std::path::Path::new(&app_id);
        if path.exists() {
            let mime = match path.extension().and_then(|e| e.to_str()) {
                Some("png") => "image/png",
                Some("svg") => "image/svg+xml",
                Some("xpm") => "image/x-xpixmap",
                _ => "image/png",
            };
            if let Some(url) = read_as_data_url(&app_id, mime) {
                return Some(url);
            }
        }
        return None;
    }

    let png_sizes = ["48x48", "64x64", "32x32", "128x128", "256x256"];
    let themes = ["hicolor", "Adwaita"];
    let mut base_dirs = vec![
        "/usr/share/icons".to_string(),
        "/usr/local/share/icons".to_string(),
    ];
    // User icon dirs (Steam, Flatpak, etc.)
    if let Some(home) = std::env::var("HOME").ok() {
        base_dirs.push(format!("{home}/.local/share/icons"));
        base_dirs.push(format!("{home}/.local/share/flatpak/exports/share/icons"));
    }
    base_dirs.push("/var/lib/flatpak/exports/share/icons".to_string());

    // Pass 1: PNG in raster sizes.
    for base in &base_dirs {
        for theme in &themes {
            for size in &png_sizes {
                let path = format!("{base}/{theme}/{size}/apps/{app_id}.png");
                if let Some(url) = read_as_data_url(&path, "image/png") {
                    log::info!("resolve_app_icon: FOUND \"{path}\"");
                    return Some(url);
                }
            }
        }
    }

    // Pass 2: scalable SVG.
    for base in &base_dirs {
        for theme in &themes {
            let path = format!("{base}/{theme}/scalable/apps/{app_id}.svg");
            if let Some(url) = read_as_data_url(&path, "image/svg+xml") {
                log::info!("resolve_app_icon: FOUND (svg) \"{path}\"");
                return Some(url);
            }
        }
    }

    // Pass 3: pixmaps.
    let pixmap_exts: &[(&str, &str)] = &[("png", "image/png"), ("svg", "image/svg+xml")];
    for (ext, mime) in pixmap_exts {
        let path = format!("/usr/share/pixmaps/{app_id}.{ext}");
        if let Some(url) = read_as_data_url(&path, mime) {
            log::info!("resolve_app_icon: FOUND (pixmaps) \"{path}\"");
            return Some(url);
        }
    }

    log::info!("resolve_app_icon: NOT FOUND for \"{app_id}\"");
    None
}

/// Reads a file and encodes it as a `data:` URL. Returns `None` if the
/// file does not exist or cannot be read.
fn read_as_data_url(path: &str, mime: &str) -> Option<String> {
    use base64::Engine;
    let bytes = std::fs::read(path).ok()?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Some(format!("data:{mime};base64,{b64}"))
}

/// Diagnostic command: logs workspace data received from the Svelte store.
#[tauri::command]
pub fn debug_workspace_update(data: String) {
    log::info!("debug_workspace_update: {data}");
}

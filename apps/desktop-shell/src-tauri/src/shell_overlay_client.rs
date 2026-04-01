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
                    if let Some(w) = state.app_handle.get_webview_window("main") {
                        if let Err(e) = w.set_ignore_cursor_events(false) {
                            log::warn!("shell_overlay_client: set_ignore_cursor_events(false) failed: {e}");
                        }
                    }
                }
            }

            overlay::Event::ContextMenuClosed { menu_id } => {
                state.pending_menus.remove(&menu_id);
                let _ = state
                    .app_handle
                    .emit("lunaris://context-menu-hide", ContextMenuHidePayload { menu_id });
                // Restore click-through below the top bar.
                if let Some(w) = state.app_handle.get_webview_window("main") {
                    if let Err(e) = w.set_ignore_cursor_events(true) {
                        log::warn!("shell_overlay_client: set_ignore_cursor_events(true) failed: {e}");
                    }
                }
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

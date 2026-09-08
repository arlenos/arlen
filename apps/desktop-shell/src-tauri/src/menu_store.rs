//! Global menu bar store.
//!
//! Apps register their menu structure via `register_menu`. The frontend
//! subscribes to changes and renders the active app's menu in the top bar.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{AppHandle, Emitter};

/// Shared menu store managed by Tauri.
pub type AppMenuStore = Arc<Mutex<HashMap<String, serde_json::Value>>>;

/// Payload emitted when a menu is registered or updated.
#[derive(Clone, Serialize)]
struct MenuRegisteredPayload {
    app_id: String,
    items: serde_json::Value,
}

/// Payload emitted when a menu is unregistered.
#[derive(Clone, Serialize)]
struct MenuUnregisteredPayload {
    app_id: String,
}

/// Payload emitted when a menu action is dispatched.
#[derive(Clone, Serialize)]
struct MenuActionPayload {
    app_id: String,
    action: String,
}

/// Insert a menu into the store and emit the registration event.
/// Used by the GTK menu bridge (which holds a raw Arc, not tauri::State).
pub fn store_register(
    app: &AppHandle,
    store: &AppMenuStore,
    app_id: String,
    items: serde_json::Value,
) {
    store.lock().unwrap().insert(app_id.clone(), items.clone());
    let _ = app.emit("arlen://menu-registered", MenuRegisteredPayload { app_id, items });
}

/// Remove a menu from the store and emit the unregistration event.
/// Drop an app's menu and tell the frontend.
///
/// A plain function rather than a command, which is the whole shape of this
/// module now. There were `register_menu` and `unregister_menu` commands beside
/// these until 9 September, and no app could ever reach them: a Tauri command
/// does not cross an app boundary, and the shell's own webview has no app menu
/// to declare. Every real registration arrives over the bus
/// (`app.menu.registered`, handled in `event_bus.rs`) or from the GTK bridge,
/// and both call in here directly.
pub fn store_unregister(app: &AppHandle, store: &AppMenuStore, app_id: &str) {
    store.lock().unwrap().remove(app_id);
    let _ = app.emit(
        "arlen://menu-unregistered",
        MenuUnregisteredPayload {
            app_id: app_id.to_string(),
        },
    );
}

/// Dispatch a menu action.
///
/// For GTK apps (reverse-domain app_id containing `.`), the action is
/// sent via D-Bus `org.gtk.Actions.Activate`. For other apps (Tauri,
/// Electron), a Tauri event is emitted for the frontend to handle.
#[tauri::command]
pub fn dispatch_menu_action(app: AppHandle, app_id: String, action: String) {
    if app_id.contains('.') {
        // GTK app: activate via D-Bus on a background thread.
        let aid = app_id.clone();
        let act = action.clone();
        std::thread::spawn(move || match crate::gtk_menu_bridge::activate_gtk_action(&aid, &act) {
            Ok(true) => {}
            // An Arlen app owns no such bus name and takes its actions off the
            // Event Bus below, so this is the normal path for one of ours.
            Ok(false) => log::debug!("dispatch_menu_action: {aid} is not a GTK app"),
            Err(e) => log::warn!("dispatch_menu_action: D-Bus activate failed: {e}"),
        });
    }
    // Record the menu interaction on the Event Bus (GAP-10) so it reaches the
    // Knowledge Graph like toolbar and shortcut actions, not only the local
    // webview. Emitted before the move into the webview payload below.
    crate::event_bus::emit_menu_action_invoked(&app_id, &action);
    // Always emit the event (frontend may want to track it).
    let _ = app.emit("arlen://menu-action", MenuActionPayload { app_id, action });
}

/// Get the current menu for a given app_id (used on initial load).
///
/// The store holds whole trees and nothing else. A `set_menu_state` command sat
/// below this until 9 September, with a recursive merge and five tests, for
/// `shell.menu.setState` - an app changing one item's `enabled`, `label` or
/// `checked` without re-publishing. It was unreachable for the same reason its
/// two neighbours were (a Tauri command does not cross an app boundary) and it
/// was also a SECOND path: `Menu::register` replaces wholesale, its own doc calls
/// that the intended way to change a menu, and `apps/meetings` already keeps a
/// checkmark current by re-registering the tree. Adding a bus topic and a handler
/// would have built a faster road to a place the live one already reaches.
#[tauri::command]
pub fn get_menu(store: tauri::State<AppMenuStore>, app_id: String) -> Option<serde_json::Value> {
    store.lock().unwrap().get(&app_id).cloned()
}

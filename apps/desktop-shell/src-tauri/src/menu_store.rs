/// Global menu bar store.
///
/// Apps register their menu structure via `register_menu`. The frontend
/// subscribes to changes and renders the active app's menu in the top bar.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
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
    let _ = app.emit("lunaris://menu-registered", MenuRegisteredPayload { app_id, items });
}

/// Remove a menu from the store and emit the unregistration event.
pub fn store_unregister(app: &AppHandle, store: &AppMenuStore, app_id: &str) {
    store.lock().unwrap().remove(app_id);
    let _ = app.emit(
        "lunaris://menu-unregistered",
        MenuUnregisteredPayload {
            app_id: app_id.to_string(),
        },
    );
}

/// Register or update the menu structure for an app.
#[tauri::command]
pub fn register_menu(
    app: AppHandle,
    store: tauri::State<AppMenuStore>,
    app_id: String,
    items: serde_json::Value,
) {
    store.lock().unwrap().insert(app_id.clone(), items.clone());
    let _ = app.emit("lunaris://menu-registered", MenuRegisteredPayload { app_id, items });
}

/// Remove the menu for an app.
#[tauri::command]
pub fn unregister_menu(
    app: AppHandle,
    store: tauri::State<AppMenuStore>,
    app_id: String,
) {
    store.lock().unwrap().remove(&app_id);
    let _ = app.emit("lunaris://menu-unregistered", MenuUnregisteredPayload { app_id });
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
        std::thread::spawn(move || {
            if let Err(e) = crate::gtk_menu_bridge::activate_gtk_action(&aid, &act) {
                log::warn!("dispatch_menu_action: D-Bus activate failed: {e}");
            }
        });
    }
    // Always emit the event (frontend may want to track it).
    let _ = app.emit("lunaris://menu-action", MenuActionPayload { app_id, action });
}

/// Get the current menu for a given app_id (used on initial load).
#[tauri::command]
pub fn get_menu(store: tauri::State<AppMenuStore>, app_id: String) -> Option<serde_json::Value> {
    store.lock().unwrap().get(&app_id).cloned()
}

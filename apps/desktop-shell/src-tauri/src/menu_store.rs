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

/// Dispatch a menu action back to the frontend (or to the app via IPC).
#[tauri::command]
pub fn dispatch_menu_action(app: AppHandle, app_id: String, action: String) {
    let _ = app.emit("lunaris://menu-action", MenuActionPayload { app_id, action });
}

/// Get the current menu for a given app_id (used on initial load).
#[tauri::command]
pub fn get_menu(store: tauri::State<AppMenuStore>, app_id: String) -> Option<serde_json::Value> {
    store.lock().unwrap().get(&app_id).cloned()
}

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

/// Payload emitted on `lunaris://menu-state-updated` when an item state
/// changes via `shell.menu.setState`.
#[derive(Clone, Serialize)]
struct MenuStateUpdatedPayload {
    app_id: String,
    action: String,
    state: serde_json::Value,
}

/// Recursively find the item whose `action` field matches `action_id`
/// and merge `state` keys (`enabled`, `label`, `checked`) into it. The
/// menu tree is untyped JSON so the walk is dynamic; this matches how
/// `register_menu` already accepts arbitrary JSON.
///
/// Returns `true` when a matching item was found and updated. Items
/// without an `action` field (separators, type:"recent" slots) are
/// skipped without consuming the search.
fn merge_state_into_items(items: &mut serde_json::Value, action_id: &str, state: &serde_json::Value) -> bool {
    let Some(arr) = items.as_array_mut() else {
        return false;
    };
    for item in arr.iter_mut() {
        if let Some(obj) = item.as_object_mut() {
            // Direct match: this item carries the `action` field equal
            // to `action_id`.
            if obj.get("action").and_then(|v| v.as_str()) == Some(action_id) {
                if let Some(state_obj) = state.as_object() {
                    for (k, v) in state_obj.iter() {
                        obj.insert(k.clone(), v.clone());
                    }
                }
                return true;
            }
            // Recurse into children if present.
            if let Some(children) = obj.get_mut("children") {
                if merge_state_into_items(children, action_id, state) {
                    return true;
                }
            }
        }
    }
    false
}

/// Update a single item's state in an app's menu by action identifier.
///
/// Implements `shell.menu.setState(action, state)` per foundation §776.
/// Apps push runtime updates without re-registering the full menu tree:
/// `enabled` toggles, `label` reflects the most-recent action (Undo Typing),
/// `checked` reflects toggle/radio state.
///
/// Items not found are silently ignored — the spec does not require an
/// error path for stale references, and surfacing one would force callers
/// to coordinate menu lifecycle with state pushes that they don't always
/// own (e.g. a background task pushing `setState` after the menu was
/// already replaced).
#[tauri::command]
pub fn set_menu_state(
    app: AppHandle,
    store: tauri::State<AppMenuStore>,
    app_id: String,
    action: String,
    state: serde_json::Value,
) {
    let updated = {
        let mut guard = store.lock().unwrap();
        if let Some(items) = guard.get_mut(&app_id) {
            merge_state_into_items(items, &action, &state)
        } else {
            false
        }
    };

    if !updated {
        log::debug!(
            "set_menu_state: action {action} not found in {app_id} menu (or app not registered)"
        );
        return;
    }

    // Re-emit the registered menu so subscribers see the updated tree.
    let items = store.lock().unwrap().get(&app_id).cloned();
    if let Some(items) = items {
        let _ = app.emit(
            "lunaris://menu-registered",
            MenuRegisteredPayload {
                app_id: app_id.clone(),
                items,
            },
        );
    }

    // Also fire a targeted event so listeners that only care about state
    // deltas can avoid re-rendering the whole menu.
    let _ = app.emit(
        "lunaris://menu-state-updated",
        MenuStateUpdatedPayload {
            app_id,
            action,
            state,
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn merge_state_top_level() {
        let mut items = json!([
            { "label": "File", "action": "file.save", "enabled": false },
            { "label": "Edit", "action": "edit.undo" },
        ]);
        let state = json!({ "enabled": true });
        let found = merge_state_into_items(&mut items, "file.save", &state);
        assert!(found);
        assert_eq!(items[0]["enabled"], json!(true));
        assert_eq!(items[1]["enabled"], json!(null));
    }

    #[test]
    fn merge_state_nested_children() {
        let mut items = json!([
            {
                "label": "View",
                "children": [
                    { "label": "Sidebar", "action": "view.sidebar", "checked": true },
                    { "label": "Toolbar", "action": "view.toolbar", "checked": false },
                ]
            }
        ]);
        let state = json!({ "checked": false });
        let found = merge_state_into_items(&mut items, "view.sidebar", &state);
        assert!(found);
        assert_eq!(items[0]["children"][0]["checked"], json!(false));
        assert_eq!(items[0]["children"][1]["checked"], json!(false));
    }

    #[test]
    fn merge_state_unknown_action_returns_false() {
        let mut items = json!([
            { "label": "File", "action": "file.save" },
        ]);
        let state = json!({ "enabled": false });
        assert!(!merge_state_into_items(&mut items, "nonexistent.action", &state));
    }

    #[test]
    fn merge_state_label_update() {
        // Foundation §776 example: Undo gets relabelled to reflect the
        // last action. setState with `label` overwrites the existing
        // label without re-registering the menu.
        let mut items = json!([
            { "label": "Undo", "action": "edit.undo" },
        ]);
        let state = json!({ "label": "Undo Typing", "enabled": true });
        merge_state_into_items(&mut items, "edit.undo", &state);
        assert_eq!(items[0]["label"], json!("Undo Typing"));
        assert_eq!(items[0]["enabled"], json!(true));
    }

    #[test]
    fn merge_state_skips_separators() {
        let mut items = json!([
            { "separator": true },
            { "label": "Save", "action": "file.save" },
        ]);
        let found = merge_state_into_items(&mut items, "file.save", &json!({"enabled": false}));
        assert!(found);
        assert_eq!(items[1]["enabled"], json!(false));
    }
}

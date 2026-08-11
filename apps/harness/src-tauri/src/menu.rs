// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

//! The harness's half of the global-menu contract.
//!
//! `apps/harness/src/lib/menu.ts` declares the menu and calls
//! `invoke("register_menu", …)`, and nothing answered: `register_menu` is a
//! command of the DESKTOP SHELL, and a Tauri command only resolves inside the
//! binary that registers it. The frontend's own comment says both calls "fail
//! silently and the menu simply isn't there", and names the missing piece a
//! coder seam. This is that seam.
//!
//! **The route is the Event Bus, not a cross-binary invoke.** `os_sdk::menu` is
//! the contract every other app already uses - the file manager publishes its
//! groups the same way - so the shell learns about this menu the same way it
//! learns about every other one. Reaching into the shell's command table would
//! have been a second mechanism for a job that has one.
//!
//! Two directions, and both were missing:
//!
//!   register   the app publishes `app.menu.registered` with its groups
//!   actions    the shell publishes `app.menu.action_invoked` on a click, and
//!              this re-emits it as the `arlen://menu-action` Tauri event the
//!              frontend already listens for
//!
//! Registering without the back-channel would have produced the worse half of
//! the bug: a menu that appears and does nothing.
//!
//! The frontend needs no change. Tauri maps its `appId` argument onto `app_id`,
//! and `MenuGroup`/`MenuItem` deserialize the item objects it already sends -
//! the shapes were designed against each other, they were simply never joined.

use os_sdk::menu::{Menu, MenuGroup};
use serde::Serialize;
use tauri::{AppHandle, Emitter};

/// The Tauri event the frontend listens on for a clicked menu item.
const MENU_ACTION_EVENT: &str = "arlen://menu-action";

/// The payload shape `menu.ts` destructures (`{ app_id, action }`).
#[derive(Debug, Clone, Serialize)]
struct MenuActionPayload {
    app_id: String,
    action: String,
}

/// The producer socket, resolved the same way every other app resolves it.
fn producer_socket() -> String {
    os_sdk::runtime::socket_path("ARLEN_PRODUCER_SOCKET", "event-bus-producer.sock")
        .to_string_lossy()
        .into_owned()
}

/// Publish the app's menu to the shell.
///
/// Best-effort by contract, not by accident: with no event bus running there is
/// no top bar to publish into either, so a failure here means the menu is absent
/// - which is exactly the state the frontend already handles. It is returned
/// rather than swallowed so the caller can log it; the frontend's `catch` keeps
/// the app running.
#[tauri::command]
pub async fn register_menu(app_id: String, items: Vec<MenuGroup>) -> Result<(), String> {
    let emitter = os_sdk::event::UnixEventEmitter::new(producer_socket());
    Menu::new(emitter, app_id)
        .register(items)
        .await
        .map_err(|e| format!("publishing the app menu: {e}"))
}

/// Relay clicked menu actions to the frontend.
///
/// The shell publishes the click onto the shared back-channel; the SDK filters
/// it to this app's id, and each action becomes the Tauri event the frontend was
/// already waiting for. Spawned once at setup and left running: dropping the
/// receiver unsubscribes, so the task owns it for the life of the app.
pub fn relay_menu_actions(app: AppHandle, app_id: String) {
    tauri::async_runtime::spawn(async move {
        let socket = os_sdk::runtime::socket_path("ARLEN_CONSUMER_SOCKET", "event-bus-consumer.sock");
        let consumer = os_sdk::event_consumer::UnixEventConsumer::new(socket.to_string_lossy().into_owned());
        let mut actions =
            match os_sdk::menu::subscribe_menu_actions(&consumer, app_id.clone()).await {
                Ok(rx) => rx,
                Err(e) => {
                    log::warn!("menu actions unavailable, the menu will not respond: {e}");
                    return;
                }
            };
        while let Some(action) = actions.recv().await {
            let payload = MenuActionPayload {
                app_id: app_id.clone(),
                action,
            };
            if let Err(e) = app.emit(MENU_ACTION_EVENT, payload) {
                log::warn!("could not deliver a menu action to the window: {e}");
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The frontend sends its menu as plain objects and this side takes
    /// `Vec<MenuGroup>`. If those shapes ever drift, the command stops accepting
    /// a menu that looks correct in the source - so the exact JSON `menu.ts`
    /// sends is the fixture.
    #[test]
    fn the_menu_the_frontend_sends_deserializes() {
        let json = serde_json::json!([{
            "label": "Chat",
            "items": [
                { "label": "New chat", "action": "chat.new", "shortcut": "Ctrl+N", "type": "item" },
                { "label": "", "action": "", "type": "separator" },
                { "label": "Import chat…", "action": "chat.import", "type": "item" }
            ]
        }]);
        let groups: Vec<MenuGroup> =
            serde_json::from_value(json).expect("the shapes were designed against each other");
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].label, "Chat");
        assert_eq!(groups[0].items.len(), 3);
        assert_eq!(groups[0].items[0].shortcut.as_deref(), Some("Ctrl+N"));
    }

    /// The payload is destructured as `{ app_id, action }` in `menu.ts`, so the
    /// field names are the contract rather than an implementation detail.
    #[test]
    fn the_action_payload_keeps_the_names_the_listener_reads() {
        let v = serde_json::to_value(MenuActionPayload {
            app_id: "dev.arlen.harness".into(),
            action: "chat.new".into(),
        })
        .expect("a plain struct serializes");
        assert_eq!(v["app_id"], "dev.arlen.harness");
        assert_eq!(v["action"], "chat.new");
    }
}

//! `shell.menu` — first-party app surface for publishing the app's
//! global menu into the topbar's GlobalMenuBar.
//!
//! Event Bus carrier: `app.menu.registered` (a JSON `{app_id, items}`
//! document) and `app.menu.unregistered` (`{app_id}`). The desktop-shell
//! consumes these (`event_bus.rs` → `arlen://menu-registered` /
//! `arlen://menu-unregistered`) and renders the focused app's menu.
//!
//! Unlike the toolbar surface (a flat, typed protobuf payload), a menu is
//! a recursive tree, so it is carried as a JSON document rather than a
//! protobuf message — no schema churn for the nested structure. The
//! cross-process carrier is the Event Bus, NOT an intra-process Tauri
//! command: an app and the shell are separate processes, so a Tauri
//! `invoke` could never reach the shell's menu store.

use std::future::Future;

use serde::{Deserialize, Serialize};

use crate::event::{EmitError, EventEmitter};
use crate::event_consumer::{EventConsumer, SubscribeError};

/// The kind of a [`MenuItem`]: a normal action, a separator, or a submenu
/// carrying `children`. Serializes to the lowercase tag the shell's menu
/// store expects (`"item"` / `"separator"` / `"submenu"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MenuItemKind {
    /// A clickable action item.
    Item,
    /// A non-interactive divider.
    Separator,
    /// A nested submenu (`children` populated).
    Submenu,
}

/// One entry in an app menu. Mirrors the desktop-shell `MenuItem` shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MenuItem {
    /// The visible label.
    pub label: String,
    /// The action id dispatched back to the app on click. Empty for
    /// separators and submenus.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub action: String,
    /// An optional shortcut hint shown right-aligned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shortcut: Option<String>,
    /// Whether the item is rendered disabled.
    #[serde(default, skip_serializing_if = "is_false")]
    pub disabled: bool,
    /// Whether the item shows a check mark.
    #[serde(default, skip_serializing_if = "is_false")]
    pub checked: bool,
    /// The item kind.
    #[serde(rename = "type")]
    pub kind: MenuItemKind,
    /// Submenu entries (only for [`MenuItemKind::Submenu`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<MenuItem>,
}

fn is_false(b: &bool) -> bool {
    !*b
}

impl MenuItem {
    /// A clickable action item.
    pub fn item(label: impl Into<String>, action: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            action: action.into(),
            shortcut: None,
            disabled: false,
            checked: false,
            kind: MenuItemKind::Item,
            children: Vec::new(),
        }
    }

    /// A separator line.
    pub fn separator() -> Self {
        Self {
            label: String::new(),
            action: String::new(),
            shortcut: None,
            disabled: false,
            checked: false,
            kind: MenuItemKind::Separator,
            children: Vec::new(),
        }
    }

    /// A submenu carrying `children`.
    pub fn submenu(label: impl Into<String>, children: Vec<MenuItem>) -> Self {
        Self {
            label: label.into(),
            action: String::new(),
            shortcut: None,
            disabled: false,
            checked: false,
            kind: MenuItemKind::Submenu,
            children,
        }
    }
}

/// A top-level menu group (e.g. "File", "Edit").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MenuGroup {
    /// The group label shown in the menu bar.
    pub label: String,
    /// The group's items.
    pub items: Vec<MenuItem>,
}

impl MenuGroup {
    /// A group with the given label and items.
    pub fn new(label: impl Into<String>, items: Vec<MenuItem>) -> Self {
        Self {
            label: label.into(),
            items,
        }
    }
}

/// A sanity bound on the number of top-level groups in a published menu.
pub const MAX_MENU_GROUPS: usize = 24;

/// Surface for the `shell.menu` API.
///
/// One menu per app process, bound to the app's id. The app re-publishes
/// its whole menu on any change ([`Menu::register`] replaces the previous
/// menu on the shell side).
pub struct Menu<E: EventEmitter> {
    emitter: E,
    app_id: String,
}

impl<E: EventEmitter> Menu<E> {
    /// Create a menu surface bound to an emitter and the app's id.
    pub fn new(emitter: E, app_id: impl Into<String>) -> Self {
        Self {
            emitter,
            app_id: app_id.into(),
        }
    }

    /// Publish (replace) this app's global menu. The shell renders it in
    /// the topbar whenever one of the app's windows is focused.
    ///
    /// # Errors
    /// [`EmitError::SerializationFailed`] if `groups.len() > MAX_MENU_GROUPS`
    /// or the payload cannot be serialized; otherwise the underlying
    /// emitter's error if the bus is unreachable.
    pub fn register(
        &self,
        groups: Vec<MenuGroup>,
    ) -> impl Future<Output = Result<(), EmitError>> + Send + '_ {
        async move {
            if groups.len() > MAX_MENU_GROUPS {
                return Err(EmitError::SerializationFailed(format!(
                    "shell.menu.register: max {MAX_MENU_GROUPS} groups, got {}",
                    groups.len()
                )));
            }
            let payload = serde_json::json!({ "app_id": self.app_id, "items": groups });
            let buf = serde_json::to_vec(&payload)
                .map_err(|e| EmitError::SerializationFailed(e.to_string()))?;
            self.emitter.emit("app.menu.registered", buf).await
        }
    }

    /// Remove this app's menu from the topbar.
    pub fn unregister(&self) -> impl Future<Output = Result<(), EmitError>> + Send + '_ {
        async move {
            let payload = serde_json::json!({ "app_id": self.app_id });
            let buf = serde_json::to_vec(&payload)
                .map_err(|e| EmitError::SerializationFailed(e.to_string()))?;
            self.emitter.emit("app.menu.unregistered", buf).await
        }
    }
}

/// The Event Bus type the shell publishes when a topbar menu item is
/// clicked, carrying the action back to the app that published the menu.
const MENU_ACTION_INVOKED: &str = "app.menu.action_invoked";

/// Subscribe to the topbar menu-action return channel for `app_id`.
///
/// [`Menu::register`] is publish-only: it pushes the menu tree into the
/// topbar. When the user clicks one of those items the shell publishes
/// [`MENU_ACTION_INVOKED`] (`{app_id, action}`) back onto the Event Bus.
/// This subscribes through `consumer`, keeps only the actions addressed
/// to `app_id` (the channel is a shared back-channel for every app), and
/// yields their action ids on the returned receiver. Drop the receiver
/// to unsubscribe — the forwarder task ends when the channel closes.
pub async fn subscribe_menu_actions<C: EventConsumer>(
    consumer: &C,
    app_id: impl Into<String>,
) -> Result<tokio::sync::mpsc::Receiver<String>, SubscribeError> {
    use prost::Message as _;

    let app_id = app_id.into();
    let mut events = consumer
        .subscribe(vec![MENU_ACTION_INVOKED.to_string()])
        .await?;
    let (tx, rx) = tokio::sync::mpsc::channel::<String>(64);
    tokio::spawn(async move {
        while let Some(event) = events.recv().await {
            let Ok(payload) =
                crate::proto::ShortcutActionInvokedPayload::decode(event.payload.as_slice())
            else {
                continue;
            };
            if payload.app_id == app_id && tx.send(payload.action).await.is_err() {
                break;
            }
        }
    });
    Ok(rx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::MockEventEmitter;

    #[tokio::test]
    async fn register_emits_app_id_keyed_json_menu() {
        let emitter = MockEventEmitter::new();
        let menu = Menu::new(emitter.clone(), "org.example.app");
        menu.register(vec![MenuGroup::new(
            "File",
            vec![
                MenuItem::item("Open", "file.open"),
                MenuItem::separator(),
                MenuItem::submenu("Recent", vec![MenuItem::item("a.txt", "file.recent.0")]),
            ],
        )])
        .await
        .unwrap();

        let events = emitter.emitted().await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "app.menu.registered");
        let v: serde_json::Value = serde_json::from_slice(&events[0].payload).unwrap();
        assert_eq!(v["app_id"], "org.example.app");
        assert_eq!(v["items"][0]["label"], "File");
        assert_eq!(v["items"][0]["items"][0]["type"], "item");
        assert_eq!(v["items"][0]["items"][0]["action"], "file.open");
        assert_eq!(v["items"][0]["items"][1]["type"], "separator");
        assert_eq!(v["items"][0]["items"][2]["type"], "submenu");
        assert_eq!(v["items"][0]["items"][2]["children"][0]["action"], "file.recent.0");
    }

    #[tokio::test]
    async fn unregister_emits_app_id_only() {
        let emitter = MockEventEmitter::new();
        let menu = Menu::new(emitter.clone(), "org.example.app");
        menu.unregister().await.unwrap();
        let events = emitter.emitted().await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "app.menu.unregistered");
        let v: serde_json::Value = serde_json::from_slice(&events[0].payload).unwrap();
        assert_eq!(v["app_id"], "org.example.app");
        assert!(v.get("items").is_none());
    }

    #[tokio::test]
    async fn too_many_groups_is_rejected() {
        let emitter = MockEventEmitter::new();
        let menu = Menu::new(emitter.clone(), "org.example.app");
        let groups: Vec<MenuGroup> = (0..MAX_MENU_GROUPS + 1)
            .map(|i| MenuGroup::new(format!("g{i}"), vec![]))
            .collect();
        assert!(menu.register(groups).await.is_err());
        assert_eq!(emitter.emit_count().await, 0);
    }

    #[tokio::test]
    async fn subscribe_menu_actions_yields_only_this_apps_actions() {
        use crate::mock::MockEventConsumer;
        use prost::Message as _;

        let bus = MockEventConsumer::new();
        let mut actions = subscribe_menu_actions(&bus, "dev.arlen.files")
            .await
            .unwrap();
        // Let the forwarder task attach before pushing.
        tokio::task::yield_now().await;

        let foreign = crate::proto::ShortcutActionInvokedPayload {
            app_id: "dev.arlen.terminal".to_string(),
            action: "edit.copy".to_string(),
            window_id: String::new(),
        };
        let mine = crate::proto::ShortcutActionInvokedPayload {
            app_id: "dev.arlen.files".to_string(),
            action: "file.new_folder".to_string(),
            window_id: String::new(),
        };
        bus.push(crate::proto::Event {
            r#type: MENU_ACTION_INVOKED.to_string(),
            payload: foreign.encode_to_vec(),
            ..Default::default()
        });
        bus.push(crate::proto::Event {
            r#type: MENU_ACTION_INVOKED.to_string(),
            payload: mine.encode_to_vec(),
            ..Default::default()
        });

        // The foreign app's action is filtered out; only ours arrives.
        let got = tokio::time::timeout(std::time::Duration::from_secs(1), actions.recv())
            .await
            .expect("menu action did not arrive")
            .expect("channel closed");
        assert_eq!(got, "file.new_folder");
    }
}

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
}
